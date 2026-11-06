//! Native Win32 editor toolbar.
//!
//! Each button sends `WM_COMMAND` to the editor parent with `wParam` equal to
//! the documented command ID below and `lParam` equal to the button HWND. The
//! high word is `BN_CLICKED`. Buttons retain accessible action names while
//! owner drawing displays compact pictograms and native tooltips expose labels.
//! This toolbar does not register or replace any keyboard shortcuts.

mod actions;
use actions::*;
mod creation;
pub use creation::create;
mod menus;
pub use menus::{form_style_applies, popup, stamp_kind_for_command, style_target};
mod drawing;
use drawing::*;

use std::{
    cell::Cell,
    collections::HashMap,
    ffi::c_void,
    ptr::{null, null_mut},
    sync::{Mutex, OnceLock},
};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Wparam = usize;
type Lparam = isize;
type Lresult = isize;

pub const WM_COMMAND: u32 = 0x0111;
pub const ID_MORE: usize = 0x5130;
pub const ID_FILE: usize = 0x5131;
pub const ID_VIEW: usize = 0x5132;
pub const ID_SAVE_AS: usize = 0x5140;
pub const ID_OPEN: usize = 0x5141;
pub const ID_PASTE: usize = 0x5142;
pub const ID_FOLDER: usize = 0x5143;
pub const ID_ZOOM_IN: usize = 0x5150;
pub const ID_ZOOM_OUT: usize = 0x5151;
pub const ID_ZOOM_ORIGINAL: usize = 0x5152;
pub const ID_ZOOM_FIT: usize = 0x5153;
pub const ID_SELECT: usize = 0x5101;
pub const ID_ARROW: usize = 0x5102;
pub const ID_LINE: usize = 0x5103;
pub const ID_RECTANGLE: usize = 0x5104;
pub const ID_ELLIPSE: usize = 0x5105;
pub const ID_TEXT: usize = 0x5106;
pub const ID_MARKER: usize = 0x5107;
pub const ID_BLUR: usize = 0x5108;
pub const ID_STEP: usize = 0x5109;
pub const ID_PEN: usize = 0x510A;
pub const ID_CROP: usize = 0x510B;
pub const ID_REDACT: usize = 0x510C;
pub const ID_GAUSSIAN_BLUR: usize = 0x510D;
pub const ID_STAMP: usize = 0x510E;
pub const ID_STAMP_CHECK: usize = 0x5160;
pub const ID_STAMP_CROSS: usize = 0x5161;
pub const ID_STAMP_WARNING: usize = 0x5162;
pub const ID_STAMP_INFO: usize = 0x5163;
pub const ID_STAMP_CURSOR: usize = 0x5164;
pub const ID_SAVE: usize = 0x5110;
pub const ID_COPY: usize = 0x5111;
pub const ID_UNDO: usize = 0x5112;
pub const ID_REDO: usize = 0x5113;
pub const ID_CAPTURE: usize = 0x5114;
pub const ID_COLOR: usize = 0x5120;
pub const ID_LINE_WIDTH: usize = 0x5121;
pub const ID_FONT_SIZE: usize = 0x5122;
pub const ID_FRAME: usize = 0x5123;
pub const ID_SWATCH_RED: usize = 0x5124;
pub const ID_SWATCH_BLACK: usize = 0x5125;
pub const ID_SWATCH_YELLOW: usize = 0x5126;
pub const ID_SWATCH_BLUE: usize = 0x5127;
pub const ID_FORM_STYLE: usize = 0x5128;
pub const ID_FILL_NONE: usize = 0x5170;
pub const ID_FILL_SOLID: usize = 0x5171;
pub const ID_FILL_TRANSLUCENT: usize = 0x5172;
pub const ID_LINE_SOLID: usize = 0x5173;
pub const ID_LINE_DASHED: usize = 0x5174;
pub const ID_LINE_DOTTED: usize = 0x5175;
pub const ID_CORNERS_SQUARE: usize = 0x5176;
pub const ID_CORNERS_ROUNDED: usize = 0x5177;
pub const ID_ARROW_SINGLE: usize = 0x5178;
pub const ID_ARROW_DOUBLE: usize = 0x5179;
pub const ID_ARROW_DOT: usize = 0x517A;

