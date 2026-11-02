//! Persistent, asynchronous UI Automation hit testing for an overlay picker.
//!
//! The caller supplies the underlying target HWND explicitly. This avoids the
//! global ElementFromPoint hit test, which can land on a visible full-screen
//! overlay. The worker uses ElementFromHandle and searches a bounded Control
//! View subtree by physical screen rectangles. All COM objects stay on the
//! worker's MTA thread; the UI exchanges only integers and rectangle values.

use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex, MutexGuard,
    },
    thread::{self, JoinHandle},
};

const MAX_TREE_NODES: usize = 128;
const MAX_TREE_DEPTH: usize = 12;
const WORK_BUDGET_MS: u64 = 900;
const UIA_TRANSACTION_TIMEOUT_MS: u32 = 700;

/// Screen coordinate in physical Win32 screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// Half-open element bounds `(left, top, right, bottom)` in physical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Why a best-effort search could not produce a complete result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerError {
    InvalidTargetWindow,
    ComInitialization(i32),
    Automation(i32),
    TimedOut,
    SearchLimitReached,
}

/// A completed or partial response for one submitted cursor position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickResult {
    /// Monotonically increasing ID assigned by [`UiaPicker::submit`].
    pub sequence: u64,
    pub target_hwnd: isize,
    /// Best matching visible rectangle found so far, when any.
    pub rect: Option<ElementRect>,
    /// `None` means the bounded search completed; `Some` marks a partial/error result.
    pub issue: Option<PickerError>,
}

#[derive(Clone, Copy)]
struct Request {
    sequence: u64,
    target_hwnd: isize,
    point: ScreenPoint,
}

struct Shared {
    pending: Mutex<Option<Request>>,
    response: Mutex<Option<PickResult>>,
    changed: Condvar,
    next_sequence: AtomicU64,
    latest_sequence: AtomicU64,
    stopping: Mutex<bool>,
}

/// Single persistent worker. A new submission replaces any not-yet-started
/// request; completed responses are tagged so callers can discard stale ones.
pub struct UiaPicker {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl UiaPicker {
    /// Create the dedicated COM worker. This starts the thread but does not wait
    /// for UI Automation or any provider.
    pub fn new() -> io::Result<Self> {
        #[cfg(not(windows))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "UI Automation is Windows-only",
        ));

        #[cfg(windows)]
        {
            let shared = Arc::new(Shared {
                pending: Mutex::new(None),
                response: Mutex::new(None),
                changed: Condvar::new(),
                next_sequence: AtomicU64::new(1),
                latest_sequence: AtomicU64::new(0),
                stopping: Mutex::new(false),
            });
            let worker_shared = Arc::clone(&shared);
            let worker = thread::Builder::new()
                .name("shotkit-uia-picker".into())
                .spawn(move || windows_worker(worker_shared))?;
            Ok(Self {
                shared,
                worker: Some(worker),
            })
        }
    }

    /// Queue a hover query without waiting for UI Automation. The HWND must be
    /// the underlying target window, excluding the picker overlay itself.
    /// Pending hover work is coalesced to the most recent point/target.
    pub fn submit(&self, target_hwnd: isize, point: ScreenPoint) -> u64 {
        let mut pending = lock(&self.shared.pending);
        let sequence = self.shared.next_sequence.fetch_add(1, Ordering::Relaxed);
        self.shared
            .latest_sequence
            .store(sequence, Ordering::Release);
        *pending = Some(Request {
            sequence,
            target_hwnd,
            point,
        });
        self.shared.changed.notify_one();
        sequence
    }

    /// Take the latest completed response if it still matches the most recent
    /// request. This is nonblocking and suitable for UI polling/message delivery.
    pub fn try_take_latest(&self) -> Option<PickResult> {
        let mut response = lock(&self.shared.response);
        if response.as_ref().map(|r| r.sequence)
            != Some(self.shared.latest_sequence.load(Ordering::Acquire))
        {
            return None;
        }
        response.take()
    }
}

