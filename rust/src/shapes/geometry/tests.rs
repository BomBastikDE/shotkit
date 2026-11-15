use super::*;

fn style(width: f32, arrow_ends: ArrowEnds) -> Style {
    Style {
        width,
        arrow_ends,
        ..Style::default()
    }
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-3
}

#[test]
fn stroke_width_is_clamped_and_nan_falls_back_to_one() {
    assert_eq!(styled_stroke_width(4.0), 4.0);
    assert_eq!(styled_stroke_width(0.0), 1.0);
    assert_eq!(styled_stroke_width(-5.0), 1.0);
    assert_eq!(styled_stroke_width(1000.0), 256.0);
    assert_eq!(styled_stroke_width(f32::INFINITY), 256.0);
    assert_eq!(styled_stroke_width(f32::NAN), 1.0);
}

#[test]
fn arrow_dot_radius_has_a_minimum_of_three() {
    assert_eq!(arrow_dot_radius(1.0), 3.0);
    assert_eq!(arrow_dot_radius(4.0), 6.0);
}

#[test]
fn arrow_head_on_a_long_horizontal_arrow_leaves_room_for_the_shaft() {
    let head = styled_arrow_head(Point::new(0.0, 0.0), Point::new(100.0, 0.0), 4.0).unwrap();
    // Head length 16 (4 × width), half base 0.45 × 16, shaft ends at 0.8 × 16.
    assert_eq!(head.tip, Point::new(100.0, 0.0));
    assert!(close(head.left.x, 84.0) && close(head.left.y, 7.2));
    assert!(close(head.right.x, 84.0) && close(head.right.y, -7.2));
    let shaft_end = head.shaft_end.unwrap();
    assert!(close(shaft_end.x, 87.2) && close(shaft_end.y, 0.0));
}

#[test]
fn arrow_head_on_a_short_arrow_starts_at_the_origin_without_shaft() {
    let head = styled_arrow_head(Point::new(0.0, 0.0), Point::new(5.0, 0.0), 4.0).unwrap();
    assert!(head.shaft_end.is_none());
    // The base sits on the start and is at least as wide as the stroke.
    assert!(close(head.left.x, 0.0) && close(head.left.y, 2.0));
    assert!(close(head.right.x, 0.0) && close(head.right.y, -2.0));
}

#[test]
fn arrow_head_rejects_tiny_and_non_finite_arrows() {
    let origin = Point::new(0.0, 0.0);
    assert!(styled_arrow_head(origin, Point::new(0.5, 0.5), 4.0).is_none());
    assert!(styled_arrow_head(origin, Point::new(f32::NAN, 10.0), 4.0).is_none());
    assert!(styled_arrow_head(origin, Point::new(f32::INFINITY, 0.0), 4.0).is_none());
}

#[test]
fn arrow_bounds_cover_the_single_head() {
    let bounds = styled_arrow_bounds(
        Point::new(0.0, 0.0),
        Point::new(100.0, 0.0),
        &style(4.0, ArrowEnds::Single),
    )
    .unwrap();
    assert!(close(bounds.x, 0.0) && close(bounds.width, 100.0));
    assert!(close(bounds.y, -7.2) && close(bounds.height, 14.4));
}

#[test]
fn arrow_bounds_cover_both_heads_of_a_double_arrow() {
    let bounds = styled_arrow_bounds(
        Point::new(0.0, 0.0),
        Point::new(100.0, 0.0),
        &style(4.0, ArrowEnds::Double),
    )
    .unwrap();
    assert!(close(bounds.x, 0.0) && close(bounds.width, 100.0));
    assert!(close(bounds.y, -7.2) && close(bounds.height, 14.4));
}

#[test]
fn arrow_bounds_cover_the_full_dot_beyond_the_end() {
    let bounds = styled_arrow_bounds(
        Point::new(0.0, 0.0),
        Point::new(100.0, 0.0),
        &style(4.0, ArrowEnds::Dot),
    )
    .unwrap();
    // Dot radius 6 extends past the endpoint in every direction.
    assert!(close(bounds.x, 0.0) && close(bounds.width, 106.0));
    assert!(close(bounds.y, -6.0) && close(bounds.height, 12.0));
}

