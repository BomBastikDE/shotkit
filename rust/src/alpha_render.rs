//! Original symbol rasterization and Windows GDI+ rendering of one annotation into a transparent straight-BGRA layer.
//!
//! The output is an independent full-size image; untouched pixels remain fully
//! transparent. Blur/pixelation is deliberately rejected because it needs the
//! original image as a sampling source and is a document-level operation.

use crate::resource_budget::{AdmissionClass, BudgetError, BudgetLease, ResourceBudget};
use crate::shapes::{Shape, ShapeKind};
use crate::stitch::Image;
mod callout;
mod stamps;
pub use callout::measure_callout_text;
#[cfg(windows)]
mod styles;
#[cfg(windows)]
use std::{ffi::c_void, ptr};

const MAX_BYTES: usize = 512 * 1024 * 1024;
#[cfg(windows)]
const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000e200b;
#[cfg(windows)]
const UNIT_PIXEL: i32 = 2;
#[cfg(windows)]
const FLUSH: i32 = 1;

/// Render errors are intentionally descriptive but do not expose GDI+ internals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    UnsupportedShape(&'static str),
    InvalidDimensions,
    AllocationFailed,
    BudgetRejected(BudgetError),
    PlatformUnavailable,
    GdiPlusStatus(i32),
}

/// Render a shape to a transparent straight-alpha BGRA layer of `width` x `height`.
/// Shape coordinates are image-space pixels. Output bounds are clipped by GDI+.
pub fn render_shape(width: usize, height: usize, shape: &Shape) -> Result<Image, RenderError> {
    let bytes = validate_request(width, height, shape)?;
    render_validated_shape(width, height, shape, bytes, None)
}

/// A temporary layer keeps its admitted pixel capacity through rendering and
/// document composition. Fields drop in declaration order: pixels before lease.
/// No mutable pixels or detached image escape from a budgeted layer.
pub(crate) struct ShapeLayer {
    image: Image,
    pixel_lease: Option<BudgetLease>,
}

impl ShapeLayer {
    pub(crate) fn image(&self) -> &Image {
        if let Some(lease) = &self.pixel_lease {
            debug_assert_eq!(lease.bytes(), self.image.pixels.capacity());
        }
        &self.image
    }
}

/// Admission uses the caller's existing pool, including reserved headroom.
/// The optional pool preserves the unbudgeted document API's existing behavior.
pub(crate) fn render_shape_layer(
    width: usize,
    height: usize,
    shape: &Shape,
    budget: Option<&ResourceBudget>,
) -> Result<ShapeLayer, RenderError> {
    let bytes = validate_request(width, height, shape)?;
    // The lease is owned before the inner renderer allocates. It survives GDI
    // startup, drawing, flush, every native disposer and session shutdown.
    let mut pixel_lease = budget
        .map(|budget| {
            budget
                .try_acquire(bytes, AdmissionClass::Reserved)
                .map_err(RenderError::BudgetRejected)
        })
        .transpose()?;
    let image = render_validated_shape(width, height, shape, bytes, pixel_lease.as_mut())?;
    Ok(ShapeLayer { image, pixel_lease })
}

fn validate_request(width: usize, height: usize, shape: &Shape) -> Result<usize, RenderError> {
    if matches!(
        shape.kind,
        ShapeKind::Spotlight { .. } | ShapeKind::Magnifier { .. } | ShapeKind::Cursor { .. }
    ) {
        return Err(RenderError::UnsupportedShape(
            "Effekt benötigt Dokumentkomposition",
        ));
    }
    if matches!(shape.kind, ShapeKind::Redact { .. }) {
        return Err(RenderError::UnsupportedShape(
            "Schwärzen ist eine Dokumentoperation",
        ));
    }
    if matches!(shape.kind, ShapeKind::Blur { .. }) {
        return Err(RenderError::UnsupportedShape(
            "Verpixeln benötigt das Originalbild als Quelle",
        ));
    }
    if matches!(shape.kind, ShapeKind::GaussianBlur { .. }) {
        return Err(RenderError::UnsupportedShape(
            "Weichzeichnen benötigt das Originalbild als Quelle",
        ));
    }
    width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| {
            width > 0
                && height > 0
                && *n <= MAX_BYTES
                && width <= i32::MAX as usize
                && height <= i32::MAX as usize
        })
        .ok_or(RenderError::InvalidDimensions)
}

