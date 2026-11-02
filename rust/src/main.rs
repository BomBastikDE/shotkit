#![windows_subsystem = "windows"]
use std::{
    ffi::c_void,
    fs::OpenOptions,
    io::Write,
    mem::{size_of, zeroed},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::null_mut,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};
mod autostart;
mod capture_targets;
mod clipboard;
mod element_picker;
mod file_dialog;
mod folder_dialog;
mod scroll_windows;
mod selection;
mod text_dialog;
mod toolbar;
mod uia_picker;

use shotkit_rust_prototype::{
    capture_output::{Advance, CaptureAction, CaptureOutputState},
    delayed_capture::{BeginError, DelayedCaptureState, Ticket as CaptureTicket},
    editor::EditorController,
    image_edit::{CropRect, FrameOptions, ShadowOptions},
    image_io, output_windows,
    scroll_capture::{self, Direction, Outcome, ScrollIo},
    settings::{self, Settings},
    shapes::{Point as ShapePoint, Tool},
    stitch::Image,
    view,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

type H = *mut c_void;
type W = usize;
type L = isize;
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct Msg {
    hwnd: H,
    msg: u32,
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
struct Notify {
    size: u32,
    hwnd: H,
    id: u32,
    flags: u32,
    callback: u32,
    icon: H,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    timeout: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon: H,
}
#[repr(C)]
struct BmiHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bits: u16,
    compression: u32,
    image_size: u32,
    xppm: i32,
    yppm: i32,
    used: u32,
    important: u32,
}
#[repr(C)]
struct Bmi {
    header: BmiHeader,
    colors: [u32; 1],
}
#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(w: *const WndClass) -> u16;
    fn CreateWindowExW(
        ex: u32,
        c: *const u16,
        n: *const u16,
        s: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: H,
        menu: H,
        inst: H,
        p: H,
    ) -> H;
    fn GetWindowLongPtrW(h: H, index: i32) -> isize;
    fn SetWindowLongPtrW(h: H, index: i32, value: isize) -> isize;
    fn DefWindowProcW(h: H, m: u32, w: W, l: L) -> L;
    fn ShowWindow(h: H, n: i32) -> i32;
    fn GetMessageW(m: *mut Msg, h: H, min: u32, max: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> L;
    fn PostQuitMessage(code: i32);
    fn DestroyWindow(h: H) -> i32;
    fn LoadIconW(h: H, n: *const u16) -> H;
    fn LoadCursorW(h: H, n: *const u16) -> H;
    fn MessageBoxW(h: H, t: *const u16, c: *const u16, k: u32) -> i32;
    fn RegisterHotKey(h: H, id: i32, mods: u32, key: u32) -> i32;
    fn UnregisterHotKey(h: H, id: i32) -> i32;
    fn GetDC(h: H) -> H;
    fn ReleaseDC(h: H, dc: H) -> i32;
    fn BeginPaint(h: H, p: *mut Paint) -> H;
    fn EndPaint(h: H, p: *const Paint) -> i32;
    fn GetClientRect(h: H, r: *mut Rect) -> i32;
    fn InvalidateRect(h: H, r: *const Rect, erase: i32) -> i32;
    fn SetWindowTextW(h: H, t: *const u16) -> i32;
    fn SetCapture(h: H) -> H;
    fn ReleaseCapture() -> i32;
    fn SetForegroundWindow(h: H) -> i32;
    fn SetFocus(h: H) -> H;
    fn GetForegroundWindow() -> H;
    fn GetShellWindow() -> H;
    fn GetAncestor(h: H, flags: u32) -> H;
    fn GetClassNameW(h: H, name: *mut u16, max: i32) -> i32;
    fn IsWindow(h: H) -> i32;
    fn GetWindowThreadProcessId(h: H, process_id: *mut u32) -> u32;
    fn GetKeyState(key: i32) -> i16;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn CreatePopupMenu() -> H;
    fn AppendMenuW(menu: H, flags: u32, id: usize, text: *const u16) -> i32;
    fn TrackPopupMenu(
        menu: H,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        owner: H,
        rect: *const Rect,
    ) -> u32;
    fn DestroyMenu(menu: H) -> i32;
    fn IsWindowVisible(h: H) -> i32;
    fn SetProcessDpiAwarenessContext(context: H) -> i32;
    fn WindowFromPoint(p: Point) -> H;
    fn PostMessageW(h: H, msg: u32, w: W, l: L) -> i32;
    fn FindWindowW(class: *const u16, title: *const u16) -> H;
    fn SendMessageTimeoutW(h: H, msg: u32, w: W, l: L, flags: u32, timeout: u32, result: *mut W) -> i32;
}
#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        h: H,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> H;
}
#[link(name = "gdi32")]
extern "system" {
    fn CreateDIBSection(
        dc: H,
        info: *const Bmi,
        usage: u32,
        bits: *mut *mut c_void,
        section: H,
        offset: u32,
    ) -> H;
    fn CreateCompatibleDC(dc: H) -> H;
    fn SelectObject(dc: H, obj: H) -> H;
    fn DeleteDC(dc: H) -> i32;
    fn DeleteObject(obj: H) -> i32;
    fn StretchBlt(
        d: H,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        s: H,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        rop: u32,
    ) -> i32;
    fn CreatePen(style: i32, width: i32, color: u32) -> H;
    fn Rectangle(dc: H, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn TextOutW(dc: H, x: i32, y: i32, text: *const u16, len: i32) -> i32;
    fn GetStockObject(index: i32) -> H;
}
#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(msg: u32, data: *const Notify) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
    fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32;
    fn CreateMutexW(attributes: *mut c_void, initial_owner: i32, name: *const u16) -> H;
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
    fn CloseHandle(handle: H) -> i32;
    fn Sleep(milliseconds: u32);
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

const INSTANCE_MUTEX_NAME: &str = r"Local\ShotKitRustPrototype.RustOnly.SingleInstance.v1";
const INSTANCE_CLASS_NAME: &str = "ShotKitRustPrototype";
const WM_COPYDATA: u32 = 0x004a;
const WM_INSTANCE_ACTIVATE: u32 = 0x8004;
const WM_FINISH_QUIT: u32 = 0x8005;
const INSTANCE_COPYDATA_TAG: usize = 0x534b_5255;
const MAX_IPC_PATH_UNITS: usize = 32_768;
static INSTANCE_FILE_BUSY: AtomicBool = AtomicBool::new(false);

struct InstanceFileBusyGuard;
impl Drop for InstanceFileBusyGuard {
    fn drop(&mut self) {
        INSTANCE_FILE_BUSY.store(false, Ordering::SeqCst);
    }
}

#[repr(C)]
struct CopyData {
    tag: usize,
    bytes: u32,
    data: *const c_void,
}

struct InstanceHandle(H);
impl Drop for InstanceHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0); }
        }
    }
}

unsafe fn create_instance_mutex() -> Result<(InstanceHandle, bool), String> {
    let name = wide(INSTANCE_MUTEX_NAME);
    SetLastError(0);
    let handle = CreateMutexW(null_mut(), 0, name.as_ptr());
    if handle.is_null() {
        return Err(format!("Einzelinstanz-Sperre konnte nicht erstellt werden (Win32 {})", GetLastError()));
    }
    let already_exists = GetLastError() == 183; // ERROR_ALREADY_EXISTS
    Ok((InstanceHandle(handle), already_exists))
}

fn absolute_argument_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() { path } else { std::env::current_dir().unwrap_or_default().join(path) }
}

unsafe fn find_primary_window(timeout_ms: u32) -> H {
    let class = wide(INSTANCE_CLASS_NAME);
    let mut waited = 0;
    while waited <= timeout_ms {
        let window = FindWindowW(class.as_ptr(), std::ptr::null());
        if !window.is_null() { return window; }
        Sleep(50);
        waited += 50;
    }
    null_mut()
}

unsafe fn send_to_primary(message: u32, path: Option<&Path>) -> Result<(), String> {
    let window = find_primary_window(5_000);
    if window.is_null() {
        return Err("Die bereits gestartete ShotKit-Instanz hat ihr Fenster nicht rechtzeitig bereitgestellt.".into());
    }
    let mut result = 0usize;
    let mut copy_data = CopyData { tag: INSTANCE_COPYDATA_TAG, bytes: 0, data: null_mut() };
    // Keep the backing buffer alive through SendMessageTimeoutW: WM_COPYDATA
    // borrows this memory synchronously in the receiving window procedure.
    let mut units: Option<Vec<u16>> = None;
    let (wparam, lparam) = if let Some(path) = path {
        let mut encoded: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if encoded.len() > MAX_IPC_PATH_UNITS {
            return Err("Der Dateipfad ist zu lang, um an die laufende ShotKit-Instanz weitergegeben zu werden.".into());
        }
        copy_data.bytes = u32::try_from(encoded.len().checked_mul(2).ok_or("Dateipfadgröße überläuft")?)
            .map_err(|_| "Dateipfadgröße überläuft")?;
        copy_data.data = encoded.as_mut_ptr().cast();
        units = Some(encoded);
        (0usize, (&copy_data as *const CopyData) as isize)
    } else {
        (0usize, 0isize)
    };
    let sent = SendMessageTimeoutW(
        window,
        if path.is_some() { WM_COPYDATA } else { message },
        wparam,
        lparam,
        0x0002, // SMTO_ABORTIFHUNG
        15_000,
        &mut result,
    );
    // Make the lifetime explicit to readers and prevent an unused-variable
    // warning from obscuring the synchronous-borrow guarantee above.
    drop(units);
    if sent == 0 {
        return Err("Keine rechtzeitige Rückmeldung von der laufenden ShotKit-Instanz. Die Datei kann dort noch zur Bestätigung angezeigt werden.".into());
    }
    if path.is_some() && result != 1 {
        return Err(match result {
            2 => "Das Öffnen der Datei wurde in der laufenden ShotKit-Instanz abgebrochen.".into(),
            3 => "ShotKit bearbeitet gerade eine andere Dateianfrage. Bitte versuchen Sie es erneut.".into(),
            _ => "Die laufende ShotKit-Instanz konnte die Datei nicht öffnen (ungültige Datei oder Ein-/Ausgabefehler).".into(),
        });
    }
    Ok(())
}

unsafe fn forward_second_instance() -> Result<(), String> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        return send_to_primary(WM_INSTANCE_ACTIVATE, None);
    }
    for path in paths {
        send_to_primary(WM_COPYDATA, Some(&absolute_argument_path(path)))?;
    }
    Ok(())
}

