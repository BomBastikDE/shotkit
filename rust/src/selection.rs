//! Full virtual-desktop selection overlay with HWND/UI Automation suggestions.
//!
//! Suggestions are best effort. Win32 child HWNDs do not expose browser DOM;
//! UI Automation providers may expose only a document/canvas or no useful
//! descendants. Rectangle drag remains available for every screen region.

use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
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
const VK_CONTROL: i32 = 0x11;
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

unsafe fn capture_desktop(origin: (i32, i32), size: (i32, i32)) -> (H, H, H) {
    let estimated_bytes = i64::from(size.0)
        .checked_mul(i64::from(size.1))
        .and_then(|pixels| pixels.checked_mul(4));
    if estimated_bytes.map_or(true, |bytes| bytes > MAX_MAGNIFIER_SNAPSHOT_BYTES) {
        return (null_mut(), null_mut(), null_mut());
    }
    let screen_dc = GetDC(null_mut());
    if screen_dc.is_null() {
        return (null_mut(), null_mut(), null_mut());
    }
    let memory_dc = CreateCompatibleDC(screen_dc);
    if memory_dc.is_null() {
        ReleaseDC(null_mut(), screen_dc);
        return (null_mut(), null_mut(), null_mut());
    }
    let bitmap = CreateCompatibleBitmap(screen_dc, size.0, size.1);
    if bitmap.is_null() {
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);
        return (null_mut(), null_mut(), null_mut());
    }
    let original = SelectObject(memory_dc, bitmap);
    if original.is_null()
        || original as isize == -1
        || BitBlt(
            memory_dc,
            0,
            0,
            size.0,
            size.1,
            screen_dc,
            origin.0,
            origin.1,
            SRCCOPY | CAPTUREBLT,
        ) == 0
    {
        if !original.is_null() && original as isize != -1 {
            SelectObject(memory_dc, original);
        }
        DeleteObject(bitmap);
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);
        return (null_mut(), null_mut(), null_mut());
    }
    ReleaseDC(null_mut(), screen_dc);
    (memory_dc, bitmap, original)
}

unsafe fn release_desktop(state: *mut State) {
    let s = &mut *state;
    if !s.source_dc.is_null() {
        if !s.source_original.is_null() {
            SelectObject(s.source_dc, s.source_original);
        }
        if !s.source_bitmap.is_null() {
            DeleteObject(s.source_bitmap);
        }
        DeleteDC(s.source_dc);
    }
    s.source_dc = null_mut();
    s.source_bitmap = null_mut();
    s.source_original = null_mut();
}

unsafe fn monitor_local_rect(point: Point, origin: (i32, i32), desktop: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
    if !monitor.is_null() {
        let mut info: MonitorInfo = zeroed();
        info.size = size_of::<MonitorInfo>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            return (
                info.monitor.left.saturating_sub(origin.0).max(desktop.0),
                info.monitor.top.saturating_sub(origin.1).max(desktop.1),
                info.monitor.right.saturating_sub(origin.0).min(desktop.2),
                info.monitor.bottom.saturating_sub(origin.1).min(desktop.3),
            );
        }
    }
    desktop
}

unsafe fn magnifier_rect(state: &State, point: Point) -> (i32, i32, i32, i32) {
    let desktop = desktop_rect(state);
    let local_cursor = Point {
        x: point.x.saturating_sub(state.origin.0),
        y: point.y.saturating_sub(state.origin.1),
    };
    let bounds = monitor_local_rect(point, state.origin, desktop);
    let mut x = local_cursor.x.saturating_add(24);
    let mut y = local_cursor.y.saturating_add(24);
    if x.saturating_add(MAG_SIZE) > bounds.2 {
        x = local_cursor.x.saturating_sub(24 + MAG_SIZE);
    }
    if y.saturating_add(MAG_SIZE + MAG_LABEL_HEIGHT + 2) > bounds.3 {
        y = local_cursor.y.saturating_sub(24 + MAG_SIZE + MAG_LABEL_HEIGHT + 2);
    }
    let max_x = bounds.2.saturating_sub(MAG_SIZE).max(bounds.0);
    let max_y = bounds.3.saturating_sub(MAG_SIZE + MAG_LABEL_HEIGHT + 2).max(bounds.1);
    x = x.clamp(bounds.0, max_x);
    y = y.clamp(bounds.1, max_y);
    (x, y, x.saturating_add(MAG_SIZE), y.saturating_add(MAG_SIZE))
}

fn magnifier_source_rect(state: &State, point: Point) -> (i32, i32, i32, i32) {
    let local_x = point.x.saturating_sub(state.origin.0);
    let local_y = point.y.saturating_sub(state.origin.1);
    let width = state.size.0.min(MAG_PIXELS).max(1);
    let height = state.size.1.min(MAG_PIXELS).max(1);
    let max_x = state.size.0.saturating_sub(width).max(0);
    let max_y = state.size.1.saturating_sub(height).max(0);
    let left = local_x.saturating_sub(width / 2).clamp(0, max_x);
    let top = local_y.saturating_sub(height / 2).clamp(0, max_y);
    (left, top, width, height)
}