#[test]
fn arrow_bounds_reject_an_arrow_shorter_than_one_pixel() {
    let p = Point::new(3.0, 3.0);
    assert!(styled_arrow_bounds(p, p, &Style::default()).is_none());
}

#[test]
fn double_arrow_hit_test_covers_shaft_and_both_heads() {
    let (start, end) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0));
    let s = style(4.0, ArrowEnds::Double);
    assert!(styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(50.0, 1.9),
        0.0
    ));
    assert!(
        styled_arrow_hit_test(start, end, &s, Point::new(86.0, 6.0), 0.0),
        "end head"
    );
    assert!(
        styled_arrow_hit_test(start, end, &s, Point::new(14.0, 6.0), 0.0),
        "start head"
    );
    assert!(!styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(50.0, 5.0),
        0.0
    ));
    assert!(styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(50.0, 5.0),
        3.5
    ));
}

#[test]
fn dot_arrow_hit_test_covers_dot_and_shaft_but_not_beyond() {
    let (start, end) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0));
    let s = style(4.0, ArrowEnds::Dot);
    assert!(styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(100.0, 5.9),
        0.0
    ));
    assert!(styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(50.0, 1.9),
        0.0
    ));
    assert!(!styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(107.0, 0.0),
        0.0
    ));
    assert!(styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(107.0, 0.0),
        1.5
    ));
    // No arrowhead triangle near the start of a dot arrow.
    assert!(!styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(14.0, 6.0),
        0.0
    ));
}

#[test]
fn arrow_hit_test_rejects_invalid_points_and_tolerances() {
    let (start, end) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0));
    let s = style(4.0, ArrowEnds::Double);
    let on_shaft = Point::new(50.0, 0.0);
    assert!(!styled_arrow_hit_test(
        start,
        end,
        &s,
        Point::new(f32::NAN, 0.0),
        1.0
    ));
    assert!(!styled_arrow_hit_test(start, end, &s, on_shaft, -1.0));
    assert!(!styled_arrow_hit_test(start, end, &s, on_shaft, f32::NAN));
    assert!(
        !styled_arrow_hit_test(start, start, &s, start, 10.0),
        "zero length"
    );
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn rounded_rectangle_radius_is_twelve_or_half_the_short_side() {
    assert_eq!(
        rounded_rectangle_radius(rect(0.0, 0.0, 100.0, 100.0)),
        Some(12.0)
    );
    assert_eq!(
        rounded_rectangle_radius(rect(0.0, 0.0, 10.0, 40.0)),
        Some(5.0)
    );
}

#[test]
fn rounded_rectangle_radius_rejects_empty_and_non_finite_bounds() {
    assert_eq!(rounded_rectangle_radius(rect(0.0, 0.0, 0.0, 10.0)), None);
    assert_eq!(rounded_rectangle_radius(rect(0.0, 0.0, 10.0, -1.0)), None);
    assert_eq!(
        rounded_rectangle_radius(rect(f32::NAN, 0.0, 10.0, 10.0)),
        None
    );
    assert_eq!(
        rounded_rectangle_radius(rect(f32::MAX, 0.0, f32::MAX, 10.0)),
        None,
        "right edge overflows f32"
    );
}

#[test]
fn rounded_rectangle_distance_is_signed() {
    let bounds = rect(0.0, 0.0, 100.0, 100.0);
    let d = |x, y| rounded_rectangle_distance(Point::new(x, y), bounds).unwrap();
    assert!((d(50.0, 50.0) + 50.0).abs() < 1e-9, "center is inside");
    assert!(d(0.0, 50.0).abs() < 1e-9, "on the edge");
    assert!((d(-10.0, 50.0) - 10.0).abs() < 1e-9, "outside");
    // The sharp corner lies outside the rounded one: √(12² + 12²) − 12.
    assert!((d(0.0, 0.0) - (288.0_f64.sqrt() - 12.0)).abs() < 1e-9);
}

