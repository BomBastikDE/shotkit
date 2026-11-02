//! Alpha-correct document rendering for Windows annotation images.
//!
//! Ordinary annotations are rendered to straight-alpha BGRA layers and
//! composited in document order. Pixelation samples the unchanged source image
//! and replaces only its clipped region in the accumulated result.

use crate::{
    alpha_composite::source_over,
    alpha_render,
    pixelate::{pixelate_region, PixelateRect},
    shapes::{Shape, ShapeKind},
    stitch::Image,
};

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_DIMENSION: usize = 30_000;
#[cfg(all(test, windows))]
const PIXELATE_BLOCK_SIZE: usize = 12;

/// Render document shapes over `source`, leaving both inputs unchanged.
///
/// Pixelation regions are clipped to the image. They are derived from the
/// shape's floating-point endpoints using floor for the near edges and ceil
/// for the far edges, then clamped to the image bounds.
pub fn render_document(source: &Image, shapes: &[Shape]) -> Result<Image, String> {
    validate_image(source)?;

    let mut output_pixels = Vec::new();
    output_pixels
        .try_reserve_exact(source.pixels.len())
        .map_err(|_| "Nicht genug Speicher für das gerenderte Dokument.".to_owned())?;
    output_pixels.extend_from_slice(&source.pixels);

    for shape in shapes {
        match &shape.kind {
            ShapeKind::Blur { start, end } => {
                let Some(region) =
                    clipped_region(source.width, source.height, start.x, start.y, end.x, end.y)?
                else {
                    continue;
                };
                if !shape.style.width.is_finite() || shape.style.width <= 0.0 {
                    return Err("Ungültige Pixelationsstärke.".into());
                }
                let block_size = ((shape.style.width * 3.0) as usize).clamp(6, 4096);
                let pixelated = pixelate_region(source, region, block_size)
                    .map_err(|error| format!("Pixelation fehlgeschlagen: {error:?}"))?;
                copy_region(&pixelated, &mut output_pixels, region)?;
            }
            _ => {
                let layer = alpha_render::render_shape(source.width, source.height, shape)
                    .map_err(|error| format!("Shape-Rendering fehlgeschlagen: {error:?}"))?;
                validate_same_dimensions(source, &layer)?;
                for (dst, src) in output_pixels
                    .chunks_exact_mut(4)
                    .zip(layer.pixels.chunks_exact(4))
                {
                    let result = source_over(
                        [dst[0], dst[1], dst[2], dst[3]],
                        [src[0], src[1], src[2], src[3]],
                        255,
                    );
                    dst.copy_from_slice(&result);
                }
            }
        }
    }

    Image::new(source.width, source.height, output_pixels)
        .map_err(|error| format!("Gerendertes Bild ist ungültig: {error}"))
}

fn validate_image(image: &Image) -> Result<(), String> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Bildgröße läuft über.".to_owned())?;
    if image.width == 0
        || image.height == 0
        || image.width > MAX_DIMENSION
        || image.height > MAX_DIMENSION
        || bytes != image.pixels.len()
        || bytes > MAX_IMAGE_BYTES
    {
        return Err("Ungültige Bildgröße oder BGRA-Pufferlänge.".into());
    }
    Ok(())
}

fn validate_same_dimensions(source: &Image, layer: &Image) -> Result<(), String> {
    validate_image(layer)?;
    if source.width != layer.width || source.height != layer.height {
        return Err("Shape-Ebene hat andere Abmessungen als das Originalbild.".into());
    }
    Ok(())
}

fn clipped_region(
    width: usize,
    height: usize,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) -> Result<Option<PixelateRect>, String> {
    if ![x1, y1, x2, y2].into_iter().all(f32::is_finite) {
        return Err("Pixelationsregion enthält ungültige Koordinaten.".into());
    }

    let left = (x1.min(x2) as f64).floor().clamp(0.0, width as f64) as usize;
    let top = (y1.min(y2) as f64).floor().clamp(0.0, height as f64) as usize;
    let right = (x1.max(x2) as f64).ceil().clamp(0.0, width as f64) as usize;
    let bottom = (y1.max(y2) as f64).ceil().clamp(0.0, height as f64) as usize;

    if right <= left || bottom <= top {
        return Ok(None);
    }
    Ok(Some(PixelateRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }))
}