unsafe fn draw_magnifier(dc: H, state: &State) {
    if state.source_dc.is_null() {
        return;
    }
    let point = state.current;
    let local_cursor = Point {
        x: point.x.saturating_sub(state.origin.0),
        y: point.y.saturating_sub(state.origin.1),
    };
    let (source_x, source_y, source_width, source_height) = magnifier_source_rect(state, point);
    let left = 0;
    let top = 0;
    let right = MAG_SIZE;
    let bottom = MAG_SIZE;
    let _ = FillRect(dc, &Rect { left, top, right, bottom }, GetStockObject(4)); // BLACK_BRUSH
    SetStretchBltMode(dc, COLORONCOLOR);
    StretchBlt(
        dc,
        left,
        top,
        MAG_SIZE,
        MAG_SIZE,
        state.source_dc,
        source_x,
        source_y,
        source_width,
        source_height,
        SRCCOPY,
    );

    let white_pen = CreatePen(PS_SOLID, 1, 0x00ff_ffff);
    let old_pen = if white_pen.is_null() { null_mut() } else { SelectObject(dc, white_pen) };
    let marker_x = local_cursor.x.saturating_sub(source_x).clamp(0, source_width - 1);
    let marker_y = local_cursor.y.saturating_sub(source_y).clamp(0, source_height - 1);
    let center_x = marker_x * MAG_SIZE / source_width;
    let center_y = marker_y * MAG_SIZE / source_height;
    let marker_width = (MAG_SIZE / source_width).max(1);
    let marker_height = (MAG_SIZE / source_height).max(1);
    Rectangle(dc, left + center_x, top + center_y, left + center_x + marker_width + 1, top + center_y + marker_height + 1);
    let accent_pen = CreatePen(PS_SOLID, 2, 0x00ff_9600);
    if !accent_pen.is_null() {
        let previous_pen = SelectObject(dc, accent_pen);
        Rectangle(dc, left, top, right, bottom);
        if !previous_pen.is_null() && previous_pen as isize != -1 {
            SelectObject(dc, previous_pen);
        }
        DeleteObject(accent_pen);
    }
    if !old_pen.is_null() && old_pen as isize != -1 {
        SelectObject(dc, old_pen);
    }
    if !white_pen.is_null() {
        DeleteObject(white_pen);
    }

    let pixel = GetPixel(state.source_dc, local_cursor.x, local_cursor.y);
    let label = if pixel == 0xffff_ffff {
        format!("{}, {}", point.x, point.y)
    } else {
        format!("{}, {}  #{:02X}{:02X}{:02X}", point.x, point.y, pixel & 0xff, (pixel >> 8) & 0xff, (pixel >> 16) & 0xff)
    };
    let wide: Vec<u16> = label.encode_utf16().collect();
    let text_rect = Rect { left, top: MAG_SIZE + 2, right: MAG_SIZE, bottom: MAG_WINDOW_HEIGHT };
    let dark = CreateSolidBrush(0x0014_1414);
    if !dark.is_null() {
        FillRect(dc, &text_rect, dark);
        DeleteObject(dark);
    }
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let old_font = if !font.is_null() { SelectObject(dc, font) } else { null_mut() };
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, 0x00ff_ffff);
    TextOutW(dc, 3, MAG_SIZE + 4, wide.as_ptr(), wide.len() as i32);
    if !old_font.is_null() { SelectObject(dc, old_font); }
}

unsafe fn move_magnifier(state: *mut State) {
    let s = &*state;
    if s.magnifier_hwnd.is_null() {
        return;
    }
    let (left, top, _, _) = magnifier_rect(s, s.current);
    MoveWindow(
        s.magnifier_hwnd,
        left.saturating_add(s.origin.0),
        top.saturating_add(s.origin.1),
        MAG_SIZE,
        MAG_WINDOW_HEIGHT,
        0,
    );
    InvalidateRect(s.magnifier_hwnd, null(), 0);
}

unsafe extern "system" fn magnifier_proc(hwnd: H, msg: u32, w: W, l: L) -> L {
    match msg {
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => HTTRANSPARENT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE,
        WM_PAINT => {
            let mut paint: Paint = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            draw_magnifier(dc, &*std::ptr::addr_of!(STATE));
            EndPaint(hwnd, &paint);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
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
            move_magnifier(state);
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
            move_magnifier(state);
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
            move_magnifier(state);
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
            let (timer_hwnd, magnifier) = {
                let s = &mut *state;
                s.done = true;
                if s.hwnd == hwnd {
                    s.hwnd = null_mut();
                }
                let magnifier = s.magnifier_hwnd;
                s.magnifier_hwnd = null_mut();
                (hwnd, magnifier)
            };
            let _ = KillTimer(timer_hwnd, UIA_TIMER_ID);
            if !magnifier.is_null() {
                DestroyWindow(magnifier);
            }
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
