mod callout;
mod canvas;
mod mouse_history;
mod preview_scaling;
mod region;
mod scroll_completion;
mod temp_recovery;
mod window_commands;
mod window_mouse;
use region::*;
mod config;
use config::*;
mod instance;
use instance::*;
mod capture_budget;
use capture_budget::*;
mod capture;
use capture::*;
mod rendering;
use rendering::*;
mod document;
use document::*;
mod import_budget;
use import_budget::*;
mod actions;
use actions::*;
mod hotkeys;
mod tray;
mod window;
use crate::{
    about_dialog, autostart, capture_targets, clipboard, file_dialog, folder_dialog, last_region,
    overlay, scroll_windows, text_dialog, toolbar,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::null_mut,
    rc::Rc,
};

use shotkit::{
    capture_output::{Advance, CaptureAction, CaptureOutputState},
    delayed_capture::{BeginError, DelayedCaptureState, Ticket as CaptureTicket},
    editor::EditorController,
    image_edit::{FrameOptions, ShadowOptions},
    image_io, output_windows,
    recovery_admission::{self, AdmissionError as SourceAdmissionError, Permit as SourcePermit},
    recovery_images::RecoveryImages,
    scroll_capture::{self, Direction, Outcome},
    scroll_completion::{ScrollCompletion, Ticket as ScrollTicket, WindowIdentity},
    settings::{self, Settings},
    shapes::{Point as ShapePoint, Tool},
    stitch::{Image, Rotation},
    view,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    fn FillRect(dc: H, rect: *const Rect, brush: H) -> i32;
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
    fn EnumDisplayMonitors(
        dc: H,
        clip: *const Rect,
        callback: Option<unsafe extern "system" fn(H, H, *mut Rect, L) -> i32>,
        data: L,
    ) -> i32;
    fn GetMonitorInfoW(monitor: H, info: *mut MonitorInfo) -> i32;
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
    fn SetTimer(
        h: H,
        id: usize,
        milliseconds: u32,
        callback: Option<unsafe extern "system" fn(H, u32, usize, u32)>,
    ) -> usize;
    fn KillTimer(h: H, id: usize) -> i32;
    fn FindWindowW(class: *const u16, title: *const u16) -> H;
    fn SendMessageTimeoutW(
        h: H,
        msg: u32,
        w: W,
        l: L,
        flags: u32,
        timeout: u32,
        result: *mut W,
    ) -> i32;
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
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
    fn SetStretchBltMode(dc: H, mode: i32) -> i32;
    fn SetBrushOrgEx(dc: H, x: i32, y: i32, old: *mut Point) -> i32;
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
    fn PatBlt(dc: H, x: i32, y: i32, width: i32, height: i32, rop: u32) -> i32;
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
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn CreateMutexW(attributes: *mut c_void, initial_owner: i32, name: *const u16) -> H;
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
    fn CloseHandle(handle: H) -> i32;
    fn Sleep(milliseconds: u32);
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

const COPY: u32 = 0x00cc0020;
const TRAY: u32 = 0x8001;
const SCROLL_DONE: u32 = 0x8002;
const DELAYED_CAPTURE: u32 = 0x8003;
static DELAYED_CAPTURE_STATE: Mutex<DelayedCaptureState> = Mutex::new(DelayedCaptureState::new());
fn delayed_capture_pending() -> bool {
    DELAYED_CAPTURE_STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_pending()
}
static SCROLL_BUSY: AtomicBool = AtomicBool::new(false);
static SCROLL_COMPLETION: Mutex<ScrollCompletion<Result<Outcome, String>>> =
    Mutex::new(ScrollCompletion::new());
const SCROLL_POLL_TIMER: usize = 0x5343_524c;
static NEXT_WINDOW_GENERATION: AtomicUsize = AtomicUsize::new(1);
static RECOVERY: Mutex<RecoveryImages<Image>> = Mutex::new(RecoveryImages::new());
// A failed horizontal return rotation must not grow RECOVERY's overflow while the
// image allocator is already under pressure. This inline slot needs no queue
// allocation. start_scroll refuses another job until it has been resolved.
static SCROLL_RECOVERY: Mutex<Option<PendingScrollRecovery>> = Mutex::new(None);
static RECOVERY_RESOLVING: AtomicBool = AtomicBool::new(false);
struct PendingScrollRecovery {
    image: Image,
    pending_rotation: Option<Rotation>,
}
fn scroll_recovery_pending() -> bool {
    SCROLL_RECOVERY
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_some()
}

fn source_recovery_pending() -> bool {
    RECOVERY_RESOLVING.load(Ordering::SeqCst)
        || !RECOVERY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
        || scroll_recovery_pending()
        || capture_recovery_pending()
        || import_transfer_active()
        || import_recovery_pending()
}

// SAFETY CONTRACT:
// Voraussetzung: h is a live dialog owner on the UI thread.
// Warum erfüllt: callers are owned window handlers guarded against closure.
// Nachrichten: MessageBoxW can reenter; all recovery locks are released first.
unsafe fn acquire_recovery_safe_source(h: H) -> Option<SourcePermit<'static>> {
    match recovery_admission::acquire_source(&SCROLL_BUSY, source_recovery_pending) {
        Ok(permit) => Some(permit),
        Err(SourceAdmissionError::Busy) => None,
        Err(SourceAdmissionError::PendingRecovery) => {
            MessageBoxW(h, wide("Ein Bild wartet noch auf seine Sicherung. Bitte zuerst im Tray-Menü „Ungesicherte Aufnahme speichern …“ wählen.").as_ptr(), wide("Ungesichertes Bild").as_ptr(), 0x30);
            None
        }
    }
}
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
static SETTINGS_WRITABLE: AtomicBool = AtomicBool::new(false);
static SETTINGS_MISSING: AtomicBool = AtomicBool::new(false);
static LAST_REGION: Mutex<Option<last_region::Rect>> = Mutex::new(None);
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
    generation: usize,
    initialization_close_blocked: Option<Rc<Cell<bool>>>,
    initialization_lifetime: Option<Rc<Cell<H>>>,
    toolbar: Option<Rc<toolbar::Toolbar>>,
    editor: Option<EditorController>,
    preview_cache: Rc<RefCell<Option<shotkit::alpha_document::PreviewCache>>>,
    bitmap: H,
    current_path: Option<PathBuf>,
    protected_original: bool,
    zoom: f64,
    pan_x: i32,
    pan_y: i32,
    width: i32,
    height: i32,
    drawing: bool,
    pen_history: Option<shotkit::mouse_history::MouseSample>,
    crop_start: Option<(i32, i32)>,
    crop_end: Option<(i32, i32)>,
    modal_depth: usize,
}
impl WindowState {
    fn new() -> Self {
        Self {
            // Exhaustion disables asynchronous work for new windows, never reuses an identity.
            generation: NEXT_WINDOW_GENERATION
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .unwrap_or(0),
            initialization_close_blocked: None,
            initialization_lifetime: None,
            toolbar: None,
            editor: None,
            preview_cache: Rc::new(RefCell::new(None)),
            bitmap: null_mut(),
            current_path: None,
            protected_original: false,
            zoom: 1.0,
            pan_x: 0,
            pan_y: 0,
            width: 0,
            height: 0,
            drawing: false,
            pen_history: None,
            crop_start: None,
            crop_end: None,
            modal_depth: 0,
        }
    }
}

