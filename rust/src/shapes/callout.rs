//! Measured callout text and the single closed contour shared by selection and rendering.

use super::{Color, Point, Rect};

/// Maximum UTF-8 size accepted by both the model and Windows renderer.
pub const MAX_CALLOUT_TEXT_BYTES: usize = 64 * 1024;

/// Platform font measurement supplies these image-space values. Body dimensions
/// include text padding; widths are per logical line, including empty lines.
#[derive(Clone, Debug, PartialEq)]
pub struct CalloutTextLayout {
    pub font_size: f32,
    pub body_width: f32,
    pub body_height: f32,
    pub line_height: f32,
    pub line_widths: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalloutData {
    pub origin: Point,
    pub text: String,
    pub fill_color: Color,
    pub text_color: Color,
    /// Kept even when inside the body, so dragging it outside restores a pointer.
    pub tip: Option<Point>,
    pub layout: CalloutTextLayout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalloutGeometry {
    pub body: Rect,
    pub corner_radius: f32,
    /// Clockwise, closed exterior contour. Draw/fill once to avoid a base seam.
    pub outline: Vec<Point>,
    /// The base endpoints and tip, in exterior contour order. No inner base is
    /// present in `outline`; this triangle is metadata for consumers and tests.
    pub pointer: Option<[Point; 3]>,
    pub bounds: Rect,
}

impl CalloutTextLayout {
    pub fn validate(&self, text: &str) -> bool {
        [
            self.font_size,
            self.body_width,
            self.body_height,
            self.line_height,
        ]
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
            && self.line_widths.len() == text.split('\n').count()
            && self
                .line_widths
                .iter()
                .all(|width| width.is_finite() && *width >= 0.0 && *width <= self.body_width)
            && self.line_height as f64 * self.line_widths.len() as f64 <= self.body_height as f64
    }
}

impl CalloutData {
    pub fn validate(&self) -> bool {
        self.checked_body().is_some()
    }

    fn checked_body(&self) -> Option<Rect> {
        if self.text.len() > MAX_CALLOUT_TEXT_BYTES
            || !self.layout.validate(&self.text)
            || !finite(self.origin)
            || self.tip.is_some_and(|tip| !finite(tip))
        {
            return None;
        }
        let body = Rect {
            x: self.origin.x,
            y: self.origin.y,
            width: self.layout.body_width,
            height: self.layout.body_height,
        };
        let right = checked_point(body.x as f64 + body.width as f64, body.y as f64)?;
        let bottom = checked_point(body.x as f64, body.y as f64 + body.height as f64)?;
        // Reject dimensions lost to f32 precision as well as numerical overflow.
        if right.x <= body.x || bottom.y <= body.y {
            return None;
        }
        let tip = self.tip.unwrap_or(self.origin);
        if !(right.x.max(tip.x) as f64 - body.x.min(tip.x) as f64 <= f32::MAX as f64
            && bottom.y.max(tip.y) as f64 - body.y.min(tip.y) as f64 <= f32::MAX as f64)
        {
            return None;
        }
        Some(body)
    }

    pub fn geometry(&self) -> Option<CalloutGeometry> {
        let body = self.checked_body()?;
        let (l, t) = (body.x as f64, body.y as f64);
        let (r, b) = (l + body.width as f64, t + body.height as f64);
        // Leave a straight attachment span even for very small bodies.
        let radius = 12.0_f64
            .min(body.width as f64 / 4.0)
            .min(body.height as f64 / 4.0);
        let starts = [
            (l + radius, t),
            (r, t + radius),
            (r - radius, b),
            (l, b - radius),
        ];
        let ends = [
            (r - radius, t),
            (r, b - radius),
            (l + radius, b),
            (l, t + radius),
        ];
        let centers = [
            (r - radius, t + radius),
            (r - radius, b - radius),
            (l + radius, b - radius),
            (l + radius, t + radius),
        ];
        let side = self.tip.and_then(|tip| {
            let (x, y) = (tip.x as f64, tip.y as f64);
            if x >= l && x <= r && y >= t && y <= b {
                return None;
            }
            let dx = (x - (l + r) / 2.0) / body.width as f64;
            let dy = (y - (t + b) / 2.0) / body.height as f64;
            Some(if dy.abs() >= dx.abs() {
                if dy < 0.0 {
                    0
                } else {
                    2
                }
            } else if dx > 0.0 {
                1
            } else {
                3
            })
        });
        let mut outline = Vec::with_capacity(42);
        let mut pointer = None;
        outline.push(checked_point(starts[0].0, starts[0].1)?);
        for edge in 0..4 {
            let (start, end) = (starts[edge], ends[edge]);
            if side == Some(edge) {
                let tip = self.tip?;
                let (vx, vy) = (end.0 - start.0, end.1 - start.1);
                let length = vx.hypot(vy);
                let (ux, uy) = (vx / length, vy / length);
                let half_base = 14.0_f64.min(length / 2.0);
                let projection = ((tip.x as f64 - start.0) * ux + (tip.y as f64 - start.1) * uy)
                    .clamp(half_base, length - half_base);
                let a = checked_point(
                    start.0 + ux * (projection - half_base),
                    start.1 + uy * (projection - half_base),
                )?;
                let z = checked_point(
                    start.0 + ux * (projection + half_base),
                    start.1 + uy * (projection + half_base),
                )?;
                outline.extend([a, tip, z]);
                pointer = Some([a, tip, z]);
            }
            outline.push(checked_point(end.0, end.1)?);
            let center = centers[edge];
            for step in 1..=8 {
                let angle = (-90.0 + edge as f64 * 90.0 + step as f64 * 90.0 / 8.0).to_radians();
                outline.push(checked_point(
                    center.0 + radius * angle.cos(),
                    center.1 + radius * angle.sin(),
                )?);
            }
        }
        // Identical endpoint avoids tiny trigonometric closure gaps.
        *outline.last_mut()? = outline[0];
        let tip = pointer.map(|points| points[1]).unwrap_or(self.origin);
        let (left, top) = (body.x.min(tip.x), body.y.min(tip.y));
        let bounds = Rect {
            x: left,
            y: top,
            width: (r.max(tip.x as f64) - left as f64) as f32,
            height: (b.max(tip.y as f64) - top as f64) as f32,
        };
        Some(CalloutGeometry {
            body,
            corner_radius: radius as f32,
            outline,
            pointer,
            bounds,
        })
    }

    /// Move the text body and the absolute pointer together, atomically.
    pub fn offset(&mut self, dx: f32, dy: f32) -> bool {
        if !dx.is_finite() || !dy.is_finite() || !self.validate() {
            return false;
        }
        let Some(origin) = checked_point(
            self.origin.x as f64 + dx as f64,
            self.origin.y as f64 + dy as f64,
        ) else {
            return false;
        };
        let tip = match self.tip {
            Some(p) => match checked_point(p.x as f64 + dx as f64, p.y as f64 + dy as f64) {
                Some(p) => Some(p),
                None => return false,
            },
            None => None,
        };
        let previous = (self.origin, self.tip);
        self.origin = origin;
        self.tip = tip;
        if self.validate() {
            true
        } else {
            (self.origin, self.tip) = previous;
            false
        }
    }

    pub fn set_tip(&mut self, tip: Option<Point>) -> bool {
        let previous = self.tip;
        self.tip = tip;
        if self.validate() {
            true
        } else {
            self.tip = previous;
            false
        }
    }
}

impl CalloutGeometry {
    pub fn hit_test(&self, p: Point, tolerance: f32) -> bool {
        if !finite(p) || !tolerance.is_finite() || tolerance < 0.0 {
            return false;
        }
        let mut inside = false;
        for pair in self.outline.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = (b.x as f64 - a.x as f64, b.y as f64 - a.y as f64);
            let (px, py) = (p.x as f64 - a.x as f64, p.y as f64 - a.y as f64);
            let length_squared = dx * dx + dy * dy;
            let along = if length_squared == 0.0 {
                0.0
            } else {
                ((px * dx + py * dy) / length_squared).clamp(0.0, 1.0)
            };
            if (px - along * dx).hypot(py - along * dy) <= tolerance as f64 {
                return true;
            }
            if (a.y > p.y) != (b.y > p.y) && (p.x as f64) < a.x as f64 + dx * py / dy {
                inside = !inside;
            }
        }
        inside
    }
}

fn finite(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn checked_point(x: f64, y: f64) -> Option<Point> {
    (x.is_finite() && y.is_finite() && x.abs() <= f32::MAX as f64 && y.abs() <= f32::MAX as f64)
        .then_some(Point::new(x as f32, y as f32))
}
