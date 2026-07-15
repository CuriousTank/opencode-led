use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Running → Done 的最小持续时间。监控器端 debounce：
/// 如果一个灯泡刚变 Running 不到这么久就收到 Done，忽略它（防抖）。
/// 这解决了插件在 tool call 之间短暂 idle 导致的红绿闪烁问题。
const MIN_RUNNING_DURATION: Duration = Duration::from_secs(10);

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
    /// 记录进入 Running 状态的时刻，用于监控器端 debounce
    #[serde(skip_serializing)]
    pub running_since: Option<Instant>,
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
        let now = Instant::now();

        // 监控器端 debounce：Running → Done 降级防抖
        // 如果当前是 Running 且距离进入 Running 不到 MIN_RUNNING_DURATION，忽略 Done 降级
        if let Some(prev) = g.get(&update.session_id) {
            if prev.state == LightState::Running
                && update.state == LightState::Done
            {
                if let Some(since) = prev.running_since {
                    if now.duration_since(since) < MIN_RUNNING_DURATION {
                        // 忽略这次降级，保持原状态和标题，只更新 last_seen
                        let entry = SessionEntry {
                            session_id: update.session_id.clone(),
                            project: prev.project.clone(),
                            title: prev.title.clone(),
                            state: LightState::Running,
                            last_seen: now,
                            running_since: prev.running_since,
                        };
                        g.insert(update.session_id, entry);
                        return false; // 没有可见变化
                    }
                }
            }
        }

        let running_since = if update.state == LightState::Running {
            // 新的 Running：重置计时器（但如果已经是 Running 就保持原计时器）
            g.get(&update.session_id)
                .filter(|p| p.state == LightState::Running)
                .and_then(|p| p.running_since)
                .or(Some(now))
        } else {
            None
        };

        let entry = SessionEntry {
            session_id: update.session_id.clone(),
            project: update.project,
            title: update.title,
            state: update.state,
            last_seen: now,
            running_since,
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

    /// 刷新 session 的 last_seen（仅当 session 已存在时）。
    /// 心跳不创建新 session，只维持已有的。
    pub fn heartbeat(&self, session_id: &str) -> bool {
        let mut g = self.sessions.write();
        if let Some(e) = g.get_mut(session_id) {
            e.last_seen = Instant::now();
            true
        } else {
            false
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
