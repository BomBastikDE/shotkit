//! Win32 window-boundary suggestions for an overlay picker.
//!
//! `suggest_at` ignores every window owned by this process (including the
//! full-screen picker overlay), scans foreign top-level windows in Z order,
//! then returns the topmost child chain at the point. [`WindowPicker`] provides
//! a process-wide asynchronous worker for pointer event handlers.
//! This is HWND geometry only: it does not inspect browser DOM or promise that
//! controls correspond to visually meaningful page elements.

#[cfg(windows)]
use std::thread;
use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
    },
};

/// A screen point in physical Win32 pixels. Call from a per-monitor DPI-aware
/// thread, as this function temporarily requests that context for rectangle
/// comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// A possible window-boundary selection. Matching children are returned from
/// outermost to deepest; the top-level window is appended as the broad suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementCandidate {
    /// Opaque HWND value. Do not retain as a durable identity; Windows may reuse it.
    pub hwnd: isize,
    /// Half-open screen rectangle `(left, top, right, bottom)`.
    pub rect: (i32, i32, i32, i32),
    pub is_top_level: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerError {
    #[cfg(not(windows))]
    UnsupportedPlatform,
    DpiAwarenessUnavailable,
    NoWindowAtPoint,
}

#[cfg(any(windows, test))]
fn layered_style_is_transparent(exstyle: u32, alpha: Option<u8>) -> bool {
    const WS_EX_TRANSPARENT: u32 = 0x20;
    const WS_EX_LAYERED: u32 = 0x0008_0000;
    exstyle & WS_EX_LAYERED != 0 && (exstyle & WS_EX_TRANSPARENT != 0 || alpha == Some(0))
}

/// Result of one asynchronous window scan. Compare `sequence` and `point` to
/// the latest submitted request before displaying or accepting the candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowScan {
    pub sequence: u64,
    pub point: ScreenPoint,
    pub candidates: Result<Vec<ElementCandidate>, PickerError>,
}

struct ScanRequest {
    sequence: u64,
    point: ScreenPoint,
    excluded_hwnd: isize,
}

struct ScanShared {
    pending: Mutex<Option<ScanRequest>>,
    result: Mutex<Option<WindowScan>>,
    wake: Condvar,
    next_sequence: AtomicU64,
    latest_sequence: AtomicU64,
}

static SCAN_SHARED: OnceLock<Arc<ScanShared>> = OnceLock::new();
static SCAN_INIT_LOCK: Mutex<()> = Mutex::new(());

/// Handle to the single process-wide Win32 scan worker. It coalesces pending
/// cursor points and stays alive for process lifetime, so opening overlays does
/// not create additional scan threads.
pub struct WindowPicker {
    shared: Arc<ScanShared>,
}

impl WindowPicker {
    pub fn new() -> io::Result<Self> {
        #[cfg(not(windows))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Win32 window scan is Windows-only",
        ));

        #[cfg(windows)]
        {
            if let Some(shared) = SCAN_SHARED.get() {
                return Ok(Self {
                    shared: Arc::clone(shared),
                });
            }
            let _guard = SCAN_INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(shared) = SCAN_SHARED.get() {
                return Ok(Self {
                    shared: Arc::clone(shared),
                });
            }
            let shared = Arc::new(ScanShared {
                pending: Mutex::new(None),
                result: Mutex::new(None),
                wake: Condvar::new(),
                next_sequence: AtomicU64::new(1),
                latest_sequence: AtomicU64::new(0),
            });
            let worker_shared = Arc::clone(&shared);
            let worker = thread::Builder::new()
                .name("shotkit-window-picker".into())
                .spawn(move || scan_worker(worker_shared))?;
            drop(worker); // The worker is process-wide and exits with the process.
            let _ = SCAN_SHARED.set(Arc::clone(&shared));
            Ok(Self { shared })
        }
    }

    pub fn submit(&self, point: ScreenPoint, excluded_hwnd: isize) -> u64 {
        let mut pending = lock(&self.shared.pending);
        let sequence = self.shared.next_sequence.fetch_add(1, Ordering::Relaxed);
        self.shared
            .latest_sequence
            .store(sequence, Ordering::Release);
        *pending = Some(ScanRequest {
            sequence,
            point,
            excluded_hwnd,
        });
        self.shared.wake.notify_one();
        sequence
    }

    pub fn try_take_latest(&self) -> Option<WindowScan> {
        let mut result = lock(&self.shared.result);
        if result.as_ref().map(|scan| scan.sequence)
            != Some(self.shared.latest_sequence.load(Ordering::Acquire))
        {
            return None;
        }
        result.take()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(windows)]
