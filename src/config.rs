use crate::store::LightState;
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
