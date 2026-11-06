use super::*;
#[test]
fn stroke_style_defaults_and_document_history_preserve_all_options() {
    let default = Style::default();
    assert_eq!(default.line_style, LineStyle::Solid);
    assert_eq!(default.arrow_ends, ArrowEnds::Single);
    assert_eq!(default.rectangle_corners, RectangleCorners::Square);
    let mut doc = ShapeDocument::default();
    doc.push(Shape::new(ShapeKind::Arrow {
        start: Point::new(16.0, 32.0),
        end: Point::new(96.0, 32.0),
    }));
    doc.edit(0, |shape| {
        shape.style.line_style = LineStyle::Dotted;
        shape.style.arrow_ends = ArrowEnds::Double;
        shape.style.rectangle_corners = RectangleCorners::Rounded;
    });
    let changed = doc.shapes()[0].clone();
    assert!(doc.undo());
    assert_eq!(doc.shapes()[0].style, default);
    assert!(doc.redo());
    assert_eq!(doc.shapes()[0], changed);
}

#[test]
fn alternative_arrow_ends_have_selectable_geometry_and_matching_bounds() {
    let mut shape = Shape::new(ShapeKind::Arrow {
        start: Point::new(16.0, 32.0),
        end: Point::new(96.0, 32.0),
    });
    assert!(!shape.hit_test(Point::new(28.0, 28.0), 0.0));
    shape.style.arrow_ends = ArrowEnds::Double;
    assert!(shape.hit_test(Point::new(28.0, 28.0), 0.0));
    assert!(!shape.hit_test(Point::new(14.0, 32.0), 0.0));
    let bounds = shape.bounds();
    assert!(bounds.y < 28.0 && bounds.y + bounds.height > 36.0);
    assert_eq!(
        shape.handles(),
        vec![Point::new(16.0, 32.0), Point::new(96.0, 32.0)]
    );
    shape.style.arrow_ends = ArrowEnds::Dot;
    assert!(shape.hit_test(Point::new(100.0, 32.0), 0.0));
    assert!(!shape.hit_test(Point::new(103.0, 32.0), 0.0));
    let bounds = shape.bounds();
    assert_eq!(bounds.x + bounds.width, 102.0);
    shape.offset(10.0, 5.0);
    assert!(shape.hit_test(Point::new(110.0, 37.0), 0.0));
    assert!(shape.set_handle(1, Point::new(150.0, 37.0)));
    assert!(shape.hit_test(Point::new(154.0, 37.0), 0.0));
    for kind in [
        ShapeKind::Arrow {
            start: Point::new(1.0, 1.0),
            end: Point::new(1.0, 1.0),
        },
        ShapeKind::Arrow {
            start: Point::new(f32::NAN, 1.0),
            end: Point::new(100.0, 1.0),
        },
    ] {
        shape.kind = kind;
        assert_eq!(shape.bounds(), Rect::default());
        assert!(!shape.hit_test(Point::new(10.0, 10.0), 0.0));
    }
}

