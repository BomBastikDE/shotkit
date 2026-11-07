use super::*;

fn callout(tip: Option<Point>) -> CalloutData {
    CalloutData {
        origin: Point::new(20.0, 20.0),
        text: "Measured text".to_owned(),
        fill_color: Color::rgba(255, 214, 0, 255),
        text_color: Color::rgba(0, 0, 0, 255),
        tip,
        layout: CalloutTextLayout {
            font_size: 18.0,
            body_width: 100.0,
            body_height: 60.0,
            line_height: 24.0,
            line_widths: vec![80.0],
        },
    }
}

#[test]
fn callout_four_sides_share_a_closed_seamless_contour_and_precise_hits() {
    for (side, tip, empty) in [
        (0, Point::new(70.0, -20.0), Point::new(21.0, -10.0)),
        (1, Point::new(160.0, 50.0), Point::new(150.0, 21.0)),
        (2, Point::new(70.0, 120.0), Point::new(21.0, 110.0)),
        (3, Point::new(-20.0, 50.0), Point::new(-10.0, 21.0)),
    ] {
        let data = callout(Some(tip));
        assert!(data.validate());
        let geometry = data.geometry().unwrap();
        let [a, actual_tip, b] = geometry.pointer.unwrap();
        assert_eq!(actual_tip, tip);
        match side {
            0 => assert!(a.y == 20.0 && b.y == 20.0 && a.x < b.x),
            1 => assert!(a.x == 120.0 && b.x == 120.0 && a.y < b.y),
            2 => assert!(a.y == 80.0 && b.y == 80.0 && a.x > b.x),
            _ => assert!(a.x == 20.0 && b.x == 20.0 && a.y > b.y),
        }
        assert_eq!(geometry.outline.first(), geometry.outline.last());
        assert!(geometry
            .outline
            .windows(3)
            .any(|triple| triple == [a, tip, b]));
        assert!(!geometry
            .outline
            .windows(2)
            .any(|pair| pair == [a, b] || pair == [b, a]));
        let shape = Shape::new(ShapeKind::Callout {
            data: Box::new(data),
        });
        assert!(shape.hit_test(Point::new(70.0, 50.0), 0.0));
        assert!(shape.hit_test(tip, 0.0));
        assert!(shape.hit_test(
            Point::new((a.x + b.x + tip.x) / 3.0, (a.y + b.y + tip.y) / 3.0),
            0.0
        ));
        assert!(
            !shape.hit_test(empty, 0.0),
            "empty bounding-box corner on side {side}"
        );
        assert!(
            !shape.hit_test(Point::new(20.0, 20.0), 0.0),
            "rounded body corner"
        );
        let expected = Rect::from_points(
            Point::new(20.0_f32.min(tip.x), 20.0_f32.min(tip.y)),
            Point::new(120.0_f32.max(tip.x), 80.0_f32.max(tip.y)),
        );
        assert_eq!(shape.bounds(), expected);
    }
}

#[test]
fn callout_inside_tip_and_none_preserve_the_rounded_body() {
    let absent = callout(None).geometry().unwrap();
    for tip in [
        Point::new(70.0, 50.0),
        Point::new(20.0, 20.0),
        Point::new(120.0, 80.0),
    ] {
        let data = callout(Some(tip));
        let geometry = data.geometry().unwrap();
        assert!(geometry.pointer.is_none());
        assert_eq!(geometry.outline, absent.outline);
        assert_eq!(geometry.bounds, absent.body);
        assert_eq!(
            Shape::new(ShapeKind::Callout {
                data: Box::new(data)
            })
            .handles(),
            vec![tip]
        );
    }
    assert!(Shape::new(ShapeKind::Callout {
        data: Box::new(callout(None))
    })
    .handles()
    .is_empty());
}

#[test]
fn callout_corner_tips_and_small_bodies_keep_valid_exterior_bases() {
    for tip in [
        Point::new(-40.0, -40.0),
        Point::new(180.0, -40.0),
        Point::new(180.0, 140.0),
        Point::new(-40.0, 140.0),
    ] {
        for (width, height) in [(100.0, 60.0), (2.0, 1.0), (1.0, 2.0)] {
            let mut data = callout(Some(tip));
            data.text.clear();
            data.layout = CalloutTextLayout {
                font_size: 0.1,
                body_width: width,
                body_height: height,
                line_height: 0.2,
                line_widths: vec![0.0],
            };
            let geometry = data.geometry().unwrap();
            assert!(geometry.corner_radius <= width.min(height) / 4.0);
            assert!(geometry
                .outline
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite()));
            let [a, _, b] = geometry.pointer.unwrap();
            assert!(a != b);
            for p in [a, b] {
                assert!(p.x >= data.origin.x && p.x <= data.origin.x + width);
                assert!(p.y >= data.origin.y && p.y <= data.origin.y + height);
            }
            assert!(geometry.hit_test(Point::new(20.0 + width / 2.0, 20.0 + height / 2.0), 0.0));
        }
    }
}

