//! Small native Unicode text-entry dialog for the Windows editor.
//!
//! The caller must invoke `prompt` on the owning window's UI thread. The owner
//! is disabled only for the lifetime of this dialog and its prior enabled state
//! is restored on all exit paths. The dialog uses a private message loop and
//! never posts WM_QUIT to the caller's thread.

use std::{
    ffi::c_void,
    mem::zeroed,
    ptr::{null, null_mut},
};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Wparam = usize;
type Lparam = isize;
type Lresult = isize;

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
struct Msg {
    hwnd: Hwnd,
    message: u32,
    wparam: Wparam,
    lparam: Lparam,
    time: u32,
    point: NativePoint,
    private: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NativePoint {
    x: i32,
    y: i32,
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

#[derive(Clone, Copy)]
struct DialogLayout {
    width: i32,
    height: i32,
    edit: (i32, i32, i32, i32),
    ok: (i32, i32, i32, i32),
    cancel: (i32, i32, i32, i32),
}

fn scale(logical: i32, dpi: u32) -> i32 {
    ((logical as i64 * dpi.max(1) as i64 + 48) / 96).clamp(1, i32::MAX as i64) as i32
}

fn layout(dpi: u32) -> DialogLayout {
    DialogLayout {
        width: scale(400, dpi),
        height: scale(128, dpi),
        edit: (
            scale(16, dpi),
            scale(18, dpi),
            scale(368, dpi),
            scale(26, dpi),
        ),
        ok: (
            scale(222, dpi),
            scale(58, dpi),
            scale(78, dpi),
            scale(28, dpi),
        ),
        cancel: (
            scale(306, dpi),
            scale(58, dpi),
            scale(78, dpi),
            scale(28, dpi),
        ),
    }
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
    fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
    fn UpdateWindow(hwnd: Hwnd) -> i32;
    fn EnableWindow(hwnd: Hwnd, enable: i32) -> i32;
    fn IsWindowEnabled(hwnd: Hwnd) -> i32;
    fn IsWindow(hwnd: Hwnd) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> Lresult;
    fn IsDialogMessageW(dialog: Hwnd, msg: *mut Msg) -> i32;
    fn SetFocus(hwnd: Hwnd) -> Hwnd;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
    fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
    fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max_count: i32) -> i32;
    fn SendMessageW(hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut NativeRect) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetWindowPos(
        hwnd: Hwnd,
        insert_after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    fn LoadCursorW(instance: Hinstance, name: *const u16) -> Hwnd;
    fn GetLastError() -> u32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
    fn GetDpiForSystem() -> u32;
    fn SystemParametersInfoForDpi(
        action: u32,
        ui_param: u32,
        data: *mut c_void,
        flags: u32,
        dpi: u32,
    ) -> i32;
}
#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(index: i32) -> Hwnd;
    fn CreateFontIndirectW(font: *const LogFontW) -> Hwnd;
    fn DeleteObject(object: Hwnd) -> i32;
}

const WM_CREATE: u32 = 0x0001;
const WM_COMMAND: u32 = 0x0111;
const WM_CLOSE: u32 = 0x0010;
const WM_SETFONT: u32 = 0x0030;
const WM_NCCREATE: u32 = 0x0081;
const WM_KEYDOWN: u32 = 0x0100;
const WM_DPICHANGED: u32 = 0x02E0;
const GWLP_USERDATA: i32 = -21;
const WS_CHILD: u32 = 0x40000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_TABSTOP: u32 = 0x00010000;
const WS_BORDER: u32 = 0x00800000;
const WS_CAPTION: u32 = 0x00C00000;
const WS_SYSMENU: u32 = 0x00080000;
const WS_POPUP: u32 = 0x80000000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const BS_PUSHBUTTON: u32 = 0x0000;
const SW_SHOW: i32 = 5;
const SWP_NOZORDER: u32 = 0x0004;
const IDC_ARROW: usize = 32512;
const DEFAULT_GUI_FONT: i32 = 17;
const SPI_GETNONCLIENTMETRICS: u32 = 0x0029;
const ID_EDIT: usize = 1001;
const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct DialogState {
    edit: Hwnd,
    ok: Hwnd,
    cancel: Hwnd,
    font: Hwnd,
    owned_fonts: Vec<Hwnd>,
    dpi: u32,
    result: Option<String>,
    done: bool,
    accepted: bool,
}

// SAFETY CONTRACT:
// Voraussetzung: dpi ist ein gültiger Windows-DPI-Wert.
// Warum erfüllt bzw. offen: Die Strukturen entsprechen NONCLIENTMETRICSW/LOGFONTW; die Fontbasis kommt von der DPI-aware System-API.
// Nachrichten: SystemParametersInfoForDpi und CreateFontIndirectW erhalten keine Fensterhandles und keine State-Referenz.
fn create_dialog_font(dpi: u32) -> Hwnd {
    unsafe {
        let mut metrics: NonClientMetricsW = zeroed();
        metrics.size = std::mem::size_of::<NonClientMetricsW>() as u32;
        let size = metrics.size;
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            size,
            (&mut metrics as *mut NonClientMetricsW).cast(),
            0,
            dpi.max(96),
        ) == 0
        {
            return null_mut();
        }
        CreateFontIndirectW(&metrics.message_font)
    }
}

