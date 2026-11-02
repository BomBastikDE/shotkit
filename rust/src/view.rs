//! Fit, zoom and pan calculations shared by painting and hit testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
/// `zoom` multiplies fit-to-window; `pan` is the scroll offset in screen pixels.
pub fn layout(
    image: (u32, u32),
    viewport: (i32, i32, i32, i32),
    zoom: f64,
    pan: (i32, i32),
) -> View {
    let (x, y, vw, vh) = viewport;
    let vw = vw.max(1);
    let vh = vh.max(1);
    let iw = image.0.max(1) as f64;
    let ih = image.1.max(1) as f64;
    let fit = (vw as f64 / iw).min(vh as f64 / ih);
    let factor = if zoom.is_finite() {
        zoom.clamp(0.05, 64.0)
    } else {
        1.0
    };
    let w = (iw * fit * factor).round().clamp(1.0, 1_000_000.0) as i32;
    let h = (ih * fit * factor).round().clamp(1.0, 1_000_000.0) as i32;
    let px = if w <= vw {
        (vw - w) / 2
    } else {
        -pan.0.clamp(0, w - vw)
    };
    let py = if h <= vh {
        (vh - h) / 2
    } else {
        -pan.1.clamp(0, h - vh)
    };
    View {
        x: x + px,
        y: y + py,
        width: w,
        height: h,
    }
}
pub fn to_image(view: View, image: (u32, u32), point: (i32, i32)) -> (i32, i32) {
    let w = image.0.max(1).min(i32::MAX as u32) as i32;
    let h = image.1.max(1).min(i32::MAX as u32) as i32;
    let x =
        ((point.0 - view.x) as i64 * w as i64 / view.width.max(1) as i64).clamp(0, (w - 1) as i64);
    let y =
        ((point.1 - view.y) as i64 * h as i64 / view.height.max(1) as i64).clamp(0, (h - 1) as i64);
    (x as i32, y as i32)
}
/// The screen-space union of old and new shape footprints, clipped to the image.
/// Text bounds are approximate in the editor; conservatively redraw its image view.
pub fn dirty_shape_region(
    view: View,
    image: (u32, u32),
    before: Option<&crate::shapes::Shape>,
    after: Option<&crate::shapes::Shape>,
) -> Option<View> {
    use crate::shapes::ShapeKind;
    // While creating a freehand stroke only its tail changes. Include the last
    // two old points so joins are repainted; moving an existing stroke falls
    // back to the conservative union of its complete old and new bounds.
    let delta = match (before, after) {
        (Some(old), Some(new)) if old.style == new.style => match (&old.kind, &new.kind) {
            (ShapeKind::Freehand { points: a }, ShapeKind::Freehand { points: b })
                if b.len() > a.len() && b.starts_with(a) =>
            {
                let mut shape = new.clone();
                shape.kind = ShapeKind::Freehand {
                    points: b[a.len().saturating_sub(2)..].to_vec(),
                };
                Some(shape)
            }
            _ => None,
        },
        _ => None,
    };
    let shapes: Vec<_> = if let Some(ref tail) = delta {
        vec![tail]
    } else {
        [before, after].into_iter().flatten().collect()
    };
    if shapes.is_empty() {
        return None;
    }
    if shapes
        .iter()
        .any(|shape| matches!(shape.kind, ShapeKind::Text { .. }))
    {
        return Some(view);
    }
    let sx = f64::from(view.width) / f64::from(image.0.max(1));
    let sy = f64::from(view.height) / f64::from(image.1.max(1));
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for shape in shapes {
        let bounds = shape.bounds();
        let values = [
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            shape.style.width,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            return Some(view);
        }
        // Arrowheads extend up to four stroke widths; 16 image pixels and
        // 8 screen pixels cover antialiasing and the editor selection handles.
        let width_factor = if matches!(shape.kind, ShapeKind::Freehand { .. }) {
            12.0
        } else {
            4.0
        };
        let margin = f64::from(shape.style.width.abs().max(1.0) * width_factor + 16.0);
        left = left.min(f64::from(view.x) + (f64::from(bounds.x) - margin) * sx - 8.0);
        top = top.min(f64::from(view.y) + (f64::from(bounds.y) - margin) * sy - 8.0);
        right =
            right.max(f64::from(view.x) + (f64::from(bounds.x + bounds.width) + margin) * sx + 8.0);
        bottom = bottom
            .max(f64::from(view.y) + (f64::from(bounds.y + bounds.height) + margin) * sy + 8.0);
    }
    let l = left.floor().max(f64::from(view.x));
    let t = top.floor().max(f64::from(view.y));
    let r = right.ceil().min(f64::from(view.x) + f64::from(view.width));
    let b = bottom
        .ceil()
        .min(f64::from(view.y) + f64::from(view.height));
    if r <= l || b <= t {
        None
    } else {
        Some(View {
            x: l as i32,
            y: t as i32,
            width: (r - l) as i32,
            height: (b - t) as i32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dirty_region_covers_old_and_new_shapes_and_clips_to_image_view() {
        use crate::shapes::{Point, Shape, ShapeKind};
        let v = View {
            x: 10,
            y: 20,
            width: 1000,
            height: 500,
        };
        let before = Shape::new(ShapeKind::Arrow {
            start: Point::new(10.0, 10.0),
            end: Point::new(100.0, 90.0),
        });
        let after = Shape::new(ShapeKind::Arrow {
            start: Point::new(100.0, 20.0),
            end: Point::new(300.0, 110.0),
        });
        let region = dirty_shape_region(v, (1000, 500), Some(&before), Some(&after)).unwrap();
        assert_eq!((region.x, region.y), (10, 20));
        assert!(region.width >= 300 && region.width < v.width);
        assert!(region.height >= 110 && region.height < v.height);
        assert_eq!(dirty_shape_region(v, (1000, 500), None, None), None);
        let text = Shape::new(ShapeKind::Text {
            location: Point::new(1.0, 1.0),
            text: "text".into(),
        });
        assert_eq!(
            dirty_shape_region(v, (1000, 500), Some(&text), None),
            Some(v)
        );
    }

    #[test]
    fn growing_freehand_invalidates_tail_but_moved_stroke_invalidates_full_union() {
        use crate::shapes::{Point, Shape, ShapeKind};
        let v = View {
            x: 0,
            y: 0,
            width: 1000,
            height: 500,
        };
        let before = Shape::new(ShapeKind::Freehand {
            points: vec![
                Point::new(20.0, 20.0),
                Point::new(400.0, 200.0),
                Point::new(700.0, 200.0),
            ],
        });
        let mut after = before.clone();
        if let ShapeKind::Freehand { points } = &mut after.kind {
            points.push(Point::new(750.0, 210.0));
        }
        let tail = dirty_shape_region(v, (1000, 500), Some(&before), Some(&after)).unwrap();
        assert!(
            tail.x > 20,
            "the unchanged first section should not be invalidated"
        );
        assert!(tail.x <= 400 && tail.x + tail.width >= 750);
        let mut moved = before.clone();
        moved.offset(20.0, 30.0);
        let union = dirty_shape_region(v, (1000, 500), Some(&before), Some(&moved)).unwrap();
        assert_eq!(union.x, 0);
        assert!(union.x + union.width >= 720);
    }

    #[cfg(windows)]
    #[test]
    fn dirty_region_contains_every_changed_rendered_pixel() {
        use crate::{
            alpha_document::render_document,
            shapes::{Point, Shape, ShapeKind},
            stitch::Image,
        };
        let image = Image::new(
            180,
            140,
            (0..180 * 140)
                .flat_map(|i| [i as u8, (i / 180) as u8, 90, 255])
                .collect(),
        )
        .unwrap();
        let view = View {
            x: 0,
            y: 0,
            width: 180,
            height: 140,
        };
        let a = Point::new(30.0, 35.0);
        let b = Point::new(90.0, 80.0);
        let kinds = [
            ShapeKind::Line { start: a, end: b },
            ShapeKind::Arrow { start: a, end: b },
            ShapeKind::Rectangle { start: a, end: b },
            ShapeKind::Ellipse { start: a, end: b },
            ShapeKind::Highlight { start: a, end: b },
            ShapeKind::Blur { start: a, end: b },
            ShapeKind::Freehand {
                points: vec![a, Point::new(48.0, 70.0), b],
            },
            ShapeKind::Step {
                center: a,
                number: 2,
            },
        ];
        for kind in kinds {
            let before = Shape::new(kind);
            let mut after = before.clone();
            after.offset(27.0, 12.0);
            let old = render_document(&image, std::slice::from_ref(&before)).unwrap();
            let new = render_document(&image, std::slice::from_ref(&after)).unwrap();
            let area = dirty_shape_region(view, (180, 140), Some(&before), Some(&after)).unwrap();
            for (index, (left, right)) in old
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(new.pixels.as_chunks::<4>().0.iter())
                .enumerate()
            {
                if left != right {
                    let x = (index % 180) as i32;
                    let y = (index / 180) as i32;
                    assert!(
                        x >= area.x
                            && x < area.x + area.width
                            && y >= area.y
                            && y < area.y + area.height,
                        "changed pixel ({x},{y}) outside dirty area for {:?}",
                        before.kind
                    );
                }
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn growing_freehand_dirty_tail_contains_every_changed_pixel() {
        use crate::{
            alpha_document::render_document,
            shapes::{Point, Shape, ShapeKind},
            stitch::Image,
        };
        let image = Image::new(400, 200, vec![255; 400 * 200 * 4]).unwrap();
        let view = View {
            x: 0,
            y: 0,
            width: 400,
            height: 200,
        };
        let before = Shape::new(ShapeKind::Freehand {
            points: vec![
                Point::new(10.0, 20.0),
                Point::new(100.0, 160.0),
                Point::new(250.0, 60.0),
            ],
        });
        let mut after = before.clone();
        if let ShapeKind::Freehand { points } = &mut after.kind {
            points.push(Point::new(340.0, 170.0));
        }
        let old = render_document(&image, std::slice::from_ref(&before)).unwrap();
        let new = render_document(&image, std::slice::from_ref(&after)).unwrap();
        let area = dirty_shape_region(view, (400, 200), Some(&before), Some(&after)).unwrap();
        for (index, (left, right)) in old
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(new.pixels.as_chunks::<4>().0.iter())
            .enumerate()
        {
            if left != right {
                let x = (index % 400) as i32;
                let y = (index / 400) as i32;
                assert!(
                    x >= area.x
                        && x < area.x + area.width
                        && y >= area.y
                        && y < area.y + area.height,
                    "changed freehand pixel ({x},{y}) outside dirty tail"
                );
            }
        }
    }

    #[test]
    fn fits_preserving_aspect() {
        assert_eq!(
            layout((400, 200), (0, 50, 200, 200), 1.0, (0, 0)),
            View {
                x: 0,
                y: 100,
                width: 200,
                height: 100
            }
        );
    }
    #[test]
    fn zoom_pan_and_hit_test() {
        let v = layout((1000, 2000), (0, 0, 500, 500), 2.0, (0, 250));
        assert_eq!(
            v,
            View {
                x: 0,
                y: -250,
                width: 500,
                height: 1000
            }
        );
        assert_eq!(to_image(v, (1000, 2000), (250, 250)), (500, 1000));
    }
    #[test]
    fn clamps_pan_and_invalid_zoom() {
        let v = layout((100, 100), (0, 0, 100, 100), f64::NAN, (99, 99));
        assert_eq!(
            v,
            View {
                x: 0,
                y: 0,
                width: 100,
                height: 100
            }
        );
        let v = layout((100, 100), (0, 0, 100, 100), 2.0, (999, 999));
        assert_eq!((v.x, v.y), (-100, -100));
    }
}