impl Drop for UiaPicker {
    fn drop(&mut self) {
        // Hold the condition-variable mutex while changing the stop condition;
        // otherwise notification can be lost between the worker check and wait.
        let _pending = lock(&self.shared.pending);
        *lock(&self.shared.stopping) = true;
        self.shared.changed.notify_one();
        // Dropping JoinHandle detaches; never wait for an unresponsive provider
        // on the caller's UI thread. UIA transaction timeout bounds COM calls.
        self.worker.take();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(windows)]
fn windows_worker(shared: Arc<Shared>) {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
        worker_loop(
            &shared,
            None,
            Some(PickerError::ComInitialization(initialized.0)),
        );
        return;
    }
    let apartment = ComApartment;
    let automation = unsafe { create_automation() };
    let (automation, startup_error) = match automation {
        Ok(a) => (Some(a), None),
        Err(e) => (None, Some(e)),
    };
    worker_loop(&shared, automation, startup_error);
    drop(apartment);
}

#[cfg(windows)]
struct ComApartment;

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() }
    }
}

#[cfg(windows)]
fn worker_loop(
    shared: &Shared,
    automation: Option<windows::Win32::UI::Accessibility::IUIAutomation>,
    startup_error: Option<PickerError>,
) {
    loop {
        let request = {
            let mut pending = lock(&shared.pending);
            while pending.is_none() && !*lock(&shared.stopping) {
                pending = shared
                    .changed
                    .wait(pending)
                    .unwrap_or_else(|p| p.into_inner());
            }
            if *lock(&shared.stopping) {
                return;
            }
            pending.take()
        };
        let Some(request) = request else { continue };
        let result = match (&automation, startup_error) {
            (Some(automation), None) => unsafe { inspect(automation, request) },
            (_, Some(error)) => PickResult {
                sequence: request.sequence,
                target_hwnd: request.target_hwnd,
                rect: None,
                issue: Some(error),
            },
            _ => PickResult {
                sequence: request.sequence,
                target_hwnd: request.target_hwnd,
                rect: None,
                issue: Some(PickerError::Automation(0)),
            },
        };
        if shared.latest_sequence.load(Ordering::Acquire) == request.sequence {
            *lock(&shared.response) = Some(result);
        }
    }
}

#[cfg(windows)]
unsafe fn create_automation(
) -> Result<windows::Win32::UI::Accessibility::IUIAutomation, PickerError> {
    use windows::{
        core::GUID,
        Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
    };

    const CLSID_CUIAUTOMATION: GUID = GUID::from_u128(0xff48dba4_60ef_4201_aa87_54103eef594e);
    CoCreateInstance(&CLSID_CUIAUTOMATION, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| PickerError::Automation(e.code().0))
}