fn render_validated_shape(
    width: usize,
    height: usize,
    shape: &Shape,
    bytes: usize,
    lease: Option<&mut BudgetLease>,
) -> Result<Image, RenderError> {
    let mut pargb = allocate_layer_pixels(bytes, lease)?;

    if let ShapeKind::Stamp { center, kind } = shape.kind {
        stamps::render(width, height, center, kind, &shape.style, &mut pargb)?;
        return Image::new(width, height, pargb).map_err(|_| RenderError::InvalidDimensions);
    }

    #[cfg(windows)]
    {
        render_gdiplus(width, height, shape, &mut pargb)?;
        // GDI+ PARGB stores premultiplied channels. Convert back to straight BGRA.
        for pixel in pargb.as_chunks_mut::<4>().0 {
            let a = pixel[3];
            if a != 0 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * 255 + u16::from(a) / 2) / u16::from(a))
                        .min(255) as u8;
                }
            } else {
                pixel[..3].fill(0);
            }
        }
        Image::new(width, height, pargb).map_err(|_| RenderError::InvalidDimensions)
    }
    #[cfg(not(windows))]
    {
        let _ = (width, height, shape, pargb);
        Err(RenderError::PlatformUnavailable)
    }
}

// Reconcile actual capacity BEFORE zeroing or any backend/GDI call. On a
// rejected growth this local Vec drops while the outer caller still owns lease.
fn allocate_layer_pixels(
    bytes: usize,
    lease: Option<&mut BudgetLease>,
) -> Result<Vec<u8>, RenderError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(bytes)
        .map_err(|_| RenderError::AllocationFailed)?;
    reconcile_layer_capacity(&pixels, lease)?;
    pixels.resize(bytes, 0);
    Ok(pixels)
}

fn reconcile_layer_capacity(
    pixels: &Vec<u8>,
    lease: Option<&mut BudgetLease>,
) -> Result<(), RenderError> {
    if let Some(lease) = lease {
        lease
            .try_resize(pixels.capacity())
            .map_err(RenderError::BudgetRejected)?;
    }
    Ok(())
}

// Preserve the previous max/min behavior for NaN: use the minimum size.
#[cfg(any(windows, test))]
fn bounded_size(value: f32, maximum: f32) -> f32 {
    if value.is_nan() {
        1.0
    } else {
        value.clamp(1.0, maximum)
    }
}

#[cfg(windows)]
#[repr(C)]
struct GdiplusStartupInput {
    version: u32,
    debug_callback: *mut c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}
#[cfg(any(windows, test))]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct PointF {
    x: f32,
    y: f32,
}
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct ArrowGeometry {
    tip: PointF,
    left: PointF,
    right: PointF,
    shaft_end: Option<PointF>,
}