const CLASS_NAME: &str = "ShotKitRustEditorToolbar";
const WM_NCCREATE: u32 = 0x0081;
const GWLP_USERDATA: i32 = -21;
const WS_CHILD: u32 = 0x40000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_TABSTOP: u32 = 0x00010000;
const BS_PUSHBUTTON: u32 = 0x0000000B; // BS_OWNERDRAW
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const DEFAULT_GUI_FONT: i32 = 17;
const TTF_IDISHWND: u32 = 0x0001;
const TTF_SUBCLASS: u32 = 0x0010;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
static ACTIVE_TOOLS: OnceLock<Mutex<HashMap<usize, (usize, usize)>>> = OnceLock::new();
static CURRENT_COLORS: OnceLock<Mutex<HashMap<usize, u32>>> = OnceLock::new();

#[repr(C)]
struct WndClassW {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    class_extra: i32,
    window_extra: i32,
    instance: Hinstance,
    icon: Hwnd,
    cursor: Hwnd,
    background: Hwnd,
    menu_name: *const u16,
    class_name: *const u16,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct DrawItem {
    control_type: u32,
    control_id: u32,
    item_id: u32,
    item_action: u32,
    item_state: u32,
    hwnd_item: Hwnd,
    hdc: Hwnd,
    rect: NativeRect,
    item_data: usize,
}
#[repr(C)]
struct ToolInfoW {
    size: u32,
    flags: u32,
    hwnd: Hwnd,
    id: usize,
    rect: NativeRect,
    instance: Hinstance,
    text: *mut u16,
    lparam: isize,
    reserved: *mut c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NativePoint {
    x: i32,
    y: i32,
}
#[repr(C)]
struct HighContrast {
    size: u32,
    flags: u32,
    default_scheme: *mut u16,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: Hwnd,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn IsWindow(hwnd: Hwnd) -> i32;
    fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
    fn GetDlgItem(parent: Hwnd, id: i32) -> Hwnd;
    fn GetClientRect(hwnd: Hwnd, rect: *mut NativeRect) -> i32;
    fn SendMessageW(hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
    fn SetWindowPos(
        hwnd: Hwnd,
        insert_after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    fn GetSysColor(index: i32) -> u32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
    fn FillRect(hdc: Hwnd, rect: *const NativeRect, brush: Hwnd) -> i32;
    fn InvalidateRect(hwnd: Hwnd, rect: *const NativeRect, erase: i32) -> i32;
    fn DrawFocusRect(hdc: Hwnd, rect: *const NativeRect) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, data: *mut c_void, flags: u32) -> i32;
}
#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControls();
}
#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: Hwnd) -> Hwnd;
    fn CreateCompatibleBitmap(hdc: Hwnd, width: i32, height: i32) -> Hwnd;
    fn DeleteDC(hdc: Hwnd) -> i32;
    fn SetMapMode(hdc: Hwnd, mode: i32) -> i32;
    fn SetWindowExtEx(hdc: Hwnd, x: i32, y: i32, old: *mut NativePoint) -> i32;
    fn SetViewportExtEx(hdc: Hwnd, x: i32, y: i32, old: *mut NativePoint) -> i32;
    fn SetStretchBltMode(hdc: Hwnd, mode: i32) -> i32;
    fn SetBrushOrgEx(hdc: Hwnd, x: i32, y: i32, old: *mut NativePoint) -> i32;
    fn StretchBlt(
        dst: Hwnd,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src: Hwnd,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        rop: u32,
    ) -> i32;
    fn GetStockObject(index: i32) -> Hwnd;
    fn CreateSolidBrush(color: u32) -> Hwnd;
    fn DeleteObject(object: Hwnd) -> i32;
    fn MoveToEx(hdc: Hwnd, x: i32, y: i32, previous: *mut NativePoint) -> i32;
    fn LineTo(hdc: Hwnd, x: i32, y: i32) -> i32;
    fn Rectangle(hdc: Hwnd, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn Ellipse(hdc: Hwnd, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn Polygon(hdc: Hwnd, points: *const NativePoint, count: i32) -> i32;
    fn PolyBezier(hdc: Hwnd, points: *const NativePoint, count: u32) -> i32;
    fn CreatePen(style: i32, width: i32, color: u32) -> Hwnd;
    fn SelectObject(hdc: Hwnd, object: Hwnd) -> Hwnd;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn GetLastError() -> u32;
    fn SetLastError(error: u32);
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

// SAFETY CONTRACT:
// Voraussetzung: Windows liefert gültiges HWND und nachrichtenspezifisch gültige, ausgerichtete Strukturen; kein Unwind über die ABI.
// Warum erfüllt bzw. offen: Registrierung verwendet diesen Callback; fremd zugestellte Pointer und Panikfreiheit sind nicht vollständig beweisbar.
// Nachrichten: SendMessageW und DefWindowProcW können synchron reentrieren.
unsafe extern "system" fn toolbar_proc(
    hwnd: Hwnd,
    message: u32,
    wparam: Wparam,
    lparam: Lparam,
) -> Lresult {
    extern "system" {
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    }
    if message == WM_NCCREATE {
        #[repr(C)]
        struct CreateStruct {
            create_params: *mut c_void,
            instance: Hinstance,
            menu: Hwnd,
            parent: Hwnd,
            height: i32,
            width: i32,
            y: i32,
            x: i32,
            style: i32,
            name: *const u16,
            class: *const u16,
            ex_style: u32,
        }
        extern "system" {
            fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
        }
        let cs = &*(lparam as *const CreateStruct);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.create_params as isize);
    }
    if message == 0x0082 {
        // WM_NCDESTROY
        menus::remove(hwnd);
        let parent = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
        if let Ok(mut active) = ACTIVE_TOOLS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            active.remove(&parent);
        }
        if let Ok(mut colors) = CURRENT_COLORS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            colors.remove(&parent);
        }
    }
    if message == WM_COMMAND {
        let parent = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as Hwnd;
        let id = wparam & 0xffff;
        if ACTIONS.iter().any(|a| a.id == id && a.tool) {
            if let Ok(mut active) = ACTIVE_TOOLS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                active.insert(parent as usize, (hwnd as usize, id));
            }
            invalidate_tools(hwnd);
        }
        if !parent.is_null() && IsWindow(parent) != 0 {
            return SendMessageW(parent, WM_COMMAND, wparam, lparam);
        }
    }
    if message == 0x002B {
        // WM_DRAWITEM
        let item = &mut *(lparam as *mut DrawItem);
        if item.control_type == 4 {
            // ODT_BUTTON
            if let Some(action) = ACTIONS.iter().find(|a| a.id == item.control_id as usize) {
                let selected = item.item_state & 0x0001 != 0;
                let checked = ACTIVE_TOOLS
                    .get_or_init(|| Mutex::new(HashMap::new()))
                    .lock()
                    .ok()
                    .and_then(|active| {
                        active
                            .get(&(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize))
                            .map(|x| x.1)
                    })
                    .unwrap_or(ID_SELECT)
                    == action.id;
                let current_color = CURRENT_COLORS
                    .get_or_init(|| Mutex::new(HashMap::new()))
                    .lock()
                    .ok()
                    .and_then(|colors| {
                        colors
                            .get(&(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize))
                            .copied()
                    })
                    .unwrap_or(0x000000FF);
                let bg = if selected || checked {
                    GetSysColor(13)
                } else {
                    GetSysColor(15)
                };
                let brush = CreateSolidBrush(bg);
                FillRect(item.hdc, &item.rect, brush);
                DeleteObject(brush);
                draw_smoothed_icon(
                    item.hwnd_item,
                    item.hdc,
                    item.rect,
                    action.icon,
                    GetSysColor(if checked { 14 } else { 18 }),
                    current_color,
                    bg,
                );
                if item.item_state & 0x0010 != 0 {
                    DrawFocusRect(item.hdc, &item.rect);
                }
                return 1;
            }
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

// SAFETY CONTRACT:
// Voraussetzung: instance ist ein gültiges Modulhandle; Callback bleibt während der Klassenlebensdauer gültig.
// Warum erfüllt bzw. offen: create prüft GetModuleHandleW; Identität einer bereits existierenden Klasse bleibt offen.
// Nachrichten: Kein expliziter Nachrichtenversand; allgemeine Reentranzfreiheit nicht belegt.
unsafe fn register_class(instance: Hinstance) -> bool {
    let class_name = wide(CLASS_NAME);
    let class = WndClassW {
        style: 0,
        wnd_proc: Some(toolbar_proc),
        class_extra: 0,
        window_extra: 0,
        instance,
        icon: null_mut(),
        cursor: null_mut(),
        background: 16usize as Hwnd,
        menu_name: null(),
        class_name: class_name.as_ptr(),
    };
    SetLastError(0);
    if RegisterClassW(&class) != 0 {
        return true;
    }
    GetLastError() == ERROR_CLASS_ALREADY_EXISTS
}

/// Native child toolbar, positioned by `resize`.
pub struct Toolbar {
    hwnd: Hwnd,
    buttons: Vec<Hwnd>,
    height: Cell<i32>,
    _tip_text: Vec<Vec<u16>>,
    resizing: Cell<bool>,
    pending_width: Cell<Option<i32>>,
}

/// Update the highlighted tool to match a tool change made by the editor,
/// including keyboard shortcuts and automatic transitions back to selection.
// SAFETY CONTRACT:
// Voraussetzung: parent und zugehörige Toolbar-Handles sind auf dem UI-Thread gültig und nicht wiederverwendet.
// Warum erfüllt bzw. offen: Einträge entstehen in create und werden bei WM_NCDESTROY entfernt; vollständige Handlelebensdauer bleibt offen.
// Nachrichten: User32-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub unsafe fn set_active(parent: *mut c_void, id: usize) {
    if !ACTIONS.iter().any(|a| a.id == id && a.tool) {
        return;
    }
    let toolbar = ACTIVE_TOOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|mut active| {
            active
                .get_mut(&(parent as usize))
                .map(|(toolbar, current)| {
                    *current = id;
                    *toolbar
                })
        });
    if let Some(toolbar) = toolbar {
        invalidate_tools(toolbar as Hwnd);
    }
}

/// Update the color swatch from the editor's current RGB drawing color.
// SAFETY CONTRACT:
// Voraussetzung: parent und zugehörige Toolbar-Handles sind auf dem UI-Thread gültig und nicht wiederverwendet.
// Warum erfüllt bzw. offen: Einträge entstehen in create und werden bei WM_NCDESTROY entfernt; vollständige Handlelebensdauer bleibt offen.
// Nachrichten: User32-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub unsafe fn set_color(parent: *mut c_void, red: u8, green: u8, blue: u8) {
    let colorref = (red as u32) | ((green as u32) << 8) | ((blue as u32) << 16);
    if let Ok(mut colors) = CURRENT_COLORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        colors.insert(parent as usize, colorref);
    }
    let toolbar = ACTIVE_TOOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|active| active.get(&(parent as usize)).map(|entry| entry.0));
    if let Some(toolbar) = toolbar {
        let button = GetDlgItem(toolbar as Hwnd, ID_COLOR as i32);
        if !button.is_null() {
            InvalidateRect(button, null(), 1);
        }
    }
}

