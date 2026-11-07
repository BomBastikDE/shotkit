use super::{ArrowEnds, Point, Rect, Style};

/// A collapsed corner radius uses the legacy rectangle path, which retains a
/// one-pixel raster extent and remains selectable after a handle crosses axes.
/// Non-finite coordinates or overflowing extents stay on the rejecting rounded path.
pub(crate) fn rectangle_has_zero_axis(start: Point, end: Point) -> bool {
    [start.x, start.y, end.x, end.y]
        .iter()
        .all(|v| v.is_finite())
        && (start.x - end.x).is_finite()
        && (start.y - end.y).is_finite()
        && (start.x == end.x || start.y == end.y)
}

/// Shared geometry for styled arrow ends; legacy single-arrow rendering keeps
/// its existing geometry path. Points include the filled head, not stroke pad.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StyledArrowHead {
    pub tip: Point,
    pub left: Point,
    pub right: Point,
    pub shaft_end: Option<Point>,
}

pub(crate) fn styled_stroke_width(width: f32) -> f32 {
    if width.is_nan() {
        1.0
    } else {
        width.clamp(1.0, 256.0)
    }
}

pub(crate) fn styled_arrow_head(start: Point, end: Point, width: f32) -> Option<StyledArrowHead> {
    let (sx, sy, ex, ey) = (start.x as f64, start.y as f64, end.x as f64, end.y as f64);
    if ![sx, sy, ex, ey].iter().all(|value| value.is_finite()) {
        return None;
    }
    let (dx, dy) = (ex - sx, ey - sy);
    let length = dx.hypot(dy);
    if length < 1.0 {
        return None;
    }
    let width = styled_stroke_width(width) as f64;
    let head = (width * 4.0).max(8.0).min(length * 0.6);
    let (ux, uy) = (dx / length, dy / length);
    let shaft_fits = length > head && head * 0.45 * 0.8 >= width * 0.5;
    let (bx, by) = if shaft_fits {
        (ex - ux * head, ey - uy * head)
    } else {
        (sx, sy)
    };
    let half_base = if shaft_fits {
        head * 0.45
    } else {
        (head * 0.45).max(width * 0.5)
    };
    let point = |x: f64, y: f64| {
        if x.abs() <= f32::MAX as f64 && y.abs() <= f32::MAX as f64 {
            Some(Point::new(x as f32, y as f32))
        } else {
            None
        }
    };
    Some(StyledArrowHead {
        tip: end,
        left: point(bx - uy * half_base, by + ux * half_base)?,
        right: point(bx + uy * half_base, by - ux * half_base)?,
        shaft_end: if shaft_fits {
            Some(point(ex - ux * head * 0.8, ey - uy * head * 0.8)?)
        } else {
            None
        },
    })
}

pub(crate) fn arrow_dot_radius(width: f32) -> f32 {
    (styled_stroke_width(width) * 1.5).max(3.0)
}

pub(crate) fn styled_arrow_bounds(start: Point, end: Point, style: &Style) -> Option<Rect> {
    let head = styled_arrow_head(start, end, style.width)?;
    let mut points = vec![start, end];
    match style.arrow_ends {
        ArrowEnds::Double => {
            let back = styled_arrow_head(end, start, style.width)?;
            points.extend([head.left, head.right, back.left, back.right]);
        }
        ArrowEnds::Dot => {
            let radius = arrow_dot_radius(style.width) as f64;
            // Include the shaft's square start and the endpoint's full circle.
            let (dx, dy) = (end.x as f64 - start.x as f64, end.y as f64 - start.y as f64);
            let length = dx.hypot(dy);
            let half = styled_stroke_width(style.width) as f64 / 2.0;
            for (x, y) in [
                (end.x as f64 - radius, end.y as f64 - radius),
                (end.x as f64 + radius, end.y as f64 + radius),
                (
                    start.x as f64 - dy / length * half,
                    start.y as f64 + dx / length * half,
                ),
                (
                    start.x as f64 + dy / length * half,
                    start.y as f64 - dx / length * half,
                ),
            ] {
                if x.abs() > f32::MAX as f64 || y.abs() > f32::MAX as f64 {
                    return None;
                }
                points.push(Point::new(x as f32, y as f32));
            }
        }
        ArrowEnds::Single => points.extend([head.left, head.right]),
    }
    let (mut lo, mut hi) = (points[0], points[0]);
    for point in points {
        lo = Point::new(lo.x.min(point.x), lo.y.min(point.y));
        hi = Point::new(hi.x.max(point.x), hi.y.max(point.y));
    }
    let bounds = Rect::from_points(lo, hi);
    (bounds.width.is_finite() && bounds.height.is_finite()).then_some(bounds)
}

pub(super) fn styled_arrow_hit_test(
    start: Point,
    end: Point,
    style: &Style,
    p: Point,
    tolerance: f32,
) -> bool {
    if !p.x.is_finite() || !p.y.is_finite() || !tolerance.is_finite() || tolerance < 0.0 {
        return false;
    }
    let Some(head) = styled_arrow_head(start, end, style.width) else {
        return false;
    };
    if styled_arrow_bounds(start, end, style).is_none() {
        return false;
    }
    if style.arrow_ends == ArrowEnds::Dot {
        return (p.x as f64 - end.x as f64).hypot(p.y as f64 - end.y as f64)
            <= (arrow_dot_radius(style.width) + tolerance) as f64
            || distance_segment(p, start, end)
                <= tolerance + styled_stroke_width(style.width) / 2.0;
    }
    let Some(back) = styled_arrow_head(end, start, style.width) else {
        return false;
    };
    triangle_hit_test(p, head, tolerance)
        || triangle_hit_test(p, back, tolerance)
        || match (back.shaft_end, head.shaft_end) {
            (Some(a), Some(b)) => {
                distance_segment(p, a, b) <= tolerance + styled_stroke_width(style.width) / 2.0
            }
            _ => false,
        }
}