#[test]
fn callout_offsets_and_tip_resize_are_atomic_and_preserve_measured_text() {
    let mut shape = Shape::new(ShapeKind::Callout {
        data: Box::new(callout(Some(Point::new(70.0, 120.0)))),
    });
    let before = shape.clone();
    shape.offset(10.0, -5.0);
    assert_eq!(shape.handles(), vec![Point::new(80.0, 115.0)]);
    assert_eq!(
        shape.bounds(),
        Rect {
            x: 30.0,
            y: 15.0,
            width: 100.0,
            height: 100.0
        }
    );
    assert!(shape.set_handle(0, Point::new(-10.0, 45.0)));
    assert_eq!(
        shape.bounds(),
        Rect {
            x: -10.0,
            y: 15.0,
            width: 140.0,
            height: 60.0
        }
    );
    let ShapeKind::Callout { data } = &shape.kind else {
        panic!()
    };
    let ShapeKind::Callout { data: original } = &before.kind else {
        panic!()
    };
    assert_eq!(data.layout, original.layout);
    assert_eq!(data.text, original.text);
    let valid = shape.clone();
    assert!(!shape.set_handle(1, Point::new(0.0, 0.0)));
    assert!(!shape.set_handle(0, Point::new(f32::NAN, 0.0)));
    shape.offset(f32::INFINITY, 0.0);
    shape.offset(f32::MAX, 0.0);
    assert_eq!(shape, valid);
    assert!(shape.set_handle(0, Point::new(80.0, 45.0)));
    let ShapeKind::Callout { data } = &shape.kind else {
        panic!()
    };
    assert!(data.geometry().unwrap().pointer.is_none());
}

#[test]
fn callout_invalid_layouts_coordinates_and_overflows_are_rejected() {
    let good = callout(Some(Point::new(70.0, 120.0)));
    let mut invalid = Vec::new();
    for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        for field in 0..4 {
            let mut data = good.clone();
            match field {
                0 => data.layout.font_size = value,
                1 => data.layout.body_width = value,
                2 => data.layout.body_height = value,
                _ => data.layout.line_height = value,
            }
            invalid.push(data);
        }
    }
    for widths in [
        vec![],
        vec![0.0, 0.0],
        vec![-1.0],
        vec![101.0],
        vec![f32::NAN],
        vec![f32::INFINITY],
    ] {
        let mut data = good.clone();
        data.layout.line_widths = widths;
        invalid.push(data);
    }
    for origin in [
        Point::new(f32::NAN, 20.0),
        Point::new(20.0, f32::INFINITY),
        Point::new(f32::MAX, 20.0),
    ] {
        let mut data = good.clone();
        data.origin = origin;
        invalid.push(data);
    }
    let mut data = good.clone();
    data.tip = Some(Point::new(f32::INFINITY, 0.0));
    invalid.push(data);
    let mut data = good.clone();
    data.layout.line_height = 61.0;
    invalid.push(data);
    let mut data = good.clone();
    data.origin.x = -f32::MAX;
    data.layout.body_width = f32::MAX;
    data.tip = Some(Point::new(f32::MAX, 50.0));
    invalid.push(data);
    for data in invalid {
        assert!(!data.validate(), "unexpected valid data: {data:?}");
        assert!(data.geometry().is_none());
        let shape = Shape::new(ShapeKind::Callout {
            data: Box::new(data),
        });
        assert_eq!(shape.bounds(), Rect::default());
        assert!(shape.handles().is_empty());
        assert!(shape.is_empty());
        assert!(!shape.hit_test(Point::new(70.0, 50.0), 10.0));
    }
    let shape = Shape::new(ShapeKind::Callout {
        data: Box::new(good),
    });
    for tolerance in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(!shape.hit_test(Point::new(70.0, 50.0), tolerance));
    }
    assert!(!shape.hit_test(Point::new(f32::NAN, 50.0), 0.0));
    assert!(shape.hit_test(Point::new(70.0, 18.0), 2.0));
    assert!(!shape.hit_test(Point::new(70.0, 17.0), 2.0));
}

