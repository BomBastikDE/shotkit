//! Renderer-independent annotation geometry and edit history.
//! Coordinates are image-space pixels; rendering remains the caller's responsibility.

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

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub color: Color,
    pub width: f32,
    pub font_size: f32,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            color: Color::PALETTE_RED,
            width: 4.0,
            font_size: 28.0,
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
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub style: Style,
    pub kind: ShapeKind,
}
impl Shape {
    pub fn new(kind: ShapeKind) -> Self {
        Self {
            style: Style::default(),
            kind,
        }
    }
    pub fn bounds(&self) -> Rect {
        match &self.kind {
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
                d.is_finite() && pad.is_finite() && pad >= 0.0 && d <= pad
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
                    && (!inner.contains(p) || inner.width == 0.0 || inner.height == 0.0)
            }
            _ => self.bounds().inflated(tolerance).contains(p),
        }
    }
}
fn distance_segment(p: Point, a: Point, b: Point) -> f32 {
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
fn distance_to_ellipse(p: Point, center: Point, rx: f32, ry: f32) -> f32 {
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
mod tests {
    use super::*;
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
}
