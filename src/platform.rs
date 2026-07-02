//! X11 手动窗口拖拽。
//!
//! 不依赖窗口管理器的 `_NET_WM_MOVERESIZE`（Mutter 在某些配置下不响应），
//! 也不依赖 egui 的 `StartDrag`（X11/glow 有 bug 劫持全局鼠标）。
//!
//! 改用最直接的方式：每帧用 `XQueryPointer` 取鼠标根坐标，
//! 用 `XMoveWindow` 直接移动窗口到「鼠标 - 初始抓取偏移」。
//! 完全绕过 WM，稳定无抖动。

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

/// 当前 app 持有的 X11 连接与窗口句柄。首次拖拽时初始化。
#[derive(Clone, Copy)]
pub struct XHandles {
    display: usize,
    window: u64,
}

/// 从 eframe Frame 提取 X11 句柄
pub fn extract_handles(frame: &eframe::Frame) -> Option<XHandles> {
    let raw_win = frame.window_handle().ok()?.as_raw();
    let raw_disp = frame.display_handle().ok()?.as_raw();
    #[cfg(target_os = "linux")]
    if let (RawDisplayHandle::Xlib(dh), RawWindowHandle::Xlib(wh)) = (raw_disp, raw_win) {
        let display = dh.display.map(|p| p.as_ptr() as usize).unwrap_or(0);
        if display != 0 {
            return Some(XHandles {
                display,
                window: wh.window as u64,
            });
        }
    }
    let _ = (raw_win, raw_disp);
    None
}

/// 取鼠标坐标。返回 (root_x, root_y, win_x, win_y)：
/// - root 坐标 = 屏幕全局坐标
/// - win 坐标 = 鼠标相对目标窗口左上角的坐标（拖拽偏移用，坐标系一致最稳）
pub fn query_pointer(h: XHandles) -> Option<(i32, i32, i32, i32)> {
    #[cfg(not(target_os = "linux"))]
    {
        return None;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        let display = h.display as *mut std::ffi::c_void;
        let mut r: u64 = 0;
        let mut c: u64 = 0;
        let mut rx: std::ffi::c_int = 0;
        let mut ry: std::ffi::c_int = 0;
        let mut wx: std::ffi::c_int = 0;
        let mut wy: std::ffi::c_int = 0;
        let mut mask: std::ffi::c_uint = 0;
        let ok = XQueryPointer(
            display, h.window, &mut r, &mut c, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask,
        );
        if ok != 0 {
            Some((rx as i32, ry as i32, wx as i32, wy as i32))
        } else {
            None
        }
    }
}

/// 取鼠标在根窗口中的坐标 (root_x, root_y)。拖拽用。
pub fn query_pointer_root(h: XHandles) -> Option<(i32, i32)> {
    query_pointer(h).map(|(rx, ry, _, _)| (rx, ry))
}

/// 直接移动窗口到根坐标 (x, y)。
pub fn move_window(h: XHandles, x: i32, y: i32) {
    #[cfg(target_os = "linux")]
    unsafe {
        XMoveWindow(h.display as *mut std::ffi::c_void, h.window, x, y);
        XFlush(h.display as *mut std::ffi::c_void);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (h, x, y);
    }
}

#[cfg(target_os = "linux")]
#[link(name = "X11")]
unsafe extern "C" {
    fn XQueryPointer(
        display: *mut std::ffi::c_void,
        window: u64,
        root_return: *mut u64,
        child_return: *mut u64,
        root_x_return: *mut std::ffi::c_int,
        root_y_return: *mut std::ffi::c_int,
        win_x_return: *mut std::ffi::c_int,
        win_y_return: *mut std::ffi::c_int,
        mask_return: *mut std::ffi::c_uint,
    ) -> std::ffi::c_int;
    fn XMoveWindow(
        display: *mut std::ffi::c_void,
        window: u64,
        x: std::ffi::c_int,
        y: std::ffi::c_int,
    ) -> std::ffi::c_int;
    fn XFlush(display: *mut std::ffi::c_void) -> std::ffi::c_int;
}