fn triangle_hit_test(p: Point, head: StyledArrowHead, tolerance: f32) -> bool {
    let points = [head.tip, head.left, head.right];
    let mut positive = false;
    let mut negative = false;
    for i in 0..3 {
        let (a, b) = (points[i], points[(i + 1) % 3]);
        if distance_segment(p, a, b) <= tolerance {
            return true;
        }
        let cross = (b.x as f64 - a.x as f64) * (p.y as f64 - a.y as f64)
            - (b.y as f64 - a.y as f64) * (p.x as f64 - a.x as f64);
        positive |= cross > 0.0;
        negative |= cross < 0.0;
    }
    (positive || negative) && !(positive && negative)
}

pub(super) fn rounded_rectangle_radius(bounds: Rect) -> Option<f64> {
    if ![bounds.x, bounds.y, bounds.width, bounds.height]
        .iter()
        .all(|v| v.is_finite())
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || (bounds.x as f64 + bounds.width as f64).abs() > f32::MAX as f64
        || (bounds.y as f64 + bounds.height as f64).abs() > f32::MAX as f64
    {
        return None;
    }
    Some(
        12.0_f64
            .min(bounds.width as f64 / 2.0)
            .min(bounds.height as f64 / 2.0),
    )
}

pub(super) fn rounded_rectangle_distance(p: Point, bounds: Rect) -> Option<f64> {
    let radius = rounded_rectangle_radius(bounds)?;
    if !p.x.is_finite() || !p.y.is_finite() {
        return None;
    }
    let (half_w, half_h) = (bounds.width as f64 / 2.0, bounds.height as f64 / 2.0);
    let qx = (p.x as f64 - (bounds.x as f64 + half_w)).abs() - (half_w - radius);
    let qy = (p.y as f64 - (bounds.y as f64 + half_h)).abs() - (half_h - radius);
    Some(qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius)
}

#[cfg(any(windows, test))]
pub(crate) fn rounded_rectangle_points(start: Point, end: Point) -> Option<[Point; 37]> {
    let bounds = Rect::from_points(start, end);
    let radius = rounded_rectangle_radius(bounds)?;
    let (left, top) = (bounds.x as f64, bounds.y as f64);
    let (right, bottom) = (left + bounds.width as f64, top + bounds.height as f64);
    let centers = [
        (right - radius, top + radius),
        (right - radius, bottom - radius),
        (left + radius, bottom - radius),
        (left + radius, top + radius),
    ];
    let mut points = [Point::default(); 37];
    for (corner, (cx, cy)) in centers.into_iter().enumerate() {
        for step in 0..=8 {
            let angle = (-90.0 + corner as f64 * 90.0 + step as f64 * 90.0 / 8.0).to_radians();
            points[corner * 9 + step] = Point::new(
                (cx + radius * angle.cos()) as f32,
                (cy + radius * angle.sin()) as f32,
            );
        }
    }
    points[36] = points[0];
    Some(points)
}

pub(super) fn distance_segment(p: Point, a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = dx * dx + dy * dy;
    if len < 0.0001 {
        return ((p.x - a.x).powi(2) + (p.y - a.y).powi(2)).sqrt();
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len).clamp(0.0, 1.0);
    ((p.x - a.x - t * dx).powi(2) + (p.y - a.y - t * dy).powi(2)).sqrt()
}

/// Approximate minimum Euclidean distance to an ellipse contour. Sampling locates
/// the best basin; golden-section refinement gives subpixel precision without
/// unstable divisions for points at the center or close to an axis.
pub(super) fn distance_to_ellipse(p: Point, center: Point, rx: f32, ry: f32) -> f32 {
    if !p.x.is_finite()
        || !p.y.is_finite()
        || !center.x.is_finite()
        || !center.y.is_finite()
        || !rx.is_finite()
        || !ry.is_finite()
    {
        return f32::NAN;
    }
    let (rx, ry) = (rx.abs(), ry.abs());
    if rx == 0.0 && ry == 0.0 {
        return distance_segment(p, center, center);
    }
    if rx == 0.0 {
        return distance_segment(
            p,
            Point::new(center.x, center.y - ry),
            Point::new(center.x, center.y + ry),
        );
    }
    if ry == 0.0 {
        return distance_segment(
            p,
            Point::new(center.x - rx, center.y),
            Point::new(center.x + rx, center.y),
        );
    }
    let (px, py) = ((p.x - center.x).abs(), (p.y - center.y).abs());
    let eval = |t: f32| {
        let dx = rx * t.cos() - px;
        let dy = ry * t.sin() - py;
        dx * dx + dy * dy
    };
    const N: usize = 64;
    let step = std::f32::consts::TAU / N as f32;
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for i in 0..N {
        let d = eval(i as f32 * step);
        if d < best_d {
            best = i;
            best_d = d
        }
    }
    let (mut lo, mut hi) = if best == 0 {
        (-step, step)
    } else {
        (best as f32 * step - step, best as f32 * step + step)
    };
    let phi = (5.0_f32.sqrt() - 1.0) / 2.0;
    let mut c = hi - phi * (hi - lo);
    let mut d = lo + phi * (hi - lo);
    let mut fc = eval(c);
    let mut fd = eval(d);
    for _ in 0..32 {
        if fc <= fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - phi * (hi - lo);
            fc = eval(c)
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + phi * (hi - lo);
            fd = eval(d)
        }
    }
    eval((lo + hi) / 2.0).max(0.0).sqrt()
}