/// Calculate export arrow geometry without relying on platform drawing APIs.
/// Non-finite, unrepresentable, and sub-pixel arrows produce no geometry.
#[cfg(any(windows, test))]
fn arrow_geometry(
    start: crate::shapes::Point,
    end: crate::shapes::Point,
    width: f32,
) -> Option<ArrowGeometry> {
    let (sx, sy, ex, ey) = (start.x as f64, start.y as f64, end.x as f64, end.y as f64);
    if ![sx, sy, ex, ey, width as f64].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (dx, dy) = (ex - sx, ey - sy);
    let len = dx.hypot(dy);
    if !len.is_finite() || len < 1.0 {
        return None;
    }
    let width = (width as f64).clamp(1.0, 256.0);
    let head = (width * 4.0).max(8.0).min(len * 0.6);
    let (ux, uy) = (dx / len, dy / len);
    let shaft_fits = len > head && head * 0.45 * 0.8 >= width * 0.5;
    // If the shaft cannot meet the head without protruding, extend the filled
    // head back to the arrow start and make its base at least stroke-width wide.
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
    let (nx, ny) = (-uy * half_base, ux * half_base);
    let point = |x: f64, y: f64| {
        if x.is_finite()
            && y.is_finite()
            && x.abs() <= f32::MAX as f64
            && y.abs() <= f32::MAX as f64
        {
            Some(PointF {
                x: x as f32,
                y: y as f32,
            })
        } else {
            None
        }
    };
    let tip = point(ex, ey)?;
    let left = point(bx + nx, by + ny)?;
    let right = point(bx - nx, by - ny)?;

    // Stop the shaft inside the head, as the GDI renderer does. For a short,
    // wide arrow the triangle may be narrower than the stroke; omit the shaft
    // in that case so its square end cannot protrude through the head sides.
    let shaft_end = if shaft_fits {
        Some(point(ex - ux * head * 0.8, ey - uy * head * 0.8)?)
    } else {
        None
    };
    Some(ArrowGeometry {
        tip,
        left,
        right,
        shaft_end,
    })
}
#[cfg(windows)]
#[repr(C)]
struct RectF {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[cfg(windows)]
type GdiDisposer = unsafe extern "system" fn(*mut c_void) -> i32;
#[cfg(windows)]
struct GdiResource {
    raw: *mut c_void,
    dispose: GdiDisposer,
}
#[cfg(windows)]
impl GdiResource {
    // SAFETY CONTRACT:
    // Voraussetzung: raw ist null oder eine exklusiv übertragene lebende GDI+-Ressource des zu dispose passenden Typs.
    // Warum erfüllt bzw. offen: Aktuelle Aufrufer prüfen Create-Status und verwenden passende Freigaben; Funktion prüft selbst nur null.
    // Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
    unsafe fn new(raw: *mut c_void, dispose: GdiDisposer) -> Result<Self, RenderError> {
        if raw.is_null() {
            Err(RenderError::GdiPlusStatus(2))
        } else {
            Ok(Self { raw, dispose })
        }
    }
    fn as_ptr(&self) -> *mut c_void {
        self.raw
    }
}
#[cfg(windows)]
impl Drop for GdiResource {
    fn drop(&mut self) {
        unsafe {
            (self.dispose)(self.raw);
        }
    }
}
#[cfg(windows)]
struct GdiPlusSession(usize);
#[cfg(windows)]
impl Drop for GdiPlusSession {
    fn drop(&mut self) {
        unsafe {
            GdiplusShutdown(self.0);
        }
    }
}

#[cfg(windows)]
#[link(name = "gdiplus")]
extern "system" {
    fn GdiplusStartup(
        token: *mut usize,
        input: *const GdiplusStartupInput,
        output: *mut c_void,
    ) -> i32;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateBitmapFromScan0(
        w: i32,
        h: i32,
        stride: i32,
        format: i32,
        scan0: *mut u8,
        bitmap: *mut *mut c_void,
    ) -> i32;
    fn GdipDisposeImage(image: *mut c_void) -> i32;
    fn GdipGetImageGraphicsContext(image: *mut c_void, graphics: *mut *mut c_void) -> i32;
    fn GdipDeleteGraphics(graphics: *mut c_void) -> i32;
    fn GdipSetSmoothingMode(graphics: *mut c_void, mode: i32) -> i32;
    fn GdipSetTextRenderingHint(graphics: *mut c_void, mode: i32) -> i32;
    fn GdipFlush(graphics: *mut c_void, intention: i32) -> i32;
    fn GdipCreatePen1(color: u32, width: f32, unit: i32, pen: *mut *mut c_void) -> i32;
    fn GdipDeletePen(pen: *mut c_void) -> i32;
    fn GdipSetPenDashStyle(pen: *mut c_void, dash_style: i32) -> i32;
    fn GdipCreateSolidFill(color: u32, brush: *mut *mut c_void) -> i32;
    fn GdipDeleteBrush(brush: *mut c_void) -> i32;
    fn GdipDrawLine(
        graphics: *mut c_void,
        pen: *mut c_void,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> i32;
    fn GdipDrawLines(
        graphics: *mut c_void,
        pen: *mut c_void,
        points: *const PointF,
        count: i32,
    ) -> i32;
    fn GdipDrawRectangle(
        graphics: *mut c_void,
        pen: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;
    fn GdipDrawEllipse(
        graphics: *mut c_void,
        pen: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;
    fn GdipFillEllipse(
        graphics: *mut c_void,
        brush: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;
    fn GdipCreateStringFormat(flags: i32, language: u16, format: *mut *mut c_void) -> i32;
    fn GdipDeleteStringFormat(format: *mut c_void) -> i32;
    fn GdipSetStringFormatAlign(format: *mut c_void, align: i32) -> i32;
    fn GdipSetStringFormatLineAlign(format: *mut c_void, align: i32) -> i32;
    fn GdipFillRectangle(
        graphics: *mut c_void,
        brush: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> i32;
    fn GdipFillPolygon(
        graphics: *mut c_void,
        brush: *mut c_void,
        points: *const PointF,
        count: i32,
        fill_mode: i32,
    ) -> i32;
    fn GdipCreateFontFamilyFromName(
        name: *const u16,
        collection: *mut c_void,
        family: *mut *mut c_void,
    ) -> i32;
    fn GdipDeleteFontFamily(family: *mut c_void) -> i32;
    fn GdipCreateFont(
        family: *mut c_void,
        size: f32,
        style: i32,
        unit: i32,
        font: *mut *mut c_void,
    ) -> i32;
    fn GdipDeleteFont(font: *mut c_void) -> i32;
    fn GdipDrawString(
        graphics: *mut c_void,
        text: *const u16,
        length: i32,
        font: *mut c_void,
        layout: *const RectF,
        format: *mut c_void,
        brush: *mut c_void,
    ) -> i32;
}

#[cfg(windows)]
fn status(result: i32) -> Result<(), RenderError> {
    if result == 0 {
        Ok(())
    } else {
        Err(RenderError::GdiPlusStatus(result))
    }
}
#[cfg(windows)]
fn argb(c: crate::shapes::Color, alpha_scale: u8) -> u32 {
    let a = (u16::from(c.a) * u16::from(alpha_scale) + 127) / 255;
    (u32::from(a as u8) << 24) | (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
}

#[cfg(windows)]
fn render_gdiplus(
    width: usize,
    height: usize,
    shape: &Shape,
    pixels: &mut [u8],
) -> Result<(), RenderError> {
    let input = GdiplusStartupInput {
        version: 1,
        debug_callback: ptr::null_mut(),
        suppress_background_thread: 0,
        suppress_external_codecs: 0,
    };
    let mut token = 0usize;
    status(unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) })?;
    let _session = GdiPlusSession(token);
    unsafe { render_started(width, height, shape, pixels) }
}

#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: GDI+ bleibt aktiv und pixels umfasst exklusiv width*height*4 gültige Bytes bis zum Bitmap-Drop.
// Warum erfüllt bzw. offen: Aufrufer prüft Maße und Obergrenze; lokale Guards enden vor der Session.
// Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
unsafe fn render_started(
    width: usize,
    height: usize,
    shape: &Shape,
    pixels: &mut [u8],
) -> Result<(), RenderError> {
    let mut bitmap = ptr::null_mut();
    status(GdipCreateBitmapFromScan0(
        width as i32,
        height as i32,
        (width * 4) as i32,
        PIXEL_FORMAT_32BPP_PARGB,
        pixels.as_mut_ptr(),
        &mut bitmap,
    ))?;
    let bitmap = GdiResource::new(bitmap, GdipDisposeImage)?;
    let mut graphics_raw = ptr::null_mut();
    status(GdipGetImageGraphicsContext(
        bitmap.as_ptr(),
        &mut graphics_raw,
    ))?;
    let graphics = GdiResource::new(graphics_raw, GdipDeleteGraphics)?;
    status(GdipSetSmoothingMode(graphics.as_ptr(), 4))?;
    draw_shape(graphics.as_ptr(), shape)?;
    // Errors here unwind through both guards, dropping Graphics before Bitmap.
    status(GdipFlush(graphics.as_ptr(), FLUSH))?;
    Ok(())
}

#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: g ist ein gültiges exklusiv verwendetes GDI+-Graphics einer aktiven Session.
// Warum erfüllt bzw. offen: render_started besitzt Graphics und Zielbild während des Aufrufs; lokale Daten leben über die FFI-Aufrufe.
// Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
unsafe fn draw_shape(g: *mut c_void, shape: &Shape) -> Result<(), RenderError> {
    use crate::shapes::ShapeKind;
    let width = bounded_size(shape.style.width, 256.0);
    let pen_color = argb(shape.style.color, 255);
    let mut pen_raw = ptr::null_mut();
    status(GdipCreatePen1(pen_color, width, UNIT_PIXEL, &mut pen_raw))?;
    let pen = GdiResource::new(pen_raw, GdipDeletePen)?;
    if matches!(
        shape.kind,
        ShapeKind::Line { .. }
            | ShapeKind::Arrow { .. }
            | ShapeKind::Rectangle { .. }
            | ShapeKind::Ellipse { .. }
            | ShapeKind::Freehand { .. }
    ) {
        styles::set_line_style(pen.as_ptr(), shape.style.line_style)?;
    }
    match &shape.kind {
        ShapeKind::Line { start, end } => status(GdipDrawLine(
            g,
            pen.as_ptr(),
            start.x,
            start.y,
            end.x,
            end.y,
        )),
        ShapeKind::Arrow { .. } if shape.style.arrow_ends != crate::shapes::ArrowEnds::Single => {
            styles::draw_arrow(g, pen.as_ptr(), shape)
        }
        ShapeKind::Arrow { start, end } => {
            let Some(geometry) = arrow_geometry(*start, *end, width) else {
                return Ok(());
            };
            if let Some(shaft_end) = geometry.shaft_end {
                status(GdipDrawLine(
                    g,
                    pen.as_ptr(),
                    start.x,
                    start.y,
                    shaft_end.x,
                    shaft_end.y,
                ))?;
            }
            let pts = [geometry.tip, geometry.left, geometry.right];
            let brush = create_brush(pen_color)?;
            status(GdipFillPolygon(g, brush.as_ptr(), pts.as_ptr(), 3, 0))
        }
        ShapeKind::Rectangle { start, end }
            if shape.style.rectangle_corners == crate::shapes::RectangleCorners::Rounded
                && !crate::shapes::rectangle_has_zero_axis(*start, *end) =>
        {
            styles::draw_rounded_rectangle(g, pen.as_ptr(), shape, *start, *end)
        }
        ShapeKind::Rectangle { start, end } => {
            let (x, y, w, h) = rect(*start, *end);
            if let Some(alpha) = shape.style.fill.alpha_scale() {
                let brush = create_brush(argb(shape.style.color, alpha))?;
                status(GdipFillRectangle(g, brush.as_ptr(), x, y, w, h))?;
            }
            status(GdipDrawRectangle(g, pen.as_ptr(), x, y, w, h))
        }
        ShapeKind::Ellipse { start, end } => {
            let (x, y, w, h) = rect(*start, *end);
            if let Some(alpha) = shape.style.fill.alpha_scale() {
                let brush = create_brush(argb(shape.style.color, alpha))?;
                status(GdipFillEllipse(g, brush.as_ptr(), x, y, w, h))?;
            }
            status(GdipDrawEllipse(g, pen.as_ptr(), x, y, w, h))
        }
        ShapeKind::Highlight { start, end } => {
            let (x, y, w, h) = rect(*start, *end);
            let brush = create_brush(argb(shape.style.color, 105))?;
            status(GdipFillRectangle(g, brush.as_ptr(), x, y, w, h))
        }
        ShapeKind::Freehand { points } => {
            if points.len() < 2 {
                return Ok(());
            }
            if points.len() > 1_000_000 {
                return Err(RenderError::InvalidDimensions);
            }
            let mut pts = Vec::new();
            pts.try_reserve_exact(points.len())
                .map_err(|_| RenderError::AllocationFailed)?;
            pts.extend(points.iter().map(|p| PointF { x: p.x, y: p.y }));
            status(GdipDrawLines(
                g,
                pen.as_ptr(),
                pts.as_ptr(),
                pts.len() as i32,
            ))
        }
        ShapeKind::Callout { data } => callout::draw(g, pen.as_ptr(), data),
        ShapeKind::Spotlight { .. } | ShapeKind::Magnifier { .. } | ShapeKind::Cursor { .. } => {
            Err(RenderError::UnsupportedShape(
                "Effekt benötigt Dokumentkomposition",
            ))
        }
        ShapeKind::Text { location, text } => {
            draw_text(g, *location, text, shape.style.font_size, pen_color)
        }
        ShapeKind::Step { center, number } => {
            let radius = (10.0 + width * 2.2).max(1.0);
            let brush = create_brush(pen_color)?;
            let x = center.x - radius;
            let y = center.y - radius;
            status(GdipFillEllipse(
                g,
                brush.as_ptr(),
                x,
                y,
                radius * 2.0,
                radius * 2.0,
            ))?;

            // Match StepShape: a crisp white ring and centered, bold, contrasting
            // numeral. A bounded layout rect avoids the old fixed left/top offset,
            // which pushed multi-digit labels off center and out of the marker.
            let white = argb(crate::shapes::Color::rgba(255, 255, 255, 255), 255);
            let ring = create_pen(white, (radius / 8.0).max(2.0))?;
            status(GdipDrawEllipse(
                g,
                ring.as_ptr(),
                x,
                y,
                radius * 2.0,
                radius * 2.0,
            ))?;
            let text = number.to_string();
            let text_color = step_text_color(shape.style.color);
            draw_centered_text(g, *center, radius, &text, text_color)
        }
        // Stamp is dispatched to the shared rasterizer before GDI+ starts.
        ShapeKind::Stamp { .. } => Err(RenderError::UnsupportedShape(
            "Stempel verwendet den eigenen Rasterrenderer",
        )),
        ShapeKind::Redact { .. } => Err(RenderError::UnsupportedShape(
            "Schwärzen ist eine Dokumentoperation",
        )),
        ShapeKind::Blur { .. } => Err(RenderError::UnsupportedShape(
            "Verpixeln benötigt das Originalbild als Quelle",
        )),
        ShapeKind::GaussianBlur { .. } => Err(RenderError::UnsupportedShape(
            "Weichzeichnen benötigt das Originalbild als Quelle",
        )),
    }
}

#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: Die aktive GDI+-Session überlebt den zurückgegebenen Brush und seinen Drop.
// Warum erfüllt bzw. offen: Status wird geprüft und der passende Disposer gespeichert; Aufrufe erfolgen innerhalb der Session.
// Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
unsafe fn create_brush(color: u32) -> Result<GdiResource, RenderError> {
    let mut raw = ptr::null_mut();
    status(GdipCreateSolidFill(color, &mut raw))?;
    GdiResource::new(raw, GdipDeleteBrush)
}

#[cfg(windows)]
fn rect(a: crate::shapes::Point, b: crate::shapes::Point) -> (f32, f32, f32, f32) {
    (
        a.x.min(b.x),
        a.y.min(b.y),
        (a.x - b.x).abs().max(1.0),
        (a.y - b.y).abs().max(1.0),
    )
}
#[cfg(any(windows, test))]
mod text;
#[cfg(test)]
use text::step_font_size;
#[cfg(all(test, not(windows)))]
use text::step_text_color;
#[cfg(windows)]
use text::{create_pen, draw_centered_text, draw_text, step_text_color};

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_pixel_tests;
