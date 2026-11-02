//! Native Win32 editor toolbar.
//!
//! Each button sends `WM_COMMAND` to the editor parent with `wParam` equal to
//! the documented command ID below and `lParam` equal to the button HWND. The
//! high word is `BN_CLICKED`. Buttons retain accessible action names while
//! owner drawing displays compact pictograms and native tooltips expose labels.
//! This toolbar does not register or replace any keyboard shortcuts.

mod actions;
use actions::*;
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

const CLASS_NAME: &str = "ShotKitRustEditorToolbar";
const WM_NCCREATE: u32 = 0x0081;
const GWLP_USERDATA: i32 = -21;
const WS_CHILD: u32 = 0x40000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_TABSTOP: u32 = 0x00010000;
const BS_PUSHBUTTON: u32 = 0x0000000B; // BS_OWNERDRAW
const SWP_NOZORDER: u32 = 0x0004;
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

fn layout(width: i32) -> (i32, i32, Vec<(i32, i32, i32, i32)>) {
    shotkit_rust_prototype::toolbar_layout::layout(width, ACTIONS.len())
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
                draw_icon(
                    item.hdc,
                    item.rect,
                    action.icon,
                    GetSysColor(if checked { 14 } else { 18 }),
                    current_color,
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
    if let Ok(mut active) = ACTIVE_TOOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        if let Some((toolbar, current)) = active.get_mut(&(parent as usize)) {
            *current = id;
            invalidate_tools(*toolbar as Hwnd);
        }
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
    if let Ok(active) = ACTIVE_TOOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        if let Some((toolbar, _)) = active.get(&(parent as usize)) {
            let button = GetDlgItem(*toolbar as Hwnd, ID_COLOR as i32);
            if !button.is_null() {
                InvalidateRect(button, null(), 1);
            }
        }
    }
}

