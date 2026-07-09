use crate::store::LightState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Return the config directory: ~/.config/opencode-traffic-light/
fn config_base() -> PathBuf {
    let mut p = dirs_config();
    p.push("opencode-traffic-light");
    p
}

/// ~/.config or fallback
fn dirs_config() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut p = PathBuf::from(home);
    p.push(".config");
    p
}

/// ~/.config/opencode-traffic-light/icons/
fn icon_dir() -> PathBuf {
    let mut p = config_base();
    p.push("icons");
    p
}

/// ~/.config/opencode-traffic-light/icons/{color}/  — each colour has its own subdir
fn color_dir(state: &LightState) -> PathBuf {
    icon_dir().join(color_name(state))
}

/// Colour name used as filename stem
fn color_name(state: &LightState) -> &'static str {
    match state {
        LightState::Running => "red",
        LightState::Input => "yellow",
        LightState::Done => "green",
    }
}

/// Check if a custom icon exists for the given state.
/// Searches icons/{color}/ for any supported image file.
pub fn custom_path(state: &LightState) -> Option<PathBuf> {
    let dir = color_dir(state);
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(ext.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif") {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Copy a user-selected file into the icon subdirectory, preserving the original filename.
/// Returns the destination path.
pub fn install_icon(src: &Path, state: &LightState) -> std::io::Result<PathBuf> {
    let dir = color_dir(state);
    fs::create_dir_all(&dir)?;

    // Clean any existing files in this colour dir (so only one icon per colour)
    remove_icon(state)?;

    // Preserve original filename
    let filename = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("icon.png");

    let dest = dir.join(filename);
    fs::copy(src, &dest)?;
    Ok(dest)
}

/// Remove custom icon(s) for the given state, reverting to embedded default.
pub fn remove_icon(state: &LightState) -> std::io::Result<()> {
    let dir = color_dir(state);
    if dir.exists() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// Human-readable description for the settings panel.
pub fn custom_description(state: &LightState) -> Option<String> {
    custom_path(state).and_then(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    })
}

// ── 图标尺寸偏好 ──

/// 用户可选的图标尺寸档位。
/// Small / Medium / Large 对应 0.75× / 1.0× / 1.25× 的基准尺寸（基准 = 64px）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconSize {
    Small,
    Medium,
    Large,
}

impl Default for IconSize {
    fn default() -> Self {
        IconSize::Medium
    }
}

impl IconSize {
    /// 尺寸倍数：Small=0.75, Medium=1.0, Large=1.25
    pub fn factor(&self) -> f32 {
        match self {
            IconSize::Small => 0.75,
            IconSize::Medium => 1.0,
            IconSize::Large => 1.25,
        }
    }

    /// 显示标签
    pub fn label(&self) -> &'static str {
        match self {
            IconSize::Small => "Small",
            IconSize::Medium => "Medium",
            IconSize::Large => "Large",
        }
    }
}

/// settings.json 的结构
#[derive(Serialize, Deserialize, Default)]
struct Settings {
    #[serde(default)]
    icon_size: IconSize,
}

/// settings.json 路径：~/.config/opencode-traffic-light/settings.json
fn settings_path() -> PathBuf {
    config_base().join("settings.json")
}

/// 读取图标尺寸偏好，读不到/解析失败时返回默认值 (Medium)
pub fn load_size() -> IconSize {
    let path = settings_path();
    match fs::read_to_string(&path) {
        Ok(content) => {
            serde_json::from_str::<Settings>(&content)
                .map(|s| s.icon_size)
                .unwrap_or_default()
        }
        Err(_) => IconSize::default(),
    }
}

/// 保存图标尺寸偏好到 settings.json（失败仅打印日志，不影响运行）
pub fn save_size(size: IconSize) {
    let path = settings_path();
    let settings = Settings { icon_size: size };
    match serde_json::to_string_pretty(&settings) {
        Ok(json) => {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::write(&path, json) {
                eprintln!("[settings] failed to save size: {}", e);
            }
        }
        Err(e) => eprintln!("[settings] failed to serialize size: {}", e),
    }
}
