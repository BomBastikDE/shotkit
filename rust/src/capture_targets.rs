//! Win32 target rectangles for screen capture.
//!
//! Rectangles use half-open physical screen coordinates `(left, top, right,
//! bottom)`. The process must establish per-monitor DPI awareness before it
//! creates windows or samples coordinates. The virtual-desktop rectangle is a
//! bounding box and may include gaps between monitors.

/// Half-open screen-space rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl ScreenRect {
    pub fn from_origin_size(x: i32, y: i32, width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 {
            return Err("Screen dimensions must be positive".into());
        }
        let right = x
            .checked_add(width)
            .ok_or_else(|| "Screen rectangle right edge overflow".to_string())?;
        let bottom = y
            .checked_add(height)
            .ok_or_else(|| "Screen rectangle bottom edge overflow".to_string())?;
        Self::new(x, y, right, bottom)
    }

    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, String> {
        if right.checked_sub(left).is_none_or(|width| width <= 0)
            || bottom.checked_sub(top).is_none_or(|height| height <= 0)
        {
            return Err("Screen rectangle must have positive dimensions".into());
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
    pub fn left(self) -> i32 {
        self.left
    }
    pub fn top(self) -> i32 {
        self.top
    }
    #[cfg(test)]
    pub fn as_tuple(self) -> (i32, i32, i32, i32) {
        (self.left, self.top, self.right, self.bottom)
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let rect = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
    }
}

#[cfg(windows)]
mod win32 {
    use super::ScreenRect;
    use std::{ffi::c_void, mem::size_of};

    type H = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn MonitorFromPoint(point: Point, flags: u32) -> H;
        fn GetMonitorInfoW(monitor: H, info: *mut MonitorInfo) -> i32;
        fn IsWindow(hwnd: H) -> i32;
        fn IsWindowVisible(hwnd: H) -> i32;
        fn IsIconic(hwnd: H) -> i32;
        fn GetWindowRect(hwnd: H, rect: *mut Rect) -> i32;
        fn GetAncestor(hwnd: H, flags: u32) -> H;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmGetWindowAttribute(hwnd: H, attribute: u32, value: *mut c_void, size: u32) -> i32;
    }

    const SM_XVIRTUALSCREEN: i32 = 76;
    const SM_YVIRTUALSCREEN: i32 = 77;
    const SM_CXVIRTUALSCREEN: i32 = 78;
    const SM_CYVIRTUALSCREEN: i32 = 79;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const GA_ROOT: u32 = 2;
    const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
    const DWMWA_CLOAKED: u32 = 14;

    fn convert(rect: Rect) -> Result<ScreenRect, String> {
        ScreenRect::new(rect.left, rect.top, rect.right, rect.bottom)
    }

    /// Bounds of the virtual desktop containing all visible displays. The
    /// rectangle can span unused gaps and can have negative left/top values.
    pub fn virtual_desktop() -> Result<ScreenRect, String> {
        unsafe {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if width <= 0 || height <= 0 {
                return Err("Windows returned invalid virtual desktop dimensions".into());
            }
            ScreenRect::from_origin_size(left, top, width, height)
                .map_err(|e| format!("Invalid virtual desktop rectangle: {e}"))
        }
    }

    /// Rectangle of the monitor containing the current cursor. If Windows
    /// reports a point in a desktop gap, `MONITOR_DEFAULTTONEAREST` chooses the
    /// nearest monitor.
    pub fn monitor_under_cursor() -> Result<ScreenRect, String> {
        unsafe {
            let mut point = Point::default();
            if GetCursorPos(&mut point) == 0 {
                return Err("GetCursorPos failed".into());
            }
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            if monitor.is_null() {
                return Err("MonitorFromPoint returned no monitor".into());
            }
            let mut info = MonitorInfo {
                size: size_of::<MonitorInfo>() as u32,
                monitor: Rect::default(),
                work: Rect::default(),
                flags: 0,
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return Err("GetMonitorInfoW failed".into());
            }
            convert(info.monitor)
        }
    }

    /// Rectangle of a previously captured foreground HWND. The caller must
    /// save the HWND before showing menus, changing foreground focus, or hiding
    /// its own UI; this function intentionally does not query foreground state.
    /// Child HWNDs are normalized to their root. Minimized, invisible, cloaked,
    /// invalid, or fully off-desktop windows are rejected. DWM frame bounds are
    /// preferred, with `GetWindowRect` as fallback, then clipped to the virtual
    /// desktop.
    pub fn saved_window(hwnd: isize) -> Result<ScreenRect, String> {
        unsafe {
            if hwnd == 0 {
                return Err("Saved window handle is null".into());
            }
            let mut target = hwnd as H;
            if IsWindow(target) == 0 {
                return Err("Saved window handle is no longer valid".into());
            }
            let root = GetAncestor(target, GA_ROOT);
            if !root.is_null() {
                target = root;
            }
            if IsWindow(target) == 0 || IsWindowVisible(target) == 0 {
                return Err("Saved window is not visible".into());
            }
            if IsIconic(target) != 0 {
                return Err("Saved window is minimized".into());
            }
            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(
                target,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            ) == 0
                && cloaked != 0
            {
                return Err("Saved window is cloaked".into());
            }

            let mut rect = Rect::default();
            let got_dwm = DwmGetWindowAttribute(
                target,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut rect as *mut Rect).cast(),
                size_of::<Rect>() as u32,
            ) == 0
                && rect.right > rect.left
                && rect.bottom > rect.top;
            if !got_dwm && GetWindowRect(target, &mut rect) == 0 {
                return Err("Could not read saved window bounds".into());
            }
            let window = convert(rect)?;
            let desktop = virtual_desktop()?;
            window
                .intersection(desktop)
                .ok_or_else(|| "Saved window is outside the virtual desktop".into())
        }
    }
}

