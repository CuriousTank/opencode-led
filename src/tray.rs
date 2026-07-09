//! System tray icon for opencode-traffic-light.
//!
//! Shows an aggregate status (red/yellow/green) in the system tray,
//! with a context menu to show/hide the widget, open settings, or quit.
//!
//! Linux: uses KDE StatusNotifierItem / AppIndicator (via the `tray-icon` crate).

use crate::store::LightState;
use image::{ImageBuffer, Rgba, RgbaImage};
use include_dir::{include_dir, Dir};
use parking_lot::Mutex;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Commands sent from the tray menu back to the main app loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    /// Toggle the floating widget visibility.
    ToggleVisible,
    /// Open the "Customize Icons" settings panel.
    OpenSettings,
    /// Quit the whole application.
    Quit,
}

/// Static assets embedded at compile time (re-declared here to load raw PNG bytes
/// for the tray icon, since egui textures can't be reused for the tray).
static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");

/// Fixed menu item ids.
const ID_TOGGLE: &str = "tl_toggle";
const ID_SETTINGS: &str = "tl_settings";
const ID_QUIT: &str = "tl_quit";

/// Aggregate the per-session states into a single tray light.
/// Priority: Running > Input > Done. Empty → Done (idle green).
pub fn aggregate_state(snap: &[crate::store::SessionEntry]) -> LightState {
    let mut has_running = false;
    let mut has_input = false;
    for e in snap {
        match e.state {
            LightState::Running => has_running = true,
            LightState::Input => has_input = true,
            LightState::Done => {}
        }
    }
    if has_running {
        LightState::Running
    } else if has_input {
        LightState::Input
    } else {
        LightState::Done
    }
}

/// Load a raw PNG from the embedded assets dir and build a tray Icon.
#[allow(dead_code)]
fn load_icon(state: LightState) -> Option<Icon> {
    let name = match state {
        LightState::Running => "red.png",
        LightState::Input => "yellow.png",
        LightState::Done => "green.png",
    };
    let file = ASSETS.get_file(name)?;
    let bytes = file.contents();
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            Icon::from_rgba(rgba.into_raw(), w, h).ok()
        }
        Err(_) => None,
    }
}

// ── 数字点阵 (5×7 像素，用于角标渲染，避免引入字体依赖) ──
// 1 = 亮(白)，0 = 暗(透明)
const DIGIT_W: u32 = 5;
const DIGIT_H: u32 = 7;
#[rustfmt::skip]
const DIGITS: [[[u8; 5]; 7]; 10] = [
    // 0
    [[0,1,1,1,0],[1,0,0,0,1],[1,0,0,1,1],[1,0,1,0,1],[1,1,0,0,1],[1,0,0,0,1],[0,1,1,1,0]],
    // 1
    [[0,0,1,0,0],[0,1,1,0,0],[1,0,1,0,0],[0,0,1,0,0],[0,0,1,0,0],[0,0,1,0,0],[1,1,1,1,1]],
    // 2
    [[0,1,1,1,0],[1,0,0,0,1],[0,0,0,0,1],[0,0,0,1,0],[0,0,1,0,0],[0,1,0,0,0],[1,1,1,1,1]],
    // 3
    [[1,1,1,1,0],[0,0,0,0,1],[0,0,0,0,1],[0,1,1,1,0],[0,0,0,0,1],[0,0,0,0,1],[1,1,1,1,0]],
    // 4
    [[0,0,0,1,0],[0,0,1,1,0],[0,1,0,1,0],[1,0,0,1,0],[1,1,1,1,1],[0,0,0,1,0],[0,0,0,1,0]],
    // 5
    [[1,1,1,1,1],[1,0,0,0,0],[1,1,1,1,0],[0,0,0,0,1],[0,0,0,0,1],[1,0,0,0,1],[0,1,1,1,0]],
    // 6
    [[0,0,1,1,0],[0,1,0,0,0],[1,0,0,0,0],[1,1,1,1,0],[1,0,0,0,1],[1,0,0,0,1],[0,1,1,1,0]],
    // 7
    [[1,1,1,1,1],[0,0,0,0,1],[0,0,0,1,0],[0,0,1,0,0],[0,1,0,0,0],[0,1,0,0,0],[0,1,0,0,0]],
    // 8
    [[0,1,1,1,0],[1,0,0,0,1],[1,0,0,0,1],[0,1,1,1,0],[1,0,0,0,1],[1,0,0,0,1],[0,1,1,1,0]],
    // 9
    [[0,1,1,1,0],[1,0,0,0,1],[1,0,0,0,1],[0,1,1,1,1],[0,0,0,0,1],[0,0,0,1,0],[0,1,1,0,0]],
];

