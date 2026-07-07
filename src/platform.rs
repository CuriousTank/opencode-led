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

/// 设置窗口的 _NET_WM_STATE_ABOVE，确保始终置顶。
/// 通过向 root 窗口发送 ClientMessage 实现（EWMH 规范）。
pub fn set_above(h: XHandles) {
    #[cfg(target_os = "linux")]
    unsafe {
        let display = h.display as *mut std::ffi::c_void;
        let root = XDefaultRootWindow(display);

        // 原子：_NET_WM_STATE / _NET_WM_STATE_ABOVE
        let atom_state = XInternAtom(display, b"_NET_WM_STATE\0".as_ptr() as *const i8, 0);
        let atom_above = XInternAtom(display, b"_NET_WM_STATE_ABOVE\0".as_ptr() as *const i8, 0);

        // 构造 XClientMessageEvent
        let mut event = XClientMessageEvent {
            type_: 33, // ClientMessage
            serial: 0,
            send_event: 1,
            display,
            window: h.window,
            message_type: atom_state,
            format: 32,
            data: ClientMessageData {
                l: [_NET_WM_STATE_ADD, atom_above as i64, 0, 1, 0],
            },
        };

        XSendEvent(
            display,
            root,
            0,
            (SubstructureRedirectMask | SubstructureNotifyMask) as i64,
            &mut event as *mut _ as *mut XEvent,
        );
        XFlush(display);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = h;
    }
}

#[cfg(target_os = "linux")]
const _NET_WM_STATE_ADD: i64 = 1;
#[cfg(target_os = "linux")]
const SubstructureRedirectMask: i64 = 1 << 20;
#[cfg(target_os = "linux")]
const SubstructureNotifyMask: i64 = 1 << 19;

/// XClientMessageEvent 的 data 联合体（取 long[5] 分支）
#[cfg(target_os = "linux")]
#[repr(C)]
union ClientMessageData {
    pub b: [std::ffi::c_char; 20],
    pub s: [std::ffi::c_short; 10],
    pub l: [std::ffi::c_long; 5],
}

/// 对应 XClientMessageEvent
#[cfg(target_os = "linux")]
#[repr(C)]
struct XClientMessageEvent {
    type_: std::ffi::c_int,
    serial: u64,
    send_event: std::ffi::c_int,
    display: *mut std::ffi::c_void,
    window: u64,
    message_type: u64,
    format: std::ffi::c_int,
    data: ClientMessageData,
}

/// XEvent 足够大的联合体（XClientMessageEvent 是其中最大的之一）
#[cfg(target_os = "linux")]
#[repr(C)]
union XEvent {
    pad: [std::ffi::c_long; 24],
}

/// 设置窗口的输入区域（XShape ShapeInput）。
/// 只有 `rects` 覆盖的区域接收鼠标事件，其余透明区域点击穿透到下层窗口。
pub fn set_input_region(h: XHandles, rects: &[egui::Rect], scale: f32) {
    #[cfg(target_os = "linux")]
    {
        let xrects: Vec<XRectangle> = rects
            .iter()
            .map(|r| XRectangle {
                x: (r.left().max(0.0) * scale) as i16,
                y: (r.top().max(0.0) * scale) as i16,
                width: (r.width().max(0.0) * scale) as u16,
                height: (r.height().max(0.0) * scale) as u16,
            })
            .collect();
        if xrects.is_empty() {
            return;
        }
        unsafe {
            XShapeCombineRectangles(
                h.display as *mut std::ffi::c_void,
                h.window,
                SHAPE_INPUT,
                0,
                0,
                xrects.as_ptr(),
                xrects.len() as std::ffi::c_int,
                SHAPE_SET,
                UNSORTED,
            );
            XFlush(h.display as *mut std::ffi::c_void);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (h, rects, scale);
    }
}

#[cfg(target_os = "linux")]
const SHAPE_INPUT: std::ffi::c_int = 2;
#[cfg(target_os = "linux")]
const SHAPE_SET: std::ffi::c_int = 0;
#[cfg(target_os = "linux")]
const UNSORTED: std::ffi::c_int = 0;

/// XRectangle: x/y 有符号短整数，width/height 无符号短整数
#[cfg(target_os = "linux")]
#[repr(C)]
struct XRectangle {
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

#[cfg(target_os = "linux")]
#[link(name = "Xext")]
unsafe extern "C" {
    fn XShapeCombineRectangles(
        display: *mut std::ffi::c_void,
        window: u64,
        dest_kind: std::ffi::c_int,
        x_offset: std::ffi::c_int,
        y_offset: std::ffi::c_int,
        rectangles: *const XRectangle,
        n_rects: std::ffi::c_int,
        op: std::ffi::c_int,
        ordering: std::ffi::c_int,
    );
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
    fn XDefaultRootWindow(display: *mut std::ffi::c_void) -> u64;
    fn XInternAtom(
        display: *mut std::ffi::c_void,
        atom_name: *const std::ffi::c_char,
        only_if_exists: std::ffi::c_int,
    ) -> u64;
    fn XSendEvent(
        display: *mut std::ffi::c_void,
        window: u64,
        propagate: std::ffi::c_int,
        event_mask: i64,
        event_send: *mut XEvent,
    ) -> std::ffi::c_int;
}
