//! 窗口管理器交互：根据进程 PID 找到对应的 X11 窗口并置顶。
//!
//! 通过 EWMH 协议（_NET_CLIENT_LIST + _NET_WM_PID + _NET_ACTIVE_WINDOW）实现，
//! 不依赖 xdotool / wmctrl 等外部工具。
//!
//! opencode 跑在终端里，进程链形如：
//!   x-terminal-emulator（拥有 X11 窗口）→ bash → opencode
//! 从 opencode PID 向上爬祖先链，匹配窗口的 _NET_WM_PID。
//!
//! 多窗口消歧：同一终端进程可能开多个窗口（如 x-terminal-emulator），
//! 此时用窗口标题（_NET_WM_NAME）匹配 opencode session title 来区分。

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, CLIENT_MESSAGE_EVENT,
};

/// 根据进程 PID + session 标题，找到对应的 X11 窗口并置顶。
///
/// - `pid`: opencode 进程的 PID
/// - `session_title`: opencode session 的标题（用于多窗口消歧）
///
/// 全程优雅降级：连不上 X / 找不到窗口 → 静默无操作。
pub fn raise_window_for_pid(pid: i32, session_title: Option<&str>) {
    let pids = collect_ancestor_pids(pid);
    if pids.is_empty() {
        return;
    }

    let (conn, screen) = match x11rb::connect(None) {
        Ok(c) => c,
        Err(_) => return,
    };
    let root = conn.setup().roots[screen].root;

    let atom_list = intern(&conn, b"_NET_CLIENT_LIST");
    let atom_pid = intern(&conn, b"_NET_WM_PID");
    let atom_name = intern(&conn, b"_NET_WM_NAME");
    let atom_active = intern(&conn, b"_NET_ACTIVE_WINDOW");
    if atom_list == 0 || atom_pid == 0 || atom_active == 0 {
        return;
    }

    // 读取 _NET_CLIENT_LIST（所有被 WM 托管的顶层窗口）
    let windows = match read_client_list(&conn, root, atom_list) {
        Some(w) => w,
        None => return,
    };

    // 收集所有 _NET_WM_PID 匹配祖先链的候选窗口
    let candidates: Vec<u32> = windows
        .into_iter()
        .filter(|&win| {
            read_wm_pid(&conn, win, atom_pid)
                .map(|wm_pid| pids.contains(&(wm_pid as i32)))
                .unwrap_or(false)
        })
        .collect();

    if candidates.is_empty() {
        return;
    }

    // 单窗口直接置顶；多窗口消歧
    let target = if candidates.len() == 1 {
        candidates[0]
    } else {
        disambiguate(&conn, &candidates, atom_name, pid, session_title)
            .unwrap_or(candidates[0])
    };

    send_active_window(&conn, root, target, atom_active);
    let _ = conn.flush();
}

/// 多窗口消歧。优先用 session 标题匹配窗口标题，其次用 CWD basename。
fn disambiguate(
    conn: &x11rb::rust_connection::RustConnection,
    candidates: &[u32],
    atom_name: u32,
    pid: i32,
    session_title: Option<&str>,
) -> Option<u32> {
    // 预读所有候选窗口标题
    let named: Vec<(u32, Option<String>)> = candidates
        .iter()
        .map(|&win| (win, read_wm_name(conn, win, atom_name)))
        .collect();

    // 优先：用 session 标题前缀评分匹配（处理终端窗口标题被截断的情况）
    if let Some(title) = session_title.filter(|t| !t.is_empty()) {
        let mut best: Option<(u32, usize)> = None;
        for (win, name) in &named {
            let score = name
                .as_ref()
                .map(|n| title_prefix_score(n, title))
                .unwrap_or(0);
            if score >= 5 && best.map_or(true, |(_, s)| score > s) {
                best = Some((*win, score));
            }
        }
        if let Some((win, _)) = best {
            return Some(win);
        }
    }

    // 次选：opencode CWD 的 basename 匹配窗口标题
    if let Some(basename) = read_cwd_basename(pid) {
        let basename_lower = basename.to_lowercase();
        for (win, name) in &named {
            if name
                .as_ref()
                .map_or(false, |n| n.to_lowercase().contains(&basename_lower))
            {
                return Some(*win);
            }
        }
    }

    None
}