unsafe fn handle_instance_file(h: H, l: L) -> L {
    if l == 0 { return 0; }
    let packet = &*(l as *const CopyData);
    if packet.tag != INSTANCE_COPYDATA_TAG
        || packet.bytes < 2
        || packet.bytes as usize > MAX_IPC_PATH_UNITS * 2
        || packet.bytes % 2 != 0
        || packet.data.is_null()
    {
        return 0;
    }
    let count = packet.bytes as usize / 2;
    let units = std::slice::from_raw_parts(packet.data.cast::<u16>(), count);
    if units.last() != Some(&0) || units[..count - 1].contains(&0) {
        return 0;
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(&units[..count - 1]));
    if INSTANCE_FILE_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return 3;
    }
    let _busy_guard = InstanceFileBusyGuard;
    let prompt = format!(
        "Eine zweite ShotKit-Instanz möchte diese Datei öffnen:\n\n{}\n\nSoll sie jetzt geöffnet werden?",
        path.display()
    );
    if MessageBoxW(h, wide(&prompt).as_ptr(), wide("Datei aus zweiter Instanz").as_ptr(), 0x24) != 6 {
        return 2;
    }
    match open_image_path(h, path) {
        Ok(()) => 1,
        Err(error) => {
            MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
            0
        }
    }
}
const COPY: u32 = 0x00cc0020;
const TRAY: u32 = 0x8001;
const SCROLL_DONE: u32 = 0x8002;
const DELAYED_CAPTURE: u32 = 0x8003;
static DELAYED_CAPTURE_STATE: Mutex<DelayedCaptureState> = Mutex::new(DelayedCaptureState::new());
fn delayed_capture_pending() -> bool {
    DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).is_pending()
}
static SCROLL_BUSY: AtomicBool = AtomicBool::new(false);
static SCROLL_RESULT: Mutex<Option<Result<Outcome, String>>> = Mutex::new(None);
static RECOVERY: Mutex<Vec<Image>> = Mutex::new(Vec::new());
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
static SETTINGS_WRITABLE: AtomicBool = AtomicBool::new(false);
static RESTORE_EDITOR: AtomicBool = AtomicBool::new(false);
struct BusyGuard(bool);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if self.0 {
            SCROLL_BUSY.store(false, Ordering::SeqCst);
        }
    }
}
// UI-thread-only state. Each editor HWND owns a separate allocation in
// GWLP_USERDATA; no active-window swapping is used, including during modal loops.
struct WindowState {
    toolbar: Option<Rc<toolbar::Toolbar>>,
    editor: Option<EditorController>,
    bitmap: H,
    current_path: Option<PathBuf>,
    dirty: bool,
    zoom: f64,
    pan_x: i32,
    pan_y: i32,
    width: i32,
    height: i32,
    drawing: bool,
    crop_start: Option<(i32, i32)>,
    crop_end: Option<(i32, i32)>,
    modal_depth: usize,
}
impl WindowState {
    fn new() -> Self {
        Self { toolbar: None, editor: None, bitmap: null_mut(), current_path: None,
            dirty: false, zoom: 1.0, pan_x: 0, pan_y: 0, width: 0, height: 0,
            drawing: false, crop_start: None, crop_end: None, modal_depth: 0 }
    }
}
static mut PRIMARY: H = null_mut();
static mut EDITOR_WINDOWS: Vec<H> = Vec::new();
static QUITTING: AtomicBool = AtomicBool::new(false);
unsafe fn window_state(h: H) -> &'static mut WindowState {
    &mut *(GetWindowLongPtrW(h, -21) as *mut WindowState)
}
unsafe fn editor(h: H) -> &'static mut EditorController {
    window_state(h).editor.get_or_insert_with(EditorController::new)
}
unsafe fn toolbar_height(h: H) -> i32 {
    window_state(h).toolbar.as_ref().map_or(0, |bar| bar.height())
}
// Do not retain a Rust borrow across a Win32 dialog/message pump.
struct ModalGuard(H);
impl ModalGuard {
    unsafe fn new(h: H) -> Self {
        window_state(h).modal_depth += 1;
        Self(h)
    }
}
impl Drop for ModalGuard {
    fn drop(&mut self) {
        unsafe { window_state(self.0).modal_depth -= 1; }
    }
}
unsafe fn editor_windows() -> Vec<H> {
    (&*std::ptr::addr_of!(EDITOR_WINDOWS)).clone()
}
unsafe fn refresh_title(h: H) {
    let state = window_state(h);
    let name = state.current_path.as_ref().map(|p| p.display().to_string())
        .unwrap_or_else(|| if state.bitmap.is_null() { "Bild öffnen oder einfügen".into() } else { "Neue Aufnahme".into() });
    let tool = state.editor.as_ref().map(|editor| format!(" · Werkzeug {:?}", editor.tool())).unwrap_or_default();
    let title = format!("{}{} – ShotKit Rust{} · Strg+S speichern", if state.dirty { "* " } else { "" }, name, tool);
    SetWindowTextW(h, wide(&title).as_ptr());
}
unsafe fn show_editors() {
    if QUITTING.load(Ordering::SeqCst) { return; }
    let mut windows = editor_windows();
    if windows.is_empty() {
        match create_editor_window() {
            Ok(h) => {
                (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(h);
                refresh_title(h);
                windows.push(h);
            }
            Err(error) => {
                MessageBoxW(PRIMARY, wide(&error).as_ptr(), wide("Editor öffnen").as_ptr(), 0x10);
                return;
            }
        }
    }
    for h in &windows { ShowWindow(*h, 9); } // SW_RESTORE also restores minimized editors
    if let Some(h) = windows.last() { SetForegroundWindow(*h); }
}
unsafe fn quit_application(h: H) {
    if SCROLL_BUSY.load(Ordering::SeqCst) || delayed_capture_pending() {
        MessageBoxW(h, wide("Bitte die laufende oder geplante Aufnahme zuerst abschließen.").as_ptr(), wide("ShotKit Rust").as_ptr(), 0x30);
        return;
    }
    if QUITTING.swap(true, Ordering::SeqCst) { return; }
    let windows = editor_windows();
    // Confirm all documents before destroying any. A cancelled save or close
    // preserves every window, including previously confirmed discard choices.
    let approved = windows.iter().all(|w| window_state(*w).modal_depth == 0)
        && resolve_recovery(h)
        && windows.iter().all(|w| confirm_replace(*w));
    if approved {
        for window in windows { DestroyWindow(window); }
        PostMessageW(PRIMARY, WM_FINISH_QUIT, 0, 0);
    } else {
        QUITTING.store(false, Ordering::SeqCst);
    }
}
fn active_settings() -> Settings {
    SETTINGS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_default()
}
unsafe fn load_settings(h: H) {
    let result = settings::settings_path().and_then(settings::load_or_default);
    match result {
        Ok(value) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
            SETTINGS_WRITABLE.store(true, Ordering::SeqCst);
        }
        Err(e) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(Settings::default());
            SETTINGS_WRITABLE.store(false, Ordering::SeqCst);
            MessageBoxW(h,wide(&format!("Einstellungen konnten nicht geladen werden: {e}\nDie Standardwerte werden vorübergehend verwendet.")).as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        }
    }
}
unsafe fn save_settings(h: H, new: Settings) -> bool {
    if !SETTINGS_WRITABLE.load(Ordering::SeqCst) {
        MessageBoxW(h,wide("Einstellungen können nicht gespeichert werden, solange die vorhandene Datei nicht lesbar ist.").as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        return false;
    }
    let result = settings::settings_path().and_then(|path| settings::save_atomic(path, &new));
    match result {
        Ok(()) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(new);
            true
        }
        Err(e) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Einstellungen konnten nicht gespeichert werden: {e}"
                ))
                .as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            false
        }
    }
}
unsafe fn cancel_gesture(h: H) {
    let had_capture = window_state(h).drawing;
    window_state(h).drawing = false;
    window_state(h).crop_start = None;
    window_state(h).crop_end = None;
    if let Some(state) = (&mut window_state(h).editor).as_mut() {
        state.cancel_drag();
    }
    if had_capture {
        ReleaseCapture();
    }
}
unsafe fn handle_capture(h: H, image: Image, was_visible: bool) -> Result<(), String> {
    let prefs = active_settings();
    let mut output = CaptureOutputState::new(&prefs);
    let mut image = Some(image);
    let mut saved = None;
    let mut opened = false;
    while let Some(action) = output.next_action() {
        let result = match action {
            CaptureAction::OpenEditor => {
                let owned = image.take().ok_or("Aufnahme ist nicht mehr verfügbar")?;
                let result = display_image(h, owned);
                match result {
                    Ok(window) => {
                        opened = true;
                        if let Some(path) = saved.take() {
                            window_state(window).current_path = Some(path);
                            window_state(window).dirty = false;
                            refresh_title(window);
                        }
                        if was_visible { ShowWindow(h, 5); }
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            _ => {
                let result = output_windows::execute_action(
                    action,
                    &prefs,
                    image.as_ref().ok_or("Aufnahme ist nicht mehr verfügbar")?,
                    h,
                );
                match result {
                    Ok(result) => {
                        if result.saved_path.is_some() {
                            saved = result.saved_path;
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        };
        let advance = match output.finish_action(action, result) {
            Ok(value) => value,
            Err(error) => {
                if let Some(image) = image.take() {
                    RECOVERY
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(image);
                }
                return Err(format!(
                    "Ausgabe konnte nicht abgeschlossen werden: {error}"
                ));
            }
        };
        if advance == Advance::RetainImage {
            return Err("Die Aufnahme konnte nicht im Editor geöffnet werden und bleibt im Speicher. Über das Tray-Menü kann sie gesichert werden.".into());
        }
    }
    if !opened {
        if was_visible {
            ShowWindow(h, 5);
        }
        let message = match saved {
            Some(path) => format!("Aufnahme gespeichert: {}", path.display()),
            None => "Aufnahme in die Zwischenablage kopiert.".into(),
        };
        MessageBoxW(
            h,
            wide(&message).as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x40,
        );
    }
    if !output.failures().is_empty() {
        let details = output
            .failures()
            .iter()
            .map(|f| format!("{:?}: {}", f.action, f.message))
            .collect::<Vec<_>>()
            .join("\n");
        MessageBoxW(
            h,
            wide(&format!(
                "Ein Teil der Ausgabe ist fehlgeschlagen:\n{details}"
            ))
            .as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x30,
        );
    }
    Ok(())
}
unsafe fn capture_rect(h: H, target: Result<capture_targets::ScreenRect, String>) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _busy = BusyGuard(true);
    let rect = match target {
        Ok(r) => r,
        Err(e) => {
            MessageBoxW(h, wide(&e).as_ptr(), wide("Aufnahme").as_ptr(), 0x10);
            return;
        }
    };
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let image = scroll_windows::WindowsScrollIo::new(
        rect.left(),
        rect.top(),
        rect.width(),
        rect.height(),
        0,
    )
    .and_then(|mut io| io.capture());
    match image.and_then(|image| handle_capture(h, image, was_visible)) {
        Ok(()) => {}
        Err(e) => {
            if was_visible {
                ShowWindow(h, 5);
            }
            MessageBoxW(
                h,
                wide(&e).as_ptr(),
                wide("Bildschirmaufnahme").as_ptr(),
                0x10,
            );
        }
    }
}
unsafe fn capture(h: H) {
    capture_rect(h, capture_targets::monitor_under_cursor());
}
unsafe fn schedule_capture(h: H, seconds: u64) {
    let request = DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).begin();
    let ticket = match request {
        Ok(ticket) => ticket,
        Err(error) => {
            let message = match error {
                BeginError::Busy => "Eine verzögerte Aufnahme läuft bereits.",
                BeginError::Exhausted => "Keine weitere verzögerte Aufnahme möglich. Bitte ShotKit neu starten.",
            };
            MessageBoxW(h, wide(message).as_ptr(), wide("Aufnahme").as_ptr(), 0x30);
            return;
        }
    };
    post_capture_after(h, ticket, std::time::Duration::from_secs(seconds));
}
fn post_capture_after(h: H, ticket: CaptureTicket, delay: std::time::Duration) {
    let hwnd = h as isize;
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if !DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).is_waiting(ticket) {
            return;
        }
        // Completion can race with this PostMessage. The receiver therefore
        // checks the same generation and claims Waiting -> Running exactly once.
        if unsafe { PostMessageW(hwnd as H, DELAYED_CAPTURE, ticket.to_wire(), 0) } == 0 {
            DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).cancel_waiting(ticket);
        }
    });
}
unsafe fn capture_active(h: H, window: H) {
    let target = match active_window_target(h, window) {
        Ok(snapshot) => resolve_active_window_target(h, &snapshot),
        Err(error) => Err(error),
    };
    capture_rect(h, target);
}
struct ActiveWindowSnapshot {
    hwnd: H,
    process_id: u32,
    thread_id: u32,
    class_name: String,
}
unsafe fn active_window_target(
    h: H,
    window: H,
) -> Result<ActiveWindowSnapshot, String> {
    if window.is_null() || IsWindow(window) == 0 {
        return Err("Windows hat kein gültiges aktives Fenster geliefert.".into());
    }
    let root = GetAncestor(window, 2);
    let root = if root.is_null() { window } else { root };
    if root == h || editor_windows().iter().any(|editor| *editor == root) {
        return Err("ShotKit war das aktive Fenster. Bitte zuerst das gewünschte Fenster aktivieren und erneut wählen.".into());
    }
    let mut class_name = [0u16; 256];
    let class_len = GetClassNameW(root, class_name.as_mut_ptr(), class_name.len() as i32);
    if class_len <= 0 {
        return Err("Windows konnte die Klasse des aktiven Fensters nicht ermitteln.".into());
    }
    let class_name = String::from_utf16_lossy(&class_name[..class_len as usize]);
    let shell = GetShellWindow();
    if root == shell
        || matches!(
            class_name.as_str(),
            "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "TrayNotifyWnd"
                | "NotifyIconOverflowWindow"
                | "TopLevelWindowForOverflowXamlIsland"
                | "Progman"
                | "WorkerW"
        )
    {
        return Err("Die Windows-Shell war im Vordergrund; ein Zielfenster lässt sich daraus nicht sicher bestimmen. Bitte gewünschtes Fenster aktivieren und erneut wählen.".into());
    }
    let mut process_id = 0;
    let thread_id = GetWindowThreadProcessId(root, &mut process_id);
    if thread_id == 0 || process_id == 0 {
        return Err("Windows konnte Prozess und Thread des aktiven Fensters nicht bestimmen.".into());
    }
    Ok(ActiveWindowSnapshot {
        hwnd: root,
        process_id,
        thread_id,
        class_name,
    })
}
unsafe fn resolve_active_window_target(
    h: H,
    saved: &ActiveWindowSnapshot,
) -> Result<capture_targets::ScreenRect, String> {
    if IsWindow(saved.hwnd) == 0 {
        return Err("Das zuvor aktive Fenster wurde geschlossen, bevor die Aufnahme startete.".into());
    }
    let current = active_window_target(h, saved.hwnd).map_err(|_| {
        "Das zuvor aktive Fenster ist nicht mehr als dasselbe gültige Zielfenster verfügbar.".to_string()
    })?;
    if current.hwnd != saved.hwnd
        || current.process_id != saved.process_id
        || current.thread_id != saved.thread_id
        || current.class_name != saved.class_name
    {
        return Err("Das zuvor aktive Fenster wurde geschlossen oder sein HWND inzwischen wiederverwendet. Bitte den Tray-Befehl erneut wählen.".into());
    }
    capture_targets::saved_window(saved.hwnd as isize)
}
unsafe fn open_storage_folder(h: H) {
    let Some(folder) = active_settings().save_folder else {
        MessageBoxW(
            h,
            wide("Es ist kein Speicherordner eingestellt. Wählen Sie zuerst unter Einstellungen einen Speicherordner.").as_ptr(),
            wide("Speicherordner öffnen").as_ptr(),
            0x30,
        );
        return;
    };
    match std::fs::metadata(&folder) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            MessageBoxW(
                h,
                wide(&format!("Der eingestellte Speicherpfad ist kein Ordner:\n{}", folder.display())).as_ptr(),
                wide("Speicherordner öffnen").as_ptr(),
                0x10,
            );
            return;
        }
        Err(error) => {
            MessageBoxW(
                h,
                wide(&format!("Der eingestellte Speicherordner ist nicht verfügbar:\n{}\n\n{error}", folder.display())).as_ptr(),
                wide("Speicherordner öffnen").as_ptr(),
                0x10,
            );
            return;
        }
    }
    let path: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = ShellExecuteW(
        h,
        wide("open").as_ptr(),
        path.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        5,
    );
    if result as usize <= 32 {
        MessageBoxW(
            h,
            wide(&format!("Windows konnte den Speicherordner nicht öffnen:\n{}\n\nShellExecuteW meldete Fehlercode {}.", folder.display(), result as usize)).as_ptr(),
            wide("Speicherordner öffnen").as_ptr(),
            0x10,
        );
    }
}
unsafe fn image_view(h: H) -> (i32, i32, i32, i32) {
    let mut r: Rect = zeroed();
    GetClientRect(h, &mut r);
    let top = toolbar_height(h);
    let v = view::layout(
        (window_state(h).width.max(1) as u32, window_state(h).height.max(1) as u32),
        (0, top, r.right - r.left, r.bottom - r.top - top),
        window_state(h).zoom,
        (window_state(h).pan_x, window_state(h).pan_y),
    );
    (v.x, v.y, v.width, v.height)
}
unsafe fn point(h: H, l: L) -> (i32, i32) {
    let (x, y, w, height) = image_view(h);
    let px = (l as u32 as u16 as i16) as i32;
    let py = ((l as u32 >> 16) as u16 as i16) as i32;
    view::to_image(
        view::View {
            x,
            y,
            width: w,
            height,
        },
        (window_state(h).width.max(1) as u32, window_state(h).height.max(1) as u32),
        (px, py),
    )
}
// Build a temporary composited bitmap. The base image remains unchanged so
// annotation objects can be selected and undone. Caller owns the returned HBITMAP.
unsafe fn composited_bitmap(h: H, preview: bool) -> Result<H, String> {
    if window_state(h).bitmap.is_null() {
        return Err("Kein Bild geöffnet.".into());
    }
    let (crop_start, crop_end) = (window_state(h).crop_start, window_state(h).crop_end);
    let state = editor(h);
    let mut draft = if preview { state.preview() } else { None };
    let mut shapes = Vec::with_capacity(state.shapes().len() + usize::from(draft.is_some()));
    for (i, shape) in state.shapes().iter().enumerate() {
        if draft.is_some() && state.tool() == Tool::Select && state.selected_index() == Some(i) {
            shapes.push(draft.take().expect("draft checked above"));
        } else {
            shapes.push(shape.clone());
        }
    }
    if let Some(shape) = draft {
        shapes.push(shape);
    }
    let source = state.image_arc();
    let rendered = shotkit_rust_prototype::alpha_document::render_document(&source, &shapes)?;
    // StretchBlt ignores the DIB alpha channel. Flatten only this on-screen
    // preview against a checkerboard so transparent source pixels and
    // translucent annotations are displayed using the same source-over math
    // as the document renderer. Export continues to use the original alpha.
    let display_image = if preview {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(rendered.pixels.len())
            .map_err(|_| "Nicht genug Speicher für die Bildvorschau.".to_owned())?;
        for (index, src) in rendered.pixels.chunks_exact(4).enumerate() {
            let x = index % rendered.width;
            let y = index / rendered.width;
            let gray = if ((x / 12) + (y / 12)) % 2 == 0 { 236 } else { 196 };
            let background = [gray, gray, gray, 255];
            let pixel = shotkit_rust_prototype::alpha_composite::source_over(
                background,
                [src[0], src[1], src[2], src[3]],
                255,
            );
            pixels.extend_from_slice(&pixel);
        }
        Image::new(rendered.width, rendered.height, pixels).map_err(str::to_owned)?
    } else {
        rendered
    };
    let output = create_bitmap(&display_image)?;
    if !preview || (state.tool() != Tool::Crop && state.tool() != Tool::Select) {
        return Ok(output);
    }
    let screen = GetDC(null_mut());
    if screen.is_null() {
        DeleteObject(output);
        return Err("Bildschirm konnte nicht geöffnet werden.".into());
    }
    let out_dc = CreateCompatibleDC(screen);
    if out_dc.is_null() {
        ReleaseDC(null_mut(), screen);
        DeleteObject(output);
        return Err("Bild konnte nicht zusammengesetzt werden.".into());
    }
    let old_out = SelectObject(out_dc, output);
    if old_out.is_null() || old_out as isize == -1 {
        DeleteDC(out_dc);
        ReleaseDC(null_mut(), screen);
        DeleteObject(output);
        return Err("Bild konnte nicht ausgewählt werden.".into());
    }
    if preview && state.tool() == Tool::Crop {
        if let (Some(a), Some(b)) = (crop_start, crop_end) {
            let pen = CreatePen(0, 2, 0x00ffffff);
            let previous_pen = SelectObject(out_dc, pen);
            let previous_brush = SelectObject(out_dc, GetStockObject(5));
            Rectangle(out_dc, a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
            SelectObject(out_dc, previous_brush);
            SelectObject(out_dc, previous_pen);
            DeleteObject(pen);
        }
    }
    if preview && state.tool() == Tool::Select {
        if let Some(shape) = state.selection() {
            let b = shape.bounds();
            let pen = CreatePen(0, 1, 0x00d77800);
            let old_pen = SelectObject(out_dc, pen);
            let old_brush = SelectObject(out_dc, GetStockObject(5));
            Rectangle(out_dc, b.x as i32 - 3, b.y as i32 - 3,
                (b.x + b.width) as i32 + 3, (b.y + b.height) as i32 + 3);
            for p in shape.handles() {
                Rectangle(out_dc, p.x as i32 - 4, p.y as i32 - 4, p.x as i32 + 5, p.y as i32 + 5);
            }
            SelectObject(out_dc, old_brush);
            SelectObject(out_dc, old_pen);
            DeleteObject(pen);
        }
    }
    SelectObject(out_dc, old_out);
    DeleteDC(out_dc);
    ReleaseDC(null_mut(), screen);
    Ok(output)
}

unsafe fn bitmap_image(h: H) -> Result<Image, String> {
    if window_state(h).bitmap.is_null() || window_state(h).width <= 0 || window_state(h).height <= 0 {
        return Err("Kein Bild geöffnet.".into());
    }
    let state = editor(h);
    shotkit_rust_prototype::alpha_document::render_document(&state.image_arc(), state.shapes())
}

fn path_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("Ungültiger Speicherordner.")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = dir.join(format!(".shotkit-{}-{stamp}.tmp", std::process::id()));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if unsafe {
            MoveFileExW(
                path_wide(&temp).as_ptr(),
                path_wide(path).as_ptr(),
                0x1 | 0x8,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
unsafe fn save(h: H, save_as: bool) -> bool {
    let _modal = ModalGuard::new(h);
    cancel_gesture(h);
    if window_state(h).bitmap.is_null() {
        return false;
    }
    let previous = (&window_state(h).current_path).clone();
    let path = if save_as || previous.is_none() {
        match file_dialog::save(h, previous.as_deref()) {
            Ok(Some(p)) => p,
            Ok(None) => return false,
            Err(e) => {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherdialog").as_ptr(), 0x10);
                return false;
            }
        }
    } else {
        previous.unwrap()
    };
    let result = (|| -> Result<(), String> {
        let format = image_io::format_from_path(&path)?;
        let image = bitmap_image(h)?;
        let bytes = image_io::encode(&image, format)?;
        write_atomic(&path, &bytes)
    })();
    match result {
        Ok(()) => {
            window_state(h).current_path = Some(path);
            window_state(h).dirty = false;
            refresh_title(h);
            true
        }
        Err(e) => {
            MessageBoxW(
                h,
                wide(&format!("Speichern fehlgeschlagen:\n{e}")).as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            false
        }
    }
}
unsafe fn resolve_recovery(h: H) -> bool {
    loop {
        if RECOVERY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
        {
            return true;
        }
        let answer=MessageBoxW(h,wide("Eine Aufnahme konnte nicht im Editor geöffnet werden und ist noch im Speicher. Jetzt speichern?").as_ptr(),wide("Ungesicherte Aufnahme").as_ptr(),0x23);
        if answer == 2 {
            return false;
        }
        if answer == 7 {
            RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).pop();
            continue;
        }
        if answer != 6 {
            return false;
        }
        let path = match file_dialog::save(h, None) {
            Ok(Some(path)) => path,
            Ok(None) => return false,
            Err(e) => {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherdialog").as_ptr(), 0x10);
                return false;
            }
        };
        let result = {
            let pending = RECOVERY.lock().unwrap_or_else(|p| p.into_inner());
            let Some(image) = pending.last() else {
                continue;
            };
            image_io::format_from_path(&path)
                .and_then(|format| image_io::encode(image, format))
                .and_then(|bytes| write_atomic(&path, &bytes))
        };
        match result {
            Ok(()) => {
                RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).pop();
            }
            Err(e) => {
                MessageBoxW(
                    h,
                    wide(&e).as_ptr(),
                    wide("Speichern fehlgeschlagen").as_ptr(),
                    0x10,
                );
                return false;
            }
        }
    }
}
unsafe fn confirm_replace(h: H) -> bool {
    let _modal = ModalGuard::new(h);
    cancel_gesture(h);
    if !window_state(h).dirty || window_state(h).bitmap.is_null() {
        return true;
    }
    match MessageBoxW(
        h,
        wide("Das Bild enthält ungespeicherte Änderungen. Jetzt speichern?").as_ptr(),
        wide("ShotKit Rust").as_ptr(),
        0x23,
    ) {
        6 => save(h, false),
        7 => true,
        _ => false,
    }
}
unsafe fn paste_image(h: H) {
    match clipboard::read_image(h) {
        Ok(Some(image)) => {
            if let Err(e) = display_image(h, image) {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Zwischenablage").as_ptr(), 0x10);
            }
        }
        Ok(None) => {
            MessageBoxW(
                h,
                wide("Die Zwischenablage enthält kein Bild.").as_ptr(),
                wide("Zwischenablage").as_ptr(),
                0x40,
            );
        }
        Err(e) => {
            MessageBoxW(h, wide(&e).as_ptr(), wide("Zwischenablage").as_ptr(), 0x10);
        }
    }
}
unsafe fn open_image(h: H) {
    let path = match file_dialog::open(h) {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(e) => {
            MessageBoxW(h, wide(&e).as_ptr(), wide("Öffnen-Dialog").as_ptr(), 0x10);
            return;
        }
    };
    if let Err(error) = open_image_path(h, path) {
        MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
    }
}
unsafe fn open_image_path(h: H, path: PathBuf) -> Result<(), String> {
    let result = (|| -> Result<Image, String> {
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > 512 * 1024 * 1024 {
            return Err("Die Bilddatei ist zu groß.".into());
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        image_io::decode(&bytes)
    })();
    match result {
        Ok(image) => match display_image(h, image) {
            Ok(window) => {
                window_state(window).current_path = Some(path);
                window_state(window).dirty = false;
                refresh_title(window);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    }
}
unsafe fn create_bitmap(image: &Image) -> Result<H, String> {
    let width = i32::try_from(image.width).map_err(|_| "Bild zu breit".to_owned())?;
    let height = i32::try_from(image.height).map_err(|_| "Bild zu hoch".to_owned())?;
    let info = Bmi {
        header: BmiHeader {
            size: 40,
            width,
            height: -height,
            planes: 1,
            bits: 32,
            compression: 0,
            image_size: image.pixels.len() as u32,
            xppm: 0,
            yppm: 0,
            used: 0,
            important: 0,
        },
        colors: [0],
    };
    let dc = GetDC(null_mut());
    if dc.is_null() {
        return Err("Bildschirm konnte nicht geöffnet werden.".into());
    }
    let mut bits: *mut c_void = null_mut();
    let bmp = CreateDIBSection(dc, &info, 0, &mut bits, null_mut(), 0);
    ReleaseDC(null_mut(), dc);
    if bmp.is_null() || bits.is_null() {
        if !bmp.is_null() {
            DeleteObject(bmp);
        }
        return Err("Ausgabebild konnte nicht angelegt werden.".into());
    }
    std::ptr::copy_nonoverlapping(image.pixels.as_ptr(), bits.cast::<u8>(), image.pixels.len());
    Ok(bmp)
}
unsafe fn install_bitmap(h: H, bmp: H, image: &Image) {
    if !window_state(h).bitmap.is_null() {
        DeleteObject(window_state(h).bitmap);
    }
    window_state(h).bitmap = bmp;
    window_state(h).width = image.width as i32;
    window_state(h).height = image.height as i32;
}
// Return a hidden, initialized editor shell. Register it in EDITOR_WINDOWS only
// after its optional image has been installed successfully.
unsafe fn create_editor_window() -> Result<H, String> {
    let class = wide("ShotKitRustEditor");
    let h = CreateWindowExW(0, class.as_ptr(), wide("ShotKit Rust").as_ptr(),
        0x00cf0000, 0x80000000u32 as i32, 0x80000000u32 as i32, 1000, 700,
        null_mut(), null_mut(), GetModuleHandleW(std::ptr::null()), null_mut());
    if h.is_null() { return Err("Editorfenster konnte nicht erstellt werden.".into()); }
    let bar = match toolbar::create(h) {
        Ok(bar) => Rc::new(bar),
        Err(error) => { DestroyWindow(h); return Err(error); }
    };
    // Drop replaced native objects only after the mutable state borrow ends.
    let previous = window_state(h).toolbar.replace(bar);
    drop(previous);
    Ok(h)
}
unsafe fn display_image(_owner: H, image: Image) -> Result<H, String> {
    if QUITTING.load(Ordering::SeqCst) {
        RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).push(image);
        return Err("ShotKit wird gerade beendet. Die Aufnahme bleibt im Speicher.".into());
    }
    let h = match create_editor_window() {
        Ok(h) => h,
        Err(error) => {
            RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).push(image);
            return Err(format!("{error} Die Aufnahme bleibt im Speicher."));
        }
    };
    match initialize_image(h, image) {
        Ok(()) => {
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(h);
            refresh_title(h);
            ShowWindow(h, 5);
            SetForegroundWindow(h);
            Ok(h)
        }
        Err(error) => { DestroyWindow(h); Err(error) }
    }
}
unsafe fn initialize_image(h: H, image: Image) -> Result<(), String> {
    let bmp = match create_bitmap(&image) {
        Ok(bmp) => bmp,
        Err(error) => {
            RECOVERY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(image);
            return Err(format!("{error} Die Aufnahme bleibt im Speicher."));
        }
    };
    let state = match EditorController::with_image_recover(image) {
        Ok(state) => state,
        Err((e, image)) => {
            DeleteObject(bmp);
            RECOVERY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(image);
            return Err(format!(
                "Bild konnte nicht geladen werden: {e:?}. Die Aufnahme bleibt im Speicher."
            ));
        }
    };
    let image = state.image_arc();
    cancel_gesture(h);
    install_bitmap(h, bmp, &image);
    window_state(h).editor = Some(state);
    window_state(h).current_path = None;
    window_state(h).dirty = true;
    window_state(h).zoom = 1.0;
    window_state(h).pan_x = 0;
    window_state(h).pan_y = 0;
    SetWindowTextW(
        h,
        wide("ShotKit Rust – Strg+S speichern · F10 Scroll-Aufnahme").as_ptr(),
    );
    InvalidateRect(h, std::ptr::null(), 1);
    Ok(())
}
unsafe fn apply_crop(h: H, a: (i32, i32), b: (i32, i32)) {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    let width = a.0.max(b.0) - x;
    let height = a.1.max(b.1) - y;
    if width < 3 || height < 3 {
        return;
    }
    let rect = CropRect {
        x: x as usize,
        y: y as usize,
        width: width as usize,
        height: height as usize,
    };
    let result = (|| -> Result<(), String> {
        editor(h)
            .document_mut()
            .begin_edit()
            .map_err(|e| format!("{e:?}"))?;
        if let Err(e) = editor(h).crop(rect) {
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{e:?}"));
        }
        let output = editor(h).image_arc();
        let bitmap = match create_bitmap(&output) {
            Ok(bmp) => bmp,
            Err(e) => {
                let _ = editor(h).document_mut().abort_edit();
                return Err(e);
            }
        };
        if let Err(e) = editor(h).document_mut().commit_edit() {
            DeleteObject(bitmap);
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{e:?}"));
        }
        install_bitmap(h, bitmap, &output);
        window_state(h).dirty = true;
        window_state(h).zoom = 1.0;
        window_state(h).pan_x = 0;
        window_state(h).pan_y = 0;
        Ok(())
    })();
    if let Err(e) = result {
        MessageBoxW(
            h,
            wide(&format!("Zuschneiden fehlgeschlagen: {e}")).as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x10,
        );
    }
}
unsafe fn history(h: H, redo: bool) {
    cancel_gesture(h);
    let before = editor(h).image_arc();
    let next = if redo {
        editor(h).document().redo_image_arc()
    } else {
        editor(h).document().undo_image_arc()
    };
    let Some(next) = next else {
        return;
    };
    // Allocate the replacement GDI bitmap before changing history. On failure,
    // both the document and the displayed image remain at their old state.
    let replacement = if std::sync::Arc::ptr_eq(&before, &next) {
        None
    } else {
        match create_bitmap(&next) {
            Ok(bitmap) => Some(bitmap),
            Err(e) => {
                MessageBoxW(
                    h,
                    wide(&e).as_ptr(),
                    wide("Rückgängig/Wiederholen").as_ptr(),
                    0x10,
                );
                return;
            }
        }
    };
    let changed = if redo {
        editor(h).redo()
    } else {
        editor(h).undo()
    };
    if changed {
        if let Some(bitmap) = replacement {
            install_bitmap(h, bitmap, &next);
        }
        window_state(h).dirty = true;
        InvalidateRect(h, std::ptr::null(), 1);
    } else if let Some(bitmap) = replacement {
        DeleteObject(bitmap);
    }
}
unsafe fn capture_region(h: H) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _busy = BusyGuard(true);
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    let Some((x, y, w, height)) = selection::select() else {
        if was_visible {
            ShowWindow(h, 5);
        }
        return;
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    let image =
        scroll_windows::WindowsScrollIo::new(x, y, w, height, 0).and_then(|mut io| io.capture());
    match image.and_then(|image| handle_capture(h, image, was_visible)) {
        Ok(()) => {}
        Err(e) => {
            if was_visible {
                ShowWindow(h, 5);
            }
            MessageBoxW(
                h,
                wide(&e).as_ptr(),
                wide("Bereichsaufnahme").as_ptr(),
                0x10,
            );
        }
    }
}

unsafe fn start_scroll(h: H) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let mut busy = BusyGuard(true);
    let previous_direction = active_settings().scroll_direction;
    let previous_label = match previous_direction {
        settings::ScrollDirection::Down => "nach unten",
        settings::ScrollDirection::Right => "nach rechts",
        settings::ScrollDirection::Left => "nach links",
    };
    let first_default = if previous_direction == settings::ScrollDirection::Down {
        0
    } else {
        0x0100 // MB_DEFBUTTON2: horizontal choice
    };
    let prompt = wide(&format!(
        "Scroll-Aufnahme (experimentell):\nJa = nach unten, Nein = nach rechts oder links, Abbrechen = schließen.\nZuletzt verwendet: {previous_label}.\nDanach den Inhaltsbereich mit der Maus aufziehen; Esc bricht ab."
    ));
    let choice = MessageBoxW(
        h,
        prompt.as_ptr(),
        wide("Scroll-Richtung").as_ptr(),
        0x23 | first_default,
    );
    let direction = match choice {
        6 => Direction::Down,
        7 => {
            let second_default = match previous_direction {
                settings::ScrollDirection::Left => 0x0100, // MB_DEFBUTTON2: left choice
                settings::ScrollDirection::Down | settings::ScrollDirection::Right => 0,
            };
            match MessageBoxW(
                h,
                wide("Ja = nach rechts, Nein = nach links, Abbrechen = schließen.").as_ptr(),
                wide("Scroll-Richtung").as_ptr(),
                0x23 | second_default,
            ) {
                6 => Direction::Right,
                7 => Direction::Left,
                _ => return,
            }
        }
        _ => return,
    };
    let mut prefs = active_settings();
    prefs.scroll_direction = match direction {
        Direction::Down => settings::ScrollDirection::Down,
        Direction::Right => settings::ScrollDirection::Right,
        Direction::Left => settings::ScrollDirection::Left,
    };
    // save_settings refuses to replace unreadable or unsupported settings
    // files; a persistence error must not prevent the requested capture.
    let _ = save_settings(h, prefs);
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    let Some((x, y, width, height)) = selection::select() else {
        if was_visible {
            ShowWindow(h, 5);
        }
        return;
    };
    if width < 100
        || height < 100
        || match direction {
            Direction::Down => height < 200,
            _ => width < 200,
        }
    {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(h, wide("Der Bereich muss mindestens 100 × 100 Pixel groß und in Scrollrichtung mindestens 200 Pixel lang sein.").as_ptr(), wide("Scroll-Aufnahme").as_ptr(), 0x30);
        return;
    }
    let target = WindowFromPoint(Point {
        x: x + width / 2,
        y: y + height / 2,
    });
    let root = if target.is_null() {
        null_mut()
    } else {
        GetAncestor(target, 2)
    };
    if root.is_null() {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Zielfenster konnte nicht ermittelt werden.").as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x30,
        );
        return;
    }
    SetForegroundWindow(root);
    if GetForegroundWindow() != root {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Zielfenster konnte nicht aktiviert werden. Bitte erneut versuchen.").as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x30,
        );
        return;
    }
    RESTORE_EDITOR.store(was_visible, Ordering::SeqCst);
    busy.0 = false;
    let hwnd = h as usize;
    let target_window = root as usize;
    std::thread::spawn(move || {
        let result = (|| -> Result<Outcome, String> {
            let mut io = scroll_windows::WindowsScrollIo::new(x, y, width, height, target_window)?;
            std::thread::sleep(std::time::Duration::from_millis(350));
            scroll_capture::run(&mut io, direction)
        })();
        *SCROLL_RESULT.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
        if PostMessageW(hwnd as H, SCROLL_DONE, 0, 0) == 0 {
            SCROLL_RESULT
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            SCROLL_BUSY.store(false, Ordering::SeqCst);
        }
    });
}

