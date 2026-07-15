use egui::{Color32, StrokeKind, Vec2, ViewportBuilder};
use eframe::egui;
use include_dir::{include_dir, Dir};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod config;
mod icons;
mod platform;
mod server;
mod store;
mod tray;
mod wm;

use icons::Icons;
use store::{LightState, SessionEntry, Store};
use tray::{Tray, TrayCmd};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");

const DEFAULT_PORT: u16 = 9912;
const BASE_ICON_PX: f32 = 64.0; // Medium 基准尺寸，实际尺寸 = BASE_ICON_PX × icon_size_factor
const GAP_PX: f32 = 6.0;
const PAD_PX: f32 = 2.0;
const TOOLTIP_ROOM: f32 = 100.0; // 给灯泡上方的 tooltip 预留渲染空间（XShape 只覆盖灯泡区域，不影响穿透）
const WIN_W_MIN: f32 = 240.0; // 最小透明边距，让窗口几乎贴合灯泡，减少对其他窗口的阻挡
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const SESSION_TIMEOUT: Duration = Duration::from_secs(12);
const SETTINGS_PANEL_H: f32 = 560.0; // 设置面板高度（标题+3图标卡片+尺寸卡片+底部提示）
const SETTINGS_PANEL_W: f32 = 420.0; // 设置面板宽度

fn main() -> eframe::Result<()> {
    let port = std::env::var("OPENCODE_TL_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let store = Store::new();
    let icons = Icons::new();
    let dirty = Arc::new(Mutex::new(false));
    let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));

    // server 线程：收到推送时置 dirty 并请求重绘
    let dirty_for_server = dirty.clone();
    let ctx_for_server = ctx_slot.clone();
    let on_change = Box::new(move || {
        *dirty_for_server.lock() = true;
        if let Some(ctx) = ctx_for_server.lock().as_ref() {
            ctx.request_repaint();
        }
    });
    server::spawn(store.clone(), port, on_change);

    eprintln!(
        "[traffic-light] listening on http://127.0.0.1:{}/status (plugin pushes here)",
        port
    );
    eprintln!("[traffic-light] right-click the window to quit.");

    // 系统托盘图标：显示聚合状态 + 菜单（显示/隐藏、自定义图标、退出）
    let tray = Tray::new(LightState::Done);

    // 读取图标尺寸偏好（启动时即应用，避免首帧闪烁）
    let icon_size = config::load_size();
    let icon_size_factor = icon_size.factor();
    let init_icon_px = BASE_ICON_PX * icon_size_factor;

    let viewport = ViewportBuilder::default()
        .with_title("opencode traffic light")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(true)
        .with_always_on_top()
        .with_position([100.0, 100.0])
        .with_inner_size(Vec2::new(
            WIN_W_MIN,
            init_icon_px + PAD_PX * 2.0 + TOOLTIP_ROOM,
        ));

    let native_opts = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "opencode-traffic-light",
        native_opts,
        Box::new(move |cc| {
            // 把 egui ctx 存起来，供 server 线程触发重绘
            *ctx_slot.lock() = Some(cc.egui_ctx.clone());

            // 加载 CJK 字体，让中文正常显示
            let mut fonts = egui::FontDefinitions::default();
            for path in [
                "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            ] {
                if let Ok(data) = std::fs::read(path) {
                    fonts.font_data.insert(
                        "cjk".to_owned(),
                        egui::FontData::from_owned(data).into(),
                    );
                    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                        fam.push("cjk".to_owned());
                    }
                    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                        fam.push("cjk".to_owned());
                    }
                    break;
                }
            }
            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(App {
                store,
                icons,
                dirty,
                last_count: 0,
                last_above_time: 0.0,
                last_sweep: Instant::now(),
                drag: None,
                last_bulb_rects: Vec::new(),
                settings_mode: false,
                settings_rects: Vec::new(),
                last_settings_mode: false,
                menu_open: false,
                menu_pos: egui::pos2(0.0, 0.0),
                file_dialog_state: None,
                format_error: None,
                tray,
                widget_visible: true,
                last_tray: None,
                icon_size_factor,
                last_icon_factor: icon_size_factor,
                override_redirect_set: false,
            }))
        }),
    )
}