#[cfg(windows)]
unsafe fn inspect(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    request: Request,
) -> PickResult {
    use std::time::{Duration, Instant};
    use windows::{
        core::Interface,
        Win32::{
            Foundation::RECT,
            UI::Accessibility::{IUIAutomation2, IUIAutomationElement},
        },
    };

    let mut out = PickResult {
        sequence: request.sequence,
        target_hwnd: request.target_hwnd,
        rect: None,
        issue: None,
    };
    if request.target_hwnd == 0 {
        out.issue = Some(PickerError::InvalidTargetWindow);
        return out;
    }
    // UIA2 exposes transaction limits. If unavailable, avoid making provider
    // calls with the much longer system default timeout.
    let automation2: IUIAutomation2 = match automation.cast() {
        Ok(a) => a,
        Err(e) => {
            out.issue = Some(PickerError::Automation(e.code().0));
            return out;
        }
    };
    if let Err(e) = automation2.SetTransactionTimeout(UIA_TRANSACTION_TIMEOUT_MS) {
        out.issue = Some(PickerError::Automation(e.code().0));
        return out;
    }
    if let Err(e) = automation2.SetConnectionTimeout(UIA_TRANSACTION_TIMEOUT_MS) {
        out.issue = Some(PickerError::Automation(e.code().0));
        return out;
    }

    let hwnd = windows::Win32::Foundation::HWND(request.target_hwnd as *mut _);
    let root = match automation.ElementFromHandle(hwnd) {
        Ok(element) => element,
        Err(e) => {
            out.issue = Some(PickerError::Automation(e.code().0));
            return out;
        }
    };
    let walker = match automation.ControlViewWalker() {
        Ok(walker) => walker,
        Err(e) => {
            out.issue = Some(PickerError::Automation(e.code().0));
            return out;
        }
    };

    let deadline = Instant::now() + Duration::from_millis(WORK_BUDGET_MS);
    let mut stack: Vec<(IUIAutomationElement, usize)> = vec![(root, 0)];
    let mut visited = 0usize;
    let mut last_error = None;
    let mut best_area = i128::MAX;
    while let Some((element, depth)) = stack.pop() {
        if Instant::now() >= deadline {
            out.issue = Some(PickerError::TimedOut);
            break;
        }
        if visited >= MAX_TREE_NODES {
            out.issue = Some(PickerError::SearchLimitReached);
            break;
        }
        visited += 1;

        let bounds: Option<RECT> = match element.CurrentBoundingRectangle() {
            Ok(rect) => Some(rect),
            // Continue down a branch without bounds, subject to the global
            // node/depth caps; descendants may still expose a usable rectangle.
            Err(e) => {
                last_error = Some(e.code().0);
                None
            }
        };
        let offscreen = match element.CurrentIsOffscreen() {
            Ok(value) => value.0 != 0,
            // Keep exploring when the bounds are usable; descendants may still
            // provide a useful hit even when this node's visibility is unknown.
            Err(e) => {
                last_error = Some(e.code().0);
                false
            }
        };
        let contains = bounds
            .map(|r| {
                request.point.x >= r.left
                    && request.point.x < r.right
                    && request.point.y >= r.top
                    && request.point.y < r.bottom
            })
            .unwrap_or(false);
        if let Some(r) =
            bounds.filter(|r| !offscreen && contains && r.right > r.left && r.bottom > r.top)
        {
            let width = i128::from(r.right) - i128::from(r.left);
            let height = i128::from(r.bottom) - i128::from(r.top);
            let area = width * height;
            if area < best_area {
                best_area = area;
                out.rect = Some(ElementRect {
                    left: r.left,
                    top: r.top,
                    right: r.right,
                    bottom: r.bottom,
                });
            }
        }
        if depth >= MAX_TREE_DEPTH {
            out.issue = Some(PickerError::SearchLimitReached);
            continue;
        }
        if offscreen || (bounds.is_some() && !contains) {
            continue;
        }

        // Fetch one child at a time via sibling links. Errors on an individual
        // provider branch are best-effort; the bounded search keeps other hits.
        let mut child = match walker.GetFirstChildElement(&element) {
            Ok(child) => Some(child),
            Err(e) => {
                if !is_no_child_error(e.code().0) {
                    last_error = Some(e.code().0);
                }
                None
            }
        };
        let mut siblings = 0usize;
        while let Some(current) = child {
            if Instant::now() >= deadline {
                out.issue = Some(PickerError::TimedOut);
                break;
            }
            if visited + stack.len() >= MAX_TREE_NODES {
                out.issue = Some(PickerError::SearchLimitReached);
                break;
            }
            siblings += 1;
            stack.push((current.clone(), depth + 1));
            if siblings >= MAX_TREE_NODES {
                break;
            }
            child = match walker.GetNextSiblingElement(&current) {
                Ok(next) => Some(next),
                Err(e) => {
                    if !is_no_child_error(e.code().0) {
                        last_error = Some(e.code().0);
                    }
                    None
                }
            };
        }
    }
    if out.issue.is_none() {
        out.issue = if let Some(error) = last_error {
            Some(PickerError::Automation(error))
        } else if visited >= MAX_TREE_NODES && !stack.is_empty() {
            Some(PickerError::SearchLimitReached)
        } else {
            None
        };
    }
    out
}

#[cfg(windows)]
fn is_no_child_error(hr: i32) -> bool {
    // A provider may expose an empty/removed branch as a null interface.
    hr == 0x8000_4003u32 as i32 || hr == 0x8004_0201u32 as i32
}