/// 在 base 图标右下角叠加数字角标，返回合成后的 Icon。
/// - count <= 1 时不画角标，直接返回纯 base 图标
/// - count >= 10 时显示 "9+"
/// 徽章：深色实心圆 + 白色点阵数字（放大到贴合圆）+ 白色描边（在红/黄/绿底色上都清晰）
fn render_badge(state: LightState, count: usize) -> Option<Icon> {
    // base PNG
    let name = match state {
        LightState::Running => "red.png",
        LightState::Input => "yellow.png",
        LightState::Done => "green.png",
    };
    let bytes = ASSETS.get_file(name)?.contents();
    let base: RgbaImage = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = base.dimensions();

    // count <= 1：无角标
    if count <= 1 {
        return Icon::from_rgba(base.into_raw(), w, h).ok();
    }

    // 决定显示的数字字符（10+ 显示 "9+"）
    let chars: Vec<usize> = if count >= 10 {
        vec![9] // 后面单独画 "+"
    } else {
        vec![count]
    };
    let has_plus = count >= 10;

    let mut buf: ImageBuffer<Rgba<u8>, Vec<u8>> = base;

    // 徽章圆参数（图标边长的 45%）
    let badge_d = ((w.min(h) as f32) * 0.62).round() as i32; // 直径
    let badge_r = badge_d / 2;
    // 右下角，留 1px 边距
    let cx = (w as i32) - badge_r - 1;
    let cy = (h as i32) - badge_r - 1;

    // 颜色：徽章用深色背景（而非深红），保证在红/黄/绿三种聚合色上都有清晰边界
    let badge_fill = Rgba([0x1A, 0x1A, 0x1A, 0xFF]); // 近黑深灰
    let badge_edge = Rgba([0xFF, 0xFF, 0xFF, 0xCC]); // 白色描边（半透明），在任何底色上勾勒轮廓
    let digit_color = Rgba([0xFF, 0xFF, 0xFF, 0xFF]); // 白

    // 先画白色描边圆（大一圈），在任何聚合底色上勾勒徽章轮廓
    for y in (cy - badge_r - 1)..=(cy + badge_r + 1) {
        for x in (cx - badge_r - 1)..=(cx + badge_r + 1) {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= (badge_r + 1) * (badge_r + 1) && d2 > badge_r * badge_r {
                if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                    buf.put_pixel(x as u32, y as u32, badge_edge);
                }
            }
        }
    }
    // 画深色实心圆
    for y in (cy - badge_r)..=(cy + badge_r) {
        for x in (cx - badge_r)..=(cx + badge_r) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= badge_r * badge_r {
                if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                    buf.put_pixel(x as u32, y as u32, badge_fill);
                }
            }
        }
    }

    // 画数字（放大到贴合徽章圆，1点 = SCALE×SCALE 方块）
    // 1位数用 4× 放大（20×28px），2位数(9+)用 3× 放大（15×21px）以保证仍能装进圆
    let scale: i32 = if chars.len() >= 2 || has_plus { 3 } else { 4 };
    let glyph_w = DIGIT_W as i32;
    let glyph_h = DIGIT_H as i32;
    let glyph_step = glyph_w * scale + scale; // 字符宽 + 间隔
    let plus_w = 3 * scale; // "+" 的宽度
    let plus_step = plus_w + scale;
    // 总宽度
    let mut total_glyph_w: i32 = chars.len() as i32 * glyph_step;
    if has_plus {
        total_glyph_w += plus_step;
    }
    total_glyph_w -= scale; // 末尾不留间隔

    // 整体居中于徽章圆，垂直略上偏 1px 视觉居中
    let glyph_total_h = glyph_h * scale;
    let start_x = cx - total_glyph_w / 2;
    let start_y = cy - glyph_total_h / 2 - 1;

    // 辅助：画一个 scale×scale 的实心方块（带边界检查）
    let put_block = |buf: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
                     px0: i32,
                     py0: i32,
                     color: Rgba<u8>,
                     w: u32,
                     h: u32| {
        for dy in 0..scale {
            for dx in 0..scale {
                let px = px0 + dx;
                let py = py0 + dy;
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    buf.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    };

    // 逐字符绘制（放大方块）
    let mut cur_x = start_x;
    for &d in &chars {
        let bitmap = &DIGITS[d];
        for (ry, row) in bitmap.iter().enumerate() {
            for (rx, &on) in row.iter().enumerate() {
                if on == 1 {
                    let px = cur_x + rx as i32 * scale;
                    let py = start_y + ry as i32 * scale;
                    put_block(&mut buf, px, py, digit_color, w, h);
                }
            }
        }
        cur_x += glyph_step;
    }
    // 画 "+"（放大方块）
    if has_plus {
        let px0 = cur_x;
        let py_center = start_y + (glyph_h * scale) / 2 - scale / 2;
        // 横（3 个方块）
        for i in 0..3 {
            put_block(&mut buf, px0 + i * scale, py_center, digit_color, w, h);
        }
        // 竖（3 个方块，中间对齐）
        let px_col = px0 + scale; // 中间列
        for i in 0..3 {
            put_block(&mut buf, px_col, py_center - scale + i * scale, digit_color, w, h);
        }
    }

    Icon::from_rgba(buf.into_raw(), w, h).ok()
}

/// Manages the system tray icon lifecycle.
pub struct Tray {
    icon: Arc<Mutex<Option<TrayIcon>>>,
    /// Handle to the "Show/Hide" menu item, so we can relabel it live.
    toggle_item: Option<MenuItem>,
    /// Receiver for menu events (polled from the egui loop).
    rx: Receiver<TrayCmd>,
    /// Cached last (state, count) to avoid needless icon rebuilds.
    last: Mutex<Option<(LightState, usize)>>,
}

impl Tray {
    /// Create the tray icon and wire up the menu.
    /// Call this from the main thread *before* `eframe::run_native`.
    /// Always returns an `Arc<Tray>` (no-op stub if the platform tray is unavailable),
    /// so callers don't have to branch on `Option`.
    pub fn new(initial: LightState) -> Arc<Self> {
        // On Linux the tray backend (appindicator + muda) sits on top of GTK,
        // which must be initialised on the main thread *before* any GTK object
        // (Menu / MenuItem / TrayIcon) is constructed — otherwise muda panics
        // with "GTK has not been initialized". eframe doesn't init GTK for us.
        #[cfg(target_os = "linux")]
        {
            // gtk::init() loads display/GDK properly; ignore failures so the app
            // still runs (without a tray) on headless / weird setups.
            if let Err(e) = gtk::init() {
                eprintln!("[traffic-light] gtk::init failed (tray disabled): {e}");
                return Arc::new(Self::stub());
            }
        }

        // Build the context menu.
        let menu = Menu::new();
        let item_toggle = MenuItem::with_id(ID_TOGGLE, "Hide widget", true, None);
        let item_settings = MenuItem::with_id(ID_SETTINGS, "Customize Icons...", true, None);
        let separator = PredefinedMenuItem::separator();
        let item_quit = MenuItem::with_id(ID_QUIT, "Quit", true, None);

        let _ = menu.append(&item_toggle);
        let _ = menu.append(&item_settings);
        let _ = menu.append(&separator);
        let _ = menu.append(&item_quit);

        // 初始图标：count=0 → 纯灯泡（无角标），真实状态由第一帧 set_state 同步
        let icon = render_badge(initial, 0);
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(format!(
                "opencode traffic light — {}",
                initial.label()
            ));
        if let Some(ic) = icon {
            builder = builder.with_icon(ic);
        }

        let tray_icon = match builder.build() {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("[traffic-light] failed to create tray icon: {e}");
                None
            }
        };

        // Menu event channel: a background listener forwards matching ids to TrayCmd.
        let (tx, rx): (Sender<TrayCmd>, Receiver<TrayCmd>) = channel();
        let menu_rx = MenuEvent::receiver().clone();
        std::thread::spawn(move || {
            while let Ok(ev) = menu_rx.recv() {
                let cmd = match ev.id.as_ref() {
                    ID_TOGGLE => Some(TrayCmd::ToggleVisible),
                    ID_SETTINGS => Some(TrayCmd::OpenSettings),
                    ID_QUIT => Some(TrayCmd::Quit),
                    _ => None,
                };
                if let Some(c) = cmd {
                    let _ = tx.send(c);
                }
            }
        });

        let toggle_handle = if tray_icon.is_some() {
            Some(item_toggle)
        } else {
            None
        };

        Arc::new(Self {
            icon: Arc::new(Mutex::new(tray_icon)),
            toggle_item: toggle_handle,
            rx,
            last: Mutex::new(None),
        })
    }

    /// Build a no-op stub (no real tray icon). Used when GTK init fails or the
    /// platform doesn't support a tray — the rest of the app still works,
    /// only without a tray indicator.
    fn stub() -> Self {
        let (_tx, rx): (Sender<TrayCmd>, Receiver<TrayCmd>) = channel();
        Self {
            icon: Arc::new(Mutex::new(None)),
            toggle_item: None,
            rx,
            last: Mutex::new(None),
        }
    }

    /// Pump pending GTK events without blocking.
    ///
    /// tray-icon's Linux backend (appindicator + muda) is built on GTK, which
    /// expects its own main loop (`gtk::main()`) to keep running so the
    /// AppIndicator can render its icon and so menu clicks get dispatched.
    /// But eframe blocks on winit's event loop and never runs `gtk::main()`,
    /// so without us manually pumping GTK events each frame the tray icon
    /// simply never appears and the menu is dead.
    ///
    /// Call this every frame from the egui `update()` loop.
    pub fn pump_events(&self) {
        #[cfg(target_os = "linux")]
        {
            // gtk events_pending() / main_iteration_do(false) are non-blocking.
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
        }
    }

    /// Update the tray icon (with badge count) + tooltip when state/count changes.
    ///
    /// - count <= 1: 显示纯聚合色灯泡（无角标）
    /// - count >= 2: 右下角叠加深红圆 + 白色数字
    /// - count >= 10: 显示 "9+"
    pub fn set_state(&self, state: LightState, count: usize) {
        let mut last = self.last.lock();
        if *last == Some((state, count)) {
            return;
        }
        *last = Some((state, count));
        drop(last);

        let mut tray = self.icon.lock();
        let Some(tray) = tray.as_mut() else {
            return;
        };
        if let Some(ic) = render_badge(state, count) {
            let _ = tray.set_icon(Some(ic));
        }
        // tooltip 带会话数量
        let tip = if count <= 1 {
            format!("opencode traffic light — {}", state.label())
        } else {
            format!("opencode traffic light — {} ({} sessions)", state.label(), count)
        };
        let _ = tray.set_tooltip(Some(tip));
    }

    /// Poll for pending tray commands. Call this every frame from the egui loop.
    pub fn poll(&self) -> Option<TrayCmd> {
        self.rx.try_recv().ok()
    }

    /// Update the "Show/Hide" menu label depending on widget visibility.
    pub fn set_visible_label(&self, widget_visible: bool) {
        let label = if widget_visible { "Hide widget" } else { "Show widget" };
        if let Some(item) = &self.toggle_item {
            item.set_text(label);
        }
    }
}
