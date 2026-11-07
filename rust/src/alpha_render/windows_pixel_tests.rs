use super::*;
use crate::shapes::{Point, ShapeKind};

#[test]
fn short_thick_arrow_has_no_shaft_sticking_out_before_its_head() {
    let mut shape = Shape::new(ShapeKind::Arrow {
        start: Point::new(10.0, 16.0),
        end: Point::new(16.0, 16.0),
    });
    shape.style.width = 12.0;
    let image = render_shape(32, 32, &shape).expect("GDI+ arrow render");
    let pixel = |x: usize, y: usize| {
        let i = (y * image.width + x) * 4;
        [
            image.pixels[i],
            image.pixels[i + 1],
            image.pixels[i + 2],
            image.pixels[i + 3],
        ]
    };
    assert!(pixel(10, 16)[3] > 0, "head reaches the arrow start");
    assert_eq!(
        pixel(8, 16)[3],
        0,
        "no shaft protrudes beyond the arrow start"
    );
    assert_eq!(
        pixel(15, 12)[3],
        0,
        "the thick shaft does not protrude beside the narrow tip"
    );
    assert!(
        pixel(15, 16)[3] > 0,
        "pixels next to the arrow tip are rendered"
    );

    let zero = Shape::new(ShapeKind::Arrow {
        start: Point::new(10.0, 16.0),
        end: Point::new(10.0, 16.0),
    });
    let blank = render_shape(32, 32, &zero).expect("zero arrow render");
    assert!(blank.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
}

#[test]
fn highlight_layer_preserves_transparency_and_marker_opacity() {
    let mut shape = Shape::new(ShapeKind::Highlight {
        start: Point::new(2.0, 2.0),
        end: Point::new(8.0, 8.0),
    });
    shape.style.color = crate::shapes::Color::RED;
    let image = render_shape(12, 12, &shape).expect("GDI+ highlight render");
    let pixel = |x: usize, y: usize| {
        let i = (y * image.width + x) * 4;
        [
            image.pixels[i],
            image.pixels[i + 1],
            image.pixels[i + 2],
            image.pixels[i + 3],
        ]
    };
    assert_eq!(
        pixel(0, 0),
        [0, 0, 0, 0],
        "untouched layer pixels stay transparent"
    );
    assert_eq!(
        pixel(4, 4),
        [0, 0, 255, 105],
        "marker keeps its intended translucency"
    );
}

#[test]
fn step_marker_is_centered_legible_and_keeps_transparent_layer() {
    for color in [
        crate::shapes::Color::rgba(0, 0, 0, 255),
        crate::shapes::Color::rgba(255, 255, 255, 255),
    ] {
        for number in [1, 9, 10, 100, 999, 1000] {
            let mut shape = Shape::new(ShapeKind::Step {
                center: Point::new(48.0, 48.0),
                number,
            });
            shape.style.color = color;
            let image = render_shape(96, 96, &shape).expect("GDI+ step render");
            let pixel = |x: usize, y: usize| {
                let i = (y * image.width + x) * 4;
                [
                    image.pixels[i],
                    image.pixels[i + 1],
                    image.pixels[i + 2],
                    image.pixels[i + 3],
                ]
            };
            assert_eq!(pixel(0, 0), [0, 0, 0, 0]);
            let mut glyph = Vec::new();
            // The white outline sits near radius 18.8; inspect only the
            // opaque center region so neither that ring nor transparent
            // pixels outside the marker can satisfy the text assertions.
            let inner_extent = (18.8_f32 * 0.70).floor() as usize;
            let lo = 48 - inner_extent;
            let hi = 48 + inner_extent;
            for y in lo..=hi {
                for x in lo..=hi {
                    let dx = x as f32 - 48.0;
                    let dy = y as f32 - 48.0;
                    if dx * dx + dy * dy > (18.8_f32 * 0.70).powi(2) {
                        continue;
                    }
                    let p = pixel(x, y);
                    if p[3] == 0 {
                        continue;
                    }
                    let visible = if color.r == 0 {
                        p[0] > 235 && p[1] > 235 && p[2] > 235
                    } else {
                        p[0] < 80 && p[1] < 80 && p[2] < 80
                    };
                    if visible {
                        glyph.push((x, y));
                    }
                }
            }
            assert!(!glyph.is_empty(), "{number} has visible contrasting glyphs");
            let min_x = glyph.iter().map(|p| p.0).min().unwrap();
            let max_x = glyph.iter().map(|p| p.0).max().unwrap();
            let min_y = glyph.iter().map(|p| p.1).min().unwrap();
            let max_y = glyph.iter().map(|p| p.1).max().unwrap();
            assert!(
                (min_x + max_x) / 2 >= 45 && (min_x + max_x) / 2 <= 51,
                "{number} centered horizontally: {min_x}..{max_x}"
            );
            assert!(
                (min_y + max_y) / 2 >= 45 && (min_y + max_y) / 2 <= 51,
                "{number} centered vertically: {min_y}..{max_y}"
            );
            assert!(
                max_x - min_x < 32,
                "{number} fits within the marker interior"
            );
        }
    }
}

#[test]
fn rectangle_and_ellipse_fill_preserve_outline_and_straight_alpha() {
    use crate::shapes::{Color, FillMode};
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
        // RGB values divisible by 2 avoid quantization ambiguity in PARGB.
        shape.style.color = Color::rgba(200, 100, 50, 255);
        let outline = render_shape(48, 48, &shape).expect("GDI+ outline render");
        let pixel = |image: &Image, x: usize, y: usize| {
            let i = (y * image.width + x) * 4;
            <[u8; 4]>::try_from(&image.pixels[i..i + 4]).unwrap()
        };
        assert_eq!(pixel(&outline, 24, 24), [0, 0, 0, 0]);
        for (fill, alpha) in [(FillMode::Solid, 255), (FillMode::Translucent, 128)] {
            shape.style.fill = fill;
            let image = render_shape(48, 48, &shape).expect("GDI+ filled box render");
            let center = pixel(&image, 24, 24);
            assert_eq!(center[3], alpha, "interior keeps the fill opacity");
            for (actual, expected) in center[..3].iter().zip([50u8, 100, 200]) {
                assert!(
                    actual.abs_diff(expected) <= 1,
                    "straight RGB survives PARGB"
                );
            }
            assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
            assert!(pixel(&image, 8, 24)[3] >= pixel(&outline, 8, 24)[3]);
            assert!(pixel(&image, 8, 24)[3] >= 240, "outline stays opaque");
            if matches!(shape.kind, ShapeKind::Ellipse { .. }) {
                assert_eq!(pixel(&image, 10, 10), [0, 0, 0, 0]);
            }
        }
        shape.style.color.a = 128;
        shape.style.fill = FillMode::Translucent;
        let image = render_shape(48, 48, &shape).expect("GDI+ alpha-scaled fill");
        assert_eq!(pixel(&image, 24, 24)[3], 64);
        shape.style.color.a = 0;
        let blank = render_shape(48, 48, &shape).expect("transparent filled box");
        assert!(blank.pixels.iter().all(|channel| *channel == 0));
    }
}