fn copy_region(source: &Image, target: &mut [u8], region: PixelateRect) -> Result<(), String> {
    for y in region.y..region.y + region.height {
        let start = y
            .checked_mul(source.width)
            .and_then(|row| row.checked_add(region.x))
            .and_then(|pixel| pixel.checked_mul(4))
            .ok_or_else(|| "Pixelationsbereich läuft über.".to_owned())?;
        let len = region
            .width
            .checked_mul(4)
            .ok_or_else(|| "Pixelationsbereich läuft über.".to_owned())?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "Pixelationsbereich läuft über.".to_owned())?;
        let src = source
            .pixels
            .get(start..end)
            .ok_or_else(|| "Pixelationsbereich liegt außerhalb des Quellbilds.".to_owned())?;
        let dst = target
            .get_mut(start..end)
            .ok_or_else(|| "Pixelationsbereich liegt außerhalb des Ergebnisbilds.".to_owned())?;
        dst.copy_from_slice(src);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use crate::shapes::{Color, Point, Style};

    #[test]
    fn pixelation_respects_line_width() {
        use crate::shapes::Point;
        let mut pixels = Vec::new();
        for y in 0..24 {
            for x in 0..24 {
                pixels.extend_from_slice(&[(x * 9) as u8, (y * 9) as u8, 0, 255]);
            }
        }
        let original = Image::new(24, 24, pixels).unwrap();
        let mut blur = Shape::new(ShapeKind::Blur {
            start: Point::new(0.0, 0.0),
            end: Point::new(24.0, 24.0),
        });
        blur.style.width = 2.0;
        let small = render_document(&original, &[blur.clone()]).unwrap();
        blur.style.width = 8.0;
        let large = render_document(&original, &[blur]).unwrap();
        assert_ne!(small, large);
    }

    #[cfg(windows)]
    fn point(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[cfg(windows)]
    fn highlight(start: Point, end: Point, color: Color) -> Shape {
        let mut shape = Shape::new(ShapeKind::Highlight { start, end });
        shape.style = Style {
            color,
            ..Style::default()
        };
        shape
    }

    #[cfg(windows)]
    #[test]
    fn highlight_composition_preserves_base_alpha_and_source() {
        let original = Image::new(16, 16, [20, 40, 60, 91].repeat(16 * 16)).unwrap();
        let before = original.clone();
        let shape = highlight(
            point(2.0, 2.0),
            point(12.0, 12.0),
            Color::rgba(255, 0, 0, 255),
        );
        let result = render_document(&original, &[shape]).unwrap();
        let outside = (0 * 16 + 0) * 4;
        assert_eq!(&result.pixels[outside..outside + 4], &[20, 40, 60, 91]);
        let inside = (7 * 16 + 7) * 4;
        assert!(result.pixels[inside + 3] > 91);
        assert!(result.pixels[inside + 2] > 60);
        assert_eq!(original, before);
    }

    #[cfg(windows)]
    #[test]
    fn normal_shapes_compose_in_document_order() {
        let original = Image::new(16, 16, vec![0; 16 * 16 * 4]).unwrap();
        let first = highlight(
            point(1.0, 1.0),
            point(14.0, 14.0),
            Color::rgba(255, 0, 0, 255),
        );
        let second = highlight(
            point(1.0, 1.0),
            point(14.0, 14.0),
            Color::rgba(0, 0, 255, 255),
        );
        let forward = render_document(&original, &[first.clone(), second.clone()]).unwrap();
        let reverse = render_document(&original, &[second, first]).unwrap();
        let center = (7 * 16 + 7) * 4;
        assert_ne!(
            &forward.pixels[center..center + 4],
            &reverse.pixels[center..center + 4]
        );
    }

    #[cfg(windows)]
    #[test]
    fn pixelation_samples_original_and_overwrites_prior_shapes_only_in_region() {
        let mut pixels = Vec::new();
        for y in 0..24 {
            for x in 0..24 {
                pixels.extend_from_slice(&[x as u8, y as u8, 40, if x == 0 { 80 } else { 255 }]);
            }
        }
        let original = Image::new(24, 24, pixels).unwrap();
        let before = original.clone();
        let marker = highlight(
            point(0.0, 0.0),
            point(12.0, 12.0),
            Color::rgba(255, 0, 0, 255),
        );
        let pixelate = Shape::new(ShapeKind::Blur {
            start: point(0.0, 0.0),
            end: point(12.0, 12.0),
        });
        let result = render_document(&original, &[marker, pixelate]).unwrap();
        let pixelated = pixelate_region(
            &original,
            PixelateRect {
                x: 0,
                y: 0,
                width: 12,
                height: 12,
            },
            PIXELATE_BLOCK_SIZE,
        )
        .unwrap();
        let replaced = (5 * 24 + 5) * 4;
        assert_eq!(
            &result.pixels[replaced..replaced + 4],
            &pixelated.pixels[replaced..replaced + 4]
        );
        let untouched = (20 * 24 + 20) * 4;
        assert_eq!(
            &result.pixels[untouched..untouched + 4],
            &original.pixels[untouched..untouched + 4]
        );
        assert_eq!(original, before);
    }

    #[test]
    fn blur_coordinates_are_finite_and_clipped_to_image() {
        assert_eq!(
            clipped_region(10, 10, -5.0, 2.2, 4.1, 12.0).unwrap(),
            Some(PixelateRect {
                x: 0,
                y: 2,
                width: 5,
                height: 8
            })
        );
        assert_eq!(
            clipped_region(10, 10, 12.0, 12.0, 14.0, 14.0).unwrap(),
            None
        );
        assert!(clipped_region(10, 10, f32::NAN, 0.0, 1.0, 1.0).is_err());
    }
}
