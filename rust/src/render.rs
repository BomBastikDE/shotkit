//! Native GDI renderer for annotation shapes.
//!
//! Coordinates are image-space pixels. Call [`draw_shape`] with a destination
//! device context whose origin is the image origin. The caller retains ownership
//! of both HDCs. GDI objects created here are selected out and deleted on every
//! return path. Blur is a pixelated downsample/nearest-neighbour enlargement;
//! text uses the Windows default GUI font, not a bundled Segoe UI font.

use crate::shapes::{Color, Point, Shape, ShapeKind};
use std::{ffi::c_void, ptr::null_mut};

type Hdc = *mut c_void;
type Hgdiobj = *mut c_void;
type Hbrush = *mut c_void;
type Hpen = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct WinPoint {
    x: i32,
    y: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    image_size: u32,
    xppm: i32,
    yppm: i32,
    colors_used: u32,
    colors_important: u32,
}
#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    color: u32,
}
#[repr(C)]
struct BlendFunction {
    operation: u8,
    flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

#[link(name = "gdi32")]
extern "system" {
    fn CreatePen(style: i32, width: i32, color: u32) -> Hpen;
    fn CreateSolidBrush(color: u32) -> Hbrush;
    fn SelectObject(dc: Hdc, obj: Hgdiobj) -> Hgdiobj;
    fn DeleteObject(obj: Hgdiobj) -> i32;
    fn MoveToEx(dc: Hdc, x: i32, y: i32, old: *mut WinPoint) -> i32;
    fn LineTo(dc: Hdc, x: i32, y: i32) -> i32;
    fn Rectangle(dc: Hdc, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn Ellipse(dc: Hdc, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn Polygon(dc: Hdc, pts: *const WinPoint, count: i32) -> i32;
    fn SetBkMode(dc: Hdc, mode: i32) -> i32;
    fn SetTextColor(dc: Hdc, color: u32) -> u32;
    fn TextOutW(dc: Hdc, x: i32, y: i32, text: *const u16, len: i32) -> i32;
    fn CreateFontW(
        height: i32,
        width: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_family: u32,
        face: *const u16,
    ) -> Hgdiobj;
    fn GetStockObject(index: i32) -> Hgdiobj;
    fn CreateCompatibleDC(dc: Hdc) -> Hdc;
    fn DeleteDC(dc: Hdc) -> i32;
    fn CreateCompatibleBitmap(dc: Hdc, width: i32, height: i32) -> Hgdiobj;
    fn CreateDIBSection(
        dc: Hdc,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: *mut c_void,
        offset: u32,
    ) -> Hgdiobj;
    fn StretchBlt(
        dst: Hdc,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src: Hdc,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        rop: u32,
    ) -> i32;
}
#[link(name = "msimg32")]
extern "system" {
    fn AlphaBlend(
        dst: Hdc,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        src: Hdc,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        blend: BlendFunction,
    ) -> i32;
}

const PS_SOLID: i32 = 0;
const NULL_BRUSH: i32 = 5;
const OPAQUE: i32 = 2;
const TRANSPARENT: i32 = 1;
const SRCCOPY: u32 = 0x00CC0020;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn pixel(v: f32) -> i32 {
    if v.is_finite() {
        v.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
    } else {
        0
    }
}
fn rgb(c: Color) -> u32 {
    (c.r as u32) | ((c.g as u32) << 8) | ((c.b as u32) << 16)
}
fn wp(p: Point) -> WinPoint {
    WinPoint {
        x: pixel(p.x),
        y: pixel(p.y),
    }
}

struct Selected {
    dc: Hdc,
    old: Hgdiobj,
}
impl Drop for Selected {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
        }
    }
}
struct OwnedObject(Hgdiobj);
impl Drop for OwnedObject {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                DeleteObject(self.0);
            }
        }
    }
}
struct OwnedDc(Hdc);
impl Drop for OwnedDc {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                DeleteDC(self.0);
            }
        }
    }
}
unsafe fn select(dc: Hdc, obj: Hgdiobj) -> Option<Selected> {
    if obj.is_null() {
        return None;
    }
    let old = SelectObject(dc, obj);
    if old.is_null() || old as isize == -1 {
        None
    } else {
        Some(Selected { dc, old })
    }
}
unsafe fn pen(dc: Hdc, color: Color, width: f32) -> Option<(OwnedObject, Selected)> {
    let object = OwnedObject(CreatePen(
        PS_SOLID,
        pixel(width.max(1.0)).max(1),
        rgb(color),
    ));
    let selected = select(dc, object.0)?;
    Some((object, selected))
}
unsafe fn brush(dc: Hdc, color: Color) -> Option<(OwnedObject, Selected)> {
    let object = OwnedObject(CreateSolidBrush(rgb(color)));
    let selected = select(dc, object.0)?;
    Some((object, selected))
}
unsafe fn line(dc: Hdc, a: Point, b: Point) -> bool {
    let a = wp(a);
    let b = wp(b);
    if MoveToEx(dc, a.x, a.y, null_mut()) == 0 {
        return false;
    }
    LineTo(dc, b.x, b.y) != 0
}
fn bounds(a: Point, b: Point) -> WinRect {
    let (a, b) = (wp(a), wp(b));
    WinRect {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    }
}