impl Toolbar {
    /// Apply the same staged policy on every resize, including growth.
    // SAFETY CONTRACT:
    // The caller retains the toolbar on the UI thread across synchronous
    // SetWindowPos/ShowWindow callbacks. No global state lock spans these calls.
    pub unsafe fn resize(&self, width: i32) {
        if self.resizing.replace(true) {
            self.pending_width.set(Some(width));
            return;
        }
        let mut width = width;
        loop {
            let plan =
                shotkit::toolbar_layout::layout(width, GetDpiForWindow(self.hwnd), &layout_items());
            self.height.set(plan.height);
            menus::set_overflow(self.hwnd, plan.overflow);
            let _ = SetWindowPos(
                self.hwnd,
                null_mut(),
                0,
                0,
                width.max(1),
                plan.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            for (button, action) in self.buttons.iter().zip(ACTIONS.iter()) {
                let position = if action.id == ID_MORE {
                    plan.more
                } else {
                    plan.buttons.iter().find(|p| p.0 == action.id).map(|p| p.1)
                };
                if let Some((x, y, w, h)) = position {
                    let _ = SetWindowPos(
                        *button,
                        null_mut(),
                        x,
                        y,
                        w,
                        h,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    ShowWindow(*button, 8); // SW_SHOWNA: resizing must not activate controls
                } else {
                    ShowWindow(*button, 0);
                }
            }
            match self.pending_width.take() {
                Some(latest) if latest != width => width = latest,
                _ => break,
            }
        }
        self.resizing.set(false);
    }

    pub fn height(&self) -> i32 {
        self.height.get()
    }
}

impl Drop for Toolbar {
    fn drop(&mut self) {
        unsafe {
            // DestroyWindow on the container recursively destroys its child
            // buttons. If the parent already tore the subtree down, do nothing.
            if !self.hwnd.is_null() && IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = null_mut();
            self.buttons.clear();
        }
    }
}
