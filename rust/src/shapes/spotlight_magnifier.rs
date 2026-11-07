//! Platform-independent geometry and validated parameters for spotlight and
//! image magnifier annotations. This module makes no rendering or privacy
//! guarantees; sampling and compositing semantics belong to a renderer.

use super::{Point, Rect};

const MAX_COORDINATE: f32 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spotlight {
    /// Image-space rectangle kept at normal brightness.
    pub focus: Rect,
    /// Opacity of the dimming layer outside `focus`, in 0..=1.
    pub dim_opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagnifierFrame {
    Circle,
    Rectangle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Magnifier {
    /// Image-space placement and size of the magnifier viewport.
    pub viewport: Rect,
    /// Image-space point sampled at the center of the viewport.
    pub focus: Point,
    /// Source-to-viewport scale. Must be finite and at least 1.0.
    pub zoom: f32,
    pub frame: MagnifierFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    NonFinite,
    OutOfRange,
    EmptyBounds,
    InvalidHandle,
}

fn valid_coord(value: f32) -> bool {
    value.is_finite() && value.abs() <= MAX_COORDINATE
}

fn valid_rect(rect: Rect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(valid_coord)
        && rect.width > 0.0
        && rect.height > 0.0
        && valid_coord(rect.x + rect.width)
        && valid_coord(rect.y + rect.height)
}

fn corners(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x, rect.y),
        Point::new(rect.x + rect.width, rect.y),
        Point::new(rect.x + rect.width, rect.y + rect.height),
        Point::new(rect.x, rect.y + rect.height),
    ]
}

fn hit_rect(rect: Rect, p: Point, tolerance: f32) -> bool {
    p.x.is_finite()
        && p.y.is_finite()
        && tolerance.is_finite()
        && tolerance >= 0.0
        && p.x >= rect.x - tolerance
        && p.x <= rect.x + rect.width + tolerance
        && p.y >= rect.y - tolerance
        && p.y <= rect.y + rect.height + tolerance
}

impl Spotlight {
    pub fn new(focus: Rect, dim_opacity: f32) -> Result<Self, GeometryError> {
        let value = Self { focus, dim_opacity };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), GeometryError> {
        if !self.dim_opacity.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if !(0.0..=1.0).contains(&self.dim_opacity) {
            return Err(GeometryError::OutOfRange);
        }
        if ![
            self.focus.x,
            self.focus.y,
            self.focus.width,
            self.focus.height,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(GeometryError::NonFinite);
        }
        if !valid_rect(self.focus) {
            return Err(GeometryError::EmptyBounds);
        }
        Ok(())
    }

    pub fn bounds(&self) -> Rect {
        self.focus
    }

    pub fn handles(&self) -> [Point; 4] {
        corners(self.focus)
    }

    pub fn hit_test(&self, p: Point, tolerance: f32) -> bool {
        self.validate().is_ok() && hit_rect(self.focus, p, tolerance)
    }

    /// Translate the whole object. Invalid or overflowing moves leave it intact.
    pub fn offset(&mut self, dx: f32, dy: f32) -> Result<(), GeometryError> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let mut next = *self;
        next.focus.x += dx;
        next.focus.y += dy;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Resize using normalized corner identity; crossing edges is supported.
    pub fn set_handle(&mut self, index: usize, point: Point) -> Result<(), GeometryError> {
        if index >= 4 {
            return Err(GeometryError::InvalidHandle);
        }
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let mut next = *self;
        let old = self.focus;
        let (mut left, mut top, mut right, mut bottom) =
            (old.x, old.y, old.x + old.width, old.y + old.height);
        match index {
            0 => (left, top) = (point.x, point.y),
            1 => (right, top) = (point.x, point.y),
            2 => (right, bottom) = (point.x, point.y),
            _ => (left, bottom) = (point.x, point.y),
        }
        next.focus = Rect {
            x: left.min(right),
            y: top.min(bottom),
            width: (left - right).abs(),
            height: (top - bottom).abs(),
        };
        next.validate()?;
        *self = next;
        Ok(())
    }
}

