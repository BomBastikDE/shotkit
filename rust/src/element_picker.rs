//! Win32 window-boundary suggestions for an overlay picker.
//!
//! `suggest_at` ignores every window owned by this process (including the
//! full-screen picker overlay), scans foreign top-level windows in Z order,
//! then returns matching child-window rectangles inside the first usable root.
//! This is HWND geometry only: it does not inspect browser DOM or promise that
//! controls correspond to visually meaningful page elements.

/// A screen point in physical Win32 pixels. Call from a per-monitor DPI-aware
/// thread, as this function temporarily requests that context for rectangle
/// comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// A possible window-boundary selection. Matching children are ordered by
/// increasing area; the top-level window is appended as the broad suggestion.
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

/// Find the topmost eligible foreign top-level window at `point` and its
/// visible child-window suggestions. The current process is always excluded,
/// which removes the active ShotKit overlay without hiding or flashing it.
///
/// Transparent hit-test windows, cloaked/minimized windows, and top-level tool
/// windows are skipped where Win32 reports those attributes. A provider's
/// custom region, per-pixel layered alpha, or non-HWND web content may not be
/// represented exactly. In particular, this API does not inspect a browser's
/// DOM; browser page areas often appear as one renderer HWND or no child HWNDs.
pub fn suggest_at(point: ScreenPoint) -> Result<Vec<ElementCandidate>, PickerError> {
    suggest_at_excluding(point, 0)
}

/// Like [`suggest_at`], with one additional HWND excluded. The process-wide
/// exclusion remains active; `excluded_hwnd` is useful for hosts that want to
/// exclude an overlay in a helper process too.
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
fn area(rect: (i32, i32, i32, i32)) -> i128 {
    let (left, top, right, bottom) = rect;
    (i128::from(right) - i128::from(left)) * (i128::from(bottom) - i128::from(top))
}

#[cfg(windows)]
mod windows {
    use super::{area, contains, ElementCandidate, PickerError, ScreenPoint};
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
        fn EnumChildWindows(parent: Hwnd, callback: EnumProc, param: isize) -> i32;
        fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn IsIconic(hwnd: Hwnd) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
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
    const WS_EX_TRANSPARENT: isize = 0x0000_0020;
    const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
    const WS_EX_LAYERED: isize = 0x0008_0000;
    const LWA_ALPHA: u32 = 0x2;
    const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
    const DWMWA_CLOAKED: u32 = 14;
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

    struct DpiContext(Hwnd);
    impl DpiContext {
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
        root_rect: Rect,
        items: Vec<ElementCandidate>,
    }

    fn tuple(rect: Rect) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

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

    unsafe fn is_hit_test_transparent(hwnd: Hwnd, exstyle: isize) -> bool {
        if exstyle & WS_EX_TRANSPARENT != 0 {
            return true;
        }
        if exstyle & WS_EX_LAYERED != 0 {
            let mut key = 0;
            let mut alpha = 255;
            let mut flags = 0;
            if GetLayeredWindowAttributes(hwnd, &mut key, &mut alpha, &mut flags) != 0
                && flags & LWA_ALPHA != 0
                && alpha == 0
            {
                return true;
            }
        }
        false
    }

    unsafe fn process_id(hwnd: Hwnd) -> Option<u32> {
        let mut pid = 0;
        if GetWindowThreadProcessId(hwnd, &mut pid) == 0 || pid == 0 {
            None
        } else {
            Some(pid)
        }
    }

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

    unsafe extern "system" fn collect_child(hwnd: Hwnd, param: isize) -> i32 {
        let search = &mut *(param as *mut ChildSearch);
        if hwnd.is_null() || IsWindowVisible(hwnd) == 0 || is_cloaked(hwnd) {
            return 1;
        }
        if process_id(hwnd) == Some(search.own_pid) {
            return 1;
        }
        let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if is_hit_test_transparent(hwnd, exstyle) {
            return 1;
        }
        if let Some(mut rect) = window_rect(hwnd, false) {
            rect.left = rect.left.max(search.root_rect.left);
            rect.top = rect.top.max(search.root_rect.top);
            rect.right = rect.right.min(search.root_rect.right);
            rect.bottom = rect.bottom.min(search.root_rect.bottom);
            if rect.right > rect.left
                && rect.bottom > rect.top
                && contains(tuple(rect), search.point)
            {
                search.items.push(ElementCandidate {
                    hwnd: hwnd as isize,
                    rect: tuple(rect),
                    is_top_level: false,
                });
            }
        }
        1
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
                root_rect,
                items: Vec::new(),
            };
            EnumChildWindows(root, Some(collect_child), &mut children as *mut _ as isize);
            children.items.sort_by_key(|candidate| area(candidate.rect));
            children.items.dedup_by_key(|candidate| candidate.hwnd);
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
    use super::{area, contains, ScreenPoint};

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
}