#[test]
fn rounded_rectangle_distance_rejects_non_finite_points() {
    let bounds = rect(0.0, 0.0, 100.0, 100.0);
    assert_eq!(
        rounded_rectangle_distance(Point::new(f32::NAN, 0.0), bounds),
        None
    );
}

#[test]
fn segment_distance_is_perpendicular_inside_and_to_the_end_outside() {
    let (a, b) = (Point::new(0.0, 0.0), Point::new(10.0, 0.0));
    assert!(close(distance_segment(Point::new(5.0, 3.0), a, b), 3.0));
    assert!(close(distance_segment(Point::new(13.0, 4.0), a, b), 5.0));
    assert!(close(distance_segment(Point::new(-3.0, -4.0), a, b), 5.0));
}

#[test]
fn segment_distance_of_a_degenerate_segment_is_point_distance() {
    let a = Point::new(1.0, 1.0);
    assert!(close(distance_segment(Point::new(4.0, 5.0), a, a), 5.0));
}

#[test]
fn ellipse_distance_for_a_circle() {
    let c = Point::new(0.0, 0.0);
    assert!(close(
        distance_to_ellipse(Point::new(0.0, 0.0), c, 10.0, 10.0),
        10.0
    ));
    assert!(close(
        distance_to_ellipse(Point::new(20.0, 0.0), c, 10.0, 10.0),
        10.0
    ));
    assert!(distance_to_ellipse(Point::new(10.0, 0.0), c, 10.0, 10.0) < 1e-3);
    let diagonal = 10.0 / 2.0_f32.sqrt();
    assert!(distance_to_ellipse(Point::new(diagonal, -diagonal), c, 10.0, 10.0) < 1e-3);
}

#[test]
fn ellipse_distance_uses_the_nearest_axis() {
    let c = Point::new(100.0, 100.0);
    assert!(
        close(distance_to_ellipse(c, c, 20.0, 10.0), 10.0),
        "center → minor axis"
    );
    assert!(close(
        distance_to_ellipse(Point::new(100.0, 115.0), c, 20.0, 10.0),
        5.0
    ));
    assert!(close(
        distance_to_ellipse(Point::new(125.0, 100.0), c, 20.0, 10.0),
        5.0
    ));
}

#[test]
fn ellipse_distance_with_negative_radii_matches_positive() {
    let (c, p) = (Point::new(0.0, 0.0), Point::new(7.0, 13.0));
    assert_eq!(
        distance_to_ellipse(p, c, -20.0, -10.0),
        distance_to_ellipse(p, c, 20.0, 10.0)
    );
}

#[test]
fn ellipse_distance_with_zero_radii_degenerates_to_segment_or_point() {
    let c = Point::new(0.0, 0.0);
    assert!(close(
        distance_to_ellipse(Point::new(3.0, 0.0), c, 0.0, 5.0),
        3.0
    ));
    assert!(close(
        distance_to_ellipse(Point::new(0.0, 7.0), c, 0.0, 5.0),
        2.0
    ));
    assert!(close(
        distance_to_ellipse(Point::new(0.0, 4.0), c, 5.0, 0.0),
        4.0
    ));
    assert!(close(
        distance_to_ellipse(Point::new(3.0, 4.0), c, 0.0, 0.0),
        5.0
    ));
}

#[test]
fn ellipse_distance_of_non_finite_input_is_nan() {
    let c = Point::new(0.0, 0.0);
    assert!(distance_to_ellipse(Point::new(f32::NAN, 0.0), c, 5.0, 5.0).is_nan());
    assert!(distance_to_ellipse(Point::new(1.0, 0.0), c, f32::INFINITY, 5.0).is_nan());
}
