//! Runner-only acceptance of the unchanged HWND -> WM_PAINT -> BeginPaint path.
//! Run alone in a dedicated cargo test process; the admission pool is global.
//! Exhausting accounting is deterministic admission failure, not physical OOM.
use super::*;
use shotkit::resource_budget::AdmissionClass;

#[link(name = "user32")]
extern "system" {
    fn UpdateWindow(hwnd: H) -> i32;
    fn GetUpdateRect(hwnd: H, rect: *mut Rect, erase: i32) -> i32;
    fn UnregisterClassW(class: *const u16, instance: H) -> i32;
}
#[link(name = "gdi32")]
extern "system" {
    fn GetPixel(dc: H, x: i32, y: i32) -> u32;
    fn GdiFlush() -> i32;
}

struct Fixture {
    hwnd: H,
    class: Vec<u16>,
    instance: H,
    lifetime: Rc<Cell<H>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            // A cleared lifetime must never target a possibly reused HWND.
            if self.lifetime.get() == self.hwnd {
                let _ = DestroyWindow(self.hwnd);
            }
            if self.lifetime.get().is_null() {
                let _ = UnregisterClassW(self.class.as_ptr(), self.instance);
            }
        }
    }
}
struct WindowDc(H, H);
impl Drop for WindowDc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(self.0, self.1) };
    }
}

unsafe fn pixel(hwnd: H, x: i32, y: i32) -> u32 {
    let dc = GetDC(hwnd);
    assert!(!dc.is_null());
    let dc = WindowDc(hwnd, dc);
    assert_ne!(GdiFlush(), 0);
    let value = GetPixel(dc.1, x, y);
    assert_ne!(value, u32::MAX, "GetPixel failed at ({x},{y})");
    value
}

