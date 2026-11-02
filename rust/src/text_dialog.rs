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
}
#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(index: i32) -> Hwnd;
}

const WM_CREATE: u32 = 0x0001;
const WM_COMMAND: u32 = 0x0111;
const WM_CLOSE: u32 = 0x0010;
const WM_SETFONT: u32 = 0x0030;
const WM_NCCREATE: u32 = 0x0081;
const WM_KEYDOWN: u32 = 0x0100;
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
const ID_EDIT: usize = 1001;
const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct DialogState {
    edit: Hwnd,
    result: Option<String>,
    done: bool,
    accepted: bool,
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

unsafe fn complete(hwnd: Hwnd, state: &mut DialogState, accepted: bool) {
    if accepted {
        state.result = read_text(state.edit);
        state.accepted = state.result.is_some();
    }
    state.done = true;
    DestroyWindow(hwnd);
}

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
            let state = &mut *ptr;
            let edit_class = wide("EDIT");
            let button_class = wide("BUTTON");
            let edit = CreateWindowExW(
                0,
                edit_class.as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
                16,
                18,
                368,
                26,
                hwnd,
                ID_EDIT as Hwnd,
                GetModuleHandleW(null()),
                null_mut(),
            );
            if edit.is_null() {
                return -1;
            }
            state.edit = edit;
            let ok = wide("OK");
            let cancel = wide("Abbrechen");
            let ok_hwnd = CreateWindowExW(
                0,
                button_class.as_ptr(),
                ok.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                222,
                58,
                78,
                28,
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
                306,
                58,
                78,
                28,
                hwnd,
                ID_CANCEL as Hwnd,
                GetModuleHandleW(null()),
                null_mut(),
            );
            if ok_hwnd.is_null() || cancel_hwnd.is_null() {
                return -1;
            }
            let font = GetStockObject(DEFAULT_GUI_FONT);
            SendMessageW(edit, WM_SETFONT, font as usize, 1);
            SendMessageW(ok_hwnd, WM_SETFONT, font as usize, 1);
            SendMessageW(cancel_hwnd, WM_SETFONT, font as usize, 1);
            let initial = wide(&state.result.take().unwrap_or_default());
            SetWindowTextW(edit, initial.as_ptr());
            state.result = None;
            SetFocus(edit);
            0
        }
        WM_COMMAND if !ptr.is_null() => {
            let id = wparam & 0xffff;
            let state = &mut *ptr;
            if id == ID_OK {
                complete(hwnd, state, true);
                return 0;
            }
            if id == ID_CANCEL {
                complete(hwnd, state, false);
                return 0;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_KEYDOWN if wparam == 0x1b && !ptr.is_null() => {
            complete(hwnd, &mut *ptr, false);
            0
        }
        WM_CLOSE if !ptr.is_null() => {
            complete(hwnd, &mut *ptr, false);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

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
        400,
        128,
        owner,
        null_mut(),
        instance,
        (&mut state as *mut DialogState).cast(),
    );
    if hwnd.is_null() {
        return None;
    }
    let (mut x, mut y) = (0, 0);
    let (width, height) = (400, 128);
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
    if state.accepted {
        state.result
    } else {
        None
    }
}
