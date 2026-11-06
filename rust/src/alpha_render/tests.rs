use super::*;
use crate::shapes::{Point, ShapeKind};

#[test]
fn arrow_geometry_stops_before_head_and_rejects_short_or_non_finite_arrows() {
    let long = arrow_geometry(Point::new(4.0, 8.0), Point::new(104.0, 8.0), 4.0)
        .expect("finite long arrow");
    let shaft_end = long.shaft_end.expect("shaft fits inside the arrowhead");
    assert!(shaft_end.x < long.tip.x);
    assert!(shaft_end.x > 104.0 - 16.0);
    assert_eq!(long.tip, PointF { x: 104.0, y: 8.0 });

    let short_thick = arrow_geometry(Point::new(4.0, 8.0), Point::new(10.0, 8.0), 12.0)
        .expect("finite short arrow keeps its head");
    assert!(short_thick.shaft_end.is_none());
    assert_eq!((short_thick.left.x + short_thick.right.x) * 0.5, 4.0);
    assert!((short_thick.left.y - short_thick.right.y).abs() >= 12.0);
    assert!(arrow_geometry(Point::new(4.0, 8.0), Point::new(4.5, 8.0), 4.0).is_none());
    assert!(arrow_geometry(Point::new(4.0, 8.0), Point::new(4.0, 8.0), 4.0).is_none());
    assert!(arrow_geometry(Point::new(f32::NAN, 8.0), Point::new(10.0, 8.0), 4.0).is_none());
}

#[test]
fn blur_is_explicitly_left_to_source_aware_renderer() {
    let shape = Shape::new(ShapeKind::Blur {
        start: Point::new(1.0, 1.0),
        end: Point::new(8.0, 8.0),
    });
    assert_eq!(
        render_shape(16, 16, &shape),
        Err(RenderError::UnsupportedShape(
            "Verpixeln benötigt das Originalbild als Quelle"
        ))
    );
}

#[test]
fn rejects_zero_or_overflowing_dimensions() {
    let shape = Shape::new(ShapeKind::Line {
        start: Point::new(0.0, 0.0),
        end: Point::new(1.0, 1.0),
    });
    assert_eq!(
        render_shape(0, 1, &shape),
        Err(RenderError::InvalidDimensions)
    );
    assert_eq!(
        render_shape(usize::MAX, 2, &shape),
        Err(RenderError::InvalidDimensions)
    );
}

#[test]
fn step_labels_scale_for_multiple_digits_and_use_contrast_color() {
    assert_eq!(step_font_size(20.0, 1), 21.0);
    assert_eq!(step_font_size(20.0, 2), 18.0);
    assert!((step_font_size(20.0, 3) - 12.0).abs() < 0.001);
    assert_eq!(step_font_size(20.0, 4), 9.0);
    assert_eq!(
        step_text_color(crate::shapes::Color::rgba(0, 0, 0, 255)),
        0xffff_ffff
    );
    assert_eq!(
        step_text_color(crate::shapes::Color::rgba(255, 255, 255, 255)),
        0xff28_2828
    );
    assert_eq!(
        step_text_color(crate::shapes::Color::rgba(100, 100, 100, 20)),
        0xffff_ffff
    );
}

#[test]
fn render_sizes_keep_nan_fallback_and_clamp_extremes() {
    for maximum in [256.0, 512.0] {
        assert_eq!(bounded_size(f32::NAN, maximum), 1.0);
        assert_eq!(bounded_size(f32::NEG_INFINITY, maximum), 1.0);
        assert_eq!(bounded_size(0.0, maximum), 1.0);
        assert_eq!(bounded_size(12.5, maximum), 12.5);
        assert_eq!(bounded_size(f32::INFINITY, maximum), maximum);
    }
}

#[test]
fn gaussian_blur_requires_document_sampling_source() {
    let shape = Shape::new(ShapeKind::GaussianBlur {
        start: Point::new(1.0, 1.0),
        end: Point::new(8.0, 8.0),
    });
    assert_eq!(
        render_shape(16, 16, &shape),
        Err(RenderError::UnsupportedShape(
            "Weichzeichnen benötigt das Originalbild als Quelle"
        ))
    );
}

#[test]
fn original_stamps_render_straight_alpha_clipped_antialiased_and_scaled() {
    use crate::shapes::{Color, StampKind};
    for kind in StampKind::ALL {
        let mut shape = Shape::new(ShapeKind::Stamp {
            center: Point::new(40.0, 40.0),
            kind,
        });
        shape.style.color = Color::rgba(200, 30, 80, 128);
        shape.style.font_size = 48.0;
        let small = render_shape(80, 80, &shape).unwrap();
        let mut count = 0;
        let mut partial = false;
        for (index, pixel) in small.pixels.as_chunks::<4>().0.iter().enumerate() {
            if pixel[3] != 0 {
                count += 1;
                assert_eq!(&pixel[..3], &[80, 30, 200]);
                assert!(pixel[3] <= 128);
                partial |= pixel[3] < 128;
                let p = Point::new((index % 80) as f32, (index / 80) as f32);
                assert!(shape.bounds().contains(p));
            }
        }
        assert!(count > 30, "{kind:?} must contain ink");
        assert!(partial, "{kind:?} must have antialiased edges");
        assert_eq!(&small.pixels[..4], &[0, 0, 0, 0]);
        shape.style.font_size = 72.0;
        let large = render_shape(80, 80, &shape).unwrap();
        assert!(
            large
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 0)
                .count()
                > count
        );
        shape.offset(-40.0, -40.0);
        let clipped = render_shape(80, 80, &shape).unwrap();
        assert!(clipped.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        shape.style.color.a = 0;
        assert!(render_shape(80, 80, &shape)
            .unwrap()
            .pixels
            .iter()
            .all(|v| *v == 0));
        shape.style.color.a = 128;
        for size in [0.0, -1.0, f32::NAN, f32::INFINITY, 513.0] {
            shape.style.font_size = size;
            assert_eq!(
                render_shape(80, 80, &shape),
                Err(RenderError::InvalidDimensions)
            );
        }
    }
}

#[test]
fn styled_arrow_head_matches_existing_single_arrow_geometry() {
    for (start, end, width) in [
        (Point::new(4.0, 8.0), Point::new(104.0, 8.0), 4.0),
        (Point::new(4.0, 8.0), Point::new(10.0, 8.0), 12.0),
        (Point::new(80.0, 90.0), Point::new(11.0, 7.0), 8.0),
    ] {
        let legacy = arrow_geometry(start, end, width).unwrap();
        let styled = crate::shapes::styled_arrow_head(start, end, width).unwrap();
        let point = |p: Point| PointF { x: p.x, y: p.y };
        assert_eq!(legacy.tip, point(styled.tip));
        assert_eq!(legacy.left, point(styled.left));
        assert_eq!(legacy.right, point(styled.right));
        assert_eq!(legacy.shaft_end, styled.shaft_end.map(point));
    }
}