#[test]
fn line_patterns_preserve_solid_default_and_have_real_transparent_gaps() {
    use crate::shapes::LineStyle;
    let mut shape = Shape::new(ShapeKind::Line {
        start: Point::new(8.0, 16.0),
        end: Point::new(120.0, 16.0),
    });
    shape.style.width = 4.0;
    let mut counts = Vec::new();
    for pattern in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
        shape.style.line_style = pattern;
        let image = render_shape(128, 32, &shape).expect("GDI+ line pattern");
        let alphas: Vec<_> = (12..116)
            .map(|x| image.pixels[(16 * 128 + x) * 4 + 3])
            .collect();
        counts.push(alphas.iter().filter(|a| **a > 0).count());
        if pattern == LineStyle::Solid {
            assert!(alphas.iter().all(|a| *a == 255));
        } else {
            assert!(alphas.contains(&0), "pattern contains transparent gaps");
            assert!(alphas.contains(&255), "pattern contains opaque marks");
        }
    }
    assert!(counts[0] > counts[1] && counts[1] > counts[2]);
}

#[test]
fn alternative_arrow_ends_paint_their_own_selectable_extents() {
    use crate::shapes::ArrowEnds;
    let mut shape = Shape::new(ShapeKind::Arrow {
        start: Point::new(16.0, 32.0),
        end: Point::new(96.0, 32.0),
    });
    let pixel_alpha =
        |image: &Image, x: usize, y: usize| image.pixels[(y * image.width + x) * 4 + 3];
    let single = render_shape(112, 64, &shape).unwrap();
    assert_eq!(pixel_alpha(&single, 28, 28), 0);
    shape.style.arrow_ends = ArrowEnds::Double;
    let double = render_shape(112, 64, &shape).unwrap();
    assert!(pixel_alpha(&double, 28, 28) > 0, "backward head exists");
    assert!(pixel_alpha(&double, 84, 28) > 0, "forward head exists");
    assert_eq!(pixel_alpha(&double, 12, 32), 0);
    shape.style.arrow_ends = ArrowEnds::Dot;
    let dot = render_shape(112, 64, &shape).unwrap();
    assert!(pixel_alpha(&dot, 100, 32) > 0, "dot extends past endpoint");
    assert_eq!(pixel_alpha(&dot, 104, 32), 0);
    assert!(shape.hit_test(Point::new(100.0, 32.0), 0.0));
    assert!(shape.bounds().contains(Point::new(100.0, 32.0)));
}

