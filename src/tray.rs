//! System tray icon for opencode-traffic-light.
//!
//! Shows an aggregate status (red/yellow/green) in the system tray,
//! with a context menu to show/hide the widget, open settings, or quit.
//!
//! Linux: uses KDE StatusNotifierItem / AppIndicator (via the `tray-icon` crate).

use crate::store::LightState;
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

/// Manages the system tray icon lifecycle.
pub struct Tray {
    icon: Arc<Mutex<Option<TrayIcon>>>,
    /// Handle to the "Show/Hide" menu item, so we can relabel it live.
    toggle_item: Option<MenuItem>,
    /// Receiver for menu events (polled from the egui loop).
    rx: Receiver<TrayCmd>,
    /// Cached last set state to avoid needless icon rebuilds.
    last_state: Mutex<Option<LightState>>,
}

impl Tray {
    /// Create the tray icon and wire up the menu.
    /// Call this from the main thread *before* `eframe::run_native`.
    /// Always returns an `Arc<Tray>` (no-op stub if the platform tray is unavailable),
    /// so callers don't have to branch on `Option`.
    pub fn new(initial: LightState) -> Arc<Self> {
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

        let icon = load_icon(initial);
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
            last_state: Mutex::new(Some(initial)),
        })
    }

    /// Update the tray icon + tooltip when the aggregate state changes.
    pub fn set_state(&self, state: LightState) {
        let mut last = self.last_state.lock();
        if *last == Some(state) {
            return;
        }
        *last = Some(state);
        drop(last);

        let mut tray = self.icon.lock();
        let Some(tray) = tray.as_mut() else {
            return;
        };
        if let Some(ic) = load_icon(state) {
            let _ = tray.set_icon(Some(ic));
        }
        let _ = tray.set_tooltip(Some(format!(
            "opencode traffic light — {}",
            state.label()
        )));
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
