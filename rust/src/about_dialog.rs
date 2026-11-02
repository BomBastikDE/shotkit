//! Small native Win32 About window with a standard, keyboard-accessible link button.

use std::{ffi::c_void, ptr::null_mut};

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

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32,
        x: i32, y: i32, width: i32, height: i32, parent: H, menu: H,
        instance: H, parameter: H) -> H;
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
    fn ShellExecuteW(hwnd: H, operation: *const u16, file: *const u16,
        parameters: *const u16, directory: *const u16, show: i32) -> H;
}

const CLASS_NAME: &str = "ShotKitRustAboutWindow";
const LINK_BUTTON: usize = 1;
const WM_CREATE: u32 = 0x0001;
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

unsafe fn child(parent: H, class: &str, text: &str, style: u32,
    x: i32, y: i32, width: i32, height: i32, id: usize, instance: H) -> H {
    CreateWindowExW(
        0, wide(class).as_ptr(), wide(text).as_ptr(), style,
        x, y, width, height, parent, id as H, instance, null_mut(),
    )
}

unsafe extern "system" fn about_proc(hwnd: H, message: u32, wparam: W, lparam: L) -> L {
    match message {
        WM_CREATE => {
            let instance = GetModuleHandleW(null_mut());
            child(hwnd, "STATIC", "ShotKit Rust", WS_CHILD_VISIBLE,
                24, 22, 340, 28, 0, instance);
            child(hwnd, "STATIC", &format!("Version: {}", env!("CARGO_PKG_VERSION")),
                WS_CHILD_VISIBLE, 24, 58, 340, 22, 0, instance);
            child(hwnd, "STATIC", "Entwickler: Oliver Muenk", WS_CHILD_VISIBLE,
                24, 84, 340, 22, 0, instance);
            child(hwnd, "STATIC", "Lizenz: MIT", WS_CHILD_VISIBLE,
                24, 110, 340, 22, 0, instance);
            let button = child(hwnd, "BUTTON", "Website öffnen: alleswasbewegt.de",
                WS_CHILD_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                24, 146, 330, 30, LINK_BUTTON, instance);
            if !button.is_null() { SetFocus(button); }
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
                MessageBoxW(hwnd, wide("Die Website konnte nicht geöffnet werden.").as_ptr(),
                    wide("Über ShotKit").as_ptr(), 0x10);
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            ABOUT_HWND = null_mut();
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[link(name = "user32")]
extern "system" {
    fn DestroyWindow(hwnd: H) -> i32;
}

unsafe fn register_class(instance: H) -> bool {
    if CLASS_REGISTERED { return true; }
    let class_name = wide(CLASS_NAME);
    let class = WndClass {
        style: 0,
        proc: Some(about_proc),
        class_extra: 0,
        window_extra: 0,
        instance,
        icon: null_mut(),
        cursor: LoadCursorW(null_mut(), 32512usize as *const u16), // IDC_ARROW
        background: 6usize as H, // COLOR_WINDOW + 1
        menu_name: null_mut(),
        class_name: class_name.as_ptr(),
    };
    if RegisterClassW(&class) == 0 { return false; }
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
            MessageBoxW(parent, wide("Das Infofenster konnte nicht vorbereitet werden.").as_ptr(),
                wide("Über ShotKit").as_ptr(), 0x10);
            return;
        }
        let window = CreateWindowExW(
            0,
            wide(CLASS_NAME).as_ptr(),
            wide("Über ShotKit").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            0x8000_0000u32 as i32, 0x8000_0000u32 as i32,
            410, 230,
            null_mut(), null_mut(), instance, null_mut(),
        );
        if window.is_null() {
            MessageBoxW(parent, wide("Das Infofenster konnte nicht angezeigt werden.").as_ptr(),
                wide("Über ShotKit").as_ptr(), 0x10);
            return;
        }
        ABOUT_HWND = window;
        ShowWindow(window, 5);
        UpdateWindow(window);
    }
}
