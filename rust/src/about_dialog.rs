//! Small native Win32 About window with a standard, keyboard-accessible link button.

use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    mem::zeroed,
    ptr::null_mut,
    rc::Rc,
};

type H = *mut c_void;
type W = usize;
type L = isize;

#[repr(C)]
struct WndClass {
    style: u32,
    proc: Option<unsafe extern "system" fn(H, u32, W, L) -> L>,
    class_extra: i32,
    window_extra: i32,
    instance: H,
    icon: H,
    cursor: H,
    background: H,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
struct LogFontW {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    output_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}

// Win32 NONCLIENTMETRICS layout, including Vista+ iPaddedBorderWidth.
// Every member has the native field order and alignment from winuser.h.
#[repr(C)]
struct NonClientMetricsW {
    size: u32,
    border_width: i32,
    scroll_width: i32,
    scroll_height: i32,
    caption_width: i32,
    caption_height: i32,
    caption_font: LogFontW,
    small_caption_width: i32,
    small_caption_height: i32,
    small_caption_font: LogFontW,
    menu_width: i32,
    menu_height: i32,
    menu_font: LogFontW,
    status_font: LogFontW,
    message_font: LogFontW,
    padded_border_width: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateFontIndirectW(font: *const LogFontW) -> H;
    fn GetStockObject(index: i32) -> H;
    fn DeleteObject(object: H) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn GetDpiForWindow(hwnd: H) -> u32;
    fn SystemParametersInfoForDpi(action: u32, size: u32, data: H, flags: u32, dpi: u32) -> i32;
    fn SetWindowLongPtrW(hwnd: H, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: H, index: i32) -> isize;
    fn SendMessageW(hwnd: H, message: u32, wparam: W, lparam: L) -> L;
    fn SetWindowPos(hwnd: H, after: H, x: i32, y: i32, width: i32, height: i32, flags: u32) -> i32;
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: H,
        menu: H,
        instance: H,
        parameter: H,
    ) -> H;
    fn DefWindowProcW(hwnd: H, message: u32, wparam: W, lparam: L) -> L;
    fn ShowWindow(hwnd: H, command: i32) -> i32;
    fn UpdateWindow(hwnd: H) -> i32;
    fn IsWindow(hwnd: H) -> i32;
    fn SetForegroundWindow(hwnd: H) -> i32;
    fn SetFocus(hwnd: H) -> H;
    fn LoadCursorW(instance: H, name: *const u16) -> H;
    fn MessageBoxW(hwnd: H, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
}

#[link(name = "shell32")]
extern "system" {
    // Keep the return type identical to the application's existing Win32 binding.
    fn ShellExecuteW(
        hwnd: H,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> H;
}

const CLASS_NAME: &str = "ShotKitRustAboutWindow";
const LINK_BUTTON: usize = 1;
const WM_CREATE: u32 = 0x0001;
const WM_DPICHANGED: u32 = 0x02e0;
const WM_SETFONT: u32 = 0x0030;
const GWLP_USERDATA: i32 = -21;
const SWP_NOZORDER: u32 = 0x0004;
const WM_COMMAND: u32 = 0x0111;
const WM_CLOSE: u32 = 0x0010;
const WM_NCDESTROY: u32 = 0x0082;
const WS_OVERLAPPED: u32 = 0;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_CHILD_VISIBLE: u32 = 0x5000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const BS_PUSHBUTTON: u32 = 0;

static mut ABOUT_HWND: H = null_mut();
static mut CLASS_REGISTERED: bool = false;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

// Logical coordinates at 96 DPI, in the same order as State::controls.
const CONTROL_RECTS: [(i32, i32, i32, i32); 5] = [
    (24, 22, 340, 28),
    (24, 58, 340, 22),
    (24, 84, 340, 22),
    (24, 110, 340, 22),
    (24, 146, 330, 30),
];

struct State {
    controls: Cell<[H; 5]>,
    fonts: RefCell<Vec<H>>,
    font: Cell<H>,
    generation: Cell<u64>,
    destroyed: Cell<bool>,
}

impl Drop for State {
    fn drop(&mut self) {
        // The userdata owner is released at WM_NCDESTROY, after child destruction.
        // Active callbacks retain an Rc, so reentrant destruction cannot free
        // fonts while a callback is still assigning them to controls.
        for font in self.fonts.get_mut().drain(..) {
            unsafe {
                DeleteObject(font);
            }
        }
    }
}

fn scale(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi) + 48) / 96).clamp(1, i64::from(i32::MAX)) as i32
}

