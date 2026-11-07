//! Renderer-independent annotation geometry and edit history.
//! Coordinates are image-space pixels; rendering remains the caller's responsibility.

mod callout;
mod cursor_sprite;
mod geometry;
mod spotlight_magnifier;
mod tool_labels;
pub use callout::{CalloutData, CalloutGeometry, CalloutTextLayout, MAX_CALLOUT_TEXT_BYTES};
pub use cursor_sprite::CursorSprite;
#[cfg(windows)]
pub(crate) use geometry::arrow_dot_radius;
pub(crate) use geometry::rectangle_has_zero_axis;
#[cfg(any(windows, test))]
pub(crate) use geometry::rounded_rectangle_points;
pub(crate) use geometry::styled_arrow_bounds;
#[cfg(any(windows, test))]
pub(crate) use geometry::styled_arrow_head;
use geometry::{
    distance_segment, distance_to_ellipse, rounded_rectangle_distance, rounded_rectangle_radius,
    styled_arrow_hit_test,
};
pub use spotlight_magnifier::{
    GeometryError as EffectGeometryError, Magnifier, MagnifierFrame, Spotlight,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn from_points(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            width: (a.x - b.x).abs(),
            height: (a.y - b.y).abs(),
        }
    }
    /// Matches `RectangleF.Contains`: left/top included, right/bottom excluded.
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }
    pub fn inflated(self, d: f32) -> Self {
        Self {
            x: self.x - d,
            y: self.y - d,
            width: self.width + 2.0 * d,
            height: self.height + 2.0 * d,
        }
    }
    pub fn corners(self) -> [Point; 4] {
        [
            Point::new(self.x, self.y),
            Point::new(self.x + self.width, self.y),
            Point::new(self.x + self.width, self.y + self.height),
            Point::new(self.x, self.y + self.height),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
impl Color {
    pub const RED: Self = Self {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    /// Default editor palette red, matching the reference UI's first swatch.
    pub const PALETTE_RED: Self = Self {
        r: 230,
        g: 40,
        b: 40,
        a: 255,
    };
    pub const MARKER_YELLOW: Self = Self {
        r: 255,
        g: 214,
        b: 0,
        a: 255,
    };
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}
impl Default for Color {
    fn default() -> Self {
        Self::RED
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Arrow,
    Line,
    Rectangle,
    Ellipse,
    Text,
    Callout,
    Spotlight,
    Magnifier,
    Highlight,
    Blur,
    GaussianBlur,
    Redact,
    Step,
    Stamp,
    Pen,
    Crop,
}

/// Original, font-independent symbols drawn in a normalized unit square.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StampKind {
    #[default]
    Check,
    Cross,
    Warning,
    Info,
    Cursor,
}
impl StampKind {
    pub const ALL: [Self; 5] = [
        Self::Check,
        Self::Cross,
        Self::Warning,
        Self::Info,
        Self::Cursor,
    ];

    /// Geometry shared by raster output and selection. `p` is relative to the
    /// unit square's top left; padding expands the ink, not its empty interior.
    pub fn hit_test(self, p: Point, padding: f32) -> bool {
        if !p.x.is_finite() || !p.y.is_finite() || !padding.is_finite() || padding < 0.0 {
            return false;
        }
        let segment = |a: (f32, f32), b: (f32, f32), radius: f32| {
            distance_segment(p, Point::new(a.0, a.1), Point::new(b.0, b.1)) <= radius + padding
        };
        match self {
            Self::Check => {
                segment((0.16, 0.52), (0.40, 0.76), 0.075)
                    || segment((0.40, 0.76), (0.84, 0.23), 0.075)
            }
            Self::Cross => {
                segment((0.23, 0.23), (0.77, 0.77), 0.075)
                    || segment((0.23, 0.77), (0.77, 0.23), 0.075)
            }
            Self::Warning => {
                segment((0.50, 0.12), (0.89, 0.84), 0.045)
                    || segment((0.89, 0.84), (0.11, 0.84), 0.045)
                    || segment((0.11, 0.84), (0.50, 0.12), 0.045)
                    || segment((0.50, 0.40), (0.50, 0.58), 0.045)
                    || segment((0.50, 0.72), (0.50, 0.72), 0.045)
            }
            Self::Info => {
                (((p.x - 0.5).hypot(p.y - 0.5) - 0.37).abs() <= 0.045 + padding)
                    || segment((0.50, 0.46), (0.50, 0.71), 0.05)
                    || segment((0.50, 0.30), (0.50, 0.30), 0.05)
            }
            Self::Cursor => {
                // A deliberately original pointer silhouette, including stem.
                let vertices = [
                    (0.18, 0.10),
                    (0.82, 0.59),
                    (0.56, 0.62),
                    (0.71, 0.87),
                    (0.57, 0.94),
                    (0.42, 0.68),
                    (0.23, 0.87),
                ];
                let mut inside = false;
                for i in 0..vertices.len() {
                    let (a, b) = (vertices[i], vertices[(i + 1) % vertices.len()]);
                    if segment(a, b, 0.0) {
                        return true;
                    }
                    if (a.1 > p.y) != (b.1 > p.y)
                        && p.x < (b.0 - a.0) * (p.y - a.1) / (b.1 - a.1) + a.0
                    {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }
}

/// Optional interior paint for rectangles and ellipses. Other tools ignore it.
/// The scale multiplies the annotation color's alpha; the outline is unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillMode {
    #[default]
    None,
    Solid,
    Translucent,
}
impl FillMode {
    pub const fn alpha_scale(self) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Solid => Some(255),
            Self::Translucent => Some(128),
        }
    }
}

/// Pattern used for line, arrow, box and freehand strokes. Gaps remain selectable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArrowEnds {
    #[default]
    Single,
    Double,
    /// A filled round endpoint replaces the arrowhead.
    Dot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RectangleCorners {
    #[default]
    Square,
    /// 12 image pixels, limited to half the shortest side.
    Rounded,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub color: Color,
    pub width: f32,
    pub font_size: f32,
    pub fill: FillMode,
    pub line_style: LineStyle,
    pub arrow_ends: ArrowEnds,
    pub rectangle_corners: RectangleCorners,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            color: Color::PALETTE_RED,
            width: 4.0,
            font_size: 28.0,
            fill: FillMode::None,
            line_style: LineStyle::Solid,
            arrow_ends: ArrowEnds::Single,
            rectangle_corners: RectangleCorners::Square,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShapeKind {
    Line {
        start: Point,
        end: Point,
    },
    Arrow {
        start: Point,
        end: Point,
    },
    Rectangle {
        start: Point,
        end: Point,
    },
    Ellipse {
        start: Point,
        end: Point,
    },
    Highlight {
        start: Point,
        end: Point,
    },
    Blur {
        start: Point,
        end: Point,
    },
    /// Gaussian-like softening; samples the sanitized original image.
    GaussianBlur {
        start: Point,
        end: Point,
    },
    /// Opaque black area applied after all other annotations.
    Redact {
        start: Point,
        end: Point,
    },
    Freehand {
        points: Vec<Point>,
    },
    Text {
        location: Point,
        text: String,
    },
    /// Captured cursor data, independent of the native handle.
    Cursor {
        data: CursorSprite,
    },
    /// Source-independent dimming outside the focus rectangle.
    Spotlight {
        data: Spotlight,
    },
    /// Magnifies the globally redaction-sanitized source into a viewport.
    Magnifier {
        data: Magnifier,
    },
    /// The heap payload keeps the inline size and history cost of other shapes stable.
    Callout {
        data: Box<CalloutData>,
    },
    /// Square symbol size in image pixels is stored in `Style::font_size`.
    Stamp {
        center: Point,
        kind: StampKind,
    },
    Step {
        center: Point,
        number: i32,
    },
}
/// Randomized pixelation state, deliberately omitted from debug output.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PixelationSeed(pub u64);
impl std::fmt::Debug for PixelationSeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PixelationSeed([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub style: Style,
    pub kind: ShapeKind,
    pub pixelation_seed: Option<PixelationSeed>,
}
impl Shape {
    pub fn new(kind: ShapeKind) -> Self {
        Self {
            style: Style::default(),
            kind,
            pixelation_seed: None,
        }
    }
    /// Assign the caller-provided operating-system seed to a pixelation shape.
    pub fn with_pixelation_seed(mut self, seed: u64) -> Self {
        self.pixelation_seed = Some(PixelationSeed(seed));
        self
    }
    pub fn bounds(&self) -> Rect {
        match &self.kind {
            ShapeKind::Cursor { data } => data.bounds(),
            ShapeKind::Spotlight { data } => data.bounds(),
            ShapeKind::Magnifier { data } => data.bounds(),
            ShapeKind::Callout { data } => data
                .geometry()
                .map(|geometry| geometry.bounds)
                .unwrap_or_default(),
            ShapeKind::Arrow { start, end } if self.style.arrow_ends != ArrowEnds::Single => {
                styled_arrow_bounds(*start, *end, &self.style).unwrap_or_default()
            }
            ShapeKind::Rectangle { start, end }
                if self.style.rectangle_corners == RectangleCorners::Rounded
                    && !rectangle_has_zero_axis(*start, *end) =>
            {
                let bounds = Rect::from_points(*start, *end);
                if rounded_rectangle_radius(bounds).is_some() {
                    bounds
                } else {
                    Rect::default()
                }
            }
            ShapeKind::Line { start, end }
            | ShapeKind::Arrow { start, end }
            | ShapeKind::Rectangle { start, end }
            | ShapeKind::Ellipse { start, end }
            | ShapeKind::Highlight { start, end }
            | ShapeKind::Blur { start, end }
            | ShapeKind::GaussianBlur { start, end }
            | ShapeKind::Redact { start, end } => Rect::from_points(*start, *end),
            ShapeKind::Freehand { points } => points
                .iter()
                .fold(None::<(Point, Point)>, |acc, p| {
                    Some(match acc {
                        None => (*p, *p),
                        Some((lo, hi)) => (
                            Point::new(lo.x.min(p.x), lo.y.min(p.y)),
                            Point::new(hi.x.max(p.x), hi.y.max(p.y)),
                        ),
                    })
                })
                .map(|(a, b)| Rect::from_points(a, b))
                .unwrap_or_default(),
            ShapeKind::Text { location, text } => {
                // Approximate glyph extents until the platform renderer supplies
                // exact font metrics; the old 10px placeholder made most text
                // impossible to select. Windows DPI/font tests remain required.
                let size = self.style.font_size.max(1.0);
                let chars = text.chars().count().max(1) as f32;
                Rect {
                    x: location.x,
                    y: location.y,
                    width: (chars * size * 0.65).max(10.0),
                    height: (size * 1.4).max(10.0),
                }
            }
            ShapeKind::Stamp { center, .. } => {
                let size = self.style.font_size;
                Rect {
                    x: center.x - size / 2.0,
                    y: center.y - size / 2.0,
                    width: size,
                    height: size,
                }
            }
            ShapeKind::Step { center, .. } => {
                let r = 10.0 + self.style.width * 2.2;
                Rect {
                    x: center.x - r,
                    y: center.y - r,
                    width: 2.0 * r,
                    height: 2.0 * r,
                }
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        match &self.kind {
            ShapeKind::Callout { data } => !data.validate() || data.text.trim().is_empty(),
            ShapeKind::Cursor { data } => !data.validate(),
            ShapeKind::Spotlight { data } => {
                data.validate().is_err() || data.focus.width < 3.0 || data.focus.height < 3.0
            }
            ShapeKind::Magnifier { data } => {
                data.validate().is_err() || data.viewport.width < 3.0 || data.viewport.height < 3.0
            }
            ShapeKind::Line { start, end }
            | ShapeKind::Arrow { start, end }
            | ShapeKind::Rectangle { start, end }
            | ShapeKind::Ellipse { start, end }
            | ShapeKind::Highlight { start, end }
            | ShapeKind::Blur { start, end }
            | ShapeKind::GaussianBlur { start, end }
            | ShapeKind::Redact { start, end } => {
                (start.x - end.x).abs() < 3.0 && (start.y - end.y).abs() < 3.0
            }
            ShapeKind::Freehand { points } => points.len() < 2,
            _ => false,
        }
    }
    pub fn offset(&mut self, dx: f32, dy: f32) {
        let add = |p: &mut Point| {
            p.x += dx;
            p.y += dy;
        };
        match &mut self.kind {
            ShapeKind::Cursor { data } => {
                let _ = data.offset(dx, dy);
            }
            ShapeKind::Spotlight { data } => {
                let _ = data.offset(dx, dy);
            }
            ShapeKind::Magnifier { data } => {
                let _ = data.offset(dx, dy);
            }
            ShapeKind::Callout { data } => {
                data.offset(dx, dy);
            }
            ShapeKind::Line { start, end }
            | ShapeKind::Arrow { start, end }
            | ShapeKind::Rectangle { start, end }
            | ShapeKind::Ellipse { start, end }
            | ShapeKind::Highlight { start, end }
            | ShapeKind::Blur { start, end }
            | ShapeKind::GaussianBlur { start, end }
            | ShapeKind::Redact { start, end } => {
                add(start);
                add(end)
            }
            ShapeKind::Freehand { points } => {
                for p in points {
                    add(p)
                }
            }
            ShapeKind::Text { location, .. } => add(location),
            ShapeKind::Step { center, .. } | ShapeKind::Stamp { center, .. } => add(center),
        }
    }
    /// Two endpoint handles for lines; four normalized corner handles for box annotations.
    pub fn handles(&self) -> Vec<Point> {
        match &self.kind {
            ShapeKind::Spotlight { data } => data.handles().to_vec(),
            ShapeKind::Magnifier { data } => data.handles().to_vec(),
            ShapeKind::Callout { data } => {
                if data.validate() {
                    data.tip.into_iter().collect()
                } else {
                    vec![]
                }
            }
            ShapeKind::Line { start, end } | ShapeKind::Arrow { start, end } => vec![*start, *end],
            ShapeKind::Stamp { .. }
            | ShapeKind::Rectangle { .. }
            | ShapeKind::Ellipse { .. }
            | ShapeKind::Highlight { .. }
            | ShapeKind::Blur { .. }
            | ShapeKind::GaussianBlur { .. }
            | ShapeKind::Redact { .. } => self.bounds().corners().to_vec(),
            _ => vec![],
        }
    }
    /// Resize by handle index. Box handles retain the original corner identity when crossing over.
    pub fn set_handle(&mut self, index: usize, p: Point) -> bool {
        match &mut self.kind {
            ShapeKind::Spotlight { data } => return data.set_handle(index, p).is_ok(),
            ShapeKind::Magnifier { data } => return data.set_handle(index, p).is_ok(),
            ShapeKind::Callout { data } => {
                return index == 0 && data.tip.is_some() && data.set_tip(Some(p))
            }
            ShapeKind::Stamp { center, .. } => {
                if index >= 4 || !p.x.is_finite() || !p.y.is_finite() {
                    return false;
                }
                // Keep the center fixed and preserve the symbol's aspect ratio.
                self.style.font_size =
                    (2.0 * (p.x - center.x).abs().max((p.y - center.y).abs())).clamp(1.0, 512.0);
            }
            ShapeKind::Line { start, end } | ShapeKind::Arrow { start, end } => match index {
                0 => *start = p,
                1 => *end = p,
                _ => return false,
            },
            ShapeKind::Rectangle { start, end }
            | ShapeKind::Ellipse { start, end }
            | ShapeKind::Highlight { start, end }
            | ShapeKind::Blur { start, end }
            | ShapeKind::GaussianBlur { start, end }
            | ShapeKind::Redact { start, end } => {
                let r = Rect::from_points(*start, *end);
                let (mut l, mut t, mut right, mut b) = (r.x, r.y, r.x + r.width, r.y + r.height);
                match index {
                    0 => {
                        l = p.x;
                        t = p.y
                    }
                    1 => {
                        right = p.x;
                        t = p.y
                    }
                    2 => {
                        right = p.x;
                        b = p.y
                    }
                    3 => {
                        l = p.x;
                        b = p.y
                    }
                    _ => return false,
                }
                *start = Point::new(l, t);
                *end = Point::new(right, b);
            }
            _ => return false,
        }
        true
    }
    /// Hit testing is geometric. Text uses placeholder bounds; rectangle outlines
    /// use an axis-aligned outer-minus-inner approximation.
    pub fn hit_test(&self, p: Point, tolerance: f32) -> bool {
        match &self.kind {
            ShapeKind::Cursor { data } => {
                data.validate()
                    && tolerance.is_finite()
                    && tolerance >= 0.0
                    && data.bounds().inflated(tolerance).contains(p)
            }
            ShapeKind::Spotlight { data } => data.hit_test(p, tolerance),
            ShapeKind::Magnifier { data } => data.hit_test(p, tolerance),
            ShapeKind::Callout { data } => data
                .geometry()
                .is_some_and(|geometry| geometry.hit_test(p, tolerance)),
            ShapeKind::Stamp { kind, .. } => {
                let bounds = self.bounds();
                let size = self.style.font_size;
                size.is_finite()
                    && size > 0.0
                    && kind.hit_test(
                        Point::new((p.x - bounds.x) / size, (p.y - bounds.y) / size),
                        tolerance / size,
                    )
            }
            ShapeKind::Arrow { start, end } if self.style.arrow_ends != ArrowEnds::Single => {
                styled_arrow_hit_test(*start, *end, &self.style, p, tolerance)
            }
            ShapeKind::Line { start, end } | ShapeKind::Arrow { start, end } => {
                distance_segment(p, *start, *end) <= tolerance + self.style.width / 2.0
            }
            ShapeKind::Freehand { points } => points
                .windows(2)
                .any(|w| distance_segment(p, w[0], w[1]) <= tolerance + self.style.width / 2.0),
            ShapeKind::Ellipse { start, end } => {
                let r = Rect::from_points(*start, *end);
                let pad = tolerance + self.style.width / 2.0;
                let d = distance_to_ellipse(
                    p,
                    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0),
                    r.width / 2.0,
                    r.height / 2.0,
                );
                let inside = r.width > 0.0
                    && r.height > 0.0
                    && (((p.x as f64 - r.x as f64) / r.width as f64 - 0.5) * 2.0)
                        .hypot(((p.y as f64 - r.y as f64) / r.height as f64 - 0.5) * 2.0)
                        <= 1.0;
                d.is_finite()
                    && pad.is_finite()
                    && pad >= 0.0
                    && (d <= pad
                        || (self.style.fill != FillMode::None && self.style.color.a > 0 && inside))
            }
            ShapeKind::Rectangle { start, end }
                if self.style.rectangle_corners == RectangleCorners::Rounded
                    && !rectangle_has_zero_axis(*start, *end) =>
            {
                let pad = tolerance + self.style.width / 2.0;
                let Some(distance) = rounded_rectangle_distance(p, Rect::from_points(*start, *end))
                else {
                    return false;
                };
                pad.is_finite()
                    && pad >= 0.0
                    && (distance.abs() <= pad as f64
                        || (self.style.fill != FillMode::None
                            && self.style.color.a > 0
                            && distance <= 0.0))
            }
            ShapeKind::Rectangle { .. } => {
                let r = self.bounds();
                let pad = tolerance + self.style.width / 2.0;
                let outer = r.inflated(pad);
                let inner = Rect {
                    x: r.x + pad,
                    y: r.y + pad,
                    width: (r.width - 2.0 * pad).max(0.0),
                    height: (r.height - 2.0 * pad).max(0.0),
                };
                outer.contains(p)
                    && ((self.style.fill != FillMode::None && self.style.color.a > 0)
                        || !inner.contains(p)
                        || inner.width == 0.0
                        || inner.height == 0.0)
            }
            _ => self.bounds().inflated(tolerance).contains(p),
        }
    }
}
/// Shape list with snapshot undo/redo. Each successful mutation records one undo point.
#[derive(Clone, Debug, Default)]
pub struct ShapeDocument {
    shapes: Vec<Shape>,
    undo: Vec<Vec<Shape>>,
    redo: Vec<Vec<Shape>>,
}
impl ShapeDocument {
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }
    pub fn push(&mut self, shape: Shape) {
        self.checkpoint();
        self.shapes.push(shape)
    }
    pub fn remove(&mut self, index: usize) -> Option<Shape> {
        if index >= self.shapes.len() {
            return None;
        }
        self.checkpoint();
        Some(self.shapes.remove(index))
    }
    pub fn edit<F>(&mut self, index: usize, edit: F) -> bool
    where
        F: FnOnce(&mut Shape),
    {
        if index >= self.shapes.len() {
            return false;
        }
        self.checkpoint();
        edit(&mut self.shapes[index]);
        true
    }
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.shapes, prev));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.shapes, next));
            true
        } else {
            false
        }
    }
    fn checkpoint(&mut self) {
        const MAX_UNDO_STATES: usize = 100;
        if self.undo.len() >= MAX_UNDO_STATES {
            self.undo.remove(0);
        }
        self.undo.push(self.shapes.clone());
        self.redo.clear()
    }
}

#[cfg(test)]
mod tests;
