//! Windows-only regression for the production preview paint body. This uses a
//! real top-down DIB and GDI clipping, not a visible HWND or BeginPaint. Passing
//! Err models composition failure without allocating a large image or changing
//! process-wide configuration. It does not simulate driver/OOM failure.
//! Run on Windows: cargo test --bin shotkit window_mouse::paint_tests
use super::*;

#[link(name = "gdi32")]
extern "system" {
    fn IntersectClipRect(dc: H, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn ExcludeClipRect(dc: H, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn GdiFlush() -> i32;
}

struct Surface {
    dc: H,
    bitmap: H,
    old: H,
    pixels: *mut u32,
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

// SAFETY: fixed positive dimensions, the owned DIB is selected only in its
// memory DC and remains live until Surface drops. No window/global state.
unsafe fn surface(width: i32, height: i32) -> Surface {
    let dc = CreateCompatibleDC(null_mut());
    assert!(!dc.is_null());
    let info = Bmi {
        header: BmiHeader {
            size: size_of::<BmiHeader>() as u32,
            width,
            height: -height,
            planes: 1,
            bits: 32,
            compression: 0,
            image_size: 0,
            xppm: 0,
            yppm: 0,
            used: 0,
            important: 0,
        },
        colors: [0],
    };
    let mut pixels = null_mut();
    let bitmap = CreateDIBSection(dc, &info, 0, &mut pixels, null_mut(), 0);
    assert!(!bitmap.is_null() && !pixels.is_null());
    let old = SelectObject(dc, bitmap);
    assert!(!old.is_null() && old as isize != -1);
    Surface {
        dc,
        bitmap,
        old,
        pixels: pixels.cast(),
    }
}

#[test]
fn failed_preview_erases_old_pixels_only_inside_dirty_image_clip() {
    unsafe {
        const WIDTH: i32 = 96;
        const HEIGHT: i32 = 80;
        const TOOLBAR_BOTTOM: i32 = 12;
        const OLD_SENSITIVE_PIXEL: u32 = 0x00a4_327b;
        let target = surface(WIDTH, HEIGHT);
        // The old screen content is distinctive. A text-only failure painter
        // leaves most of this marker visible and fails the assertions below.
        for index in 0..(WIDTH * HEIGHT) as usize {
            target.pixels.add(index).write(OLD_SENSITIVE_PIXEL);
        }
        let dirty = Rect {
            left: 8,
            top: 4,
            right: 88,
            bottom: 72,
        };
        assert_ne!(IntersectClipRect(target.dc, 8, 4, 88, 72), 0);
        // A hole inside rcPaint's bounding box models a disjoint update region.
        assert_ne!(ExcludeClipRect(target.dc, 40, 20, 56, 65), 0);
        assert!(paint_preview(
            target.dc,
            &dirty,
            TOOLBAR_BOTTOM,
            (0, TOOLBAR_BOTTOM, WIDTH, HEIGHT - TOOLBAR_BOTTOM),
            (WIDTH, HEIGHT - TOOLBAR_BOTTOM),
            true,
            Err(String::new()),
        ));
        assert_ne!(GdiFlush(), 0);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                // GDI does not preserve the unused fourth byte of BI_RGB.
                let pixel = target.pixels.add((y * WIDTH + x) as usize).read() & 0x00ff_ffff;
                let dirty_image = (8..88).contains(&x)
                    && (TOOLBAR_BOTTOM..72).contains(&y)
                    && !((40..56).contains(&x) && (20..65).contains(&y));
                if dirty_image {
                    assert_ne!(pixel, OLD_SENSITIVE_PIXEL, "old content at ({x},{y})");
                } else {
                    assert_eq!(pixel, OLD_SENSITIVE_PIXEL, "clip changed at ({x},{y})");
                }
            }
        }
        // Below the optional message, the failed preview is opaque black.
        assert_eq!(
            target.pixels.add((68 * WIDTH + 80) as usize).read() & 0x00ff_ffff,
            0
        );
    }
}

#[test]
fn failed_neutral_fill_reports_failure_instead_of_success() {
    unsafe {
        let dirty = Rect {
            left: 0,
            top: 0,
            right: 32,
            bottom: 32,
        };
        // An invalid DC deterministically fails both PatBlt and FillRect. This
        // verifies the bounded failure return, not real GDI exhaustion.
        assert!(!paint_preview(
            null_mut(),
            &dirty,
            0,
            (0, 0, 32, 32),
            (32, 32),
            false,
            Err(String::new()),
        ));
    }
}
