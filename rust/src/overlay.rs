//! Full virtual-desktop selection overlay with HWND/UI Automation suggestions.
//!
//! Suggestions are best effort. Win32 child HWNDs do not expose browser DOM;
//! UI Automation providers may expose only a document/canvas or no useful
//! descendants. Rectangle drag remains available for every screen region.
//! For keyboard correction, hover a suggested target and press an arrow key;
//! arrows move it by one screen pixel and Shift+arrows resize its right/bottom
//! edge by one pixel. Enter accepts. To correct a manually drawn rectangle,
//! hold Shift while releasing the drag, then use the same keys. Pointer motion
//! resumes hover mode, a new mouse gesture starts the usual flow, and Esc cancels.

mod hover;
use hover::*;
mod magnifier;
use magnifier::*;
mod window;
use window::*;
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::Mutex,
};

use crate::{element_picker, uia_picker};
use shotkit_rust_prototype::selection_rect;

type H = *mut c_void;
type W = usize;
type L = isize;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct Msg {
    hwnd: H,
    message: u32,
    wparam: W,
    lparam: L,
    time: u32,
    pt: Point,
    private: u32,
}
#[repr(C)]
struct WndClass {
    style: u32,
    proc: Option<unsafe extern "system" fn(H, u32, W, L) -> L>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: H,
    icon: H,
    cursor: H,
    background: H,
    menu: *const u16,
    class: *const u16,
}
#[repr(C)]
struct Paint {
    dc: H,
    erase: i32,
    rect: Rect,
    restore: i32,
    inc: i32,
    reserved: [u8; 32],
}
#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn UnregisterClassW(name: *const u16, instance: H) -> i32;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: H,
        menu: H,
        instance: H,
        param: *mut c_void,
    ) -> H;
    fn DefWindowProcW(hwnd: H, msg: u32, w: W, l: L) -> L;
    fn DestroyWindow(hwnd: H) -> i32;
    fn ShowWindow(hwnd: H, cmd: i32) -> i32;
    fn MoveWindow(hwnd: H, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
    fn UpdateWindow(hwnd: H) -> i32;
    fn GetClientRect(hwnd: H, rect: *mut Rect) -> i32;
    fn GetDC(hwnd: H) -> H;
    fn ReleaseDC(hwnd: H, dc: H) -> i32;
    fn SetForegroundWindow(hwnd: H) -> i32;
    fn SetFocus(hwnd: H) -> H;
    fn GetMessageW(msg: *mut Msg, hwnd: H, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> L;
    fn PostQuitMessage(code: i32);
    fn SetCapture(hwnd: H) -> H;
    fn ReleaseCapture() -> i32;
    fn InvalidateRect(hwnd: H, rect: *const Rect, erase: i32) -> i32;
    fn BeginPaint(hwnd: H, paint: *mut Paint) -> H;
    fn EndPaint(hwnd: H, paint: *const Paint) -> i32;
    fn FillRect(dc: H, rect: *const Rect, brush: H) -> i32;
    fn FrameRect(dc: H, rect: *const Rect, brush: H) -> i32;
    fn CreateSolidBrush(color: u32) -> H;
    fn SetLayeredWindowAttributes(hwnd: H, key: u32, alpha: u8, flags: u32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn LoadCursorW(instance: H, name: *const u16) -> H;
    fn SetThreadDpiAwarenessContext(context: H) -> H;
    fn SetTimer(
        hwnd: H,
        id: W,
        elapsed: u32,
        callback: Option<unsafe extern "system" fn(H, u32, W, u32)>,
    ) -> W;
    fn KillTimer(hwnd: H, id: W) -> i32;
    fn GetKeyState(key: i32) -> i16;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn MonitorFromPoint(point: Point, flags: u32) -> H;
    fn GetMonitorInfoW(monitor: H, info: *mut MonitorInfo) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
}
#[link(name = "gdi32")]
extern "system" {
    fn DeleteObject(object: H) -> i32;
    fn CreateCompatibleDC(dc: H) -> H;
    fn CreateCompatibleBitmap(dc: H, width: i32, height: i32) -> H;
    fn SelectObject(dc: H, object: H) -> H;
    fn DeleteDC(dc: H) -> i32;
    fn BitBlt(dst: H, x: i32, y: i32, width: i32, height: i32, src: H, sx: i32, sy: i32, rop: u32) -> i32;
    fn StretchBlt(dst: H, x: i32, y: i32, width: i32, height: i32, src: H, sx: i32, sy: i32, sw: i32, sh: i32, rop: u32) -> i32;
    fn SetStretchBltMode(dc: H, mode: i32) -> i32;
    fn CreatePen(style: i32, width: i32, color: u32) -> H;
    fn Rectangle(dc: H, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn GetPixel(dc: H, x: i32, y: i32) -> u32;
    fn SetTextColor(dc: H, color: u32) -> u32;
    fn SetBkMode(dc: H, mode: i32) -> i32;
    fn TextOutW(dc: H, x: i32, y: i32, text: *const u16, length: i32) -> i32;
    fn GetStockObject(index: i32) -> H;
}

const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000f;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_TIMER: u32 = 0x0113;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEMOVE: u32 = 0x0200;
const VK_ESCAPE: W = 0x1b;
const VK_RETURN: W = 0x0d;
const VK_CONTROL: i32 = 0x11;
const VK_SHIFT: i32 = 0x10;
const VK_LEFT: W = 0x25;
const VK_UP: W = 0x26;
const VK_RIGHT: W = 0x27;
const VK_DOWN: W = 0x28;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const LWA_ALPHA: u32 = 2;
const SW_SHOW: i32 = 5;
const SW_SHOWNOACTIVATE: i32 = 4;
const IDC_CROSS: usize = 32515;
const DPI_PER_MONITOR_AWARE_V2: isize = -4;
const UIA_TIMER_ID: W = 0x534b;
const UIA_TIMER_MS: u32 = 30;
const CLICK_SLOP: i32 = 4;
const MAG_PIXELS: i32 = 15;
const MAG_ZOOM: i32 = 8;
const MAG_SIZE: i32 = MAG_PIXELS * MAG_ZOOM;
const MAG_LABEL_HEIGHT: i32 = 24;
const MAG_WINDOW_HEIGHT: i32 = MAG_SIZE + MAG_LABEL_HEIGHT + 2;
const MAX_MAGNIFIER_SNAPSHOT_BYTES: i64 = 256 * 1024 * 1024;
const COLORONCOLOR: i32 = 3;
const SRCCOPY: u32 = 0x00cc_0020;
const CAPTUREBLT: u32 = 0x4000_0000;
const MONITOR_DEFAULTTONEAREST: u32 = 2;
const DEFAULT_GUI_FONT: i32 = 17;
const PS_SOLID: i32 = 0;
const TRANSPARENT: i32 = 1;
const WM_NCHITTEST: u32 = 0x0084;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const HTTRANSPARENT: L = -1;
const MA_NOACTIVATE: L = 3;
const CLASS_NAME: [u16; 24] = [
    83, 104, 111, 116, 75, 105, 116, 83, 101, 108, 101, 99, 116, 105, 111, 110, 79, 118, 101, 114,
    108, 97, 121, 0,
];

#[derive(Default)]
struct State {
    hwnd: H,
    magnifier_hwnd: H,
    magnifier_class_registered: bool,
    origin: (i32, i32),
    size: (i32, i32),
    source_dc: H,
    source_bitmap: H,
    source_original: H,
    down: bool,
    start: Point,
    current: Point,
    press_rect: Option<(i32, i32, i32, i32)>,
    candidates: Vec<element_picker::ElementCandidate>,
    target_hwnd: isize,
    hover_rect: Option<(i32, i32, i32, i32)>,
    visible_hover_rect: Option<(i32, i32, i32, i32)>,
    /// Keyboard edits are isolated from asynchronous hover/UIA updates.
    keyboard_rect: Option<(i32, i32, i32, i32)>,
    uia_rect: Option<(i32, i32, i32, i32)>,
    request_sequence: u64,
    window_picker: Option<element_picker::WindowPicker>,
    window_sequence: u64,
    window_point: Point,
    window_confirmed_sequence: u64,
    window_confirmed_root: isize,
    pending_click: Option<PendingClick>,
    uia: Option<uia_picker::UiaPicker>,
    result: Option<(i32, i32, i32, i32)>,
    done: bool,
}

#[derive(Clone, Copy)]
struct PendingClick {
    rect: (i32, i32, i32, i32),
    point: Point,
    root_hwnd: isize,
}

static mut STATE: State = State {
    hwnd: null_mut(),
    magnifier_hwnd: null_mut(),
    magnifier_class_registered: false,
    origin: (0, 0),
    size: (0, 0),
    source_dc: null_mut(),
    source_bitmap: null_mut(),
    source_original: null_mut(),
    down: false,
    start: Point { x: 0, y: 0 },
    current: Point { x: 0, y: 0 },
    press_rect: None,
    candidates: Vec::new(),
    target_hwnd: 0,
    hover_rect: None,
    visible_hover_rect: None,
    keyboard_rect: None,
    uia_rect: None,
    request_sequence: 0,
    window_picker: None,
    window_sequence: 0,
    window_point: Point { x: 0, y: 0 },
    window_confirmed_sequence: 0,
    window_confirmed_root: 0,
    pending_click: None,
    uia: None,
    result: None,
    done: false,
};
static SELECTION_LOCK: Mutex<()> = Mutex::new(());

fn unpack(l: L) -> (i32, i32) {
    (
        (l as u16 as i16) as i32,
        (((l as usize >> 16) & 0xffff) as u16 as i16) as i32,
    )
}

/// Display the virtual-desktop overlay. Drag selects any rectangle. A click
/// selects the current child/UIA suggestion; Ctrl-click selects the containing
/// top-level window. Hold Shift on drag release to pause for keyboard correction:
/// arrows move by one pixel, Shift+arrows resize the right/bottom edge, Enter
/// accepts, pointer motion resumes hover, and Esc cancels. UI Automation queries
/// run off the UI thread.
///
/// The returned rectangle is `(left, top, width, height)` in screen pixels.
pub unsafe fn select() -> Option<(i32, i32, i32, i32)> {
    let _guard = SELECTION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous_dpi = SetThreadDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2 as H);
    let origin = (GetSystemMetrics(76), GetSystemMetrics(77));
    let width = GetSystemMetrics(78);
    let height = GetSystemMetrics(79);
    if width <= 0 || height <= 0 {
        if !previous_dpi.is_null() {
            SetThreadDpiAwarenessContext(previous_dpi);
        }
        return None;
    }
    let instance = GetModuleHandleW(null());
    let class = WndClass {
        style: 0,
        proc: Some(window_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: null_mut(),
        cursor: LoadCursorW(null_mut(), IDC_CROSS as *const u16),
        background: null_mut(),
        menu: null(),
        class: CLASS_NAME.as_ptr(),
    };
    if RegisterClassW(&class) == 0 {
        if !previous_dpi.is_null() {
            SetThreadDpiAwarenessContext(previous_dpi);
        }
        return None;
    }
    let (source_dc, source_bitmap, source_original) = capture_desktop(origin, (width, height));
    std::ptr::addr_of_mut!(STATE).write(State {
        hwnd: null_mut(),
        magnifier_hwnd: null_mut(),
        magnifier_class_registered: false,
        origin,
        size: (width, height),
        source_dc,
        source_bitmap,
        source_original,
        down: false,
        start: Point::default(),
        current: Point::default(),
        candidates: Vec::new(),
        press_rect: None,
        target_hwnd: 0,
        hover_rect: None,
        uia_rect: None,
        request_sequence: 0,
        window_picker: element_picker::WindowPicker::new().ok(),
        window_sequence: 0,
        window_point: Point::default(),
        window_confirmed_sequence: 0,
        window_confirmed_root: 0,
        pending_click: None,
        visible_hover_rect: None,
        keyboard_rect: None,
        uia: uia_picker::UiaPicker::new().ok(),
        result: None,
        done: false,
    });
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        CLASS_NAME.as_ptr(),
        CLASS_NAME.as_ptr(),
        WS_POPUP,
        origin.0,
        origin.1,
        width,
        height,
        null_mut(),
        null_mut(),
        instance,
        null_mut(),
    );
    if hwnd.is_null() {
        release_desktop(std::ptr::addr_of_mut!(STATE));
        let state = &mut *std::ptr::addr_of_mut!(STATE);
        drop(state.uia.take());
        std::ptr::addr_of_mut!(STATE).write(State::default());
        UnregisterClassW(CLASS_NAME.as_ptr(), instance);
        if !previous_dpi.is_null() {
            SetThreadDpiAwarenessContext(previous_dpi);
        }
        return None;
    }
    (*std::ptr::addr_of_mut!(STATE)).hwnd = hwnd;
    SetLayeredWindowAttributes(hwnd, 0, 118, LWA_ALPHA);
    let mut cursor = Point::default();
    let have_cursor = GetCursorPos(&mut cursor) != 0;
    if have_cursor {
        (*std::ptr::addr_of_mut!(STATE)).current = cursor;
    }

    let magnifier_class_name: Vec<u16> = "ShotKitSelectionMagnifier"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let magnifier_class = WndClass {
        style: 0,
        proc: Some(magnifier_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: null_mut(),
        cursor: null_mut(),
        background: null_mut(),
        menu: null(),
        class: magnifier_class_name.as_ptr(),
    };
    if !(*std::ptr::addr_of!(STATE)).source_dc.is_null() && RegisterClassW(&magnifier_class) != 0 {
        (*std::ptr::addr_of_mut!(STATE)).magnifier_class_registered = true;
        let (left, top, _, _) = magnifier_rect(&*std::ptr::addr_of!(STATE), cursor);
        let magnifier = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            magnifier_class_name.as_ptr(),
            magnifier_class_name.as_ptr(),
            WS_POPUP,
            left.saturating_add(origin.0),
            top.saturating_add(origin.1),
            MAG_SIZE,
            MAG_WINDOW_HEIGHT,
            hwnd,
            null_mut(),
            instance,
            null_mut(),
        );
        if !magnifier.is_null() {
            (*std::ptr::addr_of_mut!(STATE)).magnifier_hwnd = magnifier;
        } else {
            UnregisterClassW(magnifier_class_name.as_ptr(), instance);
            (*std::ptr::addr_of_mut!(STATE)).magnifier_class_registered = false;
        }
    }
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(hwnd);
    let magnifier = (*std::ptr::addr_of!(STATE)).magnifier_hwnd;
    if !magnifier.is_null() {
        ShowWindow(magnifier, SW_SHOWNOACTIVATE);
    }
    UpdateWindow(hwnd);
    let _ = SetTimer(hwnd, UIA_TIMER_ID, UIA_TIMER_MS, None);
    if have_cursor {
        update_hover(std::ptr::addr_of_mut!(STATE), cursor);
        InvalidateRect(hwnd, null(), 0);
        move_magnifier(std::ptr::addr_of_mut!(STATE));
    }

    let mut msg: Msg = zeroed();
    let mut quit_code = None;
    loop {
        let r = GetMessageW(&mut msg, null_mut(), 0, 0);
        if r == 0 {
            (*std::ptr::addr_of_mut!(STATE)).done = true;
            quit_code = Some(msg.wparam as i32);
            break;
        }
        if r < 0 {
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if (*std::ptr::addr_of!(STATE)).done {
            break;
        }
    }
    if !(*std::ptr::addr_of!(STATE)).hwnd.is_null() {
        let active = (*std::ptr::addr_of!(STATE)).hwnd;
        let _ = KillTimer(active, UIA_TIMER_ID);
        DestroyWindow(active);
    }
    let magnifier = (*std::ptr::addr_of!(STATE)).magnifier_hwnd;
    if !magnifier.is_null() {
        DestroyWindow(magnifier);
        (*std::ptr::addr_of_mut!(STATE)).magnifier_hwnd = null_mut();
    }
    ReleaseCapture();
    UnregisterClassW(CLASS_NAME.as_ptr(), instance);
    if (*std::ptr::addr_of!(STATE)).magnifier_class_registered {
        UnregisterClassW(magnifier_class_name.as_ptr(), instance);
        (*std::ptr::addr_of_mut!(STATE)).magnifier_class_registered = false;
    }
    let state = &mut *std::ptr::addr_of_mut!(STATE);
    let result = state.result;
    let picker = state.uia.take();
    drop(picker);
    release_desktop(std::ptr::addr_of_mut!(STATE));
    std::ptr::addr_of_mut!(STATE).write(State::default());
    if !previous_dpi.is_null() {
        SetThreadDpiAwarenessContext(previous_dpi);
    }
    if let Some(code) = quit_code {
        PostQuitMessage(code);
    }
    result.and_then(|(left, top, right, bottom)| {
        let width = i32::try_from(i64::from(right) - i64::from(left)).ok()?;
        let height = i32::try_from(i64::from(bottom) - i64::from(top)).ok()?;
        Some((left, top, width, height))
    })
}