/// 计算窗口标题与 session 标题的最长前缀匹配分数（字符数）。
/// 终端可能截断窗口标题（如 "OC | 查找...(fork..." 不含完整标题），
/// 所以从最长前缀开始尝试，返回能匹配的最大字符数。
fn title_prefix_score(window_title: &str, session_title: &str) -> usize {
    let wt = window_title.to_lowercase();
    let st = session_title.to_lowercase();
    let mut boundaries: Vec<usize> = st.char_indices().map(|(i, _)| i).collect();
    boundaries.push(st.len());
    for i in (1..boundaries.len()).rev() {
        if wt.contains(&st[..boundaries[i]]) {
            return i;
        }
    }
    0
}

/// 读 _NET_CLIENT_LIST，返回所有顶层窗口 ID。
fn read_client_list(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    atom_list: u32,
) -> Option<Vec<u32>> {
    let reply = conn
        .get_property(false, root, atom_list, AtomEnum::WINDOW, 0, u32::MAX / 4)
        .ok()?
        .reply()
        .ok()?;
    if reply.format != 32 {
        return None;
    }
    Some(
        reply
            .value
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// 读窗口的 _NET_WM_PID。
fn read_wm_pid(conn: &x11rb::rust_connection::RustConnection, win: u32, atom_pid: u32) -> Option<u32> {
    let reply = conn
        .get_property(false, win, atom_pid, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    if reply.format == 32 && reply.value.len() >= 4 {
        Some(u32::from_ne_bytes([
            reply.value[0],
            reply.value[1],
            reply.value[2],
            reply.value[3],
        ]))
    } else {
        None
    }
}

/// 读窗口的 _NET_WM_NAME（UTF-8 字符串）。
fn read_wm_name(
    conn: &x11rb::rust_connection::RustConnection,
    win: u32,
    atom_name: u32,
) -> Option<String> {
    let reply = conn
        .get_property(false, win, atom_name, AtomEnum::ANY, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

/// 读 /proc/<pid>/cwd 的最后一级目录名（如 "opencode-traffic-light"）。
fn read_cwd_basename(pid: i32) -> Option<String> {
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()?;
    cwd.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Intern 一个 X11 原子，失败返回 0。
fn intern(conn: &x11rb::rust_connection::RustConnection, name: &[u8]) -> u32 {
    conn.intern_atom(true, name)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0)
}

/// 发送 _NET_ACTIVE_WINDOW ClientMessage 置顶目标窗口。
/// source indication = 2（pager），Mutter 视为权威请求，会跨工作区切换并置顶。
fn send_active_window(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    win: u32,
    atom_active: u32,
) {
    let mut data = [0u8; 20];
    data[0..4].copy_from_slice(&2u32.to_ne_bytes());

    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: win,
        type_: atom_active,
        data: data.into(),
    };
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let _ = conn.send_event(false, root, mask, event);
}

/// 从 pid 出发，沿 /proc/<pid>/status 的 PPid 向上收集祖先 PID（含自身）。
fn collect_ancestor_pids(mut pid: i32) -> Vec<i32> {
    let mut pids = vec![pid];
    for _ in 0..32 {
        match read_ppid(pid) {
            Some(ppid) if ppid > 1 => {
                pids.push(ppid);
                pid = ppid;
            }
            _ => break,
        }
    }
    pids
}

/// 读 /proc/<pid>/status 中的 PPid。
fn read_ppid(pid: i32) -> Option<i32> {
    let status = std::fs::read_to_string(format!("/proc/{}/status", pid)).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("PPid:").and_then(|s| s.trim().parse().ok()))
}