#[test]
fn rounded_rectangle_geometry_limits_radius_and_excludes_empty_corners() {
    let mut shape = Shape::new(ShapeKind::Rectangle {
        start: Point::new(72.0, 40.0),
        end: Point::new(8.0, 8.0),
    });
    assert!(shape.hit_test(Point::new(9.0, 9.0), 0.0));
    shape.style.rectangle_corners = RectangleCorners::Rounded;
    assert!(!shape.hit_test(Point::new(9.0, 9.0), 0.0));
    assert!(shape.hit_test(Point::new(40.0, 8.0), 0.0));
    assert!(!shape.hit_test(Point::new(40.0, 24.0), 0.0));
    shape.style.fill = FillMode::Solid;
    assert!(shape.hit_test(Point::new(40.0, 24.0), 0.0));
    assert!(!shape.hit_test(Point::new(f32::INFINITY, 24.0), 0.0));
    assert_eq!(
        shape.bounds(),
        Rect::from_points(Point::new(8.0, 8.0), Point::new(72.0, 40.0))
    );
    for (start, end) in [
        (Point::new(0.0, 0.0), Point::new(2.0, 4.0)),
        (Point::new(-1.0e20, -1.0e20), Point::new(1.0e20, 1.0e20)),
    ] {
        let points = rounded_rectangle_points(start, end).unwrap();
        assert_eq!(points[0], points[36]);
        assert!(points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        assert!(points
            .iter()
            .all(|p| p.x >= start.x && p.x <= end.x && p.y >= start.y && p.y <= end.y));
    }
    for (start, end) in [
        (Point::new(f32::NAN, 8.0), Point::new(40.0, 40.0)),
        (Point::new(8.0, 8.0), Point::new(f32::INFINITY, 40.0)),
        (Point::new(-f32::MAX, 8.0), Point::new(f32::MAX, 40.0)),
        (Point::new(8.0, -f32::MAX), Point::new(8.0, f32::MAX)),
    ] {
        assert!(rounded_rectangle_points(start, end).is_none());
        shape.kind = ShapeKind::Rectangle { start, end };
        assert_eq!(shape.bounds(), Rect::default());
        assert!(!shape.hit_test(Point::new(8.0, 8.0), 0.0));
    }
}

#[test]
fn box_fill_defaults_to_outline_and_selects_only_the_painted_interior() {
    assert_eq!(Style::default().fill, FillMode::None);
    assert_eq!(FillMode::None.alpha_scale(), None);
    assert_eq!(FillMode::Solid.alpha_scale(), Some(255));
    assert_eq!(FillMode::Translucent.alpha_scale(), Some(128));
    for kind in [
        ShapeKind::Rectangle {
            start: Point::new(40.0, 40.0),
            end: Point::new(8.0, 8.0),
        },
        ShapeKind::Ellipse {
            start: Point::new(40.0, 40.0),
            end: Point::new(8.0, 8.0),
        },
    ] {
        let mut shape = Shape::new(kind);
        let center = Point::new(24.0, 24.0);
        assert!(!shape.hit_test(center, 0.0), "default remains an outline");
        for fill in [FillMode::Solid, FillMode::Translucent] {
            shape.style.fill = fill;
            assert!(shape.hit_test(center, 0.0));
            assert!(!shape.hit_test(Point::new(0.0, 24.0), 0.0));
            assert!(!shape.hit_test(Point::new(f32::NAN, 24.0), 0.0));
            if matches!(shape.kind, ShapeKind::Ellipse { .. }) {
                assert!(!shape.hit_test(Point::new(10.0, 10.0), 0.0));
            }
        }
        shape.style.color.a = 0;
        assert!(
            !shape.hit_test(center, 0.0),
            "invisible fill has no interior hit"
        );
    }
}

#[test]
fn fill_follows_shape_geometry_and_document_history() {
    let mut doc = ShapeDocument::default();
    doc.push(Shape::new(ShapeKind::Rectangle {
        start: Point::new(8.0, 8.0),
        end: Point::new(40.0, 40.0),
    }));
    assert!(doc.edit(0, |shape| shape.style.fill = FillMode::Translucent));
    assert_eq!(doc.shapes()[0].style.fill, FillMode::Translucent);
    assert!(doc.undo());
    assert_eq!(doc.shapes()[0].style.fill, FillMode::None);
    assert!(doc.redo());
    assert_eq!(doc.shapes()[0].style.fill, FillMode::Translucent);
    assert!(doc.edit(0, |shape| {
        shape.offset(50.0, 10.0);
        assert!(shape.set_handle(2, Point::new(100.0, 60.0)));
    }));
    assert_eq!(doc.shapes()[0].style.fill, FillMode::Translucent);
    assert!(doc.shapes()[0].hit_test(Point::new(80.0, 40.0), 0.0));
    assert!(!doc.shapes()[0].hit_test(Point::new(24.0, 24.0), 0.0));
}

#[test]
fn stamp_hit_test_tracks_ink_and_offsets_and_preserves_aspect_ratio() {
    let mut shape = Shape::new(ShapeKind::Stamp {
        center: Point::new(100.0, 100.0),
        kind: StampKind::Check,
    });
    shape.style.font_size = 100.0;
    assert!(shape.hit_test(Point::new(90.0, 126.0), 0.0));
    assert!(!shape.hit_test(Point::new(55.0, 55.0), 0.0));
    assert!(!shape.hit_test(Point::new(f32::NAN, 100.0), 0.0));
    shape.offset(20.0, -10.0);
    assert!(shape.hit_test(Point::new(110.0, 116.0), 0.0));
    assert!(shape.set_handle(2, Point::new(200.0, 100.0)));
    assert_eq!(shape.bounds().width, 160.0);
    assert_eq!(shape.bounds().width, shape.bounds().height);
    assert_eq!(shape.bounds().x + shape.bounds().width / 2.0, 120.0);
    assert!(!shape.set_handle(4, Point::new(0.0, 0.0)));
}

#[test]
fn text_hitbox_covers_rendered_label_approximately() {
    let mut shape = Shape::new(ShapeKind::Text {
        location: Point::new(0.0, 0.0),
        text: "Testtext".into(),
    });
    shape.style.font_size = 28.0;
    assert!(shape.hit_test(Point::new(60.0, 12.0), 0.0));
    assert!(!shape.hit_test(Point::new(400.0, 12.0), 0.0));
}
#[test]
fn history_has_a_finite_limit() {
    let mut doc = ShapeDocument::default();
    for i in 0..110 {
        doc.push(Shape::new(ShapeKind::Step {
            center: Point::new(i as f32, 0.0),
            number: i,
        }));
    }
    let mut n = 0;
    while doc.undo() {
        n += 1;
    }
    assert_eq!(n, 100);
    assert_eq!(doc.shapes().len(), 10);
}
#[test]
fn normalizes_and_resizes_box() {
    let mut s = Shape::new(ShapeKind::Rectangle {
        start: Point::new(8.0, 7.0),
        end: Point::new(2.0, 1.0),
    });
    assert_eq!(
        s.bounds(),
        Rect {
            x: 2.0,
            y: 1.0,
            width: 6.0,
            height: 6.0
        }
    );
    assert!(s.set_handle(0, Point::new(10.0, 11.0)));
    assert_eq!(
        s.bounds(),
        Rect {
            x: 8.0,
            y: 7.0,
            width: 2.0,
            height: 4.0
        }
    );
}
#[test]
fn offsets_freehand_and_tests_segment() {
    let mut s = Shape::new(ShapeKind::Freehand {
        points: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)],
    });
    assert!(s.hit_test(Point::new(5.0, 1.0), 1.0));
    s.offset(2.0, 3.0);
    assert_eq!(
        s.bounds(),
        Rect {
            x: 2.0,
            y: 3.0,
            width: 10.0,
            height: 0.0
        }
    );
}
#[test]
fn undo_redo_restores_shape_edits() {
    let mut d = ShapeDocument::default();
    d.push(Shape::new(ShapeKind::Step {
        center: Point::new(1.0, 2.0),
        number: 1,
    }));
    d.edit(0, |s| s.offset(4.0, 0.0));
    assert!((d.shapes()[0].bounds().x + 13.8).abs() < 0.0001);
    assert!(d.undo());
    assert!((d.shapes()[0].bounds().x + 17.8).abs() < 0.0001);
    assert!(d.redo());
    assert!((d.shapes()[0].bounds().x + 13.8).abs() < 0.0001);
}
#[test]
fn empty_freehand_bounds_are_safe() {
    let s = Shape::new(ShapeKind::Freehand { points: vec![] });
    assert!(s.is_empty());
    assert_eq!(s.bounds(), Rect::default());
}
#[test]
fn oval_hit_test_uses_pixel_distance_to_contour() {
    let s = Shape::new(ShapeKind::Ellipse {
        start: Point::new(-100.0, -10.0),
        end: Point::new(100.0, 10.0),
    });
    assert!(s.hit_test(Point::new(99.0, 0.0), 0.0));
    assert!(!s.hit_test(Point::new(0.0, 0.0), 0.0));
}
#[test]
fn degenerate_ellipse_is_a_segment_or_point() {
    let s = Shape::new(ShapeKind::Ellipse {
        start: Point::new(0.0, 0.0),
        end: Point::new(0.0, 20.0),
    });
    assert!(s.hit_test(Point::new(1.0, 10.0), 0.0));
    assert!(!s.hit_test(Point::new(5.0, 10.0), 0.0));
}
#[test]
fn rectangle_contains_excludes_right_and_bottom_edges() {
    let r = Rect {
        x: 1.0,
        y: 2.0,
        width: 3.0,
        height: 4.0,
    };
    assert!(r.contains(Point::new(1.0, 2.0)));
    assert!(r.contains(Point::new(3.999, 5.999)));
    assert!(!r.contains(Point::new(4.0, 3.0)));
    assert!(!r.contains(Point::new(2.0, 6.0)));
}

