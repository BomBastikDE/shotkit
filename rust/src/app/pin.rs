//! Pinned image (#99): the current image with all annotations as a small,
//! always-on-top window above other applications.
//!
//! The window holds its own copy: later edits in the editor do not change it.
//! The exact export (with alpha) is kept for "Kopieren" and "Im Editor öffnen";
//! a checkerboard-flattened bitmap is used for display, like the editor preview.
use super::*;
use std::sync::atomic::AtomicBool;

const PIN_CLASS: &str = "ShotKitPinnedImage";
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
const ZOOM_STEP: f32 = 1.25;
/// Pinned windows larger than this share of the work area start scaled down.
const MAX_INITIAL_SHARE: f32 = 0.9;

const IDM_COPY: u32 = 1;
const IDM_OPEN: u32 = 2;
const IDM_ORIGINAL: u32 = 3;
const IDM_CLOSE: u32 = 4;

const WM_NCHITTEST: u32 = 0x0084;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_KEYDOWN: u32 = 0x0100;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_NCDESTROY: u32 = 0x0082;
const HTCLIENT: L = 1;
const HTCAPTION: L = 2;
const GWLP_USERDATA: i32 = -21;

static REGISTERED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct WorkAreaInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[link(name = "user32")]
extern "system" {
    fn SetWindowPos(h: H, after: H, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn GetWindowRect(h: H, rect: *mut Rect) -> i32;
    fn MonitorFromPoint(point: Point, flags: u32) -> H;
    #[link_name = "GetMonitorInfoW"]
    fn get_work_area(monitor: H, info: *mut WorkAreaInfo) -> i32;
}

struct PinState {
    /// Exact rendered image with annotations; owns its budget lease.
    export: shotkit::image_handle::ImageHandle,
    /// Checkerboard-flattened DIB section for display.
    display: H,
    width: i32,
    height: i32,
    zoom: f32,
}

/// Pin the editor's current image (with annotations) above all windows.
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes Editorfenster mit Bild auf dem UI-Thread.
// Warum erfüllt: Aufrufer prüfen das Bitmap; Editorzustand wird vor Win32-Aufrufen kopiert.
// Nachrichten: Fensteranlage und MessageBoxW können Nachrichten auslösen.
pub(super) unsafe fn pin_current(h: H) {
    if window_state(h).bitmap.is_null() {
        return;
    }
    if let Err(error) = pin(h) {
        MessageBoxW(
            h,
            wide(&format!("Anheften fehlgeschlagen: {error}")).as_ptr(),
            wide("ShotKit").as_ptr(),
            0x10,
        );
    }
}

unsafe fn pin(h: H) -> Result<(), String> {
    let export = bitmap_image(h)?;
    let width = i32::try_from(export.width).map_err(|_| "Bild zu breit".to_owned())?;
    let height = i32::try_from(export.height).map_err(|_| "Bild zu hoch".to_owned())?;
    let mut display = shotkit::alpha_document::render_document_budgeted(
        export.image(),
        &[],
        &shared_image_budget(),
    )?;
    display.flatten_checkerboard()?;
    let bitmap = create_bitmap(display.image())?;
    drop(display);
    let state = Box::new(PinState {
        export,
        display: bitmap,
        width,
        height,
        zoom: 1.0,
    });
    // Ownership passes to the window below; on any earlier failure free it here.
    let window = match create_window() {
        Ok(window) => window,
        Err(error) => {
            DeleteObject(bitmap);
            return Err(error);
        }
    };
    let state = Box::into_raw(state);
    SetWindowLongPtrW(window, GWLP_USERDATA, state as isize);
    place_initially(window, &mut *state);
    ShowWindow(window, 5); // SW_SHOW
    SetForegroundWindow(window);
    Ok(())
}

unsafe fn create_window() -> Result<H, String> {
    let instance = GetModuleHandleW(std::ptr::null());
    let class = wide(PIN_CLASS);
    if !REGISTERED.load(Ordering::SeqCst) {
        let wc = WndClass {
            style: 0x0008, // CS_DBLCLKS (non-client double click closes)
            proc: Some(proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: null_mut(),
            cursor: LoadCursorW(null_mut(), 32646usize as *const u16), // IDC_SIZEALL
            background: null_mut(),
            menu: std::ptr::null(),
            class: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return Err("Fensterklasse konnte nicht registriert werden.".into());
        }
        REGISTERED.store(true, Ordering::SeqCst);
    }
    let window = CreateWindowExW(
        0x0000_0008 | 0x0000_0080, // WS_EX_TOPMOST | WS_EX_TOOLWINDOW
        class.as_ptr(),
        wide("ShotKit – angeheftet").as_ptr(),
        0x8000_0000 | 0x0080_0000, // WS_POPUP | WS_BORDER
        0,
        0,
        1,
        1,
        null_mut(),
        null_mut(),
        instance,
        null_mut(),
    );
    if window.is_null() {
        return Err("Fenster konnte nicht erstellt werden.".into());
    }
    Ok(window)
}

/// Near the mouse, inside the work area; large images start scaled to fit.
unsafe fn place_initially(window: H, state: &mut PinState) {
    let mut cursor = Point { x: 0, y: 0 };
    GetCursorPos(&mut cursor);
    let mut info = WorkAreaInfo {
        size: std::mem::size_of::<WorkAreaInfo>() as u32,
        monitor: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        work: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        flags: 0,
    };
    let monitor = MonitorFromPoint(
        Point {
            x: cursor.x,
            y: cursor.y,
        },
        2,
    ); // MONITOR_DEFAULTTONEAREST
    let work = if !monitor.is_null() && get_work_area(monitor, &mut info) != 0 {
        info.work
    } else {
        Rect {
            left: 0,
            top: 0,
            right: 1024,
            bottom: 768,
        }
    };
    let (area_w, area_h) = (
        (work.right - work.left) as f32,
        (work.bottom - work.top) as f32,
    );
    let fit = (area_w * MAX_INITIAL_SHARE / state.width as f32)
        .min(area_h * MAX_INITIAL_SHARE / state.height as f32);
    state.zoom = initial_zoom(fit);
    let (w, h) = window_size(state);
    let x = (cursor.x - w / 2).clamp(work.left, (work.right - w).max(work.left));
    let y = (cursor.y - h / 2).clamp(work.top, (work.bottom - h).max(work.top));
    SetWindowPos(window, null_mut(), x, y, w, h, 0x0004 | 0x0010); // NOZORDER | NOACTIVATE
}

fn initial_zoom(fit: f32) -> f32 {
    if fit.is_finite() && fit < 1.0 {
        fit.max(MIN_ZOOM)
    } else {
        1.0
    }
}

fn next_zoom(zoom: f32, wheel_up: bool) -> f32 {
    let next = if wheel_up {
        zoom * ZOOM_STEP
    } else {
        zoom / ZOOM_STEP
    };
    next.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Window size for the current zoom, including the one-pixel WS_BORDER.
fn window_size(state: &PinState) -> (i32, i32) {
    let w = ((state.width as f32 * state.zoom).round() as i32).max(1);
    let h = ((state.height as f32 * state.zoom).round() as i32).max(1);
    (w + 2, h + 2)
}

unsafe fn resize(window: H, state: &PinState) {
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if GetWindowRect(window, &mut rect) == 0 {
        return;
    }
    let (w, h) = window_size(state);
    SetWindowPos(
        window,
        null_mut(),
        rect.left,
        rect.top,
        w,
        h,
        0x0004 | 0x0010,
    );
    InvalidateRect(window, std::ptr::null(), 0);
}

unsafe fn state<'a>(window: H) -> Option<&'a mut PinState> {
    (GetWindowLongPtrW(window, GWLP_USERDATA) as *mut PinState).as_mut()
}

unsafe fn copy(window: H, state: &PinState) {
    let result = create_bitmap(state.export.image()).and_then(|bitmap| {
        let result = clipboard::copy_bitmap(
            window,
            bitmap,
            state.width,
            state.height,
            active_settings().exclude_from_clipboard_history,
        );
        DeleteObject(bitmap);
        result
    });
    if let Err(error) = result {
        MessageBoxW(
            window,
            wide(&error).as_ptr(),
            wide("Zwischenablage").as_ptr(),
            0x10,
        );
    }
}

unsafe fn context_menu(window: H) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (IDM_COPY, "Kopieren\tStrg+C"),
        (IDM_OPEN, "Im Editor öffnen"),
        (IDM_ORIGINAL, "Originalgröße (100 %)"),
        (IDM_CLOSE, "Schließen\tEsc"),
    ] {
        AppendMenuW(menu, 0, id as usize, wide(label).as_ptr());
    }
    let mut p = Point { x: 0, y: 0 };
    GetCursorPos(&mut p);
    SetForegroundWindow(window);
    let choice = TrackPopupMenu(menu, 0x0100 | 0x0002, p.x, p.y, 0, window, std::ptr::null());
    DestroyMenu(menu);
    match choice {
        IDM_COPY => {
            if let Some(state) = state(window) {
                copy(window, state);
            }
        }
        IDM_OPEN => {
            if let Some(state) = state(window) {
                let image = state.export.clone();
                if let Err(error) = display_shared_image(image, None).map(|(_window, _guard)| ()) {
                    MessageBoxW(
                        window,
                        wide(&error).as_ptr(),
                        wide("ShotKit").as_ptr(),
                        0x10,
                    );
                }
            }
        }
        IDM_ORIGINAL => {
            if let Some(state) = state(window) {
                state.zoom = 1.0;
                resize(window, state);
            }
        }
        IDM_CLOSE => {
            DestroyWindow(window);
        }
        _ => {}
    }
}

unsafe fn paint(window: H) {
    let mut ps: Paint = std::mem::zeroed();
    let dc = BeginPaint(window, &mut ps);
    if dc.is_null() {
        return;
    }
    if let Some(state) = state(window) {
        let mut client = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(window, &mut client);
        let memory = CreateCompatibleDC(dc);
        if !memory.is_null() {
            let previous = SelectObject(memory, state.display);
            preview_scaling::blit_preview(
                dc,
                memory,
                (0, 0, client.right - client.left, client.bottom - client.top),
                (state.width, state.height),
                true,
            );
            SelectObject(memory, previous);
            DeleteDC(memory);
        }
    }
    EndPaint(window, &ps);
}

// SAFETY CONTRACT:
// Voraussetzung: Windows ruft die Prozedur auf dem UI-Thread mit gültigen Parametern.
// Warum erfüllt: Der Zustand liegt in GWLP_USERDATA und wird erst in WM_NCDESTROY freigegeben;
// Zugriffe kopieren benötigte Werte, bevor Menüs oder Dialoge Nachrichten auslösen.
// Nachrichten: Menüs, Zwischenablage und Editorerstellung können reentrieren.
unsafe extern "system" fn proc(window: H, message: u32, w: W, l: L) -> L {
    match message {
        WM_NCHITTEST => {
            // The whole image acts as a title bar: Windows moves the window.
            let hit = DefWindowProcW(window, message, w, l);
            if hit == HTCLIENT {
                HTCAPTION
            } else {
                hit
            }
        }
        WM_NCLBUTTONDBLCLK => {
            DestroyWindow(window);
            0
        }
        WM_CONTEXTMENU => {
            context_menu(window);
            0
        }
        WM_MOUSEWHEEL => {
            if let Some(state) = state(window) {
                let delta = ((w >> 16) & 0xFFFF) as u16 as i16;
                state.zoom = next_zoom(state.zoom, delta > 0);
                resize(window, state);
            }
            0
        }
        WM_KEYDOWN => {
            match w {
                0x1B => {
                    DestroyWindow(window);
                }
                0x43 if GetKeyState(0x11) < 0 => {
                    if let Some(state) = state(window) {
                        copy(window, state);
                    }
                }
                _ => {}
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(window);
            0
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut PinState;
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            if !raw.is_null() {
                let state = Box::from_raw(raw);
                DeleteObject(state.display);
                // Dropping `export` releases its image budget lease.
            }
            DefWindowProcW(window, message, w, l)
        }
        _ => DefWindowProcW(window, message, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::{initial_zoom, next_zoom, MAX_ZOOM, MIN_ZOOM};

    #[test]
    fn zoom_steps_stay_within_bounds_and_return_to_original() {
        let mut zoom = 1.0;
        for _ in 0..3 {
            zoom = next_zoom(zoom, true);
        }
        for _ in 0..3 {
            zoom = next_zoom(zoom, false);
        }
        assert!((zoom - 1.0).abs() < 1e-5);
        let mut zoom = 1.0;
        for _ in 0..20 {
            zoom = next_zoom(zoom, true);
        }
        assert_eq!(zoom, MAX_ZOOM);
        for _ in 0..40 {
            zoom = next_zoom(zoom, false);
        }
        assert_eq!(zoom, MIN_ZOOM);
    }

    #[test]
    fn large_images_start_scaled_to_fit_but_never_below_minimum() {
        assert_eq!(initial_zoom(2.0), 1.0);
        assert_eq!(initial_zoom(1.0), 1.0);
        assert_eq!(initial_zoom(0.5), 0.5);
        assert_eq!(initial_zoom(0.01), MIN_ZOOM);
        assert_eq!(initial_zoom(f32::NAN), 1.0);
    }
}