fn scan_worker(shared: Arc<ScanShared>) {
    loop {
        let request = {
            let mut pending = lock(&shared.pending);
            while pending.is_none() {
                pending = shared.wake.wait(pending).unwrap_or_else(|p| p.into_inner());
            }
            pending.take()
        };
        let Some(request) = request else { continue };
        let scan = WindowScan {
            sequence: request.sequence,
            point: request.point,
            candidates: suggest_at_excluding(request.point, request.excluded_hwnd),
        };
        if shared.latest_sequence.load(Ordering::Acquire) == request.sequence {
            *lock(&shared.result) = Some(scan);
        }
    }
}

/// Find the topmost eligible foreign top-level window at `point` and its
/// visible child-window suggestions. The current process is always excluded,
/// which removes the active ShotKit overlay without hiding or flashing it.
///
/// Transparent hit-test windows, cloaked/minimized windows, and top-level tool
/// windows are skipped where Win32 reports those attributes. A provider's
/// custom region, per-pixel layered alpha, or non-HWND web content may not be
/// represented exactly. In particular, this API does not inspect a browser's
/// DOM; browser page areas often appear as one renderer HWND or no child HWNDs.
/// An additional HWND can be excluded for overlays hosted by helper processes.
#[cfg(windows)]
pub fn suggest_at_excluding(
    point: ScreenPoint,
    excluded_hwnd: isize,
) -> Result<Vec<ElementCandidate>, PickerError> {
    windows::suggest_at(point, excluded_hwnd)
}

/// Win32 window suggestions are unavailable on other platforms.
#[cfg(not(windows))]
pub fn suggest_at_excluding(
    _point: ScreenPoint,
    _excluded_hwnd: isize,
) -> Result<Vec<ElementCandidate>, PickerError> {
    Err(PickerError::UnsupportedPlatform)
}

#[cfg(any(windows, test))]
fn contains(rect: (i32, i32, i32, i32), point: ScreenPoint) -> bool {
    let (left, top, right, bottom) = rect;
    point.x >= left && point.x < right && point.y >= top && point.y < bottom
}

#[cfg(any(windows, test))]
pub(crate) fn confirms_click(
    expected_hwnd: isize,
    candidate: Option<ElementCandidate>,
    point: ScreenPoint,
) -> bool {
    candidate
        .is_some_and(|candidate| candidate.hwnd == expected_hwnd && contains(candidate.rect, point))
}

#[cfg(test)]
fn area(rect: (i32, i32, i32, i32)) -> i128 {
    let (left, top, right, bottom) = rect;
    (i128::from(right) - i128::from(left)) * (i128::from(bottom) - i128::from(top))
}

#[cfg(windows)]
mod windows {
    use super::{
        contains, layered_style_is_transparent, ElementCandidate, PickerError, ScreenPoint,
    };
    use std::{ffi::c_void, mem::size_of};