/// Alpha-composite a solid-color 105/255 highlight over the destination.
/// DIB pixels are premultiplied as required by AC_SRC_ALPHA. The temporary DC
/// and bitmap are dropped in reverse acquisition order, with the bitmap first
/// restored out of the DC.
unsafe fn translucent_highlight(dc: Hdc, rect: WinRect, color: Color) -> bool {
    let (width, height) = (
        rect.right as i64 - rect.left as i64,
        rect.bottom as i64 - rect.top as i64,
    );
    if width <= 0 || height <= 0 {
        return true;
    }
    // Keep malformed or enormous shape bounds from overflowing allocation sizes.
    let Some(bytes) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
    else {
        return false;
    };
    if bytes > 256 * 1024 * 1024 || width > i32::MAX as i64 || height > i32::MAX as i64 {
        return false;
    }
    let info = BitmapInfo {
        header: BitmapInfoHeader {
            size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            width: width as i32,
            height: -(height as i32),
            planes: 1,
            bit_count: 32,
            compression: 0,
            image_size: bytes as u32,
            xppm: 0,
            yppm: 0,
            colors_used: 0,
            colors_important: 0,
        },
        color: 0,
    };
    let mut bits = null_mut();
    let bitmap = OwnedObject(CreateDIBSection(dc, &info, 0, &mut bits, null_mut(), 0));
    if bitmap.0.is_null() || bits.is_null() {
        return false;
    }
    let memory_dc = OwnedDc(CreateCompatibleDC(dc));
    if memory_dc.0.is_null() {
        return false;
    }
    let Some(_selected) = select(memory_dc.0, bitmap.0) else {
        return false;
    };

    let alpha = 105u16;
    let premul = |channel: u8| ((channel as u16 * alpha + 127) / 255) as u8;
    let bgra = [
        premul(color.b),
        premul(color.g),
        premul(color.r),
        alpha as u8,
    ];
    let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), bytes);
    for px in pixels.chunks_exact_mut(4) {
        px.copy_from_slice(&bgra);
    }

    AlphaBlend(
        dc,
        rect.left,
        rect.top,
        width as i32,
        height as i32,
        memory_dc.0,
        0,
        0,
        width as i32,
        height as i32,
        BlendFunction {
            operation: 0,
            flags: 0,
            source_constant_alpha: 255,
            alpha_format: 1,
        },
    ) != 0
}

