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

    pub fn run() {
        compare(1920, 1080);
        compare(1200, 10_000);
        compare_freehand(1920, 1080);
        compare_freehand(1200, 10_000);
    }
}
#[cfg(windows)]
fn main() {
    benchmark::run();
}
#[cfg(not(windows))]
fn main() {}