// Passed only by the protected editor constructor. WM_NCCREATE copies the
// token before any subsequent creation messages; the marker pointer is never
// retained by the HWND. Ordinary editor/primary creation passes null.
struct EditorCreationProtection {
    close_blocked: Rc<Cell<bool>>,
    lifetime: Rc<Cell<H>>,
}

enum ProtectedEditorCleanup {
    Idle,
    Creating(Rc<EditorCreationProtection>),
    Pending(Rc<EditorCreationProtection>),
    Retrying,
}
thread_local! {
    // Fixed ownership slot, independent of editor-list allocation and HWND reuse.
    static PROTECTED_EDITOR_CLEANUP: RefCell<ProtectedEditorCleanup> =
        const { RefCell::new(ProtectedEditorCleanup::Idle) };
}

fn retry_protected_editor_cleanup_with(mut destroy: impl FnMut(H)) -> Result<(), String> {
    let pending = PROTECTED_EDITOR_CLEANUP.with(|slot| {
        let mut slot = slot.borrow_mut();
        match &*slot {
            ProtectedEditorCleanup::Idle => Ok(None),
            ProtectedEditorCleanup::Pending(token) => {
                let token = token.clone();
                *slot = ProtectedEditorCleanup::Retrying;
                Ok(Some(token))
            }
            ProtectedEditorCleanup::Creating(_) | ProtectedEditorCleanup::Retrying => {
                Err("Geschützte Editoranlage oder Bereinigung läuft bereits.".to_owned())
            }
        }
    })?;
    let Some(token) = pending else {
        return Ok(());
    };
    let h = token.lifetime.get();
    if !h.is_null() {
        // No slot/state borrow crosses this reentrant call. WM_NCDESTROY is
        // the authority: a return code or IsWindow cannot detect HWND reuse.
        destroy(h);
    }
    let complete = token.lifetime.get().is_null();
    PROTECTED_EDITOR_CLEANUP.with(|slot| {
        *slot.borrow_mut() = if complete {
            ProtectedEditorCleanup::Idle
        } else {
            ProtectedEditorCleanup::Pending(token)
        };
    });
    if complete {
        Ok(())
    } else {
        Err(
            "Ein unvollständiges Editorfenster konnte nicht geschlossen werden. Erneut versuchen."
                .into(),
        )
    }
}