unsafe fn report_editor_error(h: H) {
    if let Some(error) = editor(h).take_error() {
        MessageBoxW(
            h,
            wide(&format!(
                "Die Bearbeitung konnte nicht übernommen werden: {error:?}"
            ))
            .as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x10,
        );
    }
}
unsafe fn copy_current(h: H) {
    if window_state(h).bitmap.is_null() {
        return;
    }
    let copied = composited_bitmap(h, false).and_then(|bmp| {
        let result = clipboard::copy_bitmap(h, bmp, window_state(h).width, window_state(h).height);
        DeleteObject(bmp);
        result
    });
    if let Err(e) = copied {
        MessageBoxW(h, wide(&e).as_ptr(), wide("Zwischenablage").as_ptr(), 0x10);
    }
}
#[repr(C)]
struct ChooseColor {
    size: u32,
    owner: H,
    instance: H,
    color: u32,
    custom: *mut u32,
    flags: u32,
    data: isize,
    hook: *mut c_void,
    template: *const u16,
}
#[link(name = "comdlg32")]
extern "system" {
    fn ChooseColorW(cc: *mut ChooseColor) -> i32;
}
unsafe fn choose_color(h: H) -> Option<shotkit_rust_prototype::shapes::Color> {
    let current = editor(h).current_style().color;
    let mut custom = [0u32; 16];
    let mut cc = ChooseColor {
        size: size_of::<ChooseColor>() as u32,
        owner: h,
        instance: null_mut(),
        color: current.r as u32 | ((current.g as u32) << 8) | ((current.b as u32) << 16),
        custom: custom.as_mut_ptr(),
        flags: 0x1 | 0x2,
        data: 0,
        hook: null_mut(),
        template: std::ptr::null(),
    };
    if ChooseColorW(&mut cc) == 0 {
        return None;
    }
    Some(shotkit_rust_prototype::shapes::Color::rgba(
        cc.color as u8,
        (cc.color >> 8) as u8,
        (cc.color >> 16) as u8,
        255,
    ))
}
unsafe fn choose_number(h: H, values: &[u32]) -> Option<f32> {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }
    for (i, value) in values.iter().enumerate() {
        AppendMenuW(menu, 0, i + 1, wide(&value.to_string()).as_ptr());
    }
    let mut p = Point { x: 0, y: 0 };
    if GetCursorPos(&mut p) == 0 {
        DestroyMenu(menu);
        return None;
    }
    let selected = TrackPopupMenu(menu, 0x0100 | 0x0002, p.x, p.y, 0, h, std::ptr::null());
    DestroyMenu(menu);
    values
        .get(selected.wrapping_sub(1) as usize)
        .map(|v| *v as f32)
}

