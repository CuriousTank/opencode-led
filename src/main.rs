use egui::{Color32, Vec2, ViewportBuilder};
use eframe::egui;
use include_dir::{include_dir, Dir};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod icons;
mod platform;
mod server;
mod store;
mod wm;

use icons::Icons;
use store::{LightState, SessionEntry, Store};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");

const DEFAULT_PORT: u16 = 9912;
const ICON_PX: f32 = 64.0;
const GAP_PX: f32 = 6.0;
const PAD_PX: f32 = 2.0;
const TOOLTIP_ROOM: f32 = 140.0;
const WIN_W_MIN: f32 = 240.0; // 最小透明边距，让窗口几乎贴合灯泡，减少对其他窗口的阻挡
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const SESSION_TIMEOUT: Duration = Duration::from_secs(12);

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

    let viewport = ViewportBuilder::default()
        .with_title("opencode traffic light")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        .with_position([100.0, 100.0])
        .with_inner_size(Vec2::new(
            WIN_W_MIN,
            ICON_PX + PAD_PX * 2.0 + TOOLTIP_ROOM,
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
}

/// 拖拽中：保持鼠标相对窗口左上角的偏移恒定
struct DragState {
    handles: platform::XHandles,
    offset: egui::Vec2, // 鼠标根坐标 - 窗口左上角根坐标
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.icons.ensure_loaded(ui.ctx(), &ASSETS);

        // 消费 dirty 标志（这里只是触发重绘，快照在下面直接读）
        let _ = self.dirty.lock();

        // 确保每 2 秒至少重绘一次，让 sweep 能定期执行
        // （否则无事件时 ui() 不会被调用，过期灯泡不会被清理）
        ui.ctx().request_repaint_after(Duration::from_secs(2));

        // 定期清理过期的 session（心跳超时 = opencode 进程已退出）
        if self.last_sweep.elapsed() >= SWEEP_INTERVAL {
            self.last_sweep = Instant::now();
            if self.store.sweep(SESSION_TIMEOUT) {
                *self.dirty.lock() = true;
            }
        }

        let snap = self.store.snapshot();
        let count = snap.len().max(1);

        // 透明背景
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.window_fill = Color32::TRANSPARENT;
        visuals.extreme_bg_color = Color32::TRANSPARENT;
        ui.ctx().set_visuals(visuals);

        // 灯泡行定位：窗口底部居中，上方留 TOOLTIP_ROOM 给 tooltip 展示空间
        let win_rect = ui.max_rect();
        let bulbs_w =
            count as f32 * ICON_PX + count.saturating_sub(1) as f32 * GAP_PX;
        let bulbs_rect = egui::Rect::from_min_size(
            egui::pos2(
                win_rect.center().x - bulbs_w / 2.0,
                win_rect.bottom() - PAD_PX - ICON_PX,
            ),
            egui::vec2(bulbs_w.max(ICON_PX), ICON_PX),
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
            let tint = Color32::WHITE.linear_multiply(0.15);
            let (rect, resp) =
                bulb_ui.allocate_exact_size(Vec2::splat(ICON_PX), egui::Sense::hover());
            bulb_ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                tint,
            );
            bulb_responses.push(resp.on_hover_text("waiting for opencode…"));
        } else {
            for e in &snap {
                let resp = self.render_bulb(&mut bulb_ui, e);
                bulb_responses.push(resp);
            }
        }

        // 逐灯泡拖拽检测（替代旧的单一 drag 层，避免遮挡中间灯泡的 hover）
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

        // X11 input shape：只有灯泡区域接收鼠标事件，透明区域点击穿透
        let current_rects: Vec<egui::Rect> =
            bulb_responses.iter().map(|r| r.rect).collect();
        if current_rects != self.last_bulb_rects {
            self.last_bulb_rects = current_rects.clone();
            if !current_rects.is_empty() {
                if let Some(h) = platform::extract_handles(frame) {
                    let ppp = ui.ctx().pixels_per_point();
                    platform::set_input_region(h, &current_rects, ppp);
                }
            }
        }

        // 自适应窗口大小（session 数量变化时调整）
        if self.last_count != snap.len() {
            self.last_count = snap.len();
            let want_w = (bulbs_w + PAD_PX * 2.0).max(WIN_W_MIN);
            let want_h = ICON_PX + PAD_PX * 2.0 + TOOLTIP_ROOM;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                want_w,
                want_h,
            )));
        }

        // 右键退出
        if ui.input(|i| i.pointer.secondary_clicked()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

impl App {
    fn render_bulb(&self, ui: &mut egui::Ui, e: &SessionEntry) -> egui::Response {
        let tex = match self.icons.get(&e.state) {
            Some(t) => t,
            None => return ui.allocate_response(Vec2::ZERO, egui::Sense::hover()),
        };
        let (tint, size) = pulse(e.state, ui.ctx().input(|i| i.time));

        // 固定 ICON_PX 槽位 + click/drag 交互（单击=置顶终端，拖动=移动灯泡）
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::splat(ICON_PX), egui::Sense::click_and_drag());

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

        resp.on_hover_ui(|ui| {
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
        })
    }
}

fn state_color(state: LightState) -> Color32 {
    match state {
        LightState::Running => Color32::from_rgb(255, 59, 48),
        LightState::Input => Color32::from_rgb(255, 204, 0),
        LightState::Done => Color32::from_rgb(52, 199, 89),
    }
}

fn pulse(state: LightState, t: f64) -> (Color32, f32) {
    match state {
        LightState::Running | LightState::Input => {
            // 1.1s 周期，亮度 0.85~1.15
            let phase = ((t % 1.1) / 1.1) as f32;
            let amp = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos(); // 0..1
            let brightness = 0.88 + 0.22 * amp;
            let size = ICON_PX * (1.0 + 0.03 * amp);
            (Color32::WHITE.linear_multiply(brightness.min(1.0)), size)
        }
        LightState::Done => (Color32::WHITE, ICON_PX),
    }
}