// SAFETY CONTRACT:
// Voraussetzung: Die Handles sind null oder lebende Controls; font ist null oder ein lebender HFONT/Stock-Font.
// Warum erfüllt bzw. offen: Nur Wertkopien gehen in den Aufruf; kein DialogState-Borrow bleibt über SendMessageW bestehen.
// Nachrichten: WM_SETFONT wird synchron gesendet und kann reentrieren.
unsafe fn set_control_font(edit: Hwnd, ok: Hwnd, cancel: Hwnd, font: Hwnd) {
    let selected = if font.is_null() {
        GetStockObject(DEFAULT_GUI_FONT)
    } else {
        font
    };
    for control in [edit, ok, cancel] {
        if !control.is_null() {
            SendMessageW(control, WM_SETFONT, selected as usize, 1);
        }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: Die Handles sind null oder lebende Dialog-Kindfenster.
// Warum erfüllt bzw. offen: Handles und Maße sind Wertkopien; kein DialogState-Borrow bleibt über SetWindowPos bestehen.
// Nachrichten: SetWindowPos kann synchron Fensternachrichten und Reentranz auslösen.
unsafe fn layout_controls(edit: Hwnd, ok: Hwnd, cancel: Hwnd, d: DialogLayout) {
    for (control, rect) in [(edit, d.edit), (ok, d.ok), (cancel, d.cancel)] {
        if !control.is_null() {
            SetWindowPos(
                control,
                null_mut(),
                rect.0,
                rect.1,
                rect.2,
                rect.3,
                SWP_NOZORDER,
            );
        }
    }
}

struct OwnerGuard {
    owner: Hwnd,
    was_enabled: bool,
}
impl Drop for OwnerGuard {
    fn drop(&mut self) {
        unsafe {
            if self.was_enabled && !self.owner.is_null() && IsWindow(self.owner) != 0 {
                EnableWindow(self.owner, 1);
            }
        }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: hwnd ist ein gültiges Edit-Fenster; der Ausgabepuffer fasst die angefragten UTF-16-Zeichen.
// Warum erfüllt bzw. offen: Die Größe wird geprüft und der Puffer lokal für die Dauer des Aufrufs gehalten.
// Nachrichten: GetWindowText kann synchron Kontrollnachrichten auslösen.
unsafe fn read_text(hwnd: Hwnd) -> Option<String> {
    let length = GetWindowTextLengthW(hwnd);
    if length < 0 {
        return None;
    }
    let cap = length.checked_add(1)?;
    let mut text = vec![0u16; cap as usize];
    let actual = GetWindowTextW(hwnd, text.as_mut_ptr(), cap);
    if actual < 0 {
        return None;
    }
    text.truncate(actual as usize);
    Some(String::from_utf16_lossy(&text))
}

// SAFETY CONTRACT:
// Voraussetzung: state zeigt auf lebenden DialogState und hwnd gehört zu diesem Dialog.
// Warum erfüllt bzw. offen: Das Edit-Handle wird vor read_text kopiert; der State-Borrow endet vor DestroyWindow.
// Nachrichten: Textabfrage und Zerstörung können synchron Nachrichten auslösen.
unsafe fn complete(hwnd: Hwnd, state: *mut DialogState, accepted: bool) {
    // Copy the handle first: read_text sends messages which can reenter this
    // procedure, so no DialogState reference may be alive during that call.
    let edit = (*state).edit;
    let result = if accepted { read_text(edit) } else { None };
    {
        let state = &mut *state;
        if accepted {
            state.accepted = result.is_some();
            state.result = result;
        }
        state.done = true;
    }
    // The state borrow ends before synchronous destruction can reenter.
    DestroyWindow(hwnd);
}

// SAFETY CONTRACT:
// Voraussetzung: Windows liefert gültige Callbackparameter; Userdata zeigt auf lebenden DialogState.
// Warum erfüllt bzw. offen: prompt hält den State während der modalen Schleife; Panikfreiheit über die ABI ist nicht belegt.
// Nachrichten: Erzeugung, Kontrollnachrichten und Zerstörung können synchron reentrieren.
unsafe extern "system" fn dialog_proc(
    hwnd: Hwnd,
    message: u32,
    wparam: Wparam,
    lparam: Lparam,
) -> Lresult {
    if message == WM_NCCREATE {
        // lpCreateParams carries a stack state that remains alive until this
        // modal function destroys the window and exits its message loop.
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
    extern "system" {
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogState;
    match message {
        WM_CREATE => {
            if ptr.is_null() {
                return -1;
            }
            let d = layout((*ptr).dpi);
            let edit_class = wide("EDIT");
            let button_class = wide("BUTTON");
            let edit = CreateWindowExW(
                0,
                edit_class.as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                d.edit.0,
                d.edit.1,
                d.edit.2,
                d.edit.3,
                hwnd,
                ID_EDIT as Hwnd,
                GetModuleHandleW(null()),
                null_mut(),
            );
            if edit.is_null() {
                return -1;
            }
            (*ptr).edit = edit;
            let ok = wide("OK");
            let cancel = wide("Abbrechen");
            let ok_hwnd = CreateWindowExW(
                0,
                button_class.as_ptr(),
                ok.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                d.ok.0,
                d.ok.1,
                d.ok.2,
                d.ok.3,
                hwnd,
                ID_OK as Hwnd,
                GetModuleHandleW(null()),
                null_mut(),
            );
            let cancel_hwnd = CreateWindowExW(
                0,
                button_class.as_ptr(),
                cancel.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                d.cancel.0,
                d.cancel.1,
                d.cancel.2,
                d.cancel.3,
                hwnd,
                ID_CANCEL as Hwnd,
                GetModuleHandleW(null()),
                null_mut(),
            );
            if ok_hwnd.is_null() || cancel_hwnd.is_null() {
                return -1;
            }
            let font = create_dialog_font((*ptr).dpi);
            (*ptr).edit = edit;
            (*ptr).ok = ok_hwnd;
            (*ptr).cancel = cancel_hwnd;
            (*ptr).font = font;
            if !font.is_null() {
                (*ptr).owned_fonts.push(font);
            }
            set_control_font(edit, ok_hwnd, cancel_hwnd, font);
            // SetWindowTextW may notify the parent synchronously. Do not
            // retain a DialogState borrow while that notification runs.
            let initial = wide(&(*ptr).result.take().unwrap_or_default());
            SetWindowTextW(edit, initial.as_ptr());
            (*ptr).result = None;
            SetFocus(edit);
            0
        }
        WM_COMMAND if !ptr.is_null() => {
            let id = wparam & 0xffff;
            if id == ID_OK {
                complete(hwnd, ptr, true);
                return 0;
            }
            if id == ID_CANCEL {
                complete(hwnd, ptr, false);
                return 0;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_DPICHANGED if !ptr.is_null() => {
            let dpi = (wparam & 0xffff) as u32;
            if dpi == 0 {
                return 0;
            }

            // Create the font before borrowing state; this API call must not run
            // while a mutable DialogState reference exists.
            let new_font = create_dialog_font(dpi);
            let (edit, ok, cancel, active_font) = {
                let state = &mut *ptr;
                state.dpi = dpi;
                if !new_font.is_null() {
                    state.font = new_font;
                    state.owned_fonts.push(new_font);
                }
                (state.edit, state.ok, state.cancel, state.font)
            };
            set_control_font(edit, ok, cancel, active_font);
            layout_controls(edit, ok, cancel, layout(dpi));

            // Windows supplies a RECT for WM_DPICHANGED. Null, zero-sized, or
            // inverted rectangles are ignored; never pass invalid dimensions to SetWindowPos.
            if lparam != 0 {
                let suggested = &*(lparam as *const NativeRect);
                if let (Some(width), Some(height)) = (
                    suggested.right.checked_sub(suggested.left),
                    suggested.bottom.checked_sub(suggested.top),
                ) {
                    if width > 0 && height > 0 {
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            suggested.left,
                            suggested.top,
                            width,
                            height,
                            SWP_NOZORDER,
                        );
                    }
                }
            }
            0
        }
        WM_KEYDOWN if wparam == 0x1b && !ptr.is_null() => {
            complete(hwnd, ptr, false);
            0
        }
        WM_CLOSE if !ptr.is_null() => {
            complete(hwnd, ptr, false);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

// SAFETY CONTRACT:
// Voraussetzung: instance ist ein gültiges Modulhandle; eine bestehende Klasse entspricht diesem Callback.
// Warum erfüllt bzw. offen: Der Aufrufer nutzt GetModuleHandleW; Identität einer bereits vorhandenen Klasse bleibt zu prüfen.
// Nachrichten: Kein expliziter Nachrichtenversand im Funktionskörper.
unsafe fn ensure_class(instance: Hinstance) -> bool {
    let name = wide("ShotKitRustTextEntryDialog");
    let class = WndClassW {
        style: 0,
        wnd_proc: Some(dialog_proc),
        class_extra: 0,
        window_extra: 0,
        instance,
        icon: null_mut(),
        cursor: LoadCursorW(null_mut(), IDC_ARROW as *const u16),
        background: 6usize as Hwnd,
        menu_name: null(),
        class_name: name.as_ptr(),
    };
    if RegisterClassW(&class) != 0 {
        return true;
    }
    GetLastError() == ERROR_CLASS_ALREADY_EXISTS
}

/// Show a modal Unicode text-entry window. `None` means cancel or a Win32 error;
/// `Some("")` is a valid accepted empty string. Call on the owner's UI thread.
// SAFETY CONTRACT:
// Voraussetzung: owner ist null oder ein gültiges eigenes Fenster; modale Reentranz ist zulässig.
// Warum erfüllt bzw. offen: Der State lebt bis zum Ende der Nachrichtenschleife; vollständige Threadzugehörigkeit ist nicht geprüft.
// Nachrichten: Fensteranlage, Aktivierung, Schleife und Zerstörung können Nachrichten auslösen.
pub unsafe fn prompt(owner: *mut c_void, initial: &str) -> Option<String> {
    if !owner.is_null() && IsWindow(owner) == 0 {
        return None;
    }
    let instance = GetModuleHandleW(null());
    if instance.is_null() || !ensure_class(instance) {
        return None;
    }

    let was_enabled = !owner.is_null() && IsWindowEnabled(owner) != 0;
    if was_enabled {
        EnableWindow(owner, 0);
    }
    let _owner_guard = OwnerGuard { owner, was_enabled };

    let mut state = DialogState {
        edit: null_mut(),
        ok: null_mut(),
        cancel: null_mut(),
        font: null_mut(),
        owned_fonts: Vec::new(),
        dpi: {
            let dpi = if owner.is_null() {
                GetDpiForSystem()
            } else {
                GetDpiForWindow(owner)
            };
            if dpi == 0 {
                96
            } else {
                dpi
            }
        },
        result: Some(initial.to_owned()),
        done: false,
        accepted: false,
    };
    let class_name = wide("ShotKitRustTextEntryDialog");
    let title = wide("Text eingeben");
    let hwnd = CreateWindowExW(
        0x00000001,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP | WS_CAPTION | WS_SYSMENU,
        0x80000000u32 as i32,
        0x80000000u32 as i32,
        layout(state.dpi).width,
        layout(state.dpi).height,
        owner,
        null_mut(),
        instance,
        (&mut state as *mut DialogState).cast(),
    );
    if hwnd.is_null() {
        let owned_fonts = std::mem::take(&mut state.owned_fonts);
        for font in owned_fonts {
            DeleteObject(font);
        }
        return None;
    }
    let (mut x, mut y) = (0, 0);
    let d = layout(state.dpi);
    let (width, height) = (d.width, d.height);
    if !owner.is_null() {
        let mut rect: NativeRect = zeroed();
        if GetWindowRect(owner, &mut rect) != 0 {
            x = rect.left + ((rect.right - rect.left) - width) / 2;
            y = rect.top + ((rect.bottom - rect.top) - height) / 2;
        }
    } else {
        x = (GetSystemMetrics(0) - width) / 2;
        y = (GetSystemMetrics(1) - height) / 2;
    }
    let _ = SetWindowPos(hwnd, null_mut(), x, y, width, height, SWP_NOZORDER);
    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);
    if !state.edit.is_null() {
        SetFocus(state.edit);
    }

    let mut msg: Msg = zeroed();
    while !state.done {
        let status = GetMessageW(&mut msg, null_mut(), 0, 0);
        if status <= 0 {
            break;
        }
        if IsDialogMessageW(hwnd, &mut msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    // Controls no longer exist, so none can still hold one of these owned fonts.
    let owned_fonts = std::mem::take(&mut state.owned_fonts);
    for font in owned_fonts {
        DeleteObject(font);
    }
    if state.accepted {
        state.result
    } else {
        None
    }
}

#[cfg(all(test, windows))]
#[path = "text_dialog_tests.rs"]
mod tests;