/// Draw one shape into a caller-owned image-space HDC. `source` is required only
/// for Blur and must be the original, unannotated image HDC; if absent Blur draws
/// a conspicuous diagonal fallback mark. Returns false for invalid input/HDC or
/// failed GDI allocation, true when the requested primitive was issued.
pub unsafe fn draw_shape(dc: Hdc, shape: &Shape, source: Option<Hdc>) -> bool {
    if dc.is_null() {
        return false;
    }
    match &shape.kind {
        ShapeKind::Line { start, end } => {
            let Some((_p, _s)) = pen(dc, shape.style.color, shape.style.width) else {
                return false;
            };
            line(dc, *start, *end)
        }
        ShapeKind::Arrow { start, end } => {
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let len = (dx * dx + dy * dy).sqrt();
            if !len.is_finite() || len < 1.0 {
                return true;
            }
            let (ux, uy) = (dx / len, dy / len);
            let head = len.min(12.0f32.max(shape.style.width * 3.2));
            let half = head * 0.55;
            let base = Point::new(end.x - ux * head, end.y - uy * head);
            let left = Point::new(base.x - uy * half, base.y + ux * half);
            let right = Point::new(base.x + uy * half, base.y - ux * half);
            let Some((_p, _s)) = pen(dc, shape.style.color, shape.style.width) else {
                return false;
            };
            let shaft_ok = if len > head {
                line(
                    dc,
                    *start,
                    Point::new(end.x - ux * head * 0.8, end.y - uy * head * 0.8),
                )
            } else {
                true
            };
            let Some((_b, _bs)) = brush(dc, shape.style.color) else {
                return false;
            };
            let pts = [wp(*end), wp(left), wp(right)];
            let polygon_ok = Polygon(dc, pts.as_ptr(), 3) != 0;
            shaft_ok && polygon_ok
        }
        ShapeKind::Rectangle { start, end } | ShapeKind::Ellipse { start, end } => {
            let Some((_p, _s)) = pen(dc, shape.style.color, shape.style.width) else {
                return false;
            };
            let Some(_null) = select(dc, GetStockObject(NULL_BRUSH)) else {
                return false;
            };
            let r = bounds(*start, *end);
            if matches!(shape.kind, ShapeKind::Rectangle { .. }) {
                Rectangle(dc, r.left, r.top, r.right, r.bottom) != 0
            } else {
                Ellipse(dc, r.left, r.top, r.right, r.bottom) != 0
            }
        }
        ShapeKind::Highlight { start, end } => {
            translucent_highlight(dc, bounds(*start, *end), shape.style.color)
        }
        ShapeKind::Blur { start, end } => {
            let r = bounds(*start, *end);
            let Some(src) = source else {
                let r = bounds(*start, *end);
                let _ = line(
                    dc,
                    Point::new(r.left as f32, r.top as f32),
                    Point::new(r.right as f32, r.bottom as f32),
                );
                return false;
            };
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w < 1 || h < 1 {
                return true;
            }
            let block = (shape.style.width * 3.0).max(6.0) as i32;
            let (sw, sh) = ((w / block).max(1), (h / block).max(1));
            let temp = OwnedDc(CreateCompatibleDC(src));
            if temp.0.is_null() {
                return false;
            }
            let bmp = OwnedObject(CreateCompatibleBitmap(src, sw, sh));
            if bmp.0.is_null() {
                return false;
            }
            let Some(_selected) = select(temp.0, bmp.0) else {
                return false;
            };
            if StretchBlt(temp.0, 0, 0, sw, sh, src, r.left, r.top, w, h, SRCCOPY) == 0 {
                return false;
            }
            StretchBlt(dc, r.left, r.top, w, h, temp.0, 0, 0, sw, sh, SRCCOPY) != 0
        }
        ShapeKind::Freehand { points } => {
            if points.len() < 2 {
                return true;
            }
            let Some((_p, _s)) = pen(dc, shape.style.color, shape.style.width) else {
                return false;
            };
            let mut success = true;
            for pair in points.windows(2) {
                success &= line(dc, pair[0], pair[1]);
            }
            success
        }
        ShapeKind::Text { location, text } => {
            let units: Vec<u16> = text.encode_utf16().collect();
            if units.is_empty() {
                return true;
            }
            let face = wide("Segoe UI");
            let font = OwnedObject(CreateFontW(
                -pixel(shape.style.font_size).max(1),
                0,
                0,
                0,
                700,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0x20 | 0x01,
                face.as_ptr(),
            ));
            let Some(_selected) = select(dc, font.0) else {
                return false;
            };
            let _ = SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, rgb(shape.style.color));
            TextOutW(
                dc,
                pixel(location.x),
                pixel(location.y),
                units.as_ptr(),
                units.len().min(i32::MAX as usize) as i32,
            ) != 0
        }
        ShapeKind::Step { center, number } => {
            let radius = 10.0 + shape.style.width * 2.2;
            let c = shape.style.color;
            let Some((_b, _bs)) = brush(dc, c) else {
                return false;
            };
            let _ = SetBkMode(dc, OPAQUE);
            let ring = OwnedObject(CreatePen(
                PS_SOLID,
                (radius / 8.0).max(2.0) as i32,
                rgb(Color::rgba(255, 255, 255, 255)),
            ));
            let Some(_rs) = select(dc, ring.0) else {
                return false;
            };
            let _ = Ellipse(
                dc,
                pixel(center.x - radius),
                pixel(center.y - radius),
                pixel(center.x + radius),
                pixel(center.y + radius),
            );
            let face = wide("Segoe UI");
            let font_size = radius
                * if number.to_string().len() > 1 {
                    0.85
                } else {
                    1.05
                };
            let font = OwnedObject(CreateFontW(
                -pixel(font_size).max(1),
                0,
                0,
                0,
                700,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0x20 | 0x01,
                face.as_ptr(),
            ));
            let Some(_fs) = select(dc, font.0) else {
                return false;
            };
            let _ = SetTextColor(
                dc,
                rgb(
                    if (c.r as f32 * 0.299 + c.g as f32 * 0.587 + c.b as f32 * 0.114) > 150.0 {
                        Color::rgba(40, 40, 40, 255)
                    } else {
                        Color::rgba(255, 255, 255, 255)
                    },
                ),
            );
            let s = number.to_string();
            let u: Vec<u16> = s.encode_utf16().collect();
            TextOutW(
                dc,
                pixel(center.x - (u.len() as f32 * radius * 0.28)),
                pixel(center.y - radius * 0.42),
                u.as_ptr(),
                u.len() as i32,
            ) != 0
        }
    }
}

/// Compatibility helper for a bitmap passed as a GDI device context.
pub type DeviceContext = Hdc;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgb_is_bgr_encoding_for_gdi() {
        assert_eq!(rgb(Color::rgba(1, 2, 3, 255)), 0x00030201);
    }
}
