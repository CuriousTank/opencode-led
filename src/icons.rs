use crate::config;
use crate::store::LightState;
use egui::{ColorImage, TextureHandle};
use image::AnimationDecoder;
use parking_lot::RwLock;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One frame of an animated icon.
struct IconFrame {
    texture: TextureHandle,
    delay: Duration,
}

/// A colour's icon — either a single static texture or an animation.
enum Icon {
    Static(TextureHandle),
    Animated {
        frames: Vec<IconFrame>,
        current: usize,
        next_switch: Instant,
    },
}

impl Icon {
    fn is_animated(&self) -> bool {
        matches!(self, Icon::Animated { .. })
    }
}

struct IconsInner {
    red: Icon,
    yellow: Icon,
    green: Icon,
}

/// Caches red/yellow/green icon textures (static or animated).
pub struct Icons {
    inner: RwLock<Option<IconsInner>>,
    /// Set of colours whose custom icon was loaded (to know if defaults are overridden).
    custom_loaded: RwLock<[bool; 3]>,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            inner: RwLock::new(None),
            custom_loaded: RwLock::new([false; 3]),
        }
    }
}

impl Icons {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Initialise on first update (needs egui ctx).
    /// After initial load, custom icons from disk take priority over embedded defaults.
    pub fn ensure_loaded(&self, ctx: &egui::Context, assets: &include_dir::Dir<'_>) {
        if self.inner.read().is_some() {
            return;
        }
        let mut g = self.inner.write();
        if g.is_some() {
            return;
        }

        let states = [LightState::Running, LightState::Input, LightState::Done];
        let names = ["red", "yellow", "green"];
        let mut icons_vec = Vec::with_capacity(3);
        let mut custom = [false; 3];

        for (i, state) in states.iter().enumerate() {
            // Try custom icon first
            if let Some(path) = config::custom_path(state) {
                match load_icon_from_file(ctx, &path) {
                    Ok(icon) => {
                        custom[i] = true;
                        icons_vec.push(icon);
                        eprintln!("[icons] custom icon loaded: {}", path.display());
                        continue;
                    }
                    Err(e) => {
                        eprintln!("[icons] failed to load custom {}: {}", path.display(), e);
                    }
                }
            }
            // Fallback: embedded default
            let icon = load_embedded_png(ctx, assets, names[i]);
            icons_vec.push(icon);
        }

        *g = Some(IconsInner {
            red: icons_vec.remove(0),
            yellow: icons_vec.remove(0),
            green: icons_vec.remove(0),
        });
        *self.custom_loaded.write() = custom;
    }

    /// Reload a single colour's icon from disk (or revert to default if no custom exists).
    /// Call after user installs/removes a custom icon.
    pub fn reload(&self, ctx: &egui::Context, assets: &include_dir::Dir<'_>, state: LightState) {
        let icon = if let Some(path) = config::custom_path(&state) {
            match load_icon_from_file(ctx, &path) {
                Ok(icon) => {
                    eprintln!("[icons] reloaded custom: {}", path.display());
                    icon
                }
                Err(e) => {
                    eprintln!("[icons] reload failed {}: {}", path.display(), e);
                    let name = color_name(&state);
                    load_embedded_png(ctx, assets, name)
                }
            }
        } else {
            let name = color_name(&state);
            load_embedded_png(ctx, assets, name)
        };

        let idx = color_index(&state);
        let is_custom = config::custom_path(&state).is_some();

        let mut g = self.inner.write();
        if let Some(inner) = g.as_mut() {
            match state {
                LightState::Running => inner.red = icon,
                LightState::Input => inner.yellow = icon,
                LightState::Done => inner.green = icon,
            }
            self.custom_loaded.write()[idx] = is_custom;
        }
    }

    /// Get current texture, advancing animation if needed.
    pub fn get(&self, state: &LightState) -> Option<TextureHandle> {
        let mut g = self.inner.write();
        let inner = g.as_mut()?;
        let icon = icon_for_state_mut(inner, state);
        match icon {
            Icon::Static(tex) => Some(tex.clone()),
            Icon::Animated {
                frames,
                current,
                next_switch,
            } => {
                let now = Instant::now();
                if now >= *next_switch && !frames.is_empty() {
                    *current = (*current + 1) % frames.len();
                    *next_switch = now + frames[*current].delay;
                }
                Some(frames[*current].texture.clone())
            }
        }
    }

    /// Whether a state uses an animated (GIF) icon.
    pub fn is_animated(&self, state: &LightState) -> bool {
        let g = self.inner.read();
        if let Some(inner) = g.as_ref() {
            icon_for_state(inner, state).is_animated()
        } else {
            false
        }
    }

