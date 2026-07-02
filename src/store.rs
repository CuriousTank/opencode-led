use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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
    pub state: LightState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveUpdate {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub project: Option<String>,
    pub state: LightState,
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
            state: update.state,
        };
        let changed = match g.get(&update.session_id) {
            Some(prev) => prev.state != entry.state || prev.project != entry.project,
            None => true,
        };
        g.insert(update.session_id, entry);
        changed
    }

    pub fn remove(&self, session_id: &str) -> bool {
        self.sessions.write().remove(session_id).is_some()
    }

    pub fn snapshot(&self) -> Vec<SessionEntry> {
        let g = self.sessions.read();
        let mut v: Vec<SessionEntry> = g.values().cloned().collect();
        v.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        v
    }
}
