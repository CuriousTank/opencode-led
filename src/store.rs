use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightState {
    /// opencode 正在执行任务
    Running,
    /// opencode 完成了任务
    Done,
    /// opencode 需要人回复或介入（权限请求挂起）
    Input,
}

impl LightState {
    pub fn label(&self) -> &'static str {
        match self {
            LightState::Running => "Running",
            LightState::Done => "Done",
            LightState::Input => "Needs input",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusUpdate {
    pub session_id: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub state: LightState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveUpdate {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatUpdate {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub project: Option<String>,
    pub title: Option<String>,
    pub state: LightState,
    #[serde(skip_serializing)]
    pub last_seen: Instant,
}

#[derive(Default)]
pub struct Store {
    sessions: RwLock<HashMap<String, SessionEntry>>,
}

impl Store {
    pub fn new() -> Arc<Self> {
        Arc::new(Store::default())
    }

    pub fn set(&self, update: StatusUpdate) -> bool {
        let mut g = self.sessions.write();
        let entry = SessionEntry {
            session_id: update.session_id.clone(),
            project: update.project,
            title: update.title,
            state: update.state,
            last_seen: Instant::now(),
        };
        let changed = match g.get(&update.session_id) {
            Some(prev) => prev.state != entry.state || prev.project != entry.project || prev.title != entry.title,
            None => true,
        };
        g.insert(update.session_id, entry);
        changed
    }

    pub fn remove(&self, session_id: &str) -> bool {
        self.sessions.write().remove(session_id).is_some()
    }

    /// 刷新 session 的 last_seen。
    /// 如果 session 不存在，创建一个默认 Done 状态的 session（用于监控器重启后心跳恢复）。
    pub fn heartbeat(&self, session_id: &str) -> bool {
        let mut g = self.sessions.write();
        if let Some(e) = g.get_mut(session_id) {
            e.last_seen = Instant::now();
            false
        } else {
            g.insert(
                session_id.to_string(),
                SessionEntry {
                    session_id: session_id.to_string(),
                    project: None,
                    title: None,
                    state: LightState::Done,
                    last_seen: Instant::now(),
                },
            );
            true
        }
    }

    /// 移除所有 last_seen 超过 timeout 的 session。
    /// 返回是否有 session 被移除。
    pub fn sweep(&self, timeout: Duration) -> bool {
        let mut g = self.sessions.write();
        let now = Instant::now();
        let before = g.len();
        g.retain(|_, e| now.duration_since(e.last_seen) < timeout);
        g.len() != before
    }

    pub fn snapshot(&self) -> Vec<SessionEntry> {
        let g = self.sessions.read();
        let mut v: Vec<SessionEntry> = g.values().cloned().collect();
        v.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        v
    }
}