#[test]
fn rounded_rectangle_cuts_corners_and_handles_small_clipped_and_invalid_boxes() {
    use crate::shapes::{FillMode, RectangleCorners};
    let mut shape = Shape::new(ShapeKind::Rectangle {
        start: Point::new(8.0, 8.0),
        end: Point::new(72.0, 40.0),
    });
    shape.style.fill = FillMode::Solid;
    let alpha = |image: &Image, x: usize, y: usize| image.pixels[(y * image.width + x) * 4 + 3];
    let square = render_shape(80, 48, &shape).unwrap();
    shape.style.rectangle_corners = RectangleCorners::Rounded;
    let rounded = render_shape(80, 48, &shape).unwrap();
    assert_eq!(alpha(&square, 9, 9), 255);
    assert_eq!(alpha(&rounded, 9, 9), 0);
    assert_eq!(alpha(&rounded, 40, 24), 255);
    assert!(alpha(&rounded, 40, 8) >= 240);
    shape.style.fill = FillMode::None;
    let outline = render_shape(80, 48, &shape).unwrap();
    assert_eq!(alpha(&outline, 40, 24), 0);
    for (start, end) in [
        (Point::new(2.0, 2.0), Point::new(4.0, 6.0)),
        (Point::new(-10.0, -10.0), Point::new(30.0, 30.0)),
    ] {
        shape.kind = ShapeKind::Rectangle { start, end };
        assert!(render_shape(80, 48, &shape)
            .unwrap()
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0));
    }
    for (start, end) in [
        (Point::new(f32::NAN, 2.0), Point::new(4.0, 6.0)),
        (Point::new(2.0, 2.0), Point::new(f32::INFINITY, 6.0)),
        (Point::new(-f32::MAX, 2.0), Point::new(f32::MAX, 6.0)),
        (Point::new(2.0, -f32::MAX), Point::new(2.0, f32::MAX)),
    ] {
        shape.kind = ShapeKind::Rectangle { start, end };
        assert!(render_shape(80, 48, &shape)
            .unwrap()
            .pixels
            .iter()
            .all(|c| *c == 0));
    }
}