#[test]
fn callout_multiline_layout_and_document_undo_restore_all_fields() {
    use crate::{document::Document, stitch::Image};
    let mut data = callout(Some(Point::new(70.0, 120.0)));
    data.text = "ÄÖ\n\n世界\n".into();
    data.layout.body_height = 120.0;
    data.layout.line_widths = vec![30.0, 0.0, 40.0, 0.0];
    assert!(data.validate());
    let initial = Shape::new(ShapeKind::Callout {
        data: Box::new(data),
    });
    let mut document = Document::new(Image::new(1, 1, vec![0; 4]).unwrap()).unwrap();
    document.add_shape(initial.clone()).unwrap();
    document
        .edit_shape(0, |shape| {
            shape.offset(10.0, 10.0);
            assert!(shape.set_handle(0, Point::new(-20.0, 20.0)));
            let ShapeKind::Callout { data } = &mut shape.kind else {
                panic!()
            };
            data.fill_color = Color::RED;
            data.text_color = Color::MARKER_YELLOW;
            data.text = "Reflowed".into();
            data.layout.line_widths = vec![55.0];
            data.layout.body_height = 50.0;
        })
        .unwrap();
    let changed = document.shapes()[0].clone();
    assert!(document.undo());
    assert_eq!(document.shapes()[0], initial);
    assert!(document.redo());
    assert_eq!(document.shapes()[0], changed);
}

#[test]
fn callout_history_accounts_for_text_and_measured_line_allocations() {
    use crate::{document::Document, stitch::Image};
    fn history(lines: usize) -> Document {
        let mut data = callout(Some(Point::new(70.0, 120.0)));
        data.text = std::iter::repeat_n("x", lines)
            .collect::<Vec<_>>()
            .join("\n");
        data.layout.line_widths = vec![10.0; lines];
        data.layout.body_height = 24.0 * lines as f32;
        let mut document = Document::new(Image::new(1, 1, vec![0; 4]).unwrap()).unwrap();
        document
            .add_shape(Shape::new(ShapeKind::Callout {
                data: Box::new(data),
            }))
            .unwrap();
        document
            .edit_shape(0, |shape| shape.offset(1.0, 0.0))
            .unwrap();
        document
    }
    let short = history(1);
    let mut long = history(1000);
    let mut plain = Document::new(Image::new(1, 1, vec![0; 4]).unwrap()).unwrap();
    plain
        .add_shape(Shape::new(ShapeKind::Text {
            location: Point::new(20.0, 20.0),
            text: "x".into(),
        }))
        .unwrap();
    plain.edit_shape(0, |shape| shape.offset(1.0, 0.0)).unwrap();
    let allocation_overhead = 4 * std::mem::size_of::<usize>();
    assert_eq!(
        short.history_bytes() - plain.history_bytes(),
        std::mem::size_of::<CalloutData>()
            + allocation_overhead
            + std::mem::size_of::<f32>()
            + allocation_overhead
    );
    assert!(
        std::mem::size_of::<ShapeKind>()
            <= std::mem::size_of::<String>()
                + std::mem::size_of::<Point>()
                + std::mem::align_of::<String>()
    );
    assert_eq!(
        long.history_bytes() - short.history_bytes(),
        1998 + 999 * std::mem::size_of::<f32>()
    );
    let retained = long.shapes()[0].clone();
    long.set_history_budget(short.history_bytes());
    assert!(long.history_bytes() <= short.history_bytes());
    assert_eq!(long.shapes()[0], retained);
    let result = long.edit_shape(0, |shape| shape.offset(1.0, 0.0));
    assert_eq!(
        result,
        Err(crate::document::DocumentError::HistoryBudgetExceeded)
    );
    assert_eq!(long.shapes()[0], retained);
}

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

#[test]
fn callout_model_enforces_the_renderer_utf8_byte_limit() {
    let mut data = callout(None);
    data.text = "a".repeat(MAX_CALLOUT_TEXT_BYTES);
    assert!(data.validate());
    data.text.push('a');
    assert!(!data.validate());
    data.text = "ä".repeat(MAX_CALLOUT_TEXT_BYTES / 2);
    assert!(data.validate());
    data.text.push('ä');
    assert!(!data.validate());
}