unsafe fn choose_frame_action(h: H) -> Option<u32> {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }
    for (id, label) in [
        (1, "Rahmen hinzufügen"),
        (2, "Schlagschatten hinzufügen"),
        (3, "Rahmen und Schlagschatten hinzufügen"),
    ] {
        AppendMenuW(menu, 0, id, wide(label).as_ptr());
    }
    let mut p = Point { x: 0, y: 0 };
    if GetCursorPos(&mut p) == 0 {
        DestroyMenu(menu);
        return None;
    }
    SetForegroundWindow(h);
    let selected = TrackPopupMenu(menu, 0x0100 | 0x0002, p.x, p.y, 0, h, std::ptr::null());
    DestroyMenu(menu);
    (1..=3).contains(&selected).then_some(selected)
}

unsafe fn apply_frame(h: H, action: u32) {
    let width = if action == 2 {
        0
    } else {
        let Some(width) = choose_number(h, &[4, 8, 12, 16, 24, 32]) else {
            return;
        };
        width as usize
    };
    let color = if width == 0 {
        shotkit_rust_prototype::shapes::Color::rgba(0, 0, 0, 0)
    } else {
        let Some(color) = choose_color(h) else {
            return;
        };
        color
    };
    let options = FrameOptions {
        width,
        color,
        shadow: (action != 1).then_some(ShadowOptions {
            offset_x: 8,
            offset_y: 8,
            blur_radius: 12,
            opacity: 112,
        }),
    };
    cancel_gesture(h);
    let result = (|| -> Result<(), String> {
        editor(h)
            .document_mut()
            .begin_edit()
            .map_err(|e| format!("{e:?}"))?;
        if let Err(error) = editor(h).add_frame(options) {
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{error:?}"));
        }
        let output = editor(h).image_arc();
        let bitmap = match create_bitmap(&output) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = editor(h).document_mut().abort_edit();
                return Err(error);
            }
        };
        if let Err(error) = editor(h).document_mut().commit_edit() {
            DeleteObject(bitmap);
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{error:?}"));
        }
        install_bitmap(h, bitmap, &output);
        window_state(h).dirty = true;
        window_state(h).zoom = 1.0;
        window_state(h).pan_x = 0;
        window_state(h).pan_y = 0;
        Ok(())
    })();
    if let Err(error) = result {
        MessageBoxW(
            h,
            wide(&format!("Rahmen/Schlagschatten fehlgeschlagen: {error}")).as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x10,
        );
    }
}