struct App {
    store: Arc<Store>,
    icons: Arc<Icons>,
    dirty: Arc<Mutex<bool>>,
    last_count: usize,
    last_above_time: f64,
    last_sweep: Instant,
    drag: Option<DragState>,
    last_bulb_rects: Vec<egui::Rect>,
    settings_mode: bool,
    /// 每张卡片的拖拽区 rect，用于拖拽命中检测
    settings_rects: Vec<(LightState, egui::Rect)>,
    /// 上一帧是否处于设置模式（检测切换时调整窗口大小）
    last_settings_mode: bool,
    /// 右键菜单是否打开
    menu_open: bool,
    /// 菜单打开时捕获的位置（固定不随光标移动）
    menu_pos: egui::Pos2,
    /// 文件选择对话框结果（异步回调）
    file_dialog_state: Option<(LightState, std::sync::mpsc::Receiver<Option<std::path::PathBuf>>)>,
    /// 格式校验错误提示
    format_error: Option<(Instant, String)>,
    /// 系统托盘
    tray: Arc<Tray>,
    /// 浮窗是否可见（托盘菜单可切换）
    widget_visible: bool,
    /// 上次同步给托盘的 (聚合状态, 会话数)（避免每帧刷新）
    last_tray: Option<(LightState, usize)>,
    /// 图标尺寸倍数（0.75/1.0/1.25），启动时从 config 读取，设置面板可改
    icon_size_factor: f32,
    /// 上一帧的图标尺寸倍数（检测变化时触发窗口 resize）
    last_icon_factor: f32,
    /// 是否已设置 override_redirect（一次性）
    override_redirect_set: bool,
}

/// 拖拽中：保持鼠标相对窗口左上角的偏移恒定
struct DragState {
    handles: platform::XHandles,
    /// 鼠标根坐标 - 窗口左上角根坐标（拖拽锚点）
    offset: egui::Vec2,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.icons.ensure_loaded(ui.ctx(), &ASSETS);

        // 消费 dirty 标志（这里只是触发重绘，快照在下面直接读）
        let _ = self.dirty.lock();

        // 确保定期重绘：让 sweep 能定期执行 + 托盘 GTK 事件能被及时 pump
        // （eframe 空闲时不调用 ui()，必须主动 request_repaint）
        // 200ms：兼顾托盘菜单点击的响应感与 CPU 占用
        ui.ctx().request_repaint_after(Duration::from_millis(200));

        // 系统托盘：pump GTK 事件，让 appindicator 图标和菜单正常工作
        // （eframe/winit 不跑 gtk::main()，必须手动 pump）
        self.tray.pump_events();

        // 定期清理过期的 session（心跳超时 = opencode 进程已退出）
        if self.last_sweep.elapsed() >= SWEEP_INTERVAL {
            self.last_sweep = Instant::now();
            if self.store.sweep(SESSION_TIMEOUT) {
                *self.dirty.lock() = true;
            }
        }

        // ── 系统托盘：处理菜单命令 + 同步聚合状态 ──
        while let Some(cmd) = self.tray.poll() {
            match cmd {
                TrayCmd::ToggleVisible => {
                    self.widget_visible = !self.widget_visible;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(self.widget_visible));
                    self.tray.set_visible_label(self.widget_visible);
                }
                TrayCmd::OpenSettings => {
                    self.settings_mode = true;
                    if !self.widget_visible {
                        self.widget_visible = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        self.tray.set_visible_label(true);
                    }
                }
                TrayCmd::Quit => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        let snap = self.store.snapshot();
        let count = snap.len().max(1);

        // 同步聚合状态 + 会话数量到托盘图标（仅在变化时刷新）
        let agg = tray::aggregate_state(&snap);
        if self.last_tray != Some((agg, snap.len())) {
            self.last_tray = Some((agg, snap.len()));
            self.tray.set_state(agg, snap.len());
        }

        // 设置模式下用浅色面板背景（不透明），正常模式下透明
        let mut visuals = egui::Visuals::dark();
        if self.settings_mode {
            visuals.panel_fill = Color32::from_rgb(0xF7, 0xF8, 0xFA);
            visuals.window_fill = Color32::from_rgb(0xF7, 0xF8, 0xFA);
        } else {
            visuals.panel_fill = Color32::TRANSPARENT;
            visuals.window_fill = Color32::TRANSPARENT;
        }
        visuals.extreme_bg_color = Color32::TRANSPARENT;
        ui.ctx().set_visuals(visuals);

        let win_rect = ui.max_rect();
        let icon_px = BASE_ICON_PX * self.icon_size_factor; // 当前图标尺寸（动态）

        // ── 设置面板（上方区域） ──
        if self.settings_mode {
            let panel_rect = egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(win_rect.width(), win_rect.height() - icon_px - PAD_PX * 2.0),
            );
            self.render_settings(ui, frame, panel_rect);
        }