unsafe fn retry_protected_editor_cleanup() -> Result<(), String> {
    retry_protected_editor_cleanup_with(|h| {
        DestroyWindow(h);
    })
}

unsafe fn begin_protected_editor_creation() -> Result<Rc<EditorCreationProtection>, String> {
    retry_protected_editor_cleanup()?;
    let token = Rc::new(EditorCreationProtection {
        close_blocked: Rc::new(Cell::new(true)),
        lifetime: Rc::new(Cell::new(null_mut())),
    });
    PROTECTED_EDITOR_CLEANUP.with(|slot| {
        *slot.borrow_mut() = ProtectedEditorCleanup::Creating(token.clone());
    });
    Ok(token)
}

fn finish_protected_editor_creation(token: &Rc<EditorCreationProtection>, success: bool) {
    if success {
        // Reserve the failure slot throughout the handoff. Guard completion
        // releases it; another reentrant creation cannot consume this slot.
        return;
    }
    PROTECTED_EDITOR_CLEANUP.with(|slot| {
        let mut slot = slot.borrow_mut();
        if matches!(&*slot, ProtectedEditorCleanup::Creating(current) if Rc::ptr_eq(current, token))
        {
            *slot = if token.lifetime.get().is_null() {
                ProtectedEditorCleanup::Idle
            } else {
                // Retain the live HWND and close protection after constructor
                // cleanup failed; no guard has yet been created or dropped.
                ProtectedEditorCleanup::Pending(token.clone())
            };
        }
    });
}

// CREATESTRUCTW starts with lpCreateParams on both Windows architectures.
#[repr(C)]
struct CreateStructParams {
    create_params: *mut c_void,
}

/// Keep this guard until the complete import/editor handoff is initialized.
/// Drop only touches owned tokens and the UI-thread slot, never an HWND or
/// WindowState: prior destruction is safe, including HWND reuse.
#[must_use = "keep the guard alive until editor initialization is complete"]
pub(super) struct EditorInitializationGuard(Option<Rc<EditorCreationProtection>>);
impl EditorInitializationGuard {
    fn still_owns(&self, window: H) -> bool {
        self.0
            .as_ref()
            .is_some_and(|token| token.lifetime.get() == window && !window.is_null())
    }
}
impl Drop for EditorInitializationGuard {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.close_blocked.set(false);
            PROTECTED_EDITOR_CLEANUP.with(|slot| {
                let mut slot = slot.borrow_mut();
                if matches!(&*slot, ProtectedEditorCleanup::Creating(current) if Rc::ptr_eq(current, &token)) {
                    *slot = ProtectedEditorCleanup::Idle;
                }
            });
        }
    }
}