// SAFETY CONTRACT:
// Voraussetzung: hwnd gehört zu diesem Callback und State lebt im lokalen Rc.
// Warum erfüllt bzw. offen: State-Borrows enden vor allen synchronen Fensteraufrufen;
// jede Generation hält Fonts bis nach der Kindfensterzerstörung am Leben.
// Nachrichten: SendMessageW und SetWindowPos können reentrieren, auch zerstören.
unsafe fn apply_dpi(hwnd: H, state: Rc<State>, dpi: u32, suggested: Option<Rect>) {
    if dpi == 0 || state.destroyed.get() {
        return;
    }
    let generation = state.generation.get().wrapping_add(1);
    state.generation.set(generation);
    let mut metrics: NonClientMetricsW = zeroed();
    metrics.size = std::mem::size_of::<NonClientMetricsW>() as u32;
    let font = if SystemParametersInfoForDpi(
        0x29,
        metrics.size,
        (&mut metrics as *mut NonClientMetricsW).cast(),
        0,
        dpi,
    ) != 0
    {
        CreateFontIndirectW(&metrics.message_font)
    } else {
        null_mut()
    };
    if !font.is_null() {
        state.fonts.borrow_mut().push(font);
        state.font.set(font);
    }
    let font = if state.font.get().is_null() {
        GetStockObject(17)
    } else {
        state.font.get()
    };
    let controls = state.controls.get();
    for (control, (x, y, width, height)) in controls.into_iter().zip(CONTROL_RECTS) {
        if state.destroyed.get() || state.generation.get() != generation {
            return;
        }
        if control.is_null() {
            continue;
        }
        SendMessageW(control, WM_SETFONT, font as usize, 1);
        if state.destroyed.get() || state.generation.get() != generation {
            return;
        }
        SetWindowPos(
            control,
            null_mut(),
            scale(x, dpi),
            scale(y, dpi),
            scale(width, dpi),
            scale(height, dpi),
            SWP_NOZORDER,
        );
    }
    if state.destroyed.get() || state.generation.get() != generation {
        return;
    }
    if let Some(rect) = suggested {
        if let (Some(width), Some(height)) = (
            rect.right.checked_sub(rect.left),
            rect.bottom.checked_sub(rect.top),
        ) {
            if width > 0 && height > 0 {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    width,
                    height,
                    SWP_NOZORDER,
                );
            }
        }
    } else {
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            scale(410, dpi),
            scale(230, dpi),
            SWP_NOZORDER | 0x0002,
        );
    }
}

// SAFETY CONTRACT:
// Voraussetzung: parent und instance sind gültige Handles auf dem UI-Thread.
// Warum erfüllt bzw. offen: Die Stringpuffer leben während CreateWindowExW; Handle-Gültigkeit bleibt Aufruferpflicht.
// Nachrichten: CreateWindowExW kann synchron Nachrichten auslösen.
unsafe fn child(
    parent: H,
    class: &str,
    text: &str,
    style: u32,
    bounds: (i32, i32, i32, i32),
    id: usize,
    instance: H,
) -> H {
    let (x, y, width, height) = bounds;
    CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        style,
        x,
        y,
        width,
        height,
        parent,
        id as H,
        instance,
        null_mut(),
    )
}