        // ── 灯泡行定位：窗口底部居中 ──
        let bulbs_w =
            count as f32 * icon_px + count.saturating_sub(1) as f32 * GAP_PX;
        let bulbs_rect = egui::Rect::from_min_size(
            egui::pos2(
                win_rect.center().x - bulbs_w / 2.0,
                win_rect.bottom() - PAD_PX - icon_px,
            ),
            egui::vec2(bulbs_w.max(icon_px), icon_px),
        );

        // 在灯泡行位置创建子 UI
        let mut bulb_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bulbs_rect)
                .id_salt("bulbs")
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        bulb_ui.spacing_mut().item_spacing.x = GAP_PX;

        let mut bulb_responses: Vec<egui::Response> = Vec::new();

        if snap.is_empty() {
            let tex = self.icons.get(&LightState::Done).expect("icon");
            let tint = Color32::WHITE.linear_multiply(if self.settings_mode { 0.5 } else { 0.15 });
            let (rect, resp) =
                bulb_ui.allocate_exact_size(Vec2::splat(icon_px), egui::Sense::hover());
            bulb_ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                tint,
            );
            bulb_responses.push(resp);
        } else {
            for e in &snap {
                let resp = self.render_bulb(&mut bulb_ui, e, icon_px);
                bulb_responses.push(resp);
            }
        }

        // 逐灯泡拖拽检测（设置模式下不拖拽）
        if !self.settings_mode {
            for resp in &bulb_responses {
                if resp.drag_started() && self.drag.is_none() {
                    if let Some(h) = platform::extract_handles(frame) {
                        if let Some((_mx, _my, wx, wy)) = platform::query_pointer(h) {
                            self.drag = Some(DragState {
                                handles: h,
                                offset: egui::vec2(wx as f32, wy as f32),
                            });
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        }
                    }
                    break;
                }
            }
        }

        // 定期重申 _NET_WM_STATE_ABOVE
        let now = ui.ctx().input(|i| i.time);
        if now - self.last_above_time > 1.0 {
            if let Some(h) = platform::extract_handles(frame) {
                platform::set_above(h);
            }
            self.last_above_time = now;
        }

        // 拖拽中：每帧用鼠标根坐标 - 偏移 = 窗口目标位置
        if let Some(d) = &self.drag {
            if let Some((mx, my)) = platform::query_pointer_root(d.handles) {
                let nx = (mx as f32 - d.offset.x) as i32;
                let ny = (my as f32 - d.offset.y) as i32;
                platform::move_window(d.handles, nx, ny);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            ui.ctx().request_repaint();
        }

        // 鼠标松开：结束拖拽
        let primary_down = ui.input(|i| i.pointer.primary_down());
        if self.drag.is_some() && !primary_down {
            self.drag = None;
        }

        // ── 右键：捕获位置 + 打开菜单 ──
        if ui.input(|i| i.pointer.secondary_pressed()) {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                self.menu_pos = pos;
            }
            self.menu_open = true;
        }

        // ── 菜单渲染（固定位置）──
        if self.menu_open {
            let mut close_menu = false;

            egui::Area::new(egui::Id::new("context_menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(self.menu_pos)
                .interactable(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame {
                        fill: Color32::from_rgb(250, 250, 252),
                        stroke: egui::Stroke::new(1.0, Color32::from_black_alpha(20)),
                        corner_radius: 8.0.into(),
                        inner_margin: egui::Margin::same(4),
                        ..Default::default()
                    }
                    .show(ui, |ui| {
                        ui.set_min_width(140.0);
                        let menu_label = if self.settings_mode {
                            "✓ Customize Icons"
                        } else {
                            "⚙  Customize Icons"
                        };
                        let menu_resp = ui.add(
                            egui::Button::new(
                                egui::RichText::new(menu_label)
                                    .size(13.0)
                                    .color(Color32::from_rgb(30, 30, 35)),
                            )
                            .fill(Color32::TRANSPARENT),
                        );
                        if menu_resp.clicked() {
                            self.settings_mode = !self.settings_mode;
                            close_menu = true;
                        }
                        ui.separator();
                        let quit_resp = ui.add(
                            egui::Button::new(
                                egui::RichText::new("❌ Quit")
                                    .size(13.0)
                                    .color(Color32::from_rgb(30, 30, 35)),
                            )
                            .fill(Color32::TRANSPARENT),
                        );
                        if quit_resp.clicked() {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });

            // 获取菜单 Area 的 rect（当前帧已渲染，可读到）
            let menu_rect = egui::AreaState::load(ui.ctx(), egui::Id::new("context_menu"))
                .map(|s| s.rect());

            // 自动关闭：检测到点击（左键或右键）且点击位置不在菜单内 → 关闭
            // 注意：由于 X11 input shape，正常模式下窗口外点击会穿透而收不到，
            // 所以菜单打开期间 input shape 会把整个窗口设为可交互（见下方 X11 input shape 段），
            // 从而用户点窗口任意位置（含透明区）都能触发这里的关闭判断。
            let primary_clicked = ui.input(|i| i.pointer.primary_pressed());
            let secondary_clicked = ui.input(|i| i.pointer.secondary_pressed());
            if (primary_clicked || secondary_clicked) {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                    let in_menu = menu_rect.map_or(false, |r| r.contains(pos));
                    // 点击不在菜单区 → 关闭（右键点别处则会另开新菜单，由上方 secondary_pressed 逻辑处理）
                    if !in_menu {
                        close_menu = true;
                    }
                }
            }

            // 失焦关闭：窗口失去焦点（用户点击了桌面其他应用窗口）→ 关闭菜单
            // 这是"点桌面任意位置关闭"的关键 —— 因为 input shape 穿透，
            // 点击其他应用窗口时本窗口会失去焦点，egui 收到 WindowFocused(false)
            if ui.input(|i| i.raw.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
                close_menu = true;
            }

            // ESC 关闭
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close_menu = true;
            }

            if close_menu {
                self.menu_open = false;
            }
        }

        // ── X11 input shape ──
        // 设置模式：整个面板可交互
        // 菜单打开：灯泡 + 菜单区域可交互
        // 正常：仅灯泡区域
        if self.settings_mode {
            let full_rect = ui.max_rect();
            if Some(&full_rect) != self.last_bulb_rects.first() {
                self.last_bulb_rects = vec![full_rect];
                if let Some(h) = platform::extract_handles(frame) {
                    let ppp = ui.ctx().pixels_per_point();
                    platform::set_input_region(h, &[full_rect], ppp);
                }
            }
        } else {
            let mut input_rects: Vec<egui::Rect> =
                bulb_responses.iter().map(|r| r.rect).collect();
            // 菜单打开时：把整个窗口设为可交互（取消透明区穿透），
            // 这样用户点击窗口任意位置（含透明边距区）都能被捕获以关闭菜单。
            // 同时也把菜单 rect 纳入（菜单按钮本身可点击）。
            if self.menu_open {
                let full = ui.max_rect();
                input_rects = vec![full];
            } else if let Some(mr) = egui::AreaState::load(ui.ctx(), egui::Id::new("context_menu"))
                .map(|s| s.rect())
            {
                // （此分支理论上不会进，菜单关闭时 menu_open 已 false，保留兼容）
                input_rects.push(mr);
            }
            if input_rects != self.last_bulb_rects && !input_rects.is_empty() {
                self.last_bulb_rects = input_rects.clone();
                if let Some(h) = platform::extract_handles(frame) {
                    let ppp = ui.ctx().pixels_per_point();
                    platform::set_input_region(h, &input_rects, ppp);
                }
            }
        }

        // 自适应窗口大小（session 数量 / 设置模式 / 图标尺寸 变化时调整）
        let need_resize = self.last_count != snap.len()
            || self.last_settings_mode != self.settings_mode
            || self.last_icon_factor != self.icon_size_factor;
        if need_resize {
            self.last_count = snap.len();
            self.last_settings_mode = self.settings_mode;
            self.last_icon_factor = self.icon_size_factor;
            let want_w = if self.settings_mode {
                SETTINGS_PANEL_W
            } else {
                (bulbs_w + PAD_PX * 2.0).max(WIN_W_MIN)
            };
            let want_h = if self.settings_mode {
                SETTINGS_PANEL_H + icon_px + PAD_PX * 2.0
            } else {
                icon_px + PAD_PX * 2.0 + TOOLTIP_ROOM
            };
            // with_resizable(true) + egui InnerSize command
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                want_w,
                want_h,
            )));
        }

        // 动画图标：如果有可见的 GIF 灯泡，安排下一帧重绘
        if !self.settings_mode {
            let visible: Vec<LightState> = if snap.is_empty() {
                vec![LightState::Done]
            } else {
                snap.iter().map(|e| e.state).collect()
            };
            self.icons.schedule_animation_repaint(ui.ctx(), &visible);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.settings_mode {
            [0xF7 as f32 / 255.0, 0xF8 as f32 / 255.0, 0xFA as f32 / 255.0, 1.0] // #F7F8FA
        } else {
            [0.0, 0.0, 0.0, 0.0] // 正常模式：全透明
        }
    }
}