    /// Whether a state has a custom icon (vs embedded default).
    pub fn is_custom(&self, state: &LightState) -> bool {
        self.custom_loaded.read()[color_index(state)]
    }

    /// If any animated icon is showing, schedule next repaint.
    pub fn schedule_animation_repaint(&self, ctx: &egui::Context, visible_states: &[LightState]) {
        let g = self.inner.read();
        let Some(inner) = g.as_ref() else {
            return;
        };
        let mut min_delay: Option<Duration> = None;
        for state in visible_states {
            if let Icon::Animated { frames, current, .. } = icon_for_state(inner, state) {
                let remaining = frames[*current].delay;
                min_delay = Some(match min_delay {
                    Some(d) => d.min(remaining),
                    None => remaining,
                });
            }
        }
        if let Some(delay) = min_delay {
            ctx.request_repaint_after(delay);
        }
    }
}

// ── helpers ──

fn color_name(state: &LightState) -> &'static str {
    match state {
        LightState::Running => "red",
        LightState::Input => "yellow",
        LightState::Done => "green",
    }
}

fn color_index(state: &LightState) -> usize {
    match state {
        LightState::Running => 0,
        LightState::Input => 1,
        LightState::Done => 2,
    }
}

fn icon_for_state<'a>(inner: &'a IconsInner, state: &LightState) -> &'a Icon {
    match state {
        LightState::Running => &inner.red,
        LightState::Input => &inner.yellow,
        LightState::Done => &inner.green,
    }
}

fn icon_for_state_mut<'a>(inner: &'a mut IconsInner, state: &LightState) -> &'a mut Icon {
    match state {
        LightState::Running => &mut inner.red,
        LightState::Input => &mut inner.yellow,
        LightState::Done => &mut inner.green,
    }
}

fn load_embedded_png(
    ctx: &egui::Context,
    assets: &include_dir::Dir<'_>,
    name: &str,
) -> Icon {
    let bytes = assets
        .get_file(format!("{}.png", name))
        .map(|f| f.contents())
        .unwrap_or_default();
    let tex = decode_single(ctx, name, bytes);
    Icon::Static(tex)
}

fn load_icon_from_file(ctx: &egui::Context, path: &Path) -> Result<Icon, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read: {}", e))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Animated GIF → multiple frames
    if ext == "gif" {
        return decode_gif(ctx, path, &bytes);
    }

    // Static PNG / JPEG
    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("decode: {}", e))?
        .to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let tex = ctx.load_texture(
        path.to_string_lossy(),
        ColorImage::from_rgba_unmultiplied(size, &img.into_raw()),
        egui::TextureOptions::LINEAR,
    );
    Ok(Icon::Static(tex))
}

fn decode_gif(ctx: &egui::Context, path: &Path, bytes: &[u8]) -> Result<Icon, String> {
    let decoder =
        image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| format!("gif: {}", e))?;
    let frames_raw = decoder
        .into_frames()
        .collect_frames()
        .map_err(|e| format!("gif frames: {}", e))?;

    if frames_raw.len() <= 1 {
        // Single-frame GIF — treat as static
        let f = &frames_raw[0];
        let rgba = f.buffer();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let tex = ctx.load_texture(
            path.to_string_lossy(),
            ColorImage::from_rgba_unmultiplied(size, &rgba.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        return Ok(Icon::Static(tex));
    }

    let mut frames = Vec::with_capacity(frames_raw.len());
    let id = path.to_string_lossy();
    for (i, f) in frames_raw.iter().enumerate() {
        let rgba = f.buffer();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let tex = ctx.load_texture(
            format!("{}#{}", id, i),
            ColorImage::from_rgba_unmultiplied(size, &rgba.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        let (numer, denom) = f.delay().numer_denom_ms();
        let delay_ms = if denom > 0 {
            (numer as f64 / denom as f64) as u64
        } else {
            100
        };
        let delay = Duration::from_millis(delay_ms.max(20));
        frames.push(IconFrame { texture: tex, delay });
    }

    Ok(Icon::Animated {
        frames,
        current: 0,
        next_switch: Instant::now() + Duration::from_millis(100),
    })
}

fn decode_single(ctx: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let img = image::load_from_memory(bytes)
        .unwrap_or_else(|_| {
            panic!(
                "Failed to decode embedded icon '{}.png' — assets corrupt",
                name
            )
        })
        .to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    ctx.load_texture(
        name,
        ColorImage::from_rgba_unmultiplied(size, &img.into_raw()),
        egui::TextureOptions::LINEAR,
    )
}
