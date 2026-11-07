//! Exercises synthetic WM_DPICHANGED on an isolated, hidden window. This does
//! not claim coverage of a real move between monitors with different DPI.
use super::*;
use std::mem::size_of;

#[link(name = "user32")]
extern "system" {
    fn GetWindowRect(hwnd: H, rect: *mut Rect) -> i32;
    fn MapWindowPoints(from: H, to: H, points: *mut c_void, count: u32) -> i32;
    fn SetThreadDpiAwarenessContext(context: H) -> H;
    fn UnregisterClassW(class: *const u16, instance: H) -> i32;
}
#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(object: H, bytes: i32, output: H) -> i32;
}

struct TestWindow {
    hwnd: H,
    class: Vec<u16>,
    instance: H,
    old_dpi_context: H,
}
impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            // Only our own same-thread, hidden window is touched. No message
            // loop, global window enumeration, sleeps or unbounded polling.
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
            UnregisterClassW(self.class.as_ptr(), self.instance);
            if !self.old_dpi_context.is_null() {
                SetThreadDpiAwarenessContext(self.old_dpi_context);
            }
        }
    }
}

unsafe fn rect(hwnd: H) -> Rect {
    let mut rect: Rect = zeroed();
    assert_ne!(GetWindowRect(hwnd, &mut rect), 0);
    rect
}

unsafe fn font_height(font: H) -> i32 {
    let mut font_data: LogFontW = zeroed();
    assert_eq!(
        GetObjectW(
            font,
            size_of::<LogFontW>() as i32,
            (&mut font_data as *mut LogFontW).cast()
        ),
        size_of::<LogFontW>() as i32
    );
    font_data.height
}

#[test]
fn synthetic_dpi_change_resizes_controls_and_fonts_and_releases_resources() {
    unsafe {
        let mut window = TestWindow {
            hwnd: null_mut(),
            class: wide("ShotKitAboutDpiIsolatedTest"),
            instance: GetModuleHandleW(null_mut()),
            old_dpi_context: SetThreadDpiAwarenessContext(-4isize as H),
        };
        assert!(
            !window.old_dpi_context.is_null(),
            "per-monitor-v2 test context unavailable"
        );
        let class = WndClass {
            style: 0,
            proc: Some(about_proc),
            class_extra: 0,
            window_extra: 0,
            instance: window.instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu_name: null_mut(),
            class_name: window.class.as_ptr(),
        };
        assert_ne!(RegisterClassW(&class), 0);
        window.hwnd = CreateWindowExW(
            0,
            window.class.as_ptr(),
            wide("DPI test").as_ptr(),
            WS_CAPTION | WS_SYSMENU,
            40,
            40,
            410,
            230,
            null_mut(),
            null_mut(),
            window.instance,
            null_mut(),
        );
        assert!(!window.hwnd.is_null());
        let ptr = GetWindowLongPtrW(window.hwnd, GWLP_USERDATA) as *const State;
        assert!(!ptr.is_null());
        let controls = (*ptr).controls.get();
        assert!(controls
            .iter()
            .all(|control| !control.is_null() && IsWindow(*control) != 0));
        let initial_dpi = GetDpiForWindow(window.hwnd).max(96);
        let initial = rect(window.hwnd);
        assert_eq!(initial.right - initial.left, scale(410, initial_dpi));
        assert_eq!(initial.bottom - initial.top, scale(230, initial_dpi));
        let mut previous_font: H = null_mut();
        let mut previous_height = 0;
        for dpi in [96u32, 144, 192] {
            let suggested = Rect {
                left: 40,
                top: 40,
                right: 40 + scale(410, dpi),
                bottom: 40 + scale(230, dpi),
            };
            SendMessageW(
                window.hwnd,
                WM_DPICHANGED,
                (dpi | (dpi << 16)) as usize,
                (&suggested as *const Rect) as isize,
            );
            let actual = rect(window.hwnd);
            assert_eq!(
                (actual.left, actual.top, actual.right, actual.bottom),
                (
                    suggested.left,
                    suggested.top,
                    suggested.right,
                    suggested.bottom
                )
            );
            let mut selected_font: H = null_mut();
            for (control, (x, y, width, height)) in controls.into_iter().zip(CONTROL_RECTS) {
                let mut bounds = rect(control);
                MapWindowPoints(
                    null_mut(),
                    window.hwnd,
                    (&mut bounds as *mut Rect).cast(),
                    2,
                );
                assert_eq!(
                    (
                        bounds.left,
                        bounds.top,
                        bounds.right - bounds.left,
                        bounds.bottom - bounds.top
                    ),
                    (
                        scale(x, dpi),
                        scale(y, dpi),
                        scale(width, dpi),
                        scale(height, dpi)
                    )
                );
                let font = SendMessageW(control, 0x0031, 0, 0) as H; // WM_GETFONT
                assert!(!font.is_null());
                if !selected_font.is_null() {
                    assert_eq!(font, selected_font);
                }
                selected_font = font;
            }
            let height = font_height(selected_font).abs();
            assert!(height > 0);
            if !previous_font.is_null() {
                assert_eq!(
                    font_height(previous_font).abs(),
                    previous_height,
                    "previous fonts must survive until child destruction"
                );
                let expected = if dpi == 144 {
                    (previous_height * 3 + 1) / 2
                } else {
                    (previous_height * 4 + 1) / 3
                };
                assert!(
                    (height - expected).abs() <= 1,
                    "font did not scale with DPI"
                );
            }
            previous_font = selected_font;
            previous_height = height;
        }
        let invalid = Rect {
            left: 10,
            top: 10,
            right: 0,
            bottom: 0,
        };
        SendMessageW(
            window.hwnd,
            WM_DPICHANGED,
            192 | (192 << 16),
            (&invalid as *const Rect) as isize,
        );
        let before = rect(window.hwnd);
        SendMessageW(window.hwnd, WM_DPICHANGED, 0, 0);
        let after = rect(window.hwnd);
        assert_eq!(
            (before.left, before.top, before.right, before.bottom),
            (after.left, after.top, after.right, after.bottom)
        );
        let fonts = (*ptr).fonts.borrow().clone();
        assert!(!fonts.is_empty(), "DPI-specific font creation failed");
        assert_ne!(DestroyWindow(window.hwnd), 0);
        window.hwnd = null_mut();
        for control in controls {
            assert_eq!(IsWindow(control), 0);
        }
        for font in fonts {
            let mut data: LogFontW = zeroed();
            assert_eq!(
                GetObjectW(
                    font,
                    size_of::<LogFontW>() as i32,
                    (&mut data as *mut LogFontW).cast()
                ),
                0,
                "owned font leaked"
            );
        }
    }
}