impl App {
    fn render_bulb(&self, ui: &mut egui::Ui, e: &SessionEntry, icon_px: f32) -> egui::Response {
        let tex = match self.icons.get(&e.state) {
            Some(t) => t,
            None => return ui.allocate_response(Vec2::ZERO, egui::Sense::hover()),
        };
        // GIF 动画图标跳过 pulse 效果（自身已有动画）；静态图保留呼吸效果
        let (tint, size) = if self.icons.is_animated(&e.state) {
            (Color32::WHITE, icon_px)
        } else {
            pulse(e.state, ui.ctx().input(|i| i.time), icon_px)
        };

        // 固定 icon_px 槽位 + click/drag 交互（单击=置顶终端，拖动=移动灯泡）
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::splat(icon_px), egui::Sense::click_and_drag());

        // 脉冲图像居中绘制在固定槽位内（不因脉冲改变布局）
        let img_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
        ui.painter().image(
            tex.id(),
            img_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
            tint,
        );

        // 单击：后台置顶对应 opencode 的终端窗口
        if resp.clicked() {
            let sid = e.session_id.clone();
            let title = e.title.clone();
            std::thread::spawn(move || {
                if let Some(pid) = sid.strip_prefix("pid:").and_then(|s| s.parse::<i32>().ok()) {
                    wm::raise_window_for_pid(pid, title.as_deref());
                }
            });
        }