impl Magnifier {
    pub fn new(
        viewport: Rect,
        focus: Point,
        zoom: f32,
        frame: MagnifierFrame,
    ) -> Result<Self, GeometryError> {
        let value = Self {
            viewport,
            focus,
            zoom,
            frame,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), GeometryError> {
        if ![self.focus.x, self.focus.y, self.zoom]
            .into_iter()
            .all(f32::is_finite)
        {
            return Err(GeometryError::NonFinite);
        }
        if !valid_coord(self.focus.x) || !valid_coord(self.focus.y) {
            return Err(GeometryError::OutOfRange);
        }
        if self.zoom < 1.0 {
            return Err(GeometryError::OutOfRange);
        }
        if ![
            self.viewport.x,
            self.viewport.y,
            self.viewport.width,
            self.viewport.height,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(GeometryError::NonFinite);
        }
        if !valid_rect(self.viewport) {
            return Err(GeometryError::EmptyBounds);
        }
        Ok(())
    }

    pub fn bounds(&self) -> Rect {
        self.viewport
    }

    pub fn handles(&self) -> [Point; 4] {
        corners(self.viewport)
    }

    pub fn hit_test(&self, p: Point, tolerance: f32) -> bool {
        if self.validate().is_err()
            || !p.x.is_finite()
            || !p.y.is_finite()
            || !tolerance.is_finite()
            || tolerance < 0.0
        {
            return false;
        }
        match self.frame {
            MagnifierFrame::Rectangle => hit_rect(self.viewport, p, tolerance),
            MagnifierFrame::Circle => {
                let rx = self.viewport.width / 2.0 + tolerance;
                let ry = self.viewport.height / 2.0 + tolerance;
                let cx = self.viewport.x + self.viewport.width / 2.0;
                let cy = self.viewport.y + self.viewport.height / 2.0;
                let dx = (p.x - cx) / rx;
                let dy = (p.y - cy) / ry;
                dx * dx + dy * dy <= 1.0
            }
        }
    }

    /// Move viewport and sampling point together. Invalid moves are atomic.
    pub fn offset(&mut self, dx: f32, dy: f32) -> Result<(), GeometryError> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let mut next = *self;
        next.viewport.x += dx;
        next.viewport.y += dy;
        next.focus.x += dx;
        next.focus.y += dy;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Resize the viewport without moving the sampled image point.
    pub fn set_handle(&mut self, index: usize, point: Point) -> Result<(), GeometryError> {
        if index >= 4 {
            return Err(GeometryError::InvalidHandle);
        }
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        let mut next = *self;
        let old = self.viewport;
        let (mut left, mut top, mut right, mut bottom) =
            (old.x, old.y, old.x + old.width, old.y + old.height);
        match index {
            0 => (left, top) = (point.x, point.y),
            1 => (right, top) = (point.x, point.y),
            2 => (right, bottom) = (point.x, point.y),
            _ => (left, bottom) = (point.x, point.y),
        }
        next.viewport = Rect {
            x: left.min(right),
            y: top.min(bottom),
            width: (left - right).abs(),
            height: (top - bottom).abs(),
        };
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn set_zoom(&mut self, zoom: f32) -> Result<(), GeometryError> {
        let mut next = *self;
        next.zoom = zoom;
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn set_focus(&mut self, focus: Point) -> Result<(), GeometryError> {
        let mut next = *self;
        next.focus = focus;
        next.validate()?;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        }
    }

    #[test]
    fn spotlight_geometry_resize_hit_and_atomic_validation() {
        let mut s = Spotlight::new(rect(), 0.4).unwrap();
        assert_eq!(s.bounds(), rect());
        assert_eq!(s.handles()[2], Point::new(40.0, 60.0));
        assert!(s.hit_test(Point::new(10.0, 20.0), 0.0));
        assert!(!s.hit_test(Point::new(41.0, 20.0), 0.0));
        s.set_handle(0, Point::new(45.0, 65.0)).unwrap();
        // Handle zero remains attached to the dragged corner as it crosses
        // the old opposite corner; bounds are then normalized.
        assert_eq!(
            s.bounds(),
            Rect {
                x: 40.0,
                y: 60.0,
                width: 5.0,
                height: 5.0
            }
        );
        let before = s;
        assert_eq!(
            s.set_handle(1, Point::new(f32::NAN, 0.0)),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(s, before);
        assert_eq!(s.offset(f32::INFINITY, 0.0), Err(GeometryError::NonFinite));
        assert_eq!(s, before);
    }

    #[test]
    fn magnifier_moves_viewport_and_focus_and_validates_zoom_atomically() {
        let mut m =
            Magnifier::new(rect(), Point::new(100.0, 80.0), 2.0, MagnifierFrame::Circle).unwrap();
        assert_eq!(m.handles()[0], Point::new(10.0, 20.0));
        assert!(m.hit_test(Point::new(25.0, 40.0), 0.0));
        m.offset(3.0, -2.0).unwrap();
        assert_eq!(m.viewport.x, 13.0);
        assert_eq!(m.focus, Point::new(103.0, 78.0));
        let before = m;
        assert_eq!(m.set_zoom(f32::NAN), Err(GeometryError::NonFinite));
        assert_eq!(m, before);
        assert_eq!(m.set_zoom(0.5), Err(GeometryError::OutOfRange));
        assert_eq!(m, before);
    }

    #[test]
    fn magnifier_hit_test_respects_circle_and_rectangle_frames() {
        let circle = Magnifier::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
            Point::new(5.0, 5.0),
            2.0,
            MagnifierFrame::Circle,
        )
        .unwrap();
        assert!(circle.hit_test(Point::new(10.0, 5.0), 0.0));
        assert!(!circle.hit_test(Point::new(0.0, 0.0), 0.0));
        // This point is half a pixel outside the left edge at the ellipse axis.
        assert!(circle.hit_test(Point::new(0.5, 5.0), 0.5));

        let rectangle = Magnifier {
            frame: MagnifierFrame::Rectangle,
            ..circle
        };
        assert!(rectangle.hit_test(Point::new(0.0, 0.0), 0.0));
    }

    #[test]
    fn constructors_reject_empty_or_non_finite_geometry() {
        assert_eq!(
            Spotlight::new(
                Rect {
                    width: 0.0,
                    ..rect()
                },
                0.5
            ),
            Err(GeometryError::EmptyBounds)
        );
        assert_eq!(
            Magnifier::new(
                rect(),
                Point::new(f32::NAN, 1.0),
                2.0,
                MagnifierFrame::Rectangle
            ),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(Spotlight::new(rect(), 1.1), Err(GeometryError::OutOfRange));
    }
}