#[cfg(windows)]
pub use win32::{monitor_under_cursor, saved_window, virtual_desktop};

#[cfg(not(windows))]
pub fn virtual_desktop() -> Result<ScreenRect, String> {
    Err("Capture target geometry is Windows-only".into())
}
#[cfg(not(windows))]
pub fn monitor_under_cursor() -> Result<ScreenRect, String> {
    Err("Capture target geometry is Windows-only".into())
}
#[cfg(not(windows))]
pub fn saved_window(_hwnd: isize) -> Result<ScreenRect, String> {
    Err("Capture target geometry is Windows-only".into())
}

#[cfg(test)]
mod tests {
    use super::ScreenRect;

    #[test]
    fn accepts_negative_origin_and_computes_positive_sizes() {
        let r = ScreenRect::new(-1920, -200, 0, 880).unwrap();
        assert_eq!((r.width(), r.height()), (1920, 1080));
    }

    #[test]
    fn rejects_empty_or_inverted_rectangles() {
        assert!(ScreenRect::new(4, 5, 4, 6).is_err());
        assert!(ScreenRect::new(4, 5, 8, 5).is_err());
        assert!(ScreenRect::new(5, 5, 4, 4).is_err());
    }

    #[test]
    fn intersection_clips_negative_and_outside_bounds() {
        let desktop = ScreenRect::new(-100, -50, 100, 100).unwrap();
        assert_eq!(
            ScreenRect::new(-150, 0, 20, 200)
                .unwrap()
                .intersection(desktop),
            Some(ScreenRect {
                left: -100,
                top: 0,
                right: 20,
                bottom: 100
            })
        );
        assert_eq!(
            ScreenRect::new(101, 0, 120, 20)
                .unwrap()
                .intersection(desktop),
            None
        );
    }

    #[test]
    fn rejects_edges_whose_size_overflows_i32() {
        assert!(ScreenRect::new(i32::MIN, 0, i32::MAX, 20).is_err());
    }

    #[test]
    fn origin_plus_size_checks_overflow_and_accepts_negative_origins() {
        assert_eq!(
            ScreenRect::from_origin_size(-1920, -100, 1920, 1080)
                .unwrap()
                .as_tuple(),
            (-1920, -100, 0, 980)
        );
        assert!(ScreenRect::from_origin_size(i32::MAX, 0, 1, 1).is_err());
        assert!(ScreenRect::from_origin_size(0, i32::MAX, 1, 1).is_err());
        assert!(ScreenRect::from_origin_size(0, 0, 0, 1).is_err());
    }
}
