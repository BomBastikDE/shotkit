//! Full virtual-desktop selection overlay with HWND/UI Automation suggestions.
//!
//! Suggestions are best effort. Win32 child HWNDs do not expose browser DOM;
//! UI Automation providers may expose only a document/canvas or no useful
//! descendants. Rectangle drag remains available for every screen region.

use std::{
    ffi::c_void,
    mem::zeroed,
    ptr::{null, null_mut},
    sync::Mutex,
};

use crate::{element_picker, uia_picker};

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
    fn UpdateWindow(hwnd: H) -> i32;
    fn GetClientRect(hwnd: H, rect: *mut Rect) -> i32;
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
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
}
#[link(name = "gdi32")]
extern "system" {
    fn DeleteObject(object: H) -> i32;
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
const VK_CONTROL: i32 = 0x11;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const LWA_ALPHA: u32 = 2;
const SW_SHOW: i32 = 5;
const IDC_CROSS: usize = 32515;
const DPI_PER_MONITOR_AWARE_V2: isize = -4;
const UIA_TIMER_ID: W = 0x534b;
const UIA_TIMER_MS: u32 = 30;
const CLICK_SLOP: i32 = 4;
const CLASS_NAME: [u16; 24] = [
    83, 104, 111, 116, 75, 105, 116, 83, 101, 108, 101, 99, 116, 105, 111, 110, 79, 118, 101, 114,
    108, 97, 121, 0,
];

#[derive(Default)]
struct State {
    hwnd: H,
    origin: (i32, i32),
    size: (i32, i32),
    down: bool,
    start: Point,
    current: Point,
    press_rect: Option<(i32, i32, i32, i32)>,
    candidates: Vec<element_picker::ElementCandidate>,
    target_hwnd: isize,
    hover_rect: Option<(i32, i32, i32, i32)>,
    visible_hover_rect: Option<(i32, i32, i32, i32)>,
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
    origin: (0, 0),
    size: (0, 0),
    down: false,
    start: Point { x: 0, y: 0 },
    current: Point { x: 0, y: 0 },
    press_rect: None,
    candidates: Vec::new(),
    target_hwnd: 0,
    hover_rect: None,
    visible_hover_rect: None,
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

fn valid_rect(rect: (i32, i32, i32, i32)) -> bool {
    rect.2 > rect.0 && rect.3 > rect.1
}

fn contains(rect: (i32, i32, i32, i32), p: Point) -> bool {
    p.x >= rect.0 && p.x < rect.2 && p.y >= rect.1 && p.y < rect.3
}

fn intersection(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let rect = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    valid_rect(rect).then_some(rect)
}

unsafe fn cursor_screen(l: L, origin: (i32, i32)) -> Point {
    let (x, y) = unpack(l);
    Point {
        x: x.saturating_add(origin.0),
        y: y.saturating_add(origin.1),
    }
}

unsafe fn is_control_down() -> bool {
    (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0
}

unsafe fn desktop_rect(s: &State) -> (i32, i32, i32, i32) {
    (
        s.origin.0,
        s.origin.1,
        s.origin.0.saturating_add(s.size.0),
        s.origin.1.saturating_add(s.size.1),
    )
}

unsafe fn root_candidate(s: &State) -> Option<(i32, i32, i32, i32)> {
    s.candidates.iter().find(|c| c.is_top_level).map(|c| c.rect)
}

unsafe fn update_hover(state: *mut State, point: Point) {
    let s = &mut *state;
    s.current = point;
    s.uia_rect = None;
    s.request_sequence = 0;
    s.window_confirmed_sequence = 0;
    if let Some(worker) = &s.window_picker {
        s.window_point = point;
        s.window_sequence = worker.submit(
            element_picker::ScreenPoint {
                x: point.x,
                y: point.y,
            },
            0,
        );
        // Keep the last drawn outline until the sequenced scan replaces it.
        // MouseDown still freezes only a rectangle containing the actual point,
        // then requires a fresh root-HWND confirmation before accepting it.
    } else {
        s.candidates.clear();
        s.target_hwnd = 0;
        s.hover_rect = None;
        s.visible_hover_rect = None;
    }
}

unsafe fn poll_window(state: *mut State) -> Option<H> {
    let scan = {
        let s = &*state;
        s.window_picker
            .as_ref()
            .and_then(|worker| worker.try_take_latest())
    }?;
    let (window_seq, point, down) = {
        let s = &*state;
        (s.window_sequence, s.window_point, s.down)
    };
    if scan.sequence != window_seq || scan.point.x != point.x || scan.point.y != point.y {
        return None;
    }
    let candidates = scan.candidates.unwrap_or_default();
    let root = candidates.iter().find(|c| c.is_top_level).copied();
    let (finish_result, should_choose) = {
        let s = &mut *state;
        s.window_confirmed_sequence = scan.sequence;
        s.window_confirmed_root = root.map(|candidate| candidate.hwnd).unwrap_or(0);
        if let Some(click) = s.pending_click {
            let matching = element_picker::confirms_click(
                click.root_hwnd,
                root,
                element_picker::ScreenPoint {
                    x: click.point.x,
                    y: click.point.y,
                },
            );
            if matching {
                s.pending_click = None;
                (Some(Some(click.rect)), false)
            } else {
                s.pending_click = None;
                (Some(None), false)
            }
        } else if down {
            (None, false)
        } else {
            s.candidates = candidates;
            s.target_hwnd = root.map(|candidate| candidate.hwnd).unwrap_or(0);
            s.uia_rect = None;
            s.request_sequence = if s.target_hwnd != 0 {
                s.uia
                    .as_ref()
                    .map(|uia| {
                        uia.submit(
                            s.target_hwnd,
                            uia_picker::ScreenPoint {
                                x: point.x,
                                y: point.y,
                            },
                        )
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            (None, true)
        }
    };
    if should_choose {
        choose_hover(state);
    }
    if let Some(result) = finish_result {
        let rect = result.and_then(|r| intersection(r, desktop_rect(&*state)));
        let hwnd = mark_finished(state, rect);
        return Some(hwnd);
    }
    Some(null_mut())
}

unsafe fn choose_hover(state: *mut State) {
    let control_down = is_control_down();
    let s = &mut *state;
    let fallback = if control_down {
        root_candidate(s)
    } else {
        s.candidates
            .iter()
            .rev()
            .find(|candidate| !candidate.is_top_level)
            .map(|candidate| candidate.rect)
            .or_else(|| root_candidate(s))
    };
    let rect = if !control_down {
        s.uia_rect.filter(|r| contains(*r, s.current)).or(fallback)
    } else {
        fallback
    };
    s.hover_rect = rect.and_then(|r| intersection(r, desktop_rect(s)));
}

unsafe fn poll_uia(state: *mut State) -> bool {
    let response = {
        let s = &*state;
        s.uia.as_ref().and_then(|uia| uia.try_take_latest())
    };
    let Some(response) = response else {
        return false;
    };
    let s = &mut *state;
    if response.sequence != s.request_sequence || response.target_hwnd != s.target_hwnd {
        return false;
    }
    s.uia_rect = response
        .rect
        .map(|r| (r.left, r.top, r.right, r.bottom))
        .filter(|r| valid_rect(*r) && contains(*r, s.current))
        .and_then(|r| {
            let root = root_candidate(s).unwrap_or(desktop_rect(s));
            intersection(r, root)
        });
    choose_hover(state);
    true
}

unsafe fn mark_finished(state: *mut State, result: Option<(i32, i32, i32, i32)>) -> H {
    let s = &mut *state;
    s.result = result.filter(|rect| valid_rect(*rect));
    s.done = true;
    let hwnd = s.hwnd;
    s.hwnd = null_mut();
    hwnd
}

unsafe extern "system" fn window_proc(hwnd: H, msg: u32, w: W, l: L) -> L {
    let state = std::ptr::addr_of_mut!(STATE);
    match msg {
        WM_ERASEBKGND => 1,
        WM_LBUTTONDOWN => {
            let point = cursor_screen(l, (*state).origin);
            let should_capture = {
                let s = &mut *state;
                if s.down {
                    false
                } else {
                    s.down = true;
                    s.start = point;
                    s.current = point;
                    // Freeze the rectangle that was already visible. In-flight
                    // UIA completions cannot change the accepted click target.
                    s.press_rect = s.visible_hover_rect.filter(|rect| contains(*rect, point));
                    s.pending_click = None;
                    s.window_point = point;
                    s.window_confirmed_sequence = 0;
                    s.window_confirmed_root = 0;
                    s.window_sequence = s
                        .window_picker
                        .as_ref()
                        .map(|worker| {
                            worker.submit(
                                element_picker::ScreenPoint {
                                    x: point.x,
                                    y: point.y,
                                },
                                0,
                            )
                        })
                        .unwrap_or(0);
                    true
                }
            };
            if should_capture {
                SetCapture(hwnd);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEMOVE => {
            let point = cursor_screen(l, (*state).origin);
            let is_down = (*state).down;
            if is_down {
                (*state).current = point;
            } else {
                update_hover(state, point);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP if (*state).down => {
            let point = cursor_screen(l, (*state).origin);
            let (result, click_pending, click_cancelled, root_hwnd) = {
                let s = &mut *state;
                s.current = point;
                let dx = (i64::from(s.start.x) - i64::from(s.current.x)).abs();
                let dy = (i64::from(s.start.y) - i64::from(s.current.y)).abs();
                let click = dx <= i64::from(CLICK_SLOP) && dy <= i64::from(CLICK_SLOP);
                let root_hwnd = s
                    .candidates
                    .iter()
                    .find(|c| c.is_top_level)
                    .map(|c| c.hwnd)
                    .unwrap_or(0);
                let result = if click {
                    s.press_rect
                } else {
                    Some((
                        s.start.x.min(s.current.x),
                        s.start.y.min(s.current.y),
                        s.start.x.max(s.current.x),
                        s.start.y.max(s.current.y),
                    ))
                };
                s.down = false;
                let fresh_scan =
                    s.window_confirmed_sequence == s.window_sequence && s.window_sequence != 0;
                let fresh = fresh_scan && s.window_confirmed_root == root_hwnd;
                let valid_click = click && result.is_some_and(|r| contains(r, point));
                let pending = valid_click && !fresh_scan && root_hwnd != 0;
                let cancelled = click
                    && (!valid_click || (fresh_scan && !fresh) || (!fresh_scan && root_hwnd == 0));
                if pending {
                    s.pending_click = Some(PendingClick {
                        rect: result.unwrap(),
                        point,
                        root_hwnd,
                    });
                }
                (result, pending, cancelled, root_hwnd)
            };
            ReleaseCapture();
            if !click_pending {
                let final_result = if click_cancelled { None } else { result }
                    .and_then(|r| intersection(r, desktop_rect(&*state)));
                let target = mark_finished(state, final_result);
                if !target.is_null() {
                    let _ = KillTimer(target, UIA_TIMER_ID);
                    DestroyWindow(target);
                }
            }
            let _ = root_hwnd;
            0
        }
        WM_KEYDOWN if w == VK_ESCAPE => {
            let target = mark_finished(state, None);
            if !target.is_null() {
                let _ = KillTimer(target, UIA_TIMER_ID);
                DestroyWindow(target);
            }
            0
        }
        WM_KEYDOWN | WM_KEYUP if w == VK_CONTROL as W => {
            choose_hover(state);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if w == UIA_TIMER_ID => {
            let scan_handle = poll_window(state);
            if (*state).done {
                let target = scan_handle
                    .filter(|h| !h.is_null())
                    .unwrap_or((*state).hwnd);
                if !target.is_null() {
                    let _ = KillTimer(target, UIA_TIMER_ID);
                    DestroyWindow(target);
                }
            } else if scan_handle.is_some()
                || (!(*state).down && (*state).pending_click.is_none() && poll_uia(state))
            {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_PAINT => {
            let (screen_rect, desktop) = {
                let s = &*state;
                let screen_rect = if s.down {
                    Some((
                        s.start.x.min(s.current.x),
                        s.start.y.min(s.current.y),
                        s.start.x.max(s.current.x),
                        s.start.y.max(s.current.y),
                    ))
                } else if let Some(click) = s.pending_click {
                    Some(click.rect)
                } else {
                    s.hover_rect
                };
                (screen_rect, desktop_rect(s))
            };
            let mut paint: Paint = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut client: Rect = zeroed();
            let _ = GetClientRect(hwnd, &mut client);
            let bg = CreateSolidBrush(0x0000_0000);
            let _ = FillRect(dc, &client, bg);
            let _ = DeleteObject(bg);

            let screen_rect = screen_rect.and_then(|r| intersection(r, desktop));
            if let Some(screen) = screen_rect {
                let origin = (*state).origin;
                let local = Rect {
                    left: screen.0.saturating_sub(origin.0),
                    top: screen.1.saturating_sub(origin.1),
                    right: screen.2.saturating_sub(origin.0),
                    bottom: screen.3.saturating_sub(origin.1),
                };
                let down = (*state).down;
                let brush = CreateSolidBrush(if down { 0x00ff_ffff } else { 0x0000_ff00 });
                let _ = FrameRect(dc, &local, brush);
                let _ = DeleteObject(brush);
            }
            let _ = EndPaint(hwnd, &paint);
            if !(*state).down {
                (*state).visible_hover_rect = screen_rect;
            }
            0
        }
        WM_DESTROY => {
            let timer_hwnd = {
                let s = &mut *state;
                s.done = true;
                if s.hwnd == hwnd {
                    s.hwnd = null_mut();
                }
                hwnd
            };
            let _ = KillTimer(timer_hwnd, UIA_TIMER_ID);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn unpack(l: L) -> (i32, i32) {
    (
        (l as u16 as i16) as i32,
        (((l as usize >> 16) & 0xffff) as u16 as i16) as i32,
    )
}

/// Display the virtual-desktop overlay. Drag selects any rectangle. A click
/// selects the current child/UIA suggestion; Ctrl-click selects the containing
/// top-level window. Esc cancels. UI Automation queries run off the UI thread.
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
    std::ptr::addr_of_mut!(STATE).write(State {
        hwnd: null_mut(),
        origin,
        size: (width, height),
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
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(hwnd);
    UpdateWindow(hwnd);
    let _ = SetTimer(hwnd, UIA_TIMER_ID, UIA_TIMER_MS, None);
    let mut cursor = Point::default();
    if GetCursorPos(&mut cursor) != 0 {
        update_hover(std::ptr::addr_of_mut!(STATE), cursor);
        InvalidateRect(hwnd, null(), 0);
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
    ReleaseCapture();
    UnregisterClassW(CLASS_NAME.as_ptr(), instance);
    let state = &mut *std::ptr::addr_of_mut!(STATE);
    let result = state.result;
    let picker = state.uia.take();
    drop(picker);
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