// SAFETY CONTRACT:
// Voraussetzung: Windows liefert gültige Callbackparameter auf dem Besitzerthread; ABOUT_HWND wird serialisiert.
// Warum erfüllt bzw. offen: Die Klasse registriert diesen Callback; vollständige Einthreadigkeit und Panikfreiheit sind nicht im Typ erzwungen.
// Nachrichten: Fensteranlage, Fokus, Shell, Dialoge und Zerstörung können Nachrichten auslösen.
unsafe extern "system" fn about_proc(hwnd: H, message: u32, wparam: W, lparam: L) -> L {
    match message {
        WM_CREATE => {
            let state = Rc::new(State {
                controls: Cell::new([null_mut(); 5]),
                fonts: RefCell::new(Vec::new()),
                font: Cell::new(null_mut()),
                generation: Cell::new(0),
                destroyed: Cell::new(false),
            });
            // Userdata owns one strong reference; local callbacks own separate
            // references and never borrow the state through the userdata pointer.
            let owner = Rc::into_raw(state.clone());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner as isize);
            if GetWindowLongPtrW(hwnd, GWLP_USERDATA) != owner as isize {
                // Failed attachment: no window owns the raw reference.
                drop(Rc::from_raw(owner));
                return -1;
            }
            let instance = GetModuleHandleW(null_mut());
            let dpi = GetDpiForWindow(hwnd).max(96);
            let version = format!("Version: {}", env!("CARGO_PKG_VERSION"));
            let labels = [
                "ShotKit Rust",
                &version,
                "Entwickler: Oliver Muenk",
                "Lizenz: MIT",
                "Website öffnen: alleswasbewegt.de",
            ];
            for (index, text) in labels.into_iter().enumerate() {
                let (x, y, width, height) = CONTROL_RECTS[index];
                let button = index == 4;
                let control = child(
                    hwnd,
                    if button { "BUTTON" } else { "STATIC" },
                    text,
                    WS_CHILD_VISIBLE
                        | if button {
                            WS_TABSTOP | BS_PUSHBUTTON
                        } else {
                            0
                        },
                    (
                        scale(x, dpi),
                        scale(y, dpi),
                        scale(width, dpi),
                        scale(height, dpi),
                    ),
                    if button { LINK_BUTTON } else { 0 },
                    instance,
                );
                if state.destroyed.get() || control.is_null() {
                    return -1;
                }
                let mut controls = state.controls.get();
                controls[index] = control;
                state.controls.set(controls);
            }
            apply_dpi(hwnd, state.clone(), dpi, None);
            if !state.destroyed.get() {
                let button = state.controls.get()[4];
                if !button.is_null() {
                    SetFocus(button);
                }
            }
            0
        }
        WM_DPICHANGED => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
            if ptr.is_null() || wparam & 0xffff == 0 {
                return 0;
            }
            // Clone ownership before synchronous APIs. No Rust reference into
            // userdata or the Windows-provided RECT survives those calls.
            Rc::increment_strong_count(ptr);
            let state = Rc::from_raw(ptr);
            let rect = if lparam == 0 {
                None
            } else {
                Some(*(lparam as *const Rect))
            };
            apply_dpi(hwnd, state, (wparam & 0xffff) as u32, rect);
            0
        }
        WM_COMMAND if wparam & 0xffff == LINK_BUTTON => {
            let result = ShellExecuteW(
                hwnd,
                wide("open").as_ptr(),
                wide("https://alleswasbewegt.de").as_ptr(),
                null_mut(),
                null_mut(),
                1,
            );
            if result as usize <= 32 {
                MessageBoxW(
                    hwnd,
                    wide("Die Website konnte nicht geöffnet werden.").as_ptr(),
                    wide("Über ShotKit").as_ptr(),
                    0x10,
                );
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            if ABOUT_HWND == hwnd {
                ABOUT_HWND = null_mut();
            }
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const State;
            // Children have already been destroyed. Detach first so reentrant
            // calls cannot acquire an owner that is being released.
            let state = if ptr.is_null() {
                None
            } else {
                Some(Rc::from_raw(ptr))
            };
            if let Some(state) = &state {
                state.destroyed.set(true);
            }
            let result = DefWindowProcW(hwnd, message, wparam, lparam);
            drop(state);
            result
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[link(name = "user32")]
extern "system" {
    fn DestroyWindow(hwnd: H) -> i32;
}

// SAFETY CONTRACT:
// Voraussetzung: instance ist ein gültiges Modulhandle und Klassenregistrierung wird serialisiert.
// Warum erfüllt bzw. offen: Der Aufrufer verwendet GetModuleHandleW; Einthreadigkeit bleibt Aufruferpflicht.
// Nachrichten: Kein expliziter Nachrichtendurchlauf im Funktionskörper.
unsafe fn register_class(instance: H) -> bool {
    if CLASS_REGISTERED {
        return true;
    }
    let class_name = wide(CLASS_NAME);
    let class = WndClass {
        style: 0,
        proc: Some(about_proc),
        class_extra: 0,
        window_extra: 0,
        instance,
        icon: null_mut(),
        cursor: LoadCursorW(null_mut(), 32512usize as *const u16), // IDC_ARROW
        background: 6usize as H,                                   // COLOR_WINDOW + 1
        menu_name: null_mut(),
        class_name: class_name.as_ptr(),
    };
    if RegisterClassW(&class) == 0 {
        return false;
    }
    CLASS_REGISTERED = true;
    true
}

/// Open a native About window. Reopening it focuses the existing window.
pub fn show(parent: *mut c_void) {
    unsafe {
        if !ABOUT_HWND.is_null() && IsWindow(ABOUT_HWND) != 0 {
            ShowWindow(ABOUT_HWND, 5);
            SetForegroundWindow(ABOUT_HWND);
            return;
        }
        let instance = GetModuleHandleW(null_mut());
        if !register_class(instance) {
            MessageBoxW(
                parent,
                wide("Das Infofenster konnte nicht vorbereitet werden.").as_ptr(),
                wide("Über ShotKit").as_ptr(),
                0x10,
            );
            return;
        }
        let window = CreateWindowExW(
            0,
            wide(CLASS_NAME).as_ptr(),
            wide("Über ShotKit").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            0x8000_0000u32 as i32,
            0x8000_0000u32 as i32,
            410,
            230,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if window.is_null() {
            MessageBoxW(
                parent,
                wide("Das Infofenster konnte nicht angezeigt werden.").as_ptr(),
                wide("Über ShotKit").as_ptr(),
                0x10,
            );
            return;
        }
        ABOUT_HWND = window;
        ShowWindow(window, 5);
        UpdateWindow(window);
    }
}

#[cfg(all(test, windows))]
#[path = "about_dialog_tests.rs"]
mod tests;
