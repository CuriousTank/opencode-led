//! opencode session 历史查询。
//!
//! 直接从 opencode 的 SQLite 数据库（~/.local/share/opencode/opencode.db）
//! 只读查询最近的 session 列表，用于 "Open from Session" 功能。
//!
//! 数据库使用 WAL 模式，多个只读 reader 可与单个 writer 安全共存。

use rusqlite::{Connection, OpenFlags};
use std::time::{Duration, Instant};

/// 一个 session 的元信息（从 SQLite 读取）
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    /// session 的工作目录（绝对路径）
    pub directory: String,
    /// 最后更新时间（epoch 毫秒）
    pub time_updated: i64,
}

/// 从 opencode 的 SQLite 数据库查询最近 N 个 session。
///
/// 只读打开，WAL 安全。按 time_updated 降序排列。
/// 返回 None 如果数据库不存在或查询失败（优雅降级）。
pub fn load_recent_sessions(limit: usize) -> Option<Vec<SessionInfo>> {
    let db_path = opencode_db_path()?;

    let conn = Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let mut stmt = conn
        .prepare(
            "SELECT id, title, directory, time_updated
             FROM session
             WHERE time_archived IS NULL
             ORDER BY time_updated DESC
             LIMIT ?1",
        )
        .ok()?;

    let rows: Vec<SessionInfo> = stmt
        .query_map([limit as i64], |row| {
            Ok(SessionInfo {
                id: row.get(0)?,
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                directory: row.get(2)?,
                time_updated: row.get(3)?,
            })
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    Some(rows)
}

/// 返回 opencode 数据库路径：~/.local/share/opencode/opencode.db
fn opencode_db_path() -> Option<std::path::PathBuf> {
    // 优先 XDG_DATA_HOME
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let p = std::path::Path::new(&xdg).join("opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    // 回退到 ~/.local/share
    let home = std::env::var("HOME").ok()?;
    let p = std::path::Path::new(&home)
        .join(".local/share/opencode/opencode.db");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// 将 epoch 毫秒转为相对时间描述（如 "2 min ago"、"1 hour ago"、"Yesterday"）
pub fn relative_time(time_updated: i64) -> String {
    let now_ms = chrono_now_millis();
    let diff = Duration::from_millis((now_ms - time_updated).max(0) as u64);

    if diff < Duration::from_secs(60) {
        "just now".to_string()
    } else if diff < Duration::from_secs(3600) {
        let mins = diff.as_secs() / 60;
        format!("{} min ago", mins)
    } else if diff < Duration::from_secs(86400) {
        let hours = diff.as_secs() / 3600;
        format!("{} hour{} ago", hours, if hours > 1 { "s" } else { "" })
    } else {
        let days = diff.as_secs() / 86400;
        if days == 1 {
            "Yesterday".to_string()
        } else {
            format!("{} days ago", days)
        }
    }
}

/// 取当前 epoch 毫秒（不依赖 chrono crate）
fn chrono_now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 缩短路径显示：~/project/xxx → ~/p/xxx（保持尾部两级可读）
pub fn shorten_path(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let display = if path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };

    // 如果路径太长，截取尾部
    if display.len() > 48 {
        let parts: Vec<&str> = display.split('/').collect();
        if parts.len() > 3 {
            return format!("{}/...{}/{}", parts[0], parts[parts.len() - 2], parts[parts.len() - 1]);
        }
    }
    display
}

/// 取目录的最后一级名称（basename）
pub fn dir_basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.to_string())
}