#[test]
fn rounded_rectangle_zero_axis_resize_keeps_bounds_handles_and_hit_target() {
    for end in [
        Point::new(8.0, 40.0),
        Point::new(40.0, 8.0),
        Point::new(8.0, 8.0),
    ] {
        let mut doc = ShapeDocument::default();
        let mut shape = Shape::new(ShapeKind::Rectangle {
            start: Point::new(8.0, 8.0),
            end: Point::new(40.0, 40.0),
        });
        shape.style.rectangle_corners = RectangleCorners::Rounded;
        doc.push(shape);
        assert!(doc.edit(0, |shape| assert!(shape.set_handle(2, end))));
        let collapsed = doc.shapes()[0].clone();
        let mut square = collapsed.clone();
        square.style.rectangle_corners = RectangleCorners::Square;
        assert_eq!(collapsed.bounds(), square.bounds());
        assert_ne!(collapsed.bounds(), Rect::default());
        assert_eq!(collapsed.handles(), square.handles());
        let midpoint = Point::new((8.0 + end.x) / 2.0, (8.0 + end.y) / 2.0);
        assert!(
            collapsed.hit_test(midpoint, 0.0),
            "can be selected again without a retained selection"
        );
        assert_eq!(
            collapsed.hit_test(midpoint, 0.0),
            square.hit_test(midpoint, 0.0)
        );
        assert!(!collapsed.hit_test(Point::new(60.0, 60.0), 0.0));
        assert!(rounded_rectangle_points(Point::new(8.0, 8.0), end).is_none());
        assert!(doc.undo());
        assert_eq!(doc.shapes()[0].bounds().width, 32.0);
        assert_eq!(doc.shapes()[0].bounds().height, 32.0);
        assert!(doc.redo());
        assert_eq!(doc.shapes()[0], collapsed);
        assert!(doc.shapes()[0].hit_test(midpoint, 0.0));
    }
    assert!(!rectangle_has_zero_axis(
        Point::new(f32::INFINITY, 8.0),
        Point::new(f32::INFINITY, 40.0)
    ));
}