// Consume failed initialization (including post-creation Vec::try_reserve)
// without releasing close protection. The guard already reserved this slot.
pub(super) unsafe fn abandon_protected_editor(
    guard: EditorInitializationGuard,
) -> Result<(), String> {
    abandon_protected_editor_with(guard, |h| {
        DestroyWindow(h);
    })
}

fn abandon_protected_editor_with(
    mut guard: EditorInitializationGuard,
    destroy: impl FnMut(H),
) -> Result<(), String> {
    if let Some(token) = guard.0.take() {
        finish_protected_editor_creation(&token, false);
    }
    retry_protected_editor_cleanup_with(destroy)
}
static mut PRIMARY: H = null_mut();
static mut EDITOR_WINDOWS: Vec<H> = Vec::new();
static QUITTING: AtomicBool = AtomicBool::new(false);
// Set only after every document has been approved and its window closed.
static FINISH_QUIT_READY: AtomicBool = AtomicBool::new(false);

fn take_quit_completion(ready: &AtomicBool, modal_depth: usize) -> bool {
    modal_depth == 0 && ready.swap(false, Ordering::SeqCst)
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster mit initialisiertem Zustand;
// Zugriff nur auf dem UI-Thread, ohne überlappenden Borrow oder Reentranz.
// Warum noch nicht belegt: Der Rückgabetyp &'static mut erzwingt diese Regeln nicht;
// alle Aufrufer und verschachtelten Nachrichten müssen daraufhin geprüft werden.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
unsafe fn window_state(h: H) -> &'static mut WindowState {
    &mut *(GetWindowLongPtrW(h, -21) as *mut WindowState)
}
// SAFETY CONTRACT:
// Voraussetzung: Dieselben Lebensdauer-, Thread- und Exklusivitätsregeln wie
// bei window_state gelten für den zurückgegebenen Editor.
// Warum noch nicht belegt: Reentranz und alle Aufrufer sind nicht vollständig geprüft.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
unsafe fn editor(h: H) -> &'static mut EditorController {
    window_state(h)
        .editor
        .get_or_insert_with(EditorController::new)
}
// SAFETY CONTRACT:
// Voraussetzung: h besitzt einen lebenden WindowState auf dem UI-Thread.
// Warum erfüllt: Aufruf aus der Bearbeitung eines eigenen Fensters; Borrow bleibt kurz.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
unsafe fn toolbar_height(h: H) -> i32 {
    window_state(h)
        .toolbar
        .as_ref()
        .map_or(0, |bar| bar.height())
}
// Do not retain a Rust borrow across a Win32 dialog/message pump.
struct ModalGuard(H);
impl ModalGuard {
    // SAFETY CONTRACT:
    // Voraussetzung: h bleibt bis zum Ende des Guards auf dem UI-Thread am Leben.
    // Warum erfüllt: der Nachrichtenhandler schützt den modalen Vorgang vor Schließen.
    // Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
    unsafe fn new(h: H) -> Self {
        window_state(h).modal_depth += 1;
        Self(h)
    }
}
impl Drop for ModalGuard {
    fn drop(&mut self) {
        unsafe {
            window_state(self.0).modal_depth -= 1;
            // A nested message loop may have consumed the posted wakeup while
            // this guard was active. Queue another after the outermost guard.
            if self.0 == PRIMARY
                && window_state(self.0).modal_depth == 0
                && FINISH_QUIT_READY.load(Ordering::SeqCst)
                && PostMessageW(PRIMARY, WM_FINISH_QUIT, 0, 0) == 0
            {
                FINISH_QUIT_READY.store(false, Ordering::SeqCst);
                QUITTING.store(false, Ordering::SeqCst);
            }
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: EDITOR_WINDOWS wird nur auf dem UI-Thread verändert.
// Warum erfüllt: Fenstereinrichtung und -zerstörung erfolgen im Nachrichtenhandler.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
unsafe fn editor_windows() -> Vec<H> {
    (&*std::ptr::addr_of!(EDITOR_WINDOWS)).clone()
}
// SAFETY CONTRACT:
// Voraussetzung: h und sein WindowState sind auf dem UI-Thread gültig.
// Warum erfüllt: Aufruf für eigene Editorfenster; der Titel besitzt seine Daten vor dem Win32-Aufruf.
// Nachrichten: SetWindowTextW kann synchron WM_SETTEXT auslösen; der aus dem State erzeugte Titel ist besitzend und der Borrow endet vorher.
unsafe fn refresh_title(h: H) {
    let state = window_state(h);
    let name = state
        .current_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| {
            if state.bitmap.is_null() {
                "Bild öffnen oder einfügen".into()
            } else {
                "Neue Aufnahme".into()
            }
        });
    let tool = state
        .editor
        .as_ref()
        .map(|editor| format!(" · Werkzeug {:?}", editor.tool()))
        .unwrap_or_default();
    let title = format!(
        "{}{} – ShotKit{} · Strg+S speichern",
        if state
            .editor
            .as_ref()
            .is_some_and(|editor| editor.document().is_dirty())
        {
            "* "
        } else {
            ""
        },
        name,
        tool
    );
    SetWindowTextW(h, wide(&title).as_ptr());
}
// SAFETY CONTRACT:
// Voraussetzung: Fensterliste und PRIMARY gehören zum aktiven UI-Thread.
// Warum erfüllt: der Aufruf erfolgt aus der Nachrichtenbehandlung der Anwendung.
// Nachrichten: Fensteranlage, ShowWindow und Dialoge können reentrieren; die Fensterliste ist eine Kopie und kein State-Borrow bleibt aktiv.
unsafe fn show_editors() {
    if QUITTING.load(Ordering::SeqCst) {
        return;
    }
    let mut windows = editor_windows();
    if windows.is_empty() {
        match create_editor_window() {
            Ok(h) => {
                (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(h);
                refresh_title(h);
                windows.push(h);
            }
            Err(error) => {
                MessageBoxW(
                    PRIMARY,
                    wide(&error).as_ptr(),
                    wide("Editor öffnen").as_ptr(),
                    0x10,
                );
                return;
            }
        }
    }
    for h in &windows {
        ShowWindow(*h, 9);
    } // SW_RESTORE also restores minimized editors
    if let Some(h) = windows.last() {
        SetForegroundWindow(*h);
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h und alle gelisteten Fenster gehören zum UI-Thread.
// Warum erfüllt: die Liste enthält ausschließlich von der Anwendung erzeugte Fenster.
// Nachrichten: Dialoge und DestroyWindow können reentrieren; Fensterliste und Modal-Tiefe werden vor den Aufrufen als Werte übernommen.
unsafe fn quit_application(h: H) {
    if retry_protected_editor_cleanup().is_err() {
        return;
    }
    if SCROLL_BUSY.load(Ordering::SeqCst) || delayed_capture_pending() {
        MessageBoxW(
            h,
            wide("Bitte die laufende oder geplante Aufnahme zuerst abschließen.").as_ptr(),
            wide("ShotKit").as_ptr(),
            0x30,
        );
        return;
    }
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let windows = editor_windows();
    // Confirm all documents before destroying any. A cancelled save or close
    // preserves every window, including previously confirmed discard choices.
    let approved = windows.iter().all(|w| window_state(*w).modal_depth == 0)
        && resolve_recovery(h)
        && windows.iter().all(|w| confirm_replace(*w));
    if approved {
        for window in windows {
            if DestroyWindow(window) == 0 {
                QUITTING.store(false, Ordering::SeqCst);
                return;
            }
        }
        FINISH_QUIT_READY.store(true, Ordering::SeqCst);
        if PostMessageW(PRIMARY, WM_FINISH_QUIT, 0, 0) == 0 {
            FINISH_QUIT_READY.store(false, Ordering::SeqCst);
            QUITTING.store(false, Ordering::SeqCst);
        }
    } else {
        QUITTING.store(false, Ordering::SeqCst);
    }
}
pub(crate) fn run() {
    // SAFETY: startup runs once on the entrypoint thread; run_application
    // owns the Win32 handles and message loop until their teardown.
    unsafe {
        run_application();
    }
}

// SAFETY CONTRACT:
// Voraussetzung: Aufruf genau einmal auf dem Windows-Eintrittsthread.
// Warum erfüllt: main ruft run einmal auf; Fenster und Nachrichtenpumpe werden
// hier erzeugt und auf demselben Thread verarbeitet. Callback-Panikfreiheit ist offen.
// Nachrichten: Fenstererstellung, Dialoge und die Nachrichtenpumpe können reentrieren.
unsafe fn run_application() {
    let (_instance_handle, already_running) = match create_instance_mutex() {
        Ok(result) => result,
        Err(error) => {
            MessageBoxW(
                null_mut(),
                wide(&error).as_ptr(),
                wide("ShotKit").as_ptr(),
                0x10,
            );
            return;
        }
    };
    if already_running {
        if let Err(error) = forward_second_instance() {
            MessageBoxW(
                null_mut(),
                wide(&error).as_ptr(),
                wide("ShotKit – Datei nicht geöffnet").as_ptr(),
                0x30,
            );
        }
        return;
    }
    SetProcessDpiAwarenessContext(-4isize as H);
    let inst = GetModuleHandleW(std::ptr::null());
    let class = wide("ShotKitRustPrototype");
    let wc = WndClass {
        style: 0x0008,
        proc: Some(window::proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance: inst,
        // Win32 integer resource ID 1, encoded as a pointer without dereferencing.
        icon: LoadIconW(inst, std::ptr::without_provenance::<u16>(1)),
        cursor: LoadCursorW(null_mut(), 32512usize as *const u16),
        background: (6usize as H),
        menu: std::ptr::null(),
        class: class.as_ptr(),
    };
    if RegisterClassW(&wc) == 0 {
        return;
    }
    let editor_class = wide("ShotKitRustEditor");
    let editor_wc = WndClass {
        class: editor_class.as_ptr(),
        ..wc
    };
    if RegisterClassW(&editor_wc) == 0 {
        return;
    }
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        wide("ShotKit").as_ptr(),
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
    if !tray::install(h, inst) {
        DestroyWindow(h);
        return;
    }
    hotkeys::register(h);
    temp_recovery::check(h);
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    for path in &arguments {
        let result = absolute_argument_path(path.clone()).and_then(|path| open_image_path(h, path));
        if let Err(error) = result {
            MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
        }
    }
    if shotkit::first_start::show_notice(
        SETTINGS_MISSING.load(Ordering::SeqCst),
        SETTINGS_WRITABLE.load(Ordering::SeqCst),
        !arguments.is_empty(),
        autostart::get().ok(),
    ) {
        MessageBoxW(
            h,
            wide("ShotKit läuft im Infobereich der Taskleiste. Druck oder F11: Bereich; F9: Bildschirm; F10: Scroll-Aufnahme. Rechtsklick auf das Symbol öffnet das Menü.").as_ptr(),
            wide("Willkommen bei ShotKit").as_ptr(),
            0,
        );
        if let Err(error) = persist_first_start() {
            MessageBoxW(
                h,
                wide(&format!(
                    "Der erste Start konnte nicht gespeichert werden: {error}"
                ))
                .as_ptr(),
                wide("ShotKit").as_ptr(),
                0x30,
            );
        }
    }
    let mut msg: Msg = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

#[cfg(test)]
mod quit_completion_tests {
    use super::*;

    #[test]
    fn completion_requires_approval_and_is_consumed_once() {
        let ready = AtomicBool::new(false);
        assert!(!take_quit_completion(&ready, 0));
        ready.store(true, Ordering::SeqCst);
        assert!(take_quit_completion(&ready, 0));
        assert!(!take_quit_completion(&ready, 0));
    }

    #[test]
    fn modal_handler_defers_without_consuming_completion() {
        let ready = AtomicBool::new(true);
        assert!(!take_quit_completion(&ready, 1));
        assert!(take_quit_completion(&ready, 0));
    }
}
