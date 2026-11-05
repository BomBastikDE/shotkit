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
