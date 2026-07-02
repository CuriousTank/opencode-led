use egui::{ColorImage, TextureHandle};
use parking_lot::RwLock;
use std::sync::Arc;

/// 缓存红/黄/绿三张 PNG 纹理
pub struct Icons {
    inner: RwLock<Option<IconsInner>>,
}

struct IconsInner {
    red: TextureHandle,
    yellow: TextureHandle,
    green: TextureHandle,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }
}

impl Icons {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 在首次 update 时初始化纹理（需要 egui ctx）
    pub fn ensure_loaded(&self, ctx: &egui::Context, assets: &include_dir::Dir<'_>) {
        if self.inner.read().is_some() {
            return;
        }
        let mut g = self.inner.write();
        if g.is_some() {
            return;
        }
        let load = |name: &str| -> TextureHandle {
            let bytes = assets
                .get_file(format!("{}.png", name))
                .map(|f| f.contents())
                .unwrap_or_default();
            let img = image::load_from_memory(bytes)
                .expect("decode png")
                .to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let pixels = img.into_raw();
            ctx.load_texture(
                name,
                ColorImage::from_rgba_unmultiplied(size, &pixels),
                egui::TextureOptions::LINEAR,
            )
        };
        *g = Some(IconsInner {
            red: load("red"),
            yellow: load("yellow"),
            green: load("green"),
        });
    }

    pub fn get(&self, state: &crate::store::LightState) -> Option<TextureHandle> {
        let g = self.inner.read();
        let inner = g.as_ref()?;
        Some(match state {
            crate::store::LightState::Running => inner.red.clone(),
            crate::store::LightState::Input => inner.yellow.clone(),
            crate::store::LightState::Done => inner.green.clone(),
        })
    }
}