        let display_name = match &e.title {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                if e.session_id.starts_with("pid:") {
                    "opencode".to_string()
                } else {
                    e.session_id.clone()
                }
            }
        };
        let dot_color = state_color(e.state);
        let status_label = e.state.label();

        let mut tip = egui::Tooltip::for_enabled(&resp);
        tip.popup = tip
            .popup
            .align(egui::RectAlign::TOP)
            .align_alternatives(&[])
            .gap(6.0)
            .frame(egui::Frame::NONE); // 移除外层 Popup 默认 Frame（消除黑线边框）
        tip.show(|ui| {
            egui::Frame {
                fill: Color32::from_rgba_unmultiplied(252, 252, 254, 250),
                stroke: egui::Stroke::new(1.0, Color32::from_black_alpha(15)),
                shadow: egui::Shadow {
                    offset: [0, 2],
                    blur: 12,
                    spread: 0,
                    color: Color32::from_black_alpha(50),
                },
                corner_radius: 8.0.into(),
                inner_margin: egui::Margin::same(10),
                ..Default::default()
            }
            .show(ui, |ui| {
                ui.set_width_range(80.0..=200.0);
                ui.horizontal(|ui| {
                    let (dot_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                    let center = dot_rect.center() - egui::vec2(0.0, -1.0);
                    ui.painter().circle_filled(center, 4.0, dot_color);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&display_name)
                                .size(15.0)
                                .color(Color32::from_rgb(30, 30, 35))
                                .strong(),
                        )
                        .wrap(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new(status_label)
                            .size(12.0)
                            .color(Color32::from_rgb(110, 110, 120)),
                    );
                });
            });
        });

        resp
    }

    /// 渲染自定义图标设置面板 — 现代化卡片式布局
    fn render_settings(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame, panel_rect: egui::Rect) {
        if let Some((state, rx)) = &self.file_dialog_state {
            if let Ok(path_opt) = rx.try_recv() {
                if let Some(path) = path_opt {
                    if Self::validate_icon_file(&path) {
                        match config::install_icon(&path, state) {
                            Ok(dest) => {
                                eprintln!("[settings] installed {} → {}", path.display(), dest.display());
                                self.icons.reload(ui.ctx(), &ASSETS, *state);
                            }
                            Err(e) => {
                                eprintln!("[settings] install failed: {}", e);
                            }
                        }
                    } else {
                        self.format_error = Some((
                            Instant::now(),
                            path.extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("?")
                                .to_string(),
                        ));
                    }
                }
                self.file_dialog_state = None;
            }
        }

        let has_error = self
            .format_error
            .as_ref()
            .map(|(t, _)| t.elapsed() < Duration::from_secs(3))
            .unwrap_or(false);
        if !has_error {
            self.format_error = None;
        }

        // ── 主面板容器 ──
        egui::Frame {
            fill: Color32::from_rgb(0xF7, 0xF8, 0xFA),
            inner_margin: egui::Margin {
                left: 16, right: 16, top: 14, bottom: 8,
            },
            ..Default::default()
        }
        .show(ui, |ui| {
            // ── 标题栏 ──
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Custom Status Icons")
                        .size(15.0)
                        .color(Color32::from_rgb(0x1A, 0x1A, 0x1A))
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let done_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("Done")
                                .size(12.0)
                                .color(Color32::from_rgb(0x1A, 0x1A, 0x1A)),
                        )
                        .fill(Color32::from_rgb(0xE8, 0xE8, 0xEC))
                        .corner_radius(4.0)
                        .min_size(egui::vec2(56.0, 24.0)),
                    );
                    if done_btn.clicked() {
                        self.settings_mode = false;
                    }
                });
            });

            // 格式错误提示
            if has_error {
                ui.add_space(4.0);
                egui::Frame {
                    fill: Color32::from_rgb(255, 242, 240),
                    stroke: egui::Stroke::new(1.0, Color32::from_rgb(255, 120, 117)),
                    corner_radius: 4.0.into(),
                    inner_margin: egui::Margin::same(8),
                    ..Default::default()
                }
                .show(ui, |ui| {
                    let ext = self.format_error.as_ref().map(|(_, e)| e.clone()).unwrap_or_default();
                    ui.label(
                        egui::RichText::new(format!("Unsupported: .{} — use PNG/JPG/GIF", ext))
                            .size(12.0)
                            .color(Color32::from_rgb(194, 40, 0)),
                    );
                });
            }

            ui.add_space(8.0);

            // ── 卡片列表 ──
            let states = [LightState::Running, LightState::Input, LightState::Done];
            let state_names = ["Running", "Need Manual Action", "Completed"];
            let state_colors = [
                Color32::from_rgb(0xFF, 0x4D, 0x4F),
                Color32::from_rgb(0xFA, 0xAD, 0x14),
                Color32::from_rgb(0x52, 0xC4, 0x1A),
            ];

            self.settings_rects.clear();
            let has_dragged_files = ui.input(|i| !i.raw.hovered_files.is_empty());

            for (i, state) in states.iter().enumerate() {
                if i > 0 {
                    ui.add_space(8.0);
                }
                self.render_card(ui, *state, state_names[i], state_colors[i], has_dragged_files);
            }

            ui.add_space(8.0);

            // ── 图标尺寸卡片 ──
            self.render_size_card(ui);

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Supported: PNG / JPG / GIF")
                    .size(10.5)
                    .color(Color32::from_rgb(0x99, 0x99, 0x99)),
            );
        });

        // ── 拖拽检测：检查 dropped_files ──
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                for (state, rect) in &self.settings_rects {
                    if rect.contains(pointer) {
                        if let Some(file) = dropped.first() {
                            if let Some(path) = &file.path {
                                if Self::validate_icon_file(path) {
                                    match config::install_icon(path, state) {
                                        Ok(dest) => {
                                                eprintln!("[settings] installed {} → {}", path.display(), dest.display());
                                                self.icons.reload(ui.ctx(), &ASSETS, *state);
                                        }
                                        Err(e) => {
                                            eprintln!("[settings] install failed: {}", e);
                                        }
                                    }
                                } else {
                                    let ext = path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("?")
                                        .to_string();
                                    self.format_error = Some((Instant::now(), ext));
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // 设置面板内 GIF 预览也需要动画重绘
        let all_states = vec![LightState::Running, LightState::Input, LightState::Done];
        self.icons.schedule_animation_repaint(ui.ctx(), &all_states);
    }

    /// 渲染单个状态卡片
    fn render_card(
        &mut self,
        ui: &mut egui::Ui,
        state: LightState,
        name: &str,
        color: Color32,
        has_dragged_files: bool,
    ) {
        // 卡片容器：白底 + 圆角 + 轻阴影
        egui::Frame {
            fill: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            corner_radius: 8.0.into(),
            inner_margin: egui::Margin {
                left: 14, right: 14, top: 12, bottom: 12,
            },
            shadow: egui::Shadow {
                offset: [0, 1],
                blur: 3,
                spread: 0,
                color: Color32::from_black_alpha(8),
            },
            ..Default::default()
        }
        .show(ui, |ui| {
            // ── 标题行：色点 + 状态名 ──
            ui.horizontal(|ui| {
                let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 5.0, color);
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(name)
                        .size(13.5)
                        .color(Color32::from_rgb(0x1A, 0x1A, 0x1A))
                        .strong(),
                );
            });

            ui.add_space(8.0);

            // ── 内容行：预览 + 拖拽区 + 按钮区 ──
            ui.horizontal(|ui| {
                // 左侧：48×48 预览（圆角灰底）
                let preview_size = 48.0;
                let (preview_rect, _) = ui.allocate_exact_size(
                    egui::vec2(preview_size, preview_size),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(preview_rect, 6.0, Color32::from_rgb(0xF7, 0xF8, 0xFA));
                if let Some(tex) = self.icons.get(&state) {
                    let img_size = tex.size_vec2();
                    let aspect = img_size.x / img_size.y.max(1.0);
                    let (dw, dh) = if aspect > 1.0 {
                        (preview_size, preview_size / aspect)
                    } else {
                        (preview_size * aspect, preview_size)
                    };
                    let draw_rect = egui::Rect::from_center_size(
                        preview_rect.center(),
                        egui::vec2(dw, dh),
                    );
                    ui.painter().image(
                        tex.id(),
                        draw_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }

                ui.add_space(10.0);

                // 中间：拖拽区
                let is_custom = self.icons.is_custom(&state);
                let drop_avail_w = ui.available_width();

                // 如果有 reset 按钮，预留空间
                let reset_w = if is_custom { 64.0 } else { 0.0 };
                let drop_w = (drop_avail_w - reset_w - 4.0).max(80.0);

                let (drop_rect, drop_resp) = ui.allocate_exact_size(
                    egui::vec2(drop_w, preview_size),
                    egui::Sense::click(),
                );

                // 检测拖拽区 hover / drag-hover
                let pointer = ui.input(|input| input.pointer.hover_pos());
                let is_hovered = pointer.map_or(false, |p| drop_rect.contains(p));
                let is_being_dragged = has_dragged_files && is_hovered;

                let drop_bg = if is_being_dragged {
                    Color32::from_rgb(0xE6, 0xF4, 0xFF)
                } else if is_hovered {
                    Color32::from_rgb(0xF0, 0xF5, 0xFA)
                } else {
                    Color32::from_rgb(0xFA, 0xFB, 0xFC)
                };
                let drop_stroke = if is_being_dragged {
                    egui::Stroke::new(1.5, Color32::from_rgb(0x40, 0xA9, 0xFF))
                } else {
                    egui::Stroke::new(1.0, Color32::from_rgb(0xD9, 0xD9, 0xD9))
                };
                ui.painter().rect_filled(drop_rect, 4.0, drop_bg);
                ui.painter().rect_stroke(drop_rect, 4.0, drop_stroke, StrokeKind::Inside);

                // 拖拽区文字
                let drop_text = if is_being_dragged {
                    "Drop here".to_string()
                } else if is_custom {
                    config::custom_description(&state).unwrap_or_else(|| "Custom icon".to_string())
                } else {
                    "Drag or click to browse".to_string()
                };
                let drop_color = if is_being_dragged {
                    Color32::from_rgb(0x40, 0xA9, 0xFF)
                } else if is_custom {
                    Color32::from_rgb(0x52, 0xC4, 0x1A)
                } else {
                    Color32::from_rgb(0x99, 0x99, 0x99)
                };
                ui.painter().text(
                    drop_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &drop_text,
                    egui::FontId::proportional(11.0),
                    drop_color,
                );

                // 记录拖拽区 rect
                self.settings_rects.push((state, drop_rect));

                // 点击拖拽区 → 打开文件对话框
                if drop_resp.clicked() && self.file_dialog_state.is_none() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let result = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "gif"])
                            .pick_file();
                        let _ = tx.send(result);
                    });
                    self.file_dialog_state = Some((state, rx));
                }

                // 右侧：恢复默认按钮（仅自定义时）
                if is_custom {
                    ui.add_space(4.0);
                    let reset_resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("Reset")
                                .size(11.0)
                                .color(Color32::from_rgb(0x72, 0x72, 0x72)),
                        )
                        .fill(Color32::from_rgb(0xF5, 0xF5, 0xF5))
                        .corner_radius(4.0)
                        .min_size(egui::vec2(56.0, 24.0)),
                    );
                    if reset_resp.clicked() {
                        let _ = config::remove_icon(&state);
                        self.icons.reload(ui.ctx(), &ASSETS, state);
                    }
                }
            });
        });
    }

    /// 渲染图标尺寸选择卡片（Small / Medium / Large）
    fn render_size_card(&mut self, ui: &mut egui::Ui) {
        // 卡片容器：复用 render_card 的白底圆角阴影样式
        egui::Frame {
            fill: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            corner_radius: 8.0.into(),
            inner_margin: egui::Margin {
                left: 14, right: 14, top: 12, bottom: 12,
            },
            shadow: egui::Shadow {
                offset: [0, 1],
                blur: 3,
                spread: 0,
                color: Color32::from_black_alpha(8),
            },
            ..Default::default()
        }
        .show(ui, |ui| {
            // ── 标题行：色点 + 标题 ──
            ui.horizontal(|ui| {
                let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 5.0, Color32::from_rgb(0x99, 0x99, 0x99));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Icon Size")
                        .size(13.5)
                        .color(Color32::from_rgb(0x1A, 0x1A, 0x1A))
                        .strong(),
                );
            });

            ui.add_space(8.0);

            // ── 三个分段按钮 ──
            let current = config::IconSize::default(); // 仅用于推断当前选中的 label
            let _ = current;
            let current_label = match self.icon_size_factor {
                x if (x - 0.75).abs() < 0.01 => "Small",
                x if (x - 1.25).abs() < 0.01 => "Large",
                _ => "Medium",
            };
            let current_px = (BASE_ICON_PX * self.icon_size_factor).round() as i32;

            ui.horizontal(|ui| {
                let options = [("Small", 0.75_f32), ("Medium", 1.0), ("Large", 1.25)];
                for (label, factor) in options {
                    let selected = (self.icon_size_factor - factor).abs() < 0.01;
                    let (bg, fg) = if selected {
                        (Color32::from_rgb(0x33, 0x33, 0x33), Color32::WHITE) // 选中：深底白字
                    } else {
                        (Color32::from_rgb(0xF0, 0xF0, 0xF3), Color32::from_rgb(0x33, 0x33, 0x33))
                    };
                    let btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new(label)
                                .size(12.5)
                                .color(fg)
                                .strong(),
                        )
                        .fill(bg)
                        .corner_radius(6.0)
                        .min_size(egui::vec2(96.0, 30.0)),
                    );
                    if btn.clicked() && !selected {
                        self.icon_size_factor = factor;
                        config::save_size(match factor {
                            x if (x - 0.75).abs() < 0.01 => config::IconSize::Small,
                            x if (x - 1.25).abs() < 0.01 => config::IconSize::Large,
                            _ => config::IconSize::Medium,
                        });
                        eprintln!("[settings] icon size → {} ({}px)", label, (BASE_ICON_PX * factor).round() as i32);
                    }
                }
            });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("Current: {} ({}px)", current_label, current_px))
                    .size(10.5)
                    .color(Color32::from_rgb(0x99, 0x99, 0x99)),
            );
        });
    }

    /// 校验文件是否为支持的图标格式
    fn validate_icon_file(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif"))
            .unwrap_or(false)
    }
}

fn state_color(state: LightState) -> Color32 {
    match state {
        LightState::Running => Color32::from_rgb(0xFF, 0x4D, 0x4F),
        LightState::Input => Color32::from_rgb(0xFA, 0xAD, 0x14),
        LightState::Done => Color32::from_rgb(0x52, 0xC4, 0x1A),
    }
}

fn pulse(state: LightState, t: f64, icon_px: f32) -> (Color32, f32) {
    match state {
        LightState::Running | LightState::Input => {
            // 1.1s 周期，亮度 0.85~1.15
            let phase = ((t % 1.1) / 1.1) as f32;
            let amp = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos(); // 0..1
            let brightness = 0.88 + 0.22 * amp;
            let size = icon_px * (1.0 + 0.03 * amp);
            (Color32::WHITE.linear_multiply(brightness.min(1.0)), size)
        }
        LightState::Done => (Color32::WHITE, icon_px),
    }
}
