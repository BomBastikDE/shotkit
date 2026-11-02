//! Windows GDI+ rendering of one annotation into a transparent straight-BGRA layer.
//!
//! The output is an independent full-size image; untouched pixels remain fully
//! transparent. Blur/pixelation is deliberately rejected because it needs the
//! original image as a sampling source and is a document-level operation.

use crate::shapes::{Shape, ShapeKind};
use crate::stitch::Image;
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
    PlatformUnavailable,
    GdiPlusStatus(i32),
}

/// Render a shape to a transparent straight-alpha BGRA layer of `width` x `height`.
/// Shape coordinates are image-space pixels. Output bounds are clipped by GDI+.
pub fn render_shape(width: usize, height: usize, shape: &Shape) -> Result<Image, RenderError> {
    if matches!(shape.kind, ShapeKind::Blur { .. }) {
        return Err(RenderError::UnsupportedShape(
            "Verpixeln benötigt das Originalbild als Quelle",
        ));
    }
    let bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| {
            width > 0
                && height > 0
                && *n <= MAX_BYTES
                && width <= i32::MAX as usize
                && height <= i32::MAX as usize
        })
        .ok_or(RenderError::InvalidDimensions)?;
    let mut pargb = Vec::new();
    pargb
        .try_reserve_exact(bytes)
        .map_err(|_| RenderError::AllocationFailed)?;
    pargb.resize(bytes, 0);

    #[cfg(windows)]
    {
        render_gdiplus(width, height, shape, &mut pargb)?;
        // GDI+ PARGB stores premultiplied channels. Convert back to straight BGRA.
        for pixel in pargb.chunks_exact_mut(4) {
            let a = pixel[3];
            if a != 0 {
                for c in 0..3 {
                    pixel[c] = ((u16::from(pixel[c]) * 255 + u16::from(a) / 2) / u16::from(a))
                        .min(255) as u8;
                }
            } else {
                pixel[..3].fill(0);
            }
        }
        return Image::new(width, height, pargb).map_err(|_| RenderError::InvalidDimensions);
    }
    #[cfg(not(windows))]
    {
        let _ = (width, height, shape, pargb);
        Err(RenderError::PlatformUnavailable)
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
#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct PointF {
    x: f32,
    y: f32,
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
unsafe fn draw_shape(g: *mut c_void, shape: &Shape) -> Result<(), RenderError> {
    use crate::shapes::ShapeKind;
    let width = shape.style.width.max(1.0).min(256.0);
    let pen_color = argb(shape.style.color, 255);
    let mut pen_raw = ptr::null_mut();
    status(GdipCreatePen1(pen_color, width, UNIT_PIXEL, &mut pen_raw))?;
    let pen = GdiResource::new(pen_raw, GdipDeletePen)?;
    match &shape.kind {
        ShapeKind::Line { start, end } => status(GdipDrawLine(
            g,
            pen.as_ptr(),
            start.x,
            start.y,
            end.x,
            end.y,
        )),
        ShapeKind::Arrow { start, end } => {
            status(GdipDrawLine(
                g,
                pen.as_ptr(),
                start.x,
                start.y,
                end.x,
                end.y,
            ))?;
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let head = (width * 4.0).max(8.0).min(len * 0.6);
            let bx = end.x - dx / len * head;
            let by = end.y - dy / len * head;
            let nx = -dy / len * head * 0.45;
            let ny = dx / len * head * 0.45;
            let pts = [
                PointF { x: end.x, y: end.y },
                PointF {
                    x: bx + nx,
                    y: by + ny,
                },
                PointF {
                    x: bx - nx,
                    y: by - ny,
                },
            ];
            let brush = create_brush(pen_color)?;
            status(GdipFillPolygon(g, brush.as_ptr(), pts.as_ptr(), 3, 0))
        }
        ShapeKind::Rectangle { start, end } => {
            let (x, y, w, h) = rect(*start, *end);
            status(GdipDrawRectangle(g, pen.as_ptr(), x, y, w, h))
        }
        ShapeKind::Ellipse { start, end } => {
            let (x, y, w, h) = rect(*start, *end);
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
        ShapeKind::Blur { .. } => Err(RenderError::UnsupportedShape(
            "Verpixeln benötigt das Originalbild als Quelle",
        )),
    }
}

#[cfg(windows)]
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
fn step_text_color(color: crate::shapes::Color) -> u32 {
    let luminance =
        f32::from(color.r) * 0.299 + f32::from(color.g) * 0.587 + f32::from(color.b) * 0.114;
    let (r, g, b) = if luminance > 150.0 {
        (40, 40, 40)
    } else {
        (255, 255, 255)
    };
    (255u32 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

fn step_font_size(radius: f32, digits: usize) -> f32 {
    let digits = digits.max(1) as f32;
    radius
        * if digits == 1.0 {
            1.05
        } else {
            // Roughly 0.65 em per digit. Keep the full label within the circle's
            // inner area as step numbers grow beyond two digits.
            1.7f32.min(1.8 / digits)
        }
}

#[cfg(windows)]
unsafe fn create_pen(color: u32, width: f32) -> Result<GdiResource, RenderError> {
    let mut raw = ptr::null_mut();
    status(GdipCreatePen1(color, width, UNIT_PIXEL, &mut raw))?;
    GdiResource::new(raw, GdipDeletePen)
}

#[cfg(windows)]
unsafe fn draw_centered_text(
    g: *mut c_void,
    center: crate::shapes::Point,
    radius: f32,
    text: &str,
    color: u32,
) -> Result<(), RenderError> {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut family_raw = ptr::null_mut();
    let name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    status(GdipCreateFontFamilyFromName(
        name.as_ptr(),
        ptr::null_mut(),
        &mut family_raw,
    ))?;
    let family = GdiResource::new(family_raw, GdipDeleteFontFamily)?;
    let mut font_raw = ptr::null_mut();
    status(GdipCreateFont(
        family.as_ptr(),
        step_font_size(radius, text.chars().count()).max(1.0),
        1,
        UNIT_PIXEL,
        &mut font_raw,
    ))?;
    let font = GdiResource::new(font_raw, GdipDeleteFont)?;
    let brush = create_brush(color)?;
    let mut format_raw = ptr::null_mut();
    status(GdipCreateStringFormat(0, 0, &mut format_raw))?;
    let format = GdiResource::new(format_raw, GdipDeleteStringFormat)?;
    status(GdipSetStringFormatAlign(format.as_ptr(), 1))?;
    status(GdipSetStringFormatLineAlign(format.as_ptr(), 1))?;
    // Text anti-aliasing is independent of the graphics smoothing mode.
    status(GdipSetTextRenderingHint(g, 4))?;
    let layout = RectF {
        x: center.x - radius,
        y: center.y - radius,
        width: radius * 2.0,
        height: radius * 2.0,
    };
    status(GdipDrawString(
        g,
        wide.as_ptr(),
        wide.len() as i32,
        font.as_ptr(),
        &layout,
        format.as_ptr(),
        brush.as_ptr(),
    ))
}
#[cfg(windows)]
unsafe fn draw_text(
    g: *mut c_void,
    location: crate::shapes::Point,
    text: &str,
    size: f32,
    color: u32,
) -> Result<(), RenderError> {
    if text.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidDimensions);
    }
    let mut wide = Vec::new();
    wide.try_reserve_exact(text.encode_utf16().count())
        .map_err(|_| RenderError::AllocationFailed)?;
    wide.extend(text.encode_utf16());
    let mut family_raw = ptr::null_mut();
    let name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    status(GdipCreateFontFamilyFromName(
        name.as_ptr(),
        ptr::null_mut(),
        &mut family_raw,
    ))?;
    let family = GdiResource::new(family_raw, GdipDeleteFontFamily)?;
    let mut font_raw = ptr::null_mut();
    status(GdipCreateFont(
        family.as_ptr(),
        size.max(1.0).min(512.0),
        0,
        UNIT_PIXEL,
        &mut font_raw,
    ))?;
    let font = GdiResource::new(font_raw, GdipDeleteFont)?;
    let brush = create_brush(color)?;
    let layout = RectF {
        x: location.x,
        y: location.y,
        width: 100_000.0,
        height: 100_000.0,
    };
    status(GdipDrawString(
        g,
        wide.as_ptr(),
        wide.len() as i32,
        font.as_ptr(),
        &layout,
        ptr::null_mut(),
        brush.as_ptr(),
    ))
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_pixel_tests;