#[test]
#[ignore = "requires dedicated GitHub Windows runner process and visible owned HWND"]
fn real_beginpaint_neutralizes_failed_redaction_preview_and_recovers() {
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(std::env::var("RUNNER_OS").as_deref(), Ok("Windows"));
    unsafe {
        // This process runs this exact test only. No app startup, tray, hotkeys,
        // input synthesis, filesystem fixtures, or unrelated HWND enumeration.
        let budget = shared_image_budget();
        assert_eq!(budget.snapshot().used_bytes, 0);
        assert!((&*std::ptr::addr_of!(EDITOR_WINDOWS)).is_empty());
        assert!(PRIMARY.is_null());
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(&format!("ShotKitPaintAcceptance{}", std::process::id()));
        let wc = WndClass {
            style: 0,
            proc: Some(window::proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu: std::ptr::null(),
            class: class.as_ptr(),
        };
        assert_ne!(RegisterClassW(&wc), 0);
        let lifetime = Rc::new(Cell::new(null_mut()));
        let marker = EditorCreationProtection {
            close_blocked: Rc::new(Cell::new(false)),
            lifetime: lifetime.clone(),
        };
        let hwnd = CreateWindowExW(
            0x0800_0000, // WS_EX_NOACTIVATE: never activate the fixture.
            class.as_ptr(),
            class.as_ptr(),
            0x8000_0000, // WS_POPUP: client dimensions equal fixture dimensions.
            40,
            40,
            160,
            120,
            null_mut(),
            null_mut(),
            instance,
            std::ptr::from_ref(&marker).cast_mut().cast(),
        );
        if hwnd.is_null() {
            let _ = UnregisterClassW(class.as_ptr(), instance);
            panic!("owned paint HWND creation failed");
        }
        let fixture = Fixture {
            hwnd,
            class,
            instance,
            lifetime,
        };
        assert_eq!(fixture.lifetime.get(), hwnd);
        let source = Image::new(160, 120, [31, 97, 211, 255].repeat(160 * 120)).unwrap();
        let bitmap = create_bitmap(&source).unwrap();
        install_bitmap(hwnd, bitmap, &source);
        window_state(hwnd).editor = Some(EditorController::with_image(source).unwrap());
        ShowWindow(hwnd, 4); // SW_SHOWNOACTIVATE
        assert_ne!(InvalidateRect(hwnd, std::ptr::null(), 0), 0);
        assert_ne!(UpdateWindow(hwnd), 0);
        assert_eq!(GetUpdateRect(hwnd, null_mut(), 0), 0);
        const OLD: u32 = 0x001f_61d3;
        for (x, y) in [(20, 70), (70, 70), (100, 70), (140, 100)] {
            assert_eq!(pixel(hwnd, x, y), OLD, "baseline ({x},{y})");
        }

        // Redaction overlaps both dirty islands and their clean gap. Its new
        // pixels must never fall back to the previously painted raw source.
        editor(hwnd).set_tool(Tool::Redact);
        editor(hwnd).mouse_down(ShapePoint::new(10.0, 50.0));
        editor(hwnd).mouse_move(ShapePoint::new(120.0, 90.0));
        editor(hwnd).mouse_up(ShapePoint::new(120.0, 90.0));
        assert_eq!(editor(hwnd).shapes().len(), 1);
        let before = budget.snapshot();
        let pressure = budget
            .try_acquire(
                before.ceiling_bytes - before.used_bytes,
                AdmissionClass::Reserved,
            )
            .unwrap();
        // Positive proof of the intended failure rather than just black pixels.
        let error = composited_bitmap(hwnd, true).unwrap_err();
        assert!(error.contains("Kein freies Bildbudget"), "{error}");
        let left = Rect {
            left: 10,
            top: 40,
            right: 50,
            bottom: 95,
        };
        let right = Rect {
            left: 90,
            top: 40,
            right: 125,
            bottom: 95,
        };
        assert_ne!(InvalidateRect(hwnd, &left, 0), 0);
        assert_ne!(InvalidateRect(hwnd, &right, 0), 0);
        assert_ne!(GetUpdateRect(hwnd, null_mut(), 0), 0);
        assert_ne!(UpdateWindow(hwnd), 0);
        assert_eq!(
            GetUpdateRect(hwnd, null_mut(), 0),
            0,
            "paint must not self-invalidate"
        );
        assert_eq!(
            pixel(hwnd, 20, 70),
            0,
            "left dirty island leaked old pixels"
        );
        assert_eq!(
            pixel(hwnd, 100, 70),
            0,
            "right dirty island leaked old pixels"
        );
        assert_eq!(pixel(hwnd, 20, 45), 0, "failure fill was not opaque");
        assert_eq!(pixel(hwnd, 70, 70), OLD, "clean hole was overpainted");
        assert_eq!(pixel(hwnd, 140, 100), OLD, "clean sentinel was overpainted");
        assert_eq!(editor(hwnd).shapes().len(), 1, "failure lost annotation");

        drop(pressure);
        assert_eq!(budget.snapshot().used_bytes, before.used_bytes);
        assert_ne!(InvalidateRect(hwnd, std::ptr::null(), 0), 0);
        assert_ne!(UpdateWindow(hwnd), 0);
        assert_eq!(GetUpdateRect(hwnd, null_mut(), 0), 0);
        for x in [20, 70, 100] {
            assert_eq!(pixel(hwnd, x, 70), 0, "redaction recovery ({x},70)");
        }
        assert_eq!(pixel(hwnd, 20, 45), OLD, "ordinary pixels did not recover");
        assert_eq!(
            pixel(hwnd, 140, 100),
            OLD,
            "ordinary preview did not recover"
        );
        assert_eq!(editor(hwnd).shapes().len(), 1);
        assert_ne!(DestroyWindow(hwnd), 0);
        assert!(fixture.lifetime.get().is_null());
        assert_eq!(budget.snapshot().used_bytes, 0);
        assert!((&*std::ptr::addr_of!(EDITOR_WINDOWS)).is_empty());
        drop(fixture);
        println!(
            "REAL_BEGINPAINT_FAILURE_OK: owned HWND, disjoint clip, redaction, recovery, cleanup"
        );
    }
}