impl Toolbar {
    /// Reflow buttons into as many columns as fit while keeping multiple rows
    /// when the parent is narrow. The toolbar window's height is updated too.
    // SAFETY CONTRACT:
    // Voraussetzung: self und native Toolbar-/Buttonfenster leben über alle verschachtelten Nachrichten.
    // Warum erfüllt bzw. offen: Der Fensterhandler klont Rc vor dem Aufruf; die Lebensdauer nativer HWNDs bei Parentabbau bleibt offen.
    // Nachrichten: SetWindowPos kann synchron Nachrichten auslösen.
    pub unsafe fn resize(&self, width: i32) {
        let width = width.max(1);
        let (_, height, positions) = layout(width);
        self.height.set(height);
        let _ = SetWindowPos(self.hwnd, null_mut(), 0, 0, width, height, SWP_NOZORDER);
        for (button, (x, y, w, h)) in self.buttons.iter().zip(positions) {
            let _ = SetWindowPos(*button, null_mut(), x, y, w, h, SWP_NOZORDER);
        }
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

/// Create an editor toolbar as a child of `parent`.
///
/// Commands sent to the parent:
///
/// | ID | Action |
/// | --- | --- |
/// | `ID_SELECT` / `ID_ARROW` / `ID_LINE` | Auswahl / Pfeil / Linie |
/// | `ID_RECTANGLE` / `ID_ELLIPSE` / `ID_TEXT` | Rechteck / Ellipse / Text |
/// | `ID_MARKER` / `ID_BLUR` / `ID_STEP` / `ID_PEN` | Marker / Verpixeln / Nummer / Stift |
/// | `ID_CROP` | Zuschneiden |
/// | `ID_COLOR` / `ID_LINE_WIDTH` / `ID_FONT_SIZE` / `ID_FRAME` | Farbe / Linienbreite / Schriftgröße / Rahmen und Schatten |
/// | `ID_SAVE` / `ID_CAPTURE` / `ID_COPY` / `ID_UNDO` / `ID_REDO` | Speichern / Aufnahme / Kopieren / Zurück / Wiederholen |
///
/// `WM_COMMAND.wParam` contains the ID in its low word and `BN_CLICKED` in its
/// high word; `lParam` is the clicked button HWND.
// SAFETY CONTRACT:
// Voraussetzung: parent ist ein lebendes eigenes Fenster auf dem UI-Thread und zustandsseitig callbackbereit.
// Warum erfüllt bzw. offen: Der Aufrufer erstellt das Fenster zuerst; Tooltiptexte bleiben in Toolbar gespeichert, Handlelebensdauer bei Reentranz bleibt offen.
// Nachrichten: CreateWindowExW, SendMessageW und DestroyWindow können synchron reentrieren.
pub unsafe fn create(parent: *mut c_void) -> Result<Toolbar, String> {
    if parent.is_null() || IsWindow(parent) == 0 {
        return Err("Ungültiges Editor-Fenster".into());
    }
    let instance = GetModuleHandleW(null());
    if instance.is_null() || !register_class(instance) {
        return Err("Toolbar-Fensterklasse konnte nicht registriert werden".into());
    }

    let mut client: NativeRect = std::mem::zeroed();
    let width = if GetClientRect(parent, &mut client) != 0 {
        (client.right - client.left).max(1)
    } else {
        640
    };
    let (_, height, positions) = layout(width);
    let class_name = wide(CLASS_NAME);
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        null(),
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        0,
        0,
        width,
        height,
        parent,
        null_mut(),
        instance,
        parent,
    );
    if hwnd.is_null() {
        return Err("Toolbar-Fenster konnte nicht erstellt werden".into());
    }

    let button_class = wide("BUTTON");
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let mut buttons = Vec::with_capacity(ACTIONS.len());
    let mut tip_text = Vec::with_capacity(ACTIONS.len());
    InitCommonControls();
    let tip_class = wide("tooltips_class32");
    let tooltip = CreateWindowExW(
        0x00000008,
        tip_class.as_ptr(),
        null(),
        0x80000000 | 0x00000001,
        0,
        0,
        0,
        0,
        hwnd,
        null_mut(),
        instance,
        null_mut(),
    );
    if tooltip.is_null() {
        DestroyWindow(hwnd);
        return Err("Tooltip-Fenster konnte nicht erstellt werden".into());
    }
    for (index, (action, (x, y, w, h))) in ACTIONS.iter().zip(positions).enumerate() {
        let label = wide(action.label);
        let mut tooltip_label = wide(match action.id {
            ID_CAPTURE => "Neue Aufnahme (Strg+N)",
            ID_COPY => "Kopieren (Strg+C)",
            ID_SAVE => "Speichern (Strg+S)",
            ID_UNDO => "Zurück (Strg+Z)",
            ID_REDO => "Wiederholen (Strg+Y)",
            _ => action.label,
        });
        let button_style = BS_PUSHBUTTON;
        let button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | button_style,
            x,
            y,
            w,
            h,
            hwnd,
            action.id as Hwnd,
            instance,
            null_mut(),
        );
        if button.is_null() {
            DestroyWindow(hwnd);
            return Err(format!(
                "Toolbar-Schaltfläche {} konnte nicht erstellt werden",
                action.label
            ));
        }
        SendMessageW(button, 0x0030, font as usize, 1); // WM_SETFONT
        if index == 0 {
            if let Ok(mut active) = ACTIVE_TOOLS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                active.insert(parent as usize, (hwnd as usize, ID_SELECT));
            }
            if let Ok(mut colors) = CURRENT_COLORS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                colors.insert(parent as usize, 0x000000FF); // Style::default() is red.
            }
        }
        if !tooltip.is_null() {
            // Version 5 common controls require the V2 size (without lpReserved).
            let mut info = ToolInfoW {
                size: std::mem::offset_of!(ToolInfoW, reserved) as u32,
                flags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd,
                id: button as usize,
                rect: std::mem::zeroed(),
                instance,
                text: tooltip_label.as_mut_ptr(),
                lparam: 0,
                reserved: null_mut(),
            };
            if SendMessageW(tooltip, 0x0432, 0, &mut info as *mut _ as isize) == 0 {
                // TTM_ADDTOOLW
                DestroyWindow(tooltip);
                DestroyWindow(hwnd);
                return Err(format!(
                    "Tooltip für {} konnte nicht registriert werden",
                    action.label
                ));
            }
        }
        tip_text.push(tooltip_label);
        buttons.push(button);
    }
    Ok(Toolbar {
        hwnd,
        buttons,
        height: Cell::new(height),
        _tip_text: tip_text,
    })
}
