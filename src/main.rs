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

use icons::Icons;
use store::{LightState, SessionEntry, Store};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");

const DEFAULT_PORT: u16 = 9912;
const ICON_PX: f32 = 64.0;
const GAP_PX: f32 = 6.0;
const PAD_PX: f32 = 2.0; // 最小透明边距，让窗口几乎贴合灯泡，减少对其他窗口的阻挡
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
        .with_inner_size(Vec2::new(ICON_PX + PAD_PX * 2.0, ICON_PX + PAD_PX * 2.0));

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
            Ok(Box::new(App {
                store,
                icons,
                dirty,
                last_count: 0,
                last_above_time: 0.0,
                last_sweep: Instant::now(),
                drag: None,
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

        ui.spacing_mut().item_spacing.x = GAP_PX;

        // 直接铺满窗口绘制灯泡（窗口已贴合灯泡尺寸，无多余透明边）
        ui.horizontal_centered(|ui| {
            if snap.is_empty() {
                let tex = self.icons.get(&LightState::Done).expect("icon");
                let tint = Color32::WHITE.linear_multiply(0.15);
                ui.add(
                    egui::Image::from_texture(&tex)
                        .fit_to_exact_size(Vec2::splat(ICON_PX))
                        .tint(tint),
                )
                .on_hover_text("waiting for opencode…");
            } else {
                for e in &snap {
                    self.render_bulb(ui, e);
                }
            }
        });

        // 手动 X11 拖拽：用根坐标（稳定，无正反馈）
        let win_rect = ui.max_rect();
        let center = win_rect.center();
        let bulb_area = egui::Rect::from_center_size(center, Vec2::splat(ICON_PX));
        let drag_resp = ui.interact(bulb_area, ui.id().with("drag"), egui::Sense::drag());

        if drag_resp.hovered() && self.drag.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // 拖拽开始：用 XQueryPointer 同时拿根坐标和窗口内坐标
        // win_x/win_y 是鼠标相对窗口左上角的偏移，坐标系与 XMoveWindow 一致，最稳
        if drag_resp.drag_started() && self.drag.is_none() {
            if let Some(h) = platform::extract_handles(frame) {
                if let Some((_mx, _my, wx, wy)) = platform::query_pointer(h) {
                    self.drag = Some(DragState {
                        handles: h,
                        offset: egui::vec2(wx as f32, wy as f32),
                    });
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
            }
        }

        // 定期重申 _NET_WM_STATE_ABOVE，防止被其他窗口遮挡
        // 每秒一次，开销极小
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
            // 拖拽期间持续请求重绘，确保跟得上鼠标移动
            ui.ctx().request_repaint();
        }

        // 鼠标松开：结束拖拽
        let primary_down = ui.input(|i| i.pointer.primary_down());
        if self.drag.is_some() && !primary_down {
            self.drag = None;
        }

        // 自适应窗口大小（只在 session 数量变化时才调整，避免每帧触发 resize 干扰拖拽）
        if self.last_count != snap.len() {
            self.last_count = snap.len();
            let want_w =
                count as f32 * ICON_PX + (count.saturating_sub(1)) as f32 * GAP_PX + PAD_PX * 2.0;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                want_w,
                ICON_PX + PAD_PX * 2.0,
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
    fn render_bulb(&self, ui: &mut egui::Ui, e: &SessionEntry) {
        let tex = match self.icons.get(&e.state) {
            Some(t) => t,
            None => return,
        };
        // 红灯/黄灯轻微脉冲
        let (tint, size) = pulse(e.state, ui.ctx().input(|i| i.time));
        let img = egui::Image::from_texture(&tex)
            .fit_to_exact_size(Vec2::splat(size))
            .tint(tint);
        let tip = match &e.title {
            Some(t) if !t.is_empty() => format!(
                "{}\n{}\n{}",
                t,
                e.project.as_deref().unwrap_or("(no project)"),
                e.state.label()
            ),
            _ => format!(
                "{}\n{}\n{}",
                e.session_id,
                e.project.as_deref().unwrap_or("(no project)"),
                e.state.label()
            ),
        };
        ui.add(img).on_hover_text(tip);
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
