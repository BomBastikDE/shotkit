//! GDI+ measurement and drawing deliberately share font, format and line origins.
use super::RenderError;
use crate::shapes::CalloutTextLayout;

#[cfg(windows)]
const PADDING: f32 = 12.0;
const MAX_TEXT_BYTES: usize = 64 * 1024;

fn valid_input(text: &str, font_size: f32) -> bool {
    text.len() <= MAX_TEXT_BYTES && font_size.is_finite() && (1.0..=512.0).contains(&font_size)
}

/// Measure once when editing text; retain this layout on the callout for preview,
/// selection and export. No approximate metrics are substituted off Windows.
pub fn measure_callout_text(text: &str, font_size: f32) -> Result<CalloutTextLayout, RenderError> {
    if !valid_input(text, font_size) {
        return Err(RenderError::InvalidDimensions);
    }
    #[cfg(windows)]
    {
        windows::measure(text, font_size)
    }
    #[cfg(not(windows))]
    {
        Err(RenderError::PlatformUnavailable)
    }
}

#[cfg(windows)]
pub(super) use windows::draw;

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::alpha_render::*;
    use crate::shapes::CalloutData;

    const TEXT_LIMIT: f32 = 1_000_000.0;
    // NoWrap | MeasureTrailingSpaces | NoClip. Splitting logical lines ourselves
    // preserves blank lines and makes stored metrics independent of box wrapping.
    const FORMAT_FLAGS: i32 = 0x1000 | 0x0800 | 0x4000;

    #[link(name = "gdiplus")]
    extern "system" {
        fn GdipMeasureString(
            graphics: *mut c_void,
            text: *const u16,
            length: i32,
            font: *mut c_void,
            layout: *const RectF,
            format: *mut c_void,
            bounds: *mut RectF,
            fitted: *mut i32,
            lines: *mut i32,
        ) -> i32;
        fn GdipGetFontHeight(font: *mut c_void, graphics: *mut c_void, height: *mut f32) -> i32;
    }

    // SAFETY CONTRACT:
    // Voraussetzung: Eine aktive GDI+-Session überlebt die zurückgegebenen Guards.
    // Warum erfüllt: Beide Aufrufer halten ihre Session; Typ und Disposer stimmen überein.
    // Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
    unsafe fn resources(size: f32) -> Result<(GdiResource, GdiResource), RenderError> {
        let name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let mut raw = ptr::null_mut();
        status(GdipCreateFontFamilyFromName(
            name.as_ptr(),
            ptr::null_mut(),
            &mut raw,
        ))?;
        let family = GdiResource::new(raw, GdipDeleteFontFamily)?;
        raw = ptr::null_mut();
        status(GdipCreateFont(
            family.as_ptr(),
            size,
            0,
            UNIT_PIXEL,
            &mut raw,
        ))?;
        let font = GdiResource::new(raw, GdipDeleteFont)?;
        raw = ptr::null_mut();
        status(GdipCreateStringFormat(FORMAT_FLAGS, 0, &mut raw))?;
        let format = GdiResource::new(raw, GdipDeleteStringFormat)?;
        Ok((font, format))
    }

    fn line_rect(x: f32, y: f32) -> RectF {
        RectF {
            x,
            y,
            width: TEXT_LIMIT,
            height: TEXT_LIMIT,
        }
    }

    pub(super) fn measure(text: &str, size: f32) -> Result<CalloutTextLayout, RenderError> {
        let input = GdiplusStartupInput {
            version: 1,
            debug_callback: ptr::null_mut(),
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        };
        let mut token = 0;
        // SAFETY CONTRACT:
        // Voraussetzung: FFI-Pointer zeigen auf gültige lokale Werte; Session bleibt aktiv.
        // Warum erfüllt: Guards werden nach Startup und vor Session-Drop zerstört; das
        // Bitmap besitzt seinen Speicher selbst, Strings und Rects leben bis FFI-Rückkehr.
        // Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
        unsafe {
            status(GdiplusStartup(&mut token, &input, ptr::null_mut()))?;
            let _session = GdiPlusSession(token);
            let mut raw = ptr::null_mut();
            status(GdipCreateBitmapFromScan0(
                1,
                1,
                0,
                PIXEL_FORMAT_32BPP_PARGB,
                ptr::null_mut(),
                &mut raw,
            ))?;
            let bitmap = GdiResource::new(raw, GdipDisposeImage)?;
            raw = ptr::null_mut();
            status(GdipGetImageGraphicsContext(bitmap.as_ptr(), &mut raw))?;
            let graphics = GdiResource::new(raw, GdipDeleteGraphics)?;
            status(GdipSetTextRenderingHint(graphics.as_ptr(), 4))?;
            let (font, format) = resources(size)?;
            let mut line_height = 0.0;
            status(GdipGetFontHeight(
                font.as_ptr(),
                graphics.as_ptr(),
                &mut line_height,
            ))?;
            let mut widths = Vec::new();
            widths
                .try_reserve_exact(text.split('\n').count())
                .map_err(|_| RenderError::AllocationFailed)?;
            for line in text.split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                let wide: Vec<u16> = if line.is_empty() {
                    vec![32]
                } else {
                    line.encode_utf16().collect()
                };
                let mut bounds = line_rect(0.0, 0.0);
                status(GdipMeasureString(
                    graphics.as_ptr(),
                    wide.as_ptr(),
                    wide.len() as i32,
                    font.as_ptr(),
                    &line_rect(0.0, 0.0),
                    format.as_ptr(),
                    &mut bounds,
                    ptr::null_mut(),
                    ptr::null_mut(),
                ))?;
                if ![bounds.width, bounds.height]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0 && *v < TEXT_LIMIT)
                {
                    return Err(RenderError::InvalidDimensions);
                }
                line_height = line_height.max(bounds.height);
                widths.push(if line.is_empty() { 0.0 } else { bounds.width });
            }
            let layout = CalloutTextLayout {
                font_size: size,
                body_width: widths.iter().copied().fold(0.0, f32::max).ceil() + PADDING * 2.0,
                body_height: (line_height * widths.len() as f32).ceil() + PADDING * 2.0,
                line_height,
                line_widths: widths,
            };
            if !layout.validate(text) {
                return Err(RenderError::InvalidDimensions);
            }
            Ok(layout)
        }
    }

    // SAFETY CONTRACT:
    // Voraussetzung: g und pen gehören zur lebenden exklusiven Render-Session.
    // Warum erfüllt: draw_shape besitzt diese Ressourcen; Geometrie und UTF-16-Längen
    // werden validiert, alle Daten und Guards leben bis zum Ende ihrer FFI-Aufrufe.
    // Nachrichten: GDI+-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
    pub(crate) unsafe fn draw(
        g: *mut c_void,
        pen: *mut c_void,
        data: &CalloutData,
    ) -> Result<(), RenderError> {
        if !valid_input(&data.text, data.layout.font_size) {
            return Err(RenderError::InvalidDimensions);
        }
        let geometry = data.geometry().ok_or(RenderError::InvalidDimensions)?;
        let points: Vec<PointF> = geometry
            .outline
            .iter()
            .map(|p| PointF { x: p.x, y: p.y })
            .collect();
        let fill = create_brush(argb(data.fill_color, 255))?;
        status(GdipFillPolygon(
            g,
            fill.as_ptr(),
            points.as_ptr(),
            points.len() as i32,
            0,
        ))?;
        status(GdipDrawLines(g, pen, points.as_ptr(), points.len() as i32))?;
        let (font, format) = resources(data.layout.font_size)?;
        let brush = create_brush(argb(data.text_color, 255))?;
        status(GdipSetTextRenderingHint(g, 4))?;
        for (index, line) in data.text.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            let wide: Vec<u16> = line.encode_utf16().collect();
            let layout = line_rect(
                data.origin.x + PADDING,
                data.origin.y + PADDING + index as f32 * data.layout.line_height,
            );
            status(GdipDrawString(
                g,
                wide.as_ptr(),
                wide.len() as i32,
                font.as_ptr(),
                &layout,
                format.as_ptr(),
                brush.as_ptr(),
            ))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_or_nonfinite_measurement_input() {
        for size in [f32::NAN, f32::INFINITY, 0.0, 513.0] {
            assert_eq!(
                measure_callout_text("hello", size),
                Err(RenderError::InvalidDimensions)
            );
        }
        assert_eq!(
            measure_callout_text(&"x".repeat(MAX_TEXT_BYTES + 1), 24.0),
            Err(RenderError::InvalidDimensions)
        );
    }
    #[cfg(not(windows))]
    #[test]
    fn unicode_measurement_requires_real_platform_metrics() {
        assert_eq!(
            measure_callout_text("Ä🙂\n\n漢字", 24.0),
            Err(RenderError::PlatformUnavailable)
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use crate::alpha_render::render_shape;
    use crate::shapes::{CalloutData, Color, Point, Shape, ShapeKind};

    #[test]
    fn measured_multiline_unicode_keeps_empty_lines_and_trailing_spaces() {
        let layout = measure_callout_text("Ä🙂\n\n漢字 ", 24.0).unwrap();
        assert!(layout.validate("Ä🙂\n\n漢字 "));
        assert_eq!(layout.line_widths.len(), 3);
        assert_eq!(layout.line_widths[1], 0.0);
        assert!(layout.body_height >= layout.line_height * 3.0 + 24.0);
        let plain = measure_callout_text("Hello", 24.0).unwrap();
        let spaced = measure_callout_text("Hello   ", 24.0).unwrap();
        assert!(spaced.line_widths[0] > plain.line_widths[0]);
    }

    #[test]
    fn pointer_and_body_fill_once_without_an_internal_seam() {
        let layout = measure_callout_text("Text", 20.0).unwrap();
        let (body_width, body_height) = (layout.body_width, layout.body_height);
        let data = CalloutData {
            origin: Point::new(10.0, 10.0),
            text: "Text".into(),
            fill_color: Color::rgba(20, 40, 80, 100),
            text_color: Color::rgba(0, 0, 0, 0),
            tip: Some(Point::new(
                10.0 + body_width / 2.0,
                10.0 + body_height + 35.0,
            )),
            layout,
        };
        let mut shape = Shape::new(ShapeKind::Callout {
            data: Box::new(data),
        });
        shape.style.color = Color::rgba(0, 0, 0, 0);
        let image = render_shape(256, 160, &shape).unwrap();
        let alpha = |x: usize, y: usize| image.pixels[(y * image.width + x) * 4 + 3];
        let x = (10.0 + body_width / 2.0) as usize;
        let base = (10.0 + body_height) as usize;
        for y in base - 2..=base + 2 {
            assert_eq!(
                alpha(x, y),
                100,
                "one continuous translucent fill at pointer base"
            );
        }
        assert_eq!(alpha(0, 0), 0);
        assert!(alpha(x, base + 12) > 0);
    }

    #[test]
    fn cached_preview_matches_export_for_measured_callout() {
        use crate::alpha_document::{render_document, render_preview};
        use crate::stitch::Image;
        use std::sync::Arc;
        let text = "Erste Zeile\n漢字";
        let data = CalloutData {
            origin: Point::new(20.0, 20.0),
            text: text.into(),
            fill_color: Color::rgba(255, 255, 255, 200),
            text_color: Color::rgba(0, 0, 0, 255),
            tip: Some(Point::new(70.0, 150.0)),
            layout: measure_callout_text(text, 24.0).unwrap(),
        };
        let shape = Shape::new(ShapeKind::Callout {
            data: Box::new(data),
        });
        let image = Arc::new(Image::new(320, 200, vec![180; 320 * 200 * 4]).unwrap());
        let shapes = [shape];
        let mut cache = None;
        let preview = render_preview(&image, &shapes, 0, &mut cache).unwrap();
        let export = render_document(&image, &shapes).unwrap();
        assert_eq!(preview, export);
        let cached = render_preview(&image, &shapes, 1, &mut cache).unwrap();
        assert_eq!(cached, export);
    }

    #[test]
    fn text_color_alpha_is_preserved_with_transparent_body() {
        let text = "Grüße\n漢字 🙂";
        let layout = measure_callout_text(text, 24.0).unwrap();
        let data = CalloutData {
            origin: Point::new(10.0, 10.0),
            text: text.into(),
            fill_color: Color::rgba(0, 0, 0, 0),
            text_color: Color::rgba(250, 10, 20, 96),
            tip: None,
            layout,
        };
        let mut shape = Shape::new(ShapeKind::Callout {
            data: Box::new(data),
        });
        shape.style.color = Color::rgba(0, 0, 0, 0);
        let image = render_shape(320, 180, &shape).unwrap();
        let max_alpha = image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .max()
            .unwrap();
        assert!(max_alpha > 0 && max_alpha <= 96);
        assert_eq!(image.pixels[3], 0);
    }
}