#[test]
fn rounded_rectangle_zero_axes_render_pixel_identically_to_square_rectangles() {
    use crate::shapes::{FillMode, RectangleCorners};
    for end in [
        Point::new(8.0, 40.0),
        Point::new(40.0, 8.0),
        Point::new(8.0, 8.0),
    ] {
        for fill in [FillMode::None, FillMode::Solid, FillMode::Translucent] {
            let mut rounded = Shape::new(ShapeKind::Rectangle {
                start: Point::new(8.0, 8.0),
                end: Point::new(40.0, 40.0),
            });
            rounded.style.rectangle_corners = RectangleCorners::Rounded;
            rounded.style.fill = fill;
            assert!(rounded.set_handle(2, end));
            let mut square = rounded.clone();
            square.style.rectangle_corners = RectangleCorners::Square;
            let actual = render_shape(48, 48, &rounded).expect("collapsed rounded rectangle");
            let expected = render_shape(48, 48, &square).expect("collapsed square rectangle");
            assert_eq!(
                actual, expected,
                "zero axes retain the existing square raster path"
            );
            assert!(actual.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        }
    }
}

#[test]
fn budgeted_gdi_layers_match_legacy_pixels_and_release_after_backend_error() {
    use crate::shapes::{CalloutData, Color, FillMode};
    let callout = CalloutData {
        origin: Point::new(10.0, 10.0),
        text: "Budget\nAlpha".into(),
        fill_color: Color::rgba(20, 40, 80, 100),
        text_color: Color::rgba(240, 220, 200, 128),
        tip: Some(Point::new(110.0, 130.0)),
        layout: measure_callout_text("Budget\nAlpha", 20.0).unwrap(),
    };
    let start = Point::new(12.25, 14.5);
    let end = Point::new(100.0, 85.75);
    let kinds = [
        ShapeKind::Line { start, end },
        ShapeKind::Arrow { start, end },
        ShapeKind::Rectangle { start, end },
        ShapeKind::Ellipse { start, end },
        ShapeKind::Highlight { start, end },
        ShapeKind::Freehand {
            points: vec![start, Point::new(55.5, 100.0), end],
        },
        ShapeKind::Text {
            location: start,
            text: "Alpha 42".into(),
        },
        ShapeKind::Step {
            center: Point::new(70.0, 70.0),
            number: 42,
        },
        ShapeKind::Callout {
            data: Box::new(callout.clone()),
        },
    ];
    let bytes = 160 * 160 * 4;
    let budget = ResourceBudget::new(bytes, bytes).unwrap();
    for kind in kinds {
        let mut shape = Shape::new(kind);
        shape.style.width = 4.0;
        shape.style.color = Color::rgba(200, 100, 50, 128);
        shape.style.fill = FillMode::Translucent;
        let original = shape.clone();
        let legacy = render_shape(160, 160, &shape).unwrap();
        let layer = render_shape_layer(160, 160, &shape, Some(&budget)).unwrap();
        assert_eq!(layer.image(), &legacy, "budget changed GDI layer pixels");
        assert_eq!(
            budget.snapshot().used_bytes,
            layer.image().pixels.capacity()
        );
        assert_eq!(shape, original);
        assert!(matches!(
            render_shape_layer(160, 160, &shape, Some(&budget)),
            Err(RenderError::BudgetRejected(_))
        ));
        drop(layer);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
    // A valid request enters GDI and creates native guards before callout draw
    // rejects its invalid layout. Admission must survive and unwind with them.
    let mut invalid = callout;
    invalid.layout.font_size = f32::NAN;
    let shape = Shape::new(ShapeKind::Callout {
        data: Box::new(invalid),
    });
    assert_eq!(
        render_shape(160, 160, &shape),
        Err(RenderError::InvalidDimensions)
    );
    assert!(matches!(
        render_shape_layer(160, 160, &shape, Some(&budget)),
        Err(RenderError::InvalidDimensions)
    ));
    assert_eq!(budget.snapshot().used_bytes, 0);
}
