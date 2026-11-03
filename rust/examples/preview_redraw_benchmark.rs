#[cfg(windows)]
mod benchmark {
    //! Reproducible redraw comparison; timings are evidence, not a pass/fail threshold.
    use shotkit::{
        alpha_composite::{flatten_checkerboard, source_over},
        alpha_document::{render_document, render_preview},
        alpha_render,
        shapes::{Point, Shape, ShapeKind},
        stitch::Image,
    };
    use std::{sync::Arc, time::Instant};

    fn baseline_flatten(image: &Image) -> Image {
        let mut pixels = Vec::with_capacity(image.pixels.len());
        for (i, src) in image.pixels.chunks_exact(4).enumerate() {
            let gray = if (((i % image.width) / 12) + ((i / image.width) / 12)) % 2 == 0 {
                236
            } else {
                196
            };
            pixels.extend_from_slice(&source_over(
                [gray, gray, gray, 255],
                [src[0], src[1], src[2], src[3]],
                255,
            ));
        }
        Image::new(image.width, image.height, pixels).unwrap()
    }

    fn compare(width: usize, height: usize) {
        let mut pixels = vec![0; width * height * 4];
        for (i, p) in pixels.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 251) as u8, ((i / width) % 251) as u8, 120, 255]);
        }
        let image = Arc::new(Image::new(width, height, pixels).unwrap());
        let mut shapes = vec![
            Shape::new(ShapeKind::Rectangle {
                start: Point::new(30.0, 30.0),
                end: Point::new(300.0, 200.0),
            }),
            Shape::new(ShapeKind::Ellipse {
                start: Point::new(350.0, 100.0),
                end: Point::new(620.0, 350.0),
            }),
            Shape::new(ShapeKind::Arrow {
                start: Point::new(50.0, 400.0),
                end: Point::new(900.0, 650.0),
            }),
            Shape::new(ShapeKind::Line {
                start: Point::new(20.0, 25.0),
                end: Point::new(700.0, 650.0),
            }),
        ];
        let mut cache = None;
        render_preview(&image, &shapes, 3, &mut cache).unwrap(); // warm the stable prefix
        let iterations = 3;
        let mut old = std::time::Duration::ZERO;
        let mut new = std::time::Duration::ZERO;
        for i in 0..iterations {
            shapes[3].offset(1.0 + i as f32, 0.0);
            let started = Instant::now();
            let original = baseline_flatten(&render_document(&image, &shapes).unwrap());
            old += started.elapsed();
            let started = Instant::now();
            let optimized =
                flatten_checkerboard(&render_preview(&image, &shapes, 3, &mut cache).unwrap())
                    .unwrap();
            new += started.elapsed();
            assert_eq!(original, optimized, "preview pixels must remain identical");
        }
        println!("PREVIEW_BENCHMARK {width}x{height} 4_shapes 3_frames baseline_ms={:.2} cached_ms={:.2}",
        old.as_secs_f64() * 1000.0 / f64::from(iterations), new.as_secs_f64() * 1000.0 / f64::from(iterations));
    }

    fn compare_freehand(width: usize, height: usize) {
        let pixels = (0..width * height)
            .flat_map(|i| [(i % 251) as u8, ((i / width) % 251) as u8, 120, 255])
            .collect();
        let image = Arc::new(Image::new(width, height, pixels).unwrap());
        let points = (0..600)
            .map(|i| Point::new(30.0 + i as f32, 200.0 + ((i as f32) * 0.06).sin() * 100.0))
            .collect();
        let mut shapes = vec![Shape::new(ShapeKind::Freehand { points })];
        let mut cache = None;
        render_preview(&image, &shapes, 0, &mut cache).unwrap();
        let iterations = 3;
        let mut old = std::time::Duration::ZERO;
        let mut new = std::time::Duration::ZERO;
        for i in 0..iterations {
            shapes[0].offset(1.0 + i as f32, 0.0);
            let started = Instant::now();
            let layer = alpha_render::render_shape(width, height, &shapes[0]).unwrap();
            let mut original = (*image).clone();
            for (dst, src) in original
                .pixels
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(layer.pixels.as_chunks::<4>().0.iter())
            {
                let result = source_over(*dst, *src, 255);
                dst.copy_from_slice(&result);
            }
            let original = flatten_checkerboard(&original).unwrap();
            old += started.elapsed();
            let started = Instant::now();
            let optimized =
                flatten_checkerboard(&render_preview(&image, &shapes, 0, &mut cache).unwrap())
                    .unwrap();
            new += started.elapsed();
            assert!(
                original == optimized,
                "freehand preview pixels must remain identical"
            );
        }
        println!("FREEHAND_BENCHMARK {width}x{height} 600_points 3_frames full_layer_ms={:.2} bounded_layer_ms={:.2}",
            old.as_secs_f64() * 1000.0 / f64::from(iterations), new.as_secs_f64() * 1000.0 / f64::from(iterations));
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetDC(window: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn ReleaseDC(window: *mut std::ffi::c_void, dc: *mut std::ffi::c_void) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateCompatibleDC(dc: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CreateCompatibleBitmap(
            dc: *mut std::ffi::c_void,
            width: i32,
            height: i32,
        ) -> *mut std::ffi::c_void;
        fn SelectObject(
            dc: *mut std::ffi::c_void,
            object: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn DeleteObject(object: *mut std::ffi::c_void) -> i32;
        fn DeleteDC(dc: *mut std::ffi::c_void) -> i32;
        fn SetStretchBltMode(dc: *mut std::ffi::c_void, mode: i32) -> i32;
        fn SetBrushOrgEx(
            dc: *mut std::ffi::c_void,
            x: i32,
            y: i32,
            old: *mut std::ffi::c_void,
        ) -> i32;
        fn PatBlt(
            dc: *mut std::ffi::c_void,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            rop: u32,
        ) -> i32;
        fn StretchBlt(
            dst: *mut std::ffi::c_void,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            src: *mut std::ffi::c_void,
            sx: i32,
            sy: i32,
            sw: i32,
            sh: i32,
            rop: u32,
        ) -> i32;
    }

    // SAFETY CONTRACT: The screen and memory DCs and bitmaps are created for
    // positive fixed dimensions, selected only locally, then restored and freed.
    // GDI calls do not explicitly dispatch window messages.
    unsafe fn compare_scaling(width: i32, height: i32, target: (i32, i32)) {
        use std::ptr::null_mut;
        let screen = GetDC(null_mut());
        assert!(!screen.is_null());
        let src = CreateCompatibleDC(screen);
        let dst = CreateCompatibleDC(screen);
        assert!(!src.is_null() && !dst.is_null());
        let source_bitmap = CreateCompatibleBitmap(screen, width, height);
        let target_bitmap = CreateCompatibleBitmap(screen, target.0, target.1);
        assert!(!source_bitmap.is_null() && !target_bitmap.is_null());
        let old_src = SelectObject(src, source_bitmap);
        let old_dst = SelectObject(dst, target_bitmap);
        assert!(!old_src.is_null() && !old_dst.is_null());
        ReleaseDC(null_mut(), screen);
        assert_ne!(PatBlt(src, 0, 0, width, height, 0x00ff_0062), 0); // WHITENESS
        assert_ne!(PatBlt(src, 0, 0, width / 2, height, 0x0000_0042), 0); // BLACKNESS
        let mut times = [std::time::Duration::ZERO; 2];
        for (index, mode) in [3, 4].into_iter().enumerate() {
            assert_ne!(SetStretchBltMode(dst, mode), 0);
            if mode == 4 {
                assert_ne!(SetBrushOrgEx(dst, 0, 0, null_mut()), 0);
            }
            for _ in 0..3 {
                let start = Instant::now();
                assert_ne!(
                    StretchBlt(
                        dst,
                        0,
                        0,
                        target.0,
                        target.1,
                        src,
                        0,
                        0,
                        width,
                        height,
                        0x00cc_0020
                    ),
                    0
                );
                times[index] += start.elapsed();
            }
        }
        SelectObject(src, old_src);
        SelectObject(dst, old_dst);
        DeleteObject(source_bitmap);
        DeleteObject(target_bitmap);
        DeleteDC(src);
        DeleteDC(dst);
        println!(
            "SCALING_BENCHMARK {width}x{height} to {}x{} 3_frames basic_ms={:.2} halftone_ms={:.2}",
            target.0,
            target.1,
            times[0].as_secs_f64() * 1000.0 / 3.0,
            times[1].as_secs_f64() * 1000.0 / 3.0
        );
    }

    pub fn run() {
        compare(1920, 1080);
        compare(1200, 10_000);
        compare_freehand(1920, 1080);
        compare_freehand(1200, 10_000);
        unsafe {
            compare_scaling(1920, 1080, (960, 540));
            compare_scaling(1200, 10_000, (120, 1000));
        }
    }
}
#[cfg(windows)]
fn main() {
    benchmark::run();
}
#[cfg(not(windows))]
fn main() {}
