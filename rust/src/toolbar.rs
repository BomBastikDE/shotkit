//! Native Win32 editor toolbar.
//!
//! Each button sends `WM_COMMAND` to the editor parent with `wParam` equal to
//! the documented command ID below and `lParam` equal to the button HWND. The
//! high word is `BN_CLICKED`. Tool labels show the existing keyboard key; this
//! toolbar does not register or replace any keyboard shortcuts.

use std::{
    cell::Cell,
    ffi::c_void,
    ptr::{null, null_mut},
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
pub const ID_SAVE: usize = 0x5110;
pub const ID_COPY: usize = 0x5111;
pub const ID_UNDO: usize = 0x5112;
pub const ID_REDO: usize = 0x5113;
pub const ID_COLOR: usize = 0x5120;
pub const ID_LINE_WIDTH: usize = 0x5121;
pub const ID_FONT_SIZE: usize = 0x5122;

const CLASS_NAME: &str = "ShotKitRustEditorToolbar";
const WM_NCCREATE: u32 = 0x0081;
const GWLP_USERDATA: i32 = -21;
const WS_CHILD: u32 = 0x40000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_TABSTOP: u32 = 0x00010000;
const BS_PUSHBUTTON: u32 = 0;
const SWP_NOZORDER: u32 = 0x0004;
const DEFAULT_GUI_FONT: i32 = 17;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const MARGIN: i32 = 6;
const GAP: i32 = 4;
const BUTTON_HEIGHT: i32 = 30;
const ROW_GAP: i32 = 4;
// Fits the longest German label ("Wiederholen") plus native button padding.
const MIN_BUTTON_WIDTH: i32 = 120;

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
}
#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(index: i32) -> Hwnd;
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

#[derive(Clone, Copy)]
struct Action {
    id: usize,
    label: &'static str,
}
const ACTIONS: [Action; 17] = [
    Action {
        id: ID_SELECT,
        label: "Auswahl (V)",
    },
    Action {
        id: ID_ARROW,
        label: "Pfeil (A)",
    },
    Action {
        id: ID_LINE,
        label: "Linie (L)",
    },
    Action {
        id: ID_RECTANGLE,
        label: "Rechteck (R)",
    },
    Action {
        id: ID_ELLIPSE,
        label: "Ellipse (E)",
    },
    Action {
        id: ID_TEXT,
        label: "Text (T)",
    },
    Action {
        id: ID_MARKER,
        label: "Marker (M)",
    },
    Action {
        id: ID_BLUR,
        label: "Verpixeln (B)",
    },
    Action {
        id: ID_STEP,
        label: "Nummer (N)",
    },
    Action {
        id: ID_PEN,
        label: "Stift (P)",
    },
    Action {
        id: ID_COLOR,
        label: "Farbe…",
    },
    Action {
        id: ID_LINE_WIDTH,
        label: "Linienbreite…",
    },
    Action {
        id: ID_FONT_SIZE,
        label: "Schriftgröße…",
    },
    Action {
        id: ID_SAVE,
        label: "Speichern",
    },
    Action {
        id: ID_COPY,
        label: "Kopieren",
    },
    Action {
        id: ID_UNDO,
        label: "Zurück",
    },
    Action {
        id: ID_REDO,
        label: "Wiederholen",
    },
];

fn layout(width: i32) -> (i32, i32, Vec<(i32, i32, i32, i32)>) {
    let width = width.max(1);
    let available = (width - 2 * MARGIN).max(1);
    let columns = ((available + GAP) / (MIN_BUTTON_WIDTH + GAP)).max(1) as usize;
    let columns = columns.min(ACTIONS.len());
    let rows = ACTIONS.len().div_ceil(columns);
    let button_width = ((available - GAP * (columns as i32 - 1)) / columns as i32).max(1);
    let height = MARGIN * 2 + rows as i32 * BUTTON_HEIGHT + (rows as i32 - 1) * ROW_GAP;
    let positions = (0..ACTIONS.len())
        .map(|index| {
            let col = index % columns;
            let row = index / columns;
            (
                MARGIN + col as i32 * (button_width + GAP),
                MARGIN + row as i32 * (BUTTON_HEIGHT + ROW_GAP),
                button_width,
                BUTTON_HEIGHT,
            )
        })
        .collect();
    (columns as i32, height, positions)
}

unsafe extern "system" fn toolbar_proc(
    hwnd: Hwnd,
    message: u32,
    wparam: Wparam,
    lparam: Lparam,
) -> Lresult {
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
    if message == WM_COMMAND {
        extern "system" {
            fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
        }
        let parent = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as Hwnd;
        if !parent.is_null() && IsWindow(parent) != 0 {
            return SendMessageW(parent, WM_COMMAND, wparam, lparam);
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

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
}

impl Toolbar {
    /// Reflow buttons into as many columns as fit while keeping multiple rows
    /// when the parent is narrow. The toolbar window's height is updated too.
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
/// | `ID_COLOR` / `ID_LINE_WIDTH` / `ID_FONT_SIZE` | Farbe / Linienbreite / Schriftgröße |
/// | `ID_SAVE` / `ID_COPY` / `ID_UNDO` / `ID_REDO` | Speichern / Kopieren / Zurück / Wiederholen |
///
/// `WM_COMMAND.wParam` contains the ID in its low word and `BN_CLICKED` in its
/// high word; `lParam` is the clicked button HWND.
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
    for (action, (x, y, w, h)) in ACTIONS.iter().zip(positions) {
        let label = wide(action.label);
        let button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
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
        buttons.push(button);
    }
    Ok(Toolbar {
        hwnd,
        buttons,
        height: Cell::new(height),
    })
}