unsafe fn edit_style(h: H, id: usize) {
    let selected = editor(h).selection().map(|s| s.style.clone());
    let mut replacement = selected.unwrap_or_else(|| editor(h).current_style().clone());
    match id {
        toolbar::ID_COLOR => {
            let Some(color) = choose_color(h) else { return };
            editor(h).set_current_color(color);
            replacement.color = color;
        }
        toolbar::ID_LINE_WIDTH => {
            let Some(width) = choose_number(h, &[1, 2, 3, 4, 6, 8, 12, 16]) else {
                return;
            };
            editor(h).set_current_width(width);
            replacement.width = width;
        }
        toolbar::ID_FONT_SIZE => {
            let Some(size) = choose_number(h, &[14, 18, 24, 28, 36, 48, 64, 96]) else {
                return;
            };
            editor(h).set_current_font_size(size);
            replacement.font_size = size;
        }
        _ => return,
    }
    if editor(h).set_selected_style(replacement) {
        window_state(h).dirty = true;
    }
}

unsafe fn toolbar_action(h: H, id: usize) {
    let tool = match id {
        toolbar::ID_SELECT => Some(Tool::Select),
        toolbar::ID_ARROW => Some(Tool::Arrow),
        toolbar::ID_LINE => Some(Tool::Line),
        toolbar::ID_RECTANGLE => Some(Tool::Rectangle),
        toolbar::ID_ELLIPSE => Some(Tool::Ellipse),
        toolbar::ID_TEXT => Some(Tool::Text),
        toolbar::ID_MARKER => Some(Tool::Highlight),
        toolbar::ID_BLUR => Some(Tool::Blur),
        toolbar::ID_STEP => Some(Tool::Step),
        toolbar::ID_PEN => Some(Tool::Pen),
        toolbar::ID_CROP => Some(Tool::Crop),
        _ => None,
    };
    if let Some(tool) = tool {
        cancel_gesture(h);
        editor(h).set_tool(tool);
        SetWindowTextW(
            h,
            wide(&format!(
                "ShotKit Rust – Werkzeug {tool:?} · Strg+S speichern · F10 Scroll"
            ))
            .as_ptr(),
        );
    } else {
        match id {
            toolbar::ID_COLOR | toolbar::ID_LINE_WIDTH | toolbar::ID_FONT_SIZE => edit_style(h, id),
            toolbar::ID_FRAME => {
                if window_state(h).bitmap.is_null() {
                    MessageBoxW(h, wide("Öffne zuerst ein Bild.").as_ptr(), wide("ShotKit Rust").as_ptr(), 0x30);
                } else if let Some(action) = choose_frame_action(h) {
                    apply_frame(h, action);
                }
            }
            toolbar::ID_SAVE => {
                save(h, false);
            }
            toolbar::ID_COPY => copy_current(h),
            toolbar::ID_UNDO => history(h, false),
            toolbar::ID_REDO => history(h, true),
            _ => {}
        }
    }
    report_editor_error(h);
    SetFocus(h);
    InvalidateRect(h, std::ptr::null(), 0);
}

