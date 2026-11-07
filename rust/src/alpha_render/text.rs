#[cfg(windows)]
use super::*;

#[cfg(any(windows, test))]
pub(super) fn step_text_color(color: crate::shapes::Color) -> u32 {
    let luminance =
        f32::from(color.r) * 0.299 + f32::from(color.g) * 0.587 + f32::from(color.b) * 0.114;
    let (r, g, b) = if luminance > 150.0 {
        (40, 40, 40)
    } else {
        (255, 255, 255)
    };
    (255u32 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

#[cfg(any(windows, test))]
pub(super) fn step_font_size(radius: f32, digits: usize) -> f32 {
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
// SAFETY CONTRACT:
// Voraussetzung: Die aktive GDI+-Session überlebt den Pen; width ist für GDI+ gültig.
// Warum erfüllt bzw. offen: Status und Disposer werden gebunden; Aufrufe erfolgen innerhalb der Session.
// Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn create_pen(color: u32, width: f32) -> Result<GdiResource, RenderError> {
    let mut raw = ptr::null_mut();
    status(GdipCreatePen1(color, width, UNIT_PIXEL, &mut raw))?;
    GdiResource::new(raw, GdipDeletePen)
}

#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: g ist gültig, die GDI+-Session lebt und die UTF-16-Länge passt in i32.
// Warum erfüllt bzw. offen: Aktuell wird eine kurze Schrittnummer gezeichnet; Text und Ressourcen leben bis zur FFI-Rückkehr.
// Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn draw_centered_text(
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
// SAFETY CONTRACT:
// Voraussetzung: g ist gültig und die GDI+-Session lebt während der Zeichnung.
// Warum erfüllt bzw. offen: Textlänge ist begrenzt und alle Pointer zeigen auf lebende lokale Werte und Ressourcen.
// Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn draw_text(
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
        bounded_size(size, 512.0),
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