    type Hwnd = *mut c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    type EnumProc = Option<unsafe extern "system" fn(Hwnd, isize) -> i32>;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: EnumProc, param: isize) -> i32;
        fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn IsIconic(hwnd: Hwnd) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
        fn GetTopWindow(hwnd: Hwnd) -> Hwnd;
        fn GetWindow(hwnd: Hwnd, command: u32) -> Hwnd;
        fn GetLayeredWindowAttributes(
            hwnd: Hwnd,
            key: *mut u32,
            alpha: *mut u8,
            flags: *mut u32,
        ) -> i32;
        fn SetThreadDpiAwarenessContext(context: Hwnd) -> Hwnd;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmGetWindowAttribute(hwnd: Hwnd, attribute: u32, value: *mut c_void, size: u32) -> i32;
    }

    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
    const WS_EX_LAYERED: isize = 0x0008_0000;
    const LWA_ALPHA: u32 = 0x2;
    const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
    const DWMWA_CLOAKED: u32 = 14;
    const GW_HWNDNEXT: u32 = 2;
    const MAX_SIBLINGS: usize = 256;
    const MAX_CHILD_DEPTH: usize = 32;
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

    struct DpiContext(Hwnd);
    impl DpiContext {
        // SAFETY CONTRACT:
        // Voraussetzung: Der Guard wird auf demselben Thread und in passender Reihenfolge beendet.
        // Warum erfüllt bzw. offen: suggest_at hält ihn lokal; der alte DPI-Kontext stammt von Windows.
        // Nachrichten: Kein expliziter Fensternachrichtenversand.
        unsafe fn enter() -> Option<Self> {
            let previous =
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 as Hwnd);
            if previous.is_null() {
                None
            } else {
                Some(Self(previous))
            }
        }
    }
    impl Drop for DpiContext {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
    }

    struct RootSearch {
        point: ScreenPoint,
        own_pid: u32,
        excluded_hwnd: Hwnd,
        root: Option<(Hwnd, Rect)>,
    }

    struct ChildSearch {
        point: ScreenPoint,
        own_pid: u32,
        items: Vec<ElementCandidate>,
    }

    fn tuple(rect: Rect) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    // SAFETY CONTRACT:
    // Voraussetzung: hwnd wird als opakes Handle behandelt; Ausgabepuffer sind gültig.
    // Warum erfüllt bzw. offen: Ein lokales Rect und dessen korrekte Größe werden übergeben; dauerhafte Handle-Identität ist offen.
    // Nachrichten: Keine allgemeine Reentranzfreiheit bei DWM-Abfragen belegt.
    unsafe fn window_rect(hwnd: Hwnd, extended: bool) -> Option<Rect> {
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let got = if extended {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut rect as *mut _ as *mut c_void,
                size_of::<Rect>() as u32,
            ) == 0
        } else {
            false
        };
        if !got && GetWindowRect(hwnd, &mut rect) == 0 {
            return None;
        }
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return None;
        }
        Some(rect)
    }

    // SAFETY CONTRACT:
    // Voraussetzung: hwnd ist ein opaker Handlewert und der Ausgabepuffer ist gültig.
    // Warum erfüllt bzw. offen: DWM erhält einen lokalen u32 mit passender Größe; Fehler belegen keinen unverhüllten Zustand.
    // Nachrichten: Kein expliziter Nachrichtenversand; Reentranzfreiheit nicht belegt.
    unsafe fn is_cloaked(hwnd: Hwnd) -> bool {
        let mut cloaked: u32 = 0;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            size_of::<u32>() as u32,
        ) == 0
            && cloaked != 0
    }

    // SAFETY CONTRACT:
    // Voraussetzung: hwnd und exstyle beziehen sich für die Aussage auf dasselbe Fenster.
    // Warum erfüllt bzw. offen: Ausgabewerte liegen lokal; Fenster und Stil können sich zwischen Abfragen ändern.
    // Nachrichten: Kein expliziter Nachrichtenversand; Reentranzfreiheit nicht belegt.
    unsafe fn is_hit_test_transparent(hwnd: Hwnd, exstyle: isize) -> bool {
        if exstyle & WS_EX_LAYERED != 0 {
            // WS_EX_TRANSPARENT makes a layered window pass mouse hit tests
            // through; on a non-layered top-level window it is a paint-order
            // style, not sufficient evidence of hit-test transparency.
            if layered_style_is_transparent(exstyle as u32, None) {
                return true;
            }
            let mut key = 0;
            let mut alpha = 255;
            let mut flags = 0;
            if GetLayeredWindowAttributes(hwnd, &mut key, &mut alpha, &mut flags) != 0
                && flags & LWA_ALPHA != 0
                && alpha == 0
            {
                return layered_style_is_transparent(exstyle as u32, Some(alpha));
            }
        }
        false
    }

    // SAFETY CONTRACT:
    // Voraussetzung: hwnd wird als opaker Handlewert behandelt.
    // Warum erfüllt bzw. offen: Ein lokaler u32 wird beschrieben und das API-Ergebnis geprüft; die Zuordnung kann später veralten.
    // Nachrichten: Kein expliziter Nachrichtenversand; Reentranzfreiheit nicht belegt.
    unsafe fn process_id(hwnd: Hwnd) -> Option<u32> {
        let mut pid = 0;
        if GetWindowThreadProcessId(hwnd, &mut pid) == 0 || pid == 0 {
            None
        } else {
            Some(pid)
        }
    }

    // SAFETY CONTRACT:
    // Voraussetzung: param zeigt exklusiv auf eine lebende, ausgerichtete RootSearch; kein Unwind über die Callback-ABI.
    // Warum erfüllt bzw. offen: EnumWindows erhält einen lokalen Zustand und speichert den Pointer nicht; Panikfreiheit bleibt offen.
    // Nachrichten: EnumWindows ruft diesen Callback synchron auf; API-Abfragen sind nicht pauschal reentranzfrei.
    unsafe extern "system" fn find_root(hwnd: Hwnd, param: isize) -> i32 {
        let search = &mut *(param as *mut RootSearch);
        if hwnd.is_null()
            || hwnd == search.excluded_hwnd
            || IsWindowVisible(hwnd) == 0
            || IsIconic(hwnd) != 0
        {
            return 1;
        }
        if process_id(hwnd) != Some(search.own_pid) && !is_cloaked(hwnd) {
            let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            // Tool windows commonly represent transient palettes/tooltips. Skip
            // them as roots so they do not hide the app window below the cursor.
            if exstyle & WS_EX_TOOLWINDOW == 0 && !is_hit_test_transparent(hwnd, exstyle) {
                if let Some(rect) = window_rect(hwnd, true) {
                    if contains(tuple(rect), search.point) {
                        search.root = Some((hwnd, rect));
                        return 0; // EnumWindows visits top-level windows in Z order.
                    }
                }
            }
        }
        1
    }

    // SAFETY CONTRACT:
    // Voraussetzung: search bleibt exklusiv geliehen und HWNDs sind opake Handles.
    // Warum erfüllt bzw. offen: Lokaler Suchzustand wird rekursiv neu geborrowt; Grenzen verhindern endlose Suche, nicht veraltete Fensterdaten.
    // Nachrichten: Kein expliziter Nachrichtenversand; allgemeine Reentranzfreiheit nicht belegt.
    unsafe fn collect_topmost_child(
        parent: Hwnd,
        parent_rect: Rect,
        search: &mut ChildSearch,
        depth: usize,
    ) {
        if depth >= MAX_CHILD_DEPTH {
            return;
        }
        let mut child = GetTopWindow(parent);
        let mut visited = Vec::new();
        for _ in 0..MAX_SIBLINGS {
            if child.is_null() || visited.contains(&(child as usize)) {
                break;
            }
            visited.push(child as usize);
            let next = GetWindow(child, GW_HWNDNEXT);
            let eligible = IsWindowVisible(child) != 0
                && !is_cloaked(child)
                && process_id(child) != Some(search.own_pid)
                && !is_hit_test_transparent(child, GetWindowLongPtrW(child, GWL_EXSTYLE));
            if eligible {
                if let Some(mut rect) = window_rect(child, false) {
                    rect.left = rect.left.max(parent_rect.left);
                    rect.top = rect.top.max(parent_rect.top);
                    rect.right = rect.right.min(parent_rect.right);
                    rect.bottom = rect.bottom.min(parent_rect.bottom);
                    if rect.right > rect.left
                        && rect.bottom > rect.top
                        && contains(tuple(rect), search.point)
                    {
                        search.items.push(ElementCandidate {
                            hwnd: child as isize,
                            rect: tuple(rect),
                            is_top_level: false,
                        });
                        collect_topmost_child(child, rect, search, depth + 1);
                        return; // Lower siblings are obscured at this point.
                    }
                }
            }
            child = next;
        }
    }

    pub(super) fn suggest_at(
        point: ScreenPoint,
        excluded_hwnd: isize,
    ) -> Result<Vec<ElementCandidate>, PickerError> {
        unsafe {
            let Some(_dpi) = DpiContext::enter() else {
                return Err(PickerError::DpiAwarenessUnavailable);
            };
            let mut search = RootSearch {
                point,
                own_pid: GetCurrentProcessId(),
                excluded_hwnd: excluded_hwnd as Hwnd,
                root: None,
            };
            EnumWindows(Some(find_root), &mut search as *mut _ as isize);
            let Some((root, root_rect)) = search.root else {
                return Err(PickerError::NoWindowAtPoint);
            };

            let mut children = ChildSearch {
                point,
                own_pid: search.own_pid,
                items: Vec::new(),
            };
            collect_topmost_child(root, root_rect, &mut children, 0);
            children.items.push(ElementCandidate {
                hwnd: root as isize,
                rect: tuple(root_rect),
                is_top_level: true,
            });
            Ok(children.items)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        area, confirms_click, contains, layered_style_is_transparent, ElementCandidate, ScreenPoint,
    };

    #[test]
    fn rectangle_contains_uses_half_open_edges_and_negative_desktop_origin() {
        let r = (-100, -50, 20, 30);
        assert!(contains(r, ScreenPoint { x: -100, y: -50 }));
        assert!(contains(r, ScreenPoint { x: 19, y: 29 }));
        assert!(!contains(r, ScreenPoint { x: 20, y: 0 }));
        assert!(!contains(r, ScreenPoint { x: 0, y: 30 }));
    }

    #[test]
    fn area_orders_nested_window_suggestions_and_handles_full_i32_span() {
        assert!(area((1, 1, 10, 10)) < area((0, 0, 20, 20)));
        assert_eq!(
            area((i32::MIN, i32::MIN, i32::MAX, i32::MAX)),
            (i128::from(u32::MAX)) * i128::from(u32::MAX)
        );
    }

    #[test]
    fn transparent_style_requires_layered_window_context() {
        assert!(!layered_style_is_transparent(0x20, None));
        assert!(layered_style_is_transparent(0x0008_0020, None));
        assert!(layered_style_is_transparent(0x0008_0000, Some(0)));
        assert!(!layered_style_is_transparent(0x0008_0000, Some(128)));
    }

    #[test]
    fn click_confirmation_rejects_reused_or_moved_window_suggestions() {
        let point = ScreenPoint { x: 10, y: 10 };
        let candidate = ElementCandidate {
            hwnd: 7,
            rect: (0, 0, 20, 20),
            is_top_level: true,
        };
        assert!(confirms_click(7, Some(candidate), point));
        assert!(!confirms_click(8, Some(candidate), point));
        let moved = ElementCandidate {
            rect: (20, 20, 30, 30),
            ..candidate
        };
        assert!(!confirms_click(7, Some(moved), point));
        assert!(!confirms_click(7, None, point));
    }
}