unsafe fn tray_menu(h: H, active_target: Result<ActiveWindowSnapshot, String>) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (1, "Bildschirm aufnehmen (F9)"),
        (9, "Bereich nach 3 Sekunden auswählen"),
        (10, "Bereich nach 5 Sekunden auswählen"),
        (11, "Bereich nach 10 Sekunden auswählen"),
        (2, "Bereich aufnehmen (F11)"),
        (3, "Scroll-Aufnahme (F10)"),
        (7, "Alle Bildschirme aufnehmen"),
        (13, "Aktives Fenster aufnehmen"),
        (4, "Editor anzeigen"),
        (5, "Bild öffnen …"),
        (12, "Bild aus Zwischenablage öffnen"),
        (14, "Speicherordner öffnen"),
        (8, "Ungesicherte Aufnahme speichern …"),
        (6, "Beenden"),
    ] {
        AppendMenuW(menu, 0, id, wide(label).as_ptr());
    }
    let prefs = active_settings();
    let submenu = CreatePopupMenu();
    if !submenu.is_null() {
        for (id, label, checked) in [
            (101, "Nach Aufnahme Editor öffnen", prefs.open_editor),
            (102, "In Zwischenablage kopieren", prefs.copy_to_clipboard),
            (103, "Automatisch als PNG speichern", prefs.auto_save),
        ] {
            AppendMenuW(
                submenu,
                if checked { 0x0008 } else { 0 },
                id,
                wide(label).as_ptr(),
            );
        }
        AppendMenuW(submenu, 0, 104, wide("Speicherordner wählen …").as_ptr());
        match autostart::get() {
            Ok(enabled) => {
                AppendMenuW(
                    submenu,
                    if enabled { 0x0008 } else { 0 },
                    105,
                    wide("Mit Windows starten").as_ptr(),
                );
            }
            Err(_) => {
                AppendMenuW(
                    submenu,
                    0x0003,
                    105,
                    wide("Autostart nicht verfügbar").as_ptr(),
                );
            }
        }

        AppendMenuW(
            menu,
            0x0010,
            submenu as usize,
            wide("Einstellungen").as_ptr(),
        );
    }
    let mut pos = Point { x: 0, y: 0 };
    if GetCursorPos(&mut pos) == 0 {
        DestroyMenu(menu);
        return;
    }
    SetForegroundWindow(h);
    let choice = TrackPopupMenu(menu, 0x0100 | 0x0002, pos.x, pos.y, 0, h, std::ptr::null());
    DestroyMenu(menu);
    match choice {
        1 => capture(h),
        9 => schedule_capture(h, 3),
        10 => schedule_capture(h, 5),
        11 => schedule_capture(h, 10),
        2 => capture_region(h),
        3 => start_scroll(h),
        7 => capture_rect(h, capture_targets::virtual_desktop()),
        13 => {
            let target = active_target.and_then(|saved| resolve_active_window_target(h, &saved));
            capture_rect(h, target);
        }
        4 => show_editors(),
        5 => open_image(h),
        12 => paste_image(h),
        14 => open_storage_folder(h),
        8 => {
            resolve_recovery(h);
        }
        101 | 102 | 103 => {
            let mut prefs = active_settings();
            match choice {
                101 => prefs.open_editor = !prefs.open_editor,
                102 => prefs.copy_to_clipboard = !prefs.copy_to_clipboard,
                103 => {
                    if !prefs.auto_save && prefs.save_folder.is_none() {
                        match folder_dialog::choose_folder(h) {
                            Ok(Some(path)) => prefs.save_folder = Some(path),
                            Ok(None) => return,
                            Err(e) => {
                                MessageBoxW(
                                    h,
                                    wide(&e).as_ptr(),
                                    wide("Speicherordner").as_ptr(),
                                    0x10,
                                );
                                return;
                            }
                        }
                    }
                    prefs.auto_save = !prefs.auto_save;
                }
                _ => {}
            }
            save_settings(h, prefs);
        }
        105 => {
            let result = autostart::get().and_then(|value| autostart::set(!value));
            if let Err(e) = result {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Autostart").as_ptr(), 0x10);
            }
        }
        104 => match folder_dialog::choose_folder(h) {
            Ok(Some(path)) => {
                let mut prefs = active_settings();
                prefs.save_folder = Some(path);
                save_settings(h, prefs);
            }
            Ok(None) => {}
            Err(e) => {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherordner").as_ptr(), 0x10);
            }
        },
        6 => quit_application(h),
        _ => {}
    }
}

unsafe extern "system" fn proc(h: H, m: u32, w: W, l: L) -> L {
    if m == 0x0081 { // WM_NCCREATE
        let state = Box::into_raw(Box::new(WindowState::new()));
        SetWindowLongPtrW(h, -21, state as isize);
    }
    if GetWindowLongPtrW(h, -21) == 0 { return DefWindowProcW(h, m, w, l); }
    if m == 0x0082 { // WM_NCDESTROY, after child controls and WM_DESTROY
        let state = GetWindowLongPtrW(h, -21) as *mut WindowState;
        SetWindowLongPtrW(h, -21, 0);
        drop(Box::from_raw(state));
        return DefWindowProcW(h, m, w, l);
    }
    let input = m == toolbar::WM_COMMAND || (0x0100..=0x0109).contains(&m)
        || (0x0200..=0x020e).contains(&m);
    let starts_operation = input || m == TRAY || m == 0x0312 || m == DELAYED_CAPTURE;
    if m == DELAYED_CAPTURE {
        let Some(ticket) = CaptureTicket::from_wire(w) else { return 0; };
        if !DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).is_waiting(ticket) {
            return 0;
        }
        if window_state(h).modal_depth > 0 || SCROLL_BUSY.load(Ordering::SeqCst)
            || QUITTING.load(Ordering::SeqCst) {
            // Retry only this still-waiting generation. Stale or duplicate
            // wakeups cannot start a new selection or a newer request early.
            post_capture_after(h, ticket, std::time::Duration::from_millis(250));
            return 0;
        }
    }
    if m == WM_COPYDATA && (window_state(h).modal_depth > 0 || QUITTING.load(Ordering::SeqCst)) { return 3; }
    if (window_state(h).modal_depth > 0 && (m == 0x0010 || starts_operation))
        || (QUITTING.load(Ordering::SeqCst) && starts_operation) { return 0; }
    let modal_event = input || m == WM_COPYDATA || m == TRAY || m == 0x0312
        || m == SCROLL_DONE || m == DELAYED_CAPTURE;
    // Keep windows alive while handlers run nested Win32 modal loops. Paint and
    // resize remain available; another document can still be used normally.
    let modal = modal_event.then(|| ModalGuard::new(h));
    let result = dispatch_window(h, m, w, l);
    drop(modal);
    if input && h != PRIMARY && GetWindowLongPtrW(h, -21) != 0 { refresh_title(h); }
    result
}
unsafe fn dispatch_window(h: H, m: u32, w: W, l: L) -> L {
    match m {
        WM_FINISH_QUIT if h == PRIMARY => { DestroyWindow(h); 0 }
        WM_COPYDATA => handle_instance_file(h, l),
        WM_INSTANCE_ACTIVATE => { show_editors(); 1 }
        0x0312 if w == 6 || w == 10 => {
            capture(h);
            0
        }
        0x0312 if w == 7 => {
            capture_rect(h, capture_targets::virtual_desktop());
            0
        }
        0x0312 if w == 8 || w == 11 => {
            let window = GetForegroundWindow();
            capture_active(h, window);
            0
        }
        0x0312 if w == 9 || w == 12 => {
            start_scroll(h);
            0
        }
        0x0312 if w == 1 => {
            capture(h);
            0
        }
        TRAY if l as u32 == 0x0202 => {
            capture_region(h);
            0
        }
        0x0312 if w == 2 => {
            start_scroll(h);
            0
        }
        0x0312 if w == 3 || w == 4 || w == 5 => {
            capture_region(h);
            0
        }
        SCROLL_DONE => {
            let result = SCROLL_RESULT
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            let Some(result) = result else {
                return 0;
            };
            SCROLL_BUSY.store(false, Ordering::SeqCst);
            match result {
                Ok(outcome) => {
                    let message = outcome.reason;
                    let was_visible = RESTORE_EDITOR.swap(false, Ordering::SeqCst);
                    if let Err(e) = handle_capture(h, outcome.image, was_visible) {
                        if was_visible {
                            ShowWindow(h, 5);
                        }
                        MessageBoxW(h, wide(&e).as_ptr(), wide("ShotKit Rust").as_ptr(), 0x10);
                    } else {
                        MessageBoxW(
                            h,
                            wide(&message).as_ptr(),
                            wide("Scroll-Aufnahme beendet").as_ptr(),
                            0,
                        );
                    }
                }
                Err(e) => {
                    if RESTORE_EDITOR.swap(false, Ordering::SeqCst) {
                        ShowWindow(h, 5);
                    }
                    MessageBoxW(
                        h,
                        wide(&e).as_ptr(),
                        wide("Scroll-Aufnahme fehlgeschlagen").as_ptr(),
                        0x10,
                    );
                }
            }
            0
        }
        DELAYED_CAPTURE => {
            let Some(ticket) = CaptureTicket::from_wire(w) else { return 0; };
            if !DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).try_start(ticket) {
                return 0;
            }
            // Running remains pending through accepted, cancelled and failed
            // selections; duplicate messages are rejected during modal loops.
            capture_region(h);
            DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).finish(ticket);
            0
        }
        TRAY if l as u32 == 0x0205 => {
            // Save the target before the menu activates ShotKit. The menu
            // command later uses these validated bounds without querying the
            // menu or Shell as foreground again.
            let active_target = active_window_target(h, GetForegroundWindow());
            tray_menu(h, active_target);
            0
        }
        0x0100 if w == 0x56 && GetKeyState(0x11) < 0 => {
            paste_image(h);
            0
        }
        0x0100 if w == 0x4f && GetKeyState(0x11) < 0 => {
            open_image(h);
            0
        }
        0x0100 if w == 0x4e && GetKeyState(0x11) < 0 => {
            capture_region(h);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && GetKeyState(0x10) < 0 && w == 0x5a => {
            history(h, true);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x6b || w == 0xbb) => {
            window_state(h).zoom = (window_state(h).zoom * 1.25).clamp(0.05, 64.0);
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x6d || w == 0xbd) => {
            window_state(h).zoom = (window_state(h).zoom * 0.8).clamp(0.05, 64.0);
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x020a if !window_state(h).bitmap.is_null() => {
            let delta = ((w >> 16) as u16 as i16) as i32;
            if GetKeyState(0x11) < 0 {
                window_state(h).zoom = (window_state(h).zoom * if delta > 0 { 1.2 } else { 1.0 / 1.2 }).clamp(0.05, 64.0);
            } else if GetKeyState(0x10) < 0 {
                window_state(h).pan_x = window_state(h).pan_x.saturating_sub(delta);
            } else {
                window_state(h).pan_y = window_state(h).pan_y.saturating_sub(delta);
            }
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && w == 0x30 => {
            window_state(h).zoom = 1.0;
            window_state(h).pan_x = 0;
            window_state(h).pan_y = 0;
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && w == 0x39 && !window_state(h).bitmap.is_null() => {
            let v = view::layout(
                (window_state(h).width as u32, window_state(h).height as u32),
                (
                    0,
                    toolbar_height(h),
                    {
                        let mut r: Rect = zeroed();
                        GetClientRect(h, &mut r);
                        r.right
                    },
                    {
                        let mut r: Rect = zeroed();
                        GetClientRect(h, &mut r);
                        r.bottom - toolbar_height(h)
                    },
                ),
                1.0,
                (0, 0),
            );
            window_state(h).zoom = (window_state(h).width as f64 / v.width as f64).clamp(0.05, 64.0);
            window_state(h).pan_x = 0;
            window_state(h).pan_y = 0;
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        toolbar::WM_COMMAND => {
            toolbar_action(h, w & 0xffff);
            0
        }
        0x0005 => {
            // Rc keeps native controls owned during synchronous SetWindowPos
            // callbacks without retaining a borrow into the parent's state.
            let bar = window_state(h).toolbar.clone();
            if let Some(bar) = bar {
                let mut rect: Rect = zeroed();
                GetClientRect(h, &mut rect);
                bar.resize(rect.right - rect.left);
                InvalidateRect(h, std::ptr::null(), 1);
            }
            0
        }
        0x0100 if w == 0x43 && GetKeyState(0x11) < 0 => {
            copy_current(h);
            0
        }
        0x0100 if w == 0x53 && GetKeyState(0x11) < 0 => {
            save(h, GetKeyState(0x10) < 0);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x5a || w == 0x59) => {
            history(h, w == 0x59);
            0
        }
        0x0100 if w == 0x2e || w == 0x08 => {
            if editor(h).delete_selected() {
                window_state(h).dirty = true;
                InvalidateRect(h, std::ptr::null(), 0);
            }
            report_editor_error(h);
            0
        }
        0x0100 if w == 0x1b => {
            if editor(h).selected_index().is_some() {
                cancel_gesture(h);
                editor(h).clear_selection();
            } else if window_state(h).drawing {
                cancel_gesture(h);
            } else if editor(h).tool() != Tool::Select {
                cancel_gesture(h);
                editor(h).set_tool(Tool::Select);
            } else {
                return 0;
            }
            report_editor_error(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0100 if matches!(w, 0x25..=0x28)
            && GetKeyState(0x11) >= 0
            && GetKeyState(0x12) >= 0
            && !window_state(h).drawing
            && editor(h).selected_index().is_some() =>
        {
            let step = if GetKeyState(0x10) < 0 { 10.0 } else { 1.0 };
            let (dx, dy) = match w {
                0x25 => (-step, 0.0),
                0x26 => (0.0, -step),
                0x27 => (step, 0.0),
                _ => (0.0, step),
            };
            if editor(h).nudge_selected(dx, dy) {
                window_state(h).dirty = true;
                InvalidateRect(h, std::ptr::null(), 0);
            }
            report_editor_error(h);
            0
        }
        0x0100 if GetKeyState(0x11) >= 0 => {
            let tool = match w {
                0x56 => Some(Tool::Select),
                0x41 => Some(Tool::Arrow),
                0x4c => Some(Tool::Line),
                0x52 => Some(Tool::Rectangle),
                0x45 => Some(Tool::Ellipse),
                0x54 => Some(Tool::Text),
                0x4d => Some(Tool::Highlight),
                0x42 => Some(Tool::Blur),
                0x4e => Some(Tool::Step),
                0x50 => Some(Tool::Pen),
                0x43 => Some(Tool::Crop),
                _ => None,
            };
            if let Some(tool) = tool {
                cancel_gesture(h);
                editor(h).set_tool(tool);
                SetWindowTextW(
                    h,
                    wide(&format!(
                        "ShotKit Rust – Werkzeug {tool:?} · Strg+S speichern · F10 Scroll"
                    ))
                    .as_ptr(),
                );
                InvalidateRect(h, std::ptr::null(), 0);
            }
            0
        }
        0x0203 if !window_state(h).bitmap.is_null() => {
            let (x, y) = point(h, l);
            editor(h).set_tool(Tool::Select);
            editor(h).mouse_down(ShapePoint::new(x as f32, y as f32));
            editor(h).mouse_up(ShapePoint::new(x as f32, y as f32));
            let current = editor(h).selected_text().map(str::to_owned);
            if let Some(text) = current {
                if let Some(changed) = text_dialog::prompt(h, &text) {
                    if editor(h).set_selected_text(changed) {
                        window_state(h).dirty = true;
                    }
                }
            }
            report_editor_error(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0201 if !window_state(h).bitmap.is_null() => {
            let (vx, vy, vw, vh) = image_view(h);
            let sx = (l as u32 as u16 as i16) as i32;
            let sy = ((l as u32 >> 16) as u16 as i16) as i32;
            if sx < vx || sy < vy || sx >= vx + vw || sy >= vy + vh {
                return 0;
            }
            if editor(h).tool() == Tool::Text {
                let (x, y) = point(h, l);
                if let Some(text) = text_dialog::prompt(h, "") {
                    if editor(h).place_text(ShapePoint::new(x as f32, y as f32), text) {
                        window_state(h).dirty = true;
                        InvalidateRect(h, std::ptr::null(), 0);
                    }
                }
                report_editor_error(h);
                return 0;
            }
            window_state(h).drawing = true;
            let (x, y) = point(h, l);
            if editor(h).tool() == Tool::Crop {
                window_state(h).crop_start = Some((x, y));
                window_state(h).crop_end = Some((x, y));
                SetCapture(h);
                InvalidateRect(h, std::ptr::null(), 0);
                return 0;
            }
            editor(h).mouse_down(ShapePoint::new(x as f32, y as f32));
            SetCapture(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0200 if window_state(h).drawing && !window_state(h).bitmap.is_null() => {
            let (x, y) = point(h, l);
            if matches!(window_state(h).crop_start, Some(_)) {
                window_state(h).crop_end = Some((x, y));
                InvalidateRect(h, std::ptr::null(), 0);
                return 0;
            }
            editor(h).mouse_move(ShapePoint::new(x as f32, y as f32));
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0202 if window_state(h).drawing => {
            let (x, y) = point(h, l);
            if let Some(start) = std::ptr::replace(&mut window_state(h).crop_start, None) {
                window_state(h).crop_end = None;
                window_state(h).drawing = false;
                ReleaseCapture();
                apply_crop(h, start, (x, y));
                InvalidateRect(h, std::ptr::null(), 1);
                return 0;
            }
            let before = editor(h).shapes().to_vec();
            editor(h).mouse_up(ShapePoint::new(x as f32, y as f32));
            if before != editor(h).shapes() {
                window_state(h).dirty = true;
            }
            window_state(h).drawing = false;
            ReleaseCapture();
            report_editor_error(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0215 if window_state(h).drawing => {
            window_state(h).drawing = false;
            window_state(h).crop_start = None;
            window_state(h).crop_end = None;
            editor(h).cancel_drag();
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x000f => {
            let mut ps: Paint = zeroed();
            let dc = BeginPaint(h, &mut ps);
            if !window_state(h).bitmap.is_null() && !dc.is_null() {
                let mut ok = false;
                if let Ok(flattened) = composited_bitmap(h, true) {
                    let mem = CreateCompatibleDC(dc);
                    if !mem.is_null() {
                        let old = SelectObject(mem, flattened);
                        if !old.is_null() && old as isize != -1 {
                            let (x, y, w, height) = image_view(h);
                            ok = StretchBlt(dc, x, y, w, height, mem, 0, 0, window_state(h).width, window_state(h).height, COPY) != 0;
                            SelectObject(mem, old);
                        }
                        DeleteDC(mem);
                    }
                    DeleteObject(flattened);
                }
                if !ok {
                    let message = wide("Bilddarstellung fehlgeschlagen. Bitte erneut versuchen.");
                    TextOutW(
                        dc,
                        12,
                        toolbar_height(h) + 15,
                        message.as_ptr(),
                        (message.len() - 1) as i32,
                    );
                }
            }
            if window_state(h).bitmap.is_null() && !dc.is_null() {
                let message = wide("Bild öffnen: Strg+O · Bild einfügen: Strg+V · Aufnahme: F9 oder F11");
                TextOutW(dc, 12, toolbar_height(h) + 15, message.as_ptr(), (message.len() - 1) as i32);
            }
            EndPaint(h, &ps);
            0
        }
        0x0010 => {
            if h == PRIMARY { quit_application(h); }
            else if !QUITTING.load(Ordering::SeqCst) && confirm_replace(h) { DestroyWindow(h); }
            0
        }
        0x0002 => {
            let toolbar = window_state(h).toolbar.take();
            drop(toolbar); // DestroyWindow(child) may synchronously reenter this parent.
            let bitmap = std::mem::replace(&mut window_state(h).bitmap, null_mut());
            if !bitmap.is_null() { DeleteObject(bitmap); }
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).retain(|window| *window != h);
            if h == PRIMARY {
                for id in 1..=12 { UnregisterHotKey(h, id); }
                let mut n: Notify = zeroed();
                n.size = size_of::<Notify>() as u32;
                n.hwnd = h;
                n.id = 1;
                Shell_NotifyIconW(2, &n);
                PostQuitMessage(0);
            }
            0
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}
fn main() {
    unsafe {
        let (_instance_handle, already_running) = match create_instance_mutex() {
            Ok(result) => result,
            Err(error) => {
                MessageBoxW(null_mut(), wide(&error).as_ptr(), wide("ShotKit Rust").as_ptr(), 0x10);
                return;
            }
        };
        if already_running {
            if let Err(error) = forward_second_instance() {
                MessageBoxW(null_mut(), wide(&error).as_ptr(), wide("ShotKit Rust – Datei nicht geöffnet").as_ptr(), 0x30);
            }
            return;
        }
        SetProcessDpiAwarenessContext(-4isize as H);
        let inst = GetModuleHandleW(std::ptr::null());
        let class = wide("ShotKitRustPrototype");
        let wc = WndClass {
            style: 0x0008,
            proc: Some(proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: inst,
            icon: LoadIconW(null_mut(), 32512usize as *const u16),
            cursor: LoadCursorW(null_mut(), 32512usize as *const u16),
            background: (6usize as H),
            menu: std::ptr::null(),
            class: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let editor_class = wide("ShotKitRustEditor");
        let editor_wc = WndClass { class: editor_class.as_ptr(), ..wc };
        if RegisterClassW(&editor_wc) == 0 { return; }
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x00cf0000,
            0x80000000u32 as i32,
            0x80000000u32 as i32,
            1000,
            700,
            null_mut(),
            null_mut(),
            inst,
            null_mut(),
        );
        if h.is_null() {
            return;
        }
        PRIMARY = h;
        load_settings(h);
        let mut n: Notify = zeroed();
        n.size = size_of::<Notify>() as u32;
        n.hwnd = h;
        n.id = 1;
        n.flags = 1 | 2 | 4;
        n.callback = TRAY;
        n.icon = LoadIconW(null_mut(), 32512usize as *const u16);
        let tip = wide("ShotKit Rust: F9 = Bildschirm, Klick = Bereich, Rechtsklick = Menü");
        n.tip[..tip.len()].copy_from_slice(&tip);
        if Shell_NotifyIconW(0, &n) == 0 {
            MessageBoxW(
                h,
                wide("Tray-Symbol konnte nicht erstellt werden.").as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            DestroyWindow(h);
            return;
        }
        if RegisterHotKey(h, 1, 0x4000, 0x78) == 0 {
            MessageBoxW(
                h,
                wide("F9 ist belegt. Bereichsaufnahme über Einzelklick auf das Tray-Symbol.").as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x30,
            );
        }
        if RegisterHotKey(h, 2, 0x4000, 0x79) == 0 {
            MessageBoxW(h,wide("F10 ist belegt. Die experimentelle Scroll-Aufnahme kann nicht gestartet werden.").as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        }
        if RegisterHotKey(h, 3, 0x4000, 0x7a) == 0 {
            MessageBoxW(
                h,
                wide("F11 ist belegt; Bereichsaufnahme ist derzeit nicht verfügbar.").as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x30,
            );
        }
        let print_ok = RegisterHotKey(h, 4, 0x4000, 0x2c) != 0;
        let alt_ok = RegisterHotKey(h, 5, 0x4000 | 0x0002 | 0x0004, 0x31) != 0;
        if !print_ok && !alt_ok {
            MessageBoxW(h,wide("Druck und Strg+Umschalt+1 sind belegt. Für Bereichsaufnahmen F11 oder das Tray-Menü verwenden.").as_ptr(),wide("Tastenkürzel" ).as_ptr(),0x30);
        }
        let mut unavailable = Vec::new();
        for (id, mods, key, label) in [
            (6, 2, 0x2c, "Strg+Druck"),
            (7, 6, 0x2c, "Strg+Umschalt+Druck"),
            (8, 4, 0x2c, "Umschalt+Druck"),
            (9, 3, 0x2c, "Strg+Alt+Druck"),
            (10, 6, 0x32, "Strg+Umschalt+2"),
            (11, 6, 0x33, "Strg+Umschalt+3"),
            (12, 6, 0x34, "Strg+Umschalt+4"),
        ] {
            if RegisterHotKey(h, id, mods | 0x4000, key) == 0 {
                unavailable.push(label);
            }
        }
        if !unavailable.is_empty() {
            MessageBoxW(
                h,
                wide(&format!(
                    "Nicht verfügbare Tastenkürzel: {}",
                    unavailable.join(", ")
                ))
                .as_ptr(),
                wide("Tastenkürzel").as_ptr(),
                0x30,
            );
        }
        for path in std::env::args_os().skip(1).map(PathBuf::from) {
            if let Err(error) = open_image_path(h, absolute_argument_path(path)) {
                MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
            }
        }
        MessageBoxW(h,wide("Prototyp gestartet. F9: Bildschirm aufnehmen; Druck oder F11: Bereich wählen; F10: Scroll-Aufnahme. Im Editor: Strg+S speichern. Rechtsklick auf das Tray-Symbol öffnet das Menü.").as_ptr(),wide("ShotKit Rust – Prototyp").as_ptr(),0);
        let mut msg: Msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
