//! In-place spotlight and source-sampled image magnifier rendering.
//! The magnifier must receive the source after applying all document redactions.
use crate::alpha_composite::source_over;
use crate::shapes::{Magnifier, MagnifierFrame, Spotlight, Style};
use crate::stitch::Image;

fn area(image: &Image) -> Result<usize, String> {
    image
        .width
        .checked_mul(image.height)
        .and_then(|n| n.checked_mul(4))
        .filter(|&n| n == image.pixels.len() && image.width > 0 && image.height > 0)
        .ok_or_else(|| "Ungültige Bilddaten für den Effekt.".into())
}

pub(super) fn spotlight(
    pixels: &mut [u8],
    source: &Image,
    effect: &Spotlight,
) -> Result<(), String> {
    effect
        .validate()
        .map_err(|e| format!("Ungültiger Spotlight: {e:?}"))?;
    if pixels.len() != area(source)? {
        return Err("Ungültiges Effektziel.".into());
    }
    let r = effect.focus;
    let alpha = (effect.dim_opacity * 255.0).round() as u8;
    if alpha == 0 {
        return Ok(());
    }
    // Pixel centres inside [near, far) stay unchanged. Compute the two
    // horizontal spans once rather than dividing and testing every pixel.
    let clip =
        |value: f32, limit: usize| -> usize { ((value - 0.5).ceil().max(0.0) as usize).min(limit) };
    let left = clip(r.x, source.width);
    let right = clip(r.x + r.width, source.width);
    let top = clip(r.y, source.height);
    let bottom = clip(r.y + r.height, source.height);
    let mut opaque_table = [0u8; 256];
    for (value, result) in opaque_table.iter_mut().enumerate() {
        *result = ((value as u16 * u16::from(255 - alpha) + 127) / 255) as u8;
    }
    let dim = |span: &mut [u8]| {
        for pixel in span.as_chunks_mut::<4>().0 {
            if pixel[3] == 255 {
                // Exact opaque-black source-over result with a small lookup,
                // avoiding repeated arithmetic on ordinary screenshots.
                for channel in &mut pixel[..3] {
                    *channel = opaque_table[usize::from(*channel)];
                }
            } else {
                *pixel = source_over(*pixel, [0, 0, 0, alpha], 255);
            }
        }
    };
    for (y, row) in pixels.chunks_exact_mut(source.width * 4).enumerate() {
        if y >= top && y < bottom {
            let (before, rest) = row.split_at_mut(left * 4);
            let (_, after) = rest.split_at_mut((right - left) * 4);
            dim(before);
            dim(after);
        } else {
            dim(row);
        }
    }
    Ok(())
}

fn inside(x: f32, y: f32, l: f32, t: f32, w: f32, h: f32, frame: MagnifierFrame) -> bool {
    if w <= 0.0 || h <= 0.0 || x < l || y < t || x >= l + w || y >= t + h {
        return false;
    }
    match frame {
        MagnifierFrame::Rectangle => true,
        MagnifierFrame::Circle => {
            let dx = (x - (l + w / 2.0)) / (w / 2.0);
            let dy = (y - (t + h / 2.0)) / (h / 2.0);
            dx * dx + dy * dy <= 1.0
        }
    }
}

/// Combine disjoint inner and border samples in premultiplied space before
/// compositing once, so the background cannot leak through an opaque edge.
fn coverage_pixel(
    sample: [u8; 4],
    border: [u8; 4],
    inner_count: u32,
    border_count: u32,
) -> [u8; 4] {
    let sample_weight = u32::from(sample[3]) * inner_count;
    let border_weight = u32::from(border[3]) * border_count;
    let total = sample_weight + border_weight;
    if total == 0 {
        return [0; 4];
    }
    let mut combined = [0u8; 4];
    for channel in 0..3 {
        let weighted =
            u32::from(sample[channel]) * sample_weight + u32::from(border[channel]) * border_weight;
        combined[channel] = ((weighted + total / 2) / total) as u8;
    }
    combined[3] = ((total + 8) / 16) as u8;
    combined
}

/// Sample only `source` (which has already been sanitized by all redactions).
/// Out-of-image samples are transparent; a separate destination avoids aliases.
pub(super) fn magnifier(
    pixels: &mut [u8],
    source: &Image,
    effect: &Magnifier,
    style: &Style,
) -> Result<(), String> {
    effect
        .validate()
        .map_err(|e| format!("Ungültige Bildlupe: {e:?}"))?;
    if pixels.len() != area(source)? {
        return Err("Ungültiges Effektziel.".into());
    }
    if !style.width.is_finite() || !(0.25..=256.0).contains(&style.width) {
        return Err("Ungültige Rahmenbreite der Bildlupe.".into());
    }
    let r = effect.viewport;
    let left = (r.x.floor().max(0.0) as usize).min(source.width);
    let top = (r.y.floor().max(0.0) as usize).min(source.height);
    let right = ((r.x + r.width).ceil().max(0.0) as usize).min(source.width);
    let bottom = ((r.y + r.height).ceil().max(0.0) as usize).min(source.height);
    let inset = style.width.min(r.width / 2.0).min(r.height / 2.0);
    for y in top..bottom {
        for x in left..right {
            if effect.frame == MagnifierFrame::Circle {
                // A convex ellipse cannot cover the pixel if its nearest point
                // to the centre is already outside the ellipse.
                let cx = r.x + r.width / 2.0;
                let cy = r.y + r.height / 2.0;
                let near_x = cx.clamp(x as f32, x as f32 + 1.0);
                let near_y = cy.clamp(y as f32, y as f32 + 1.0);
                if !inside(near_x, near_y, r.x, r.y, r.width, r.height, effect.frame) {
                    continue;
                }
            }
            let mut outer = 0u32;
            let mut border = 0u32;
            // Interior pixels do not need multisampling. Only the boundary is
            // supersampled, keeping large magnifiers responsive.
            let inner = (
                r.x + inset,
                r.y + inset,
                r.width - 2.0 * inset,
                r.height - 2.0 * inset,
            );
            let whole_inner =
                [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                    .iter()
                    .all(|(dx, dy)| {
                        inside(
                            x as f32 + dx,
                            y as f32 + dy,
                            inner.0,
                            inner.1,
                            inner.2,
                            inner.3,
                            effect.frame,
                        )
                    });
            if whole_inner {
                outer = 16;
            } else {
                // Four-by-four coverage softens frame edges without a new layer.
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                        if inside(px, py, r.x, r.y, r.width, r.height, effect.frame) {
                            outer += 1;
                            if !inside(px, py, inner.0, inner.1, inner.2, inner.3, effect.frame) {
                                border += 1;
                            }
                        }
                    }
                }
            }
            if outer == 0 {
                continue;
            }
            let sample_x = effect.focus.x + (x as f32 + 0.5 - (r.x + r.width / 2.0)) / effect.zoom;
            let sample_y = effect.focus.y + (y as f32 + 0.5 - (r.y + r.height / 2.0)) / effect.zoom;
            let index = (y * source.width + x) * 4;
            let dst = &mut pixels[index..index + 4];
            let sample = if sample_x >= 0.0
                && sample_y >= 0.0
                && sample_x < source.width as f32
                && sample_y < source.height as f32
            {
                let sample =
                    (sample_y.floor() as usize * source.width + sample_x.floor() as usize) * 4;
                source.pixels[sample..sample + 4]
                    .try_into()
                    .expect("validated image pixel")
            } else {
                [0, 0, 0, 0]
            };
            let color = style.color;
            let layer = coverage_pixel(
                sample,
                [color.b, color.g, color.r, color.a],
                outer - border,
                border,
            );
            let result = source_over(dst.try_into().expect("four-byte pixel"), layer, 255);
            dst.copy_from_slice(&result);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alpha_document::{render_document, render_preview};
    use crate::shapes::{Color, Point, Rect, Shape, ShapeKind};
    use std::sync::Arc;

    fn solid(width: usize, height: usize, pixel: [u8; 4]) -> Image {
        Image::new(width, height, pixel.repeat(width * height)).unwrap()
    }

    #[test]
    fn spotlight_dims_only_pixels_outside_focus() {
        let image = solid(3, 1, [100, 100, 100, 255]);
        let mut pixels = image.pixels.clone();
        let effect = Spotlight::new(
            Rect {
                x: 1.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            0.5,
        )
        .unwrap();
        spotlight(&mut pixels, &image, &effect).unwrap();
        assert_eq!(&pixels[0..4], &[50, 50, 50, 255]);
        assert_eq!(&pixels[4..8], &[100, 100, 100, 255]);
        assert_eq!(&pixels[8..12], &[50, 50, 50, 255]);
    }

    #[test]
    fn optimized_spotlight_matches_reference_for_alpha_and_fractional_edges() {
        let mut image = solid(7, 5, [0, 0, 0, 0]);
        for (i, p) in image.pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *p = [
                (i * 17) as u8,
                (i * 23) as u8,
                (i * 31) as u8,
                if i % 2 == 0 { 255 } else { (i * 7) as u8 },
            ];
        }
        for focus in [
            Rect {
                x: 1.5,
                y: 0.5,
                width: 3.1,
                height: 2.0,
            },
            Rect {
                x: -2.0,
                y: 3.25,
                width: 4.0,
                height: 4.0,
            },
            Rect {
                x: 20.0,
                y: 20.0,
                width: 2.0,
                height: 2.0,
            },
        ] {
            for opacity in [0.0_f32, 0.25, 0.5, 0.9, 1.0] {
                let effect = Spotlight::new(focus, opacity).unwrap();
                let mut actual = image.pixels.clone();
                let mut expected = image.pixels.clone();
                let alpha = (opacity * 255.0).round() as u8;
                for (i, pixel) in expected.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let x = (i % image.width) as f32 + 0.5;
                    let y = (i / image.width) as f32 + 0.5;
                    if x < focus.x
                        || y < focus.y
                        || x >= focus.x + focus.width
                        || y >= focus.y + focus.height
                    {
                        *pixel = source_over(*pixel, [0, 0, 0, alpha], 255);
                    }
                }
                spotlight(&mut actual, &image, &effect).unwrap();
                assert_eq!(actual, expected, "focus={focus:?}, opacity={opacity}");
            }
        }
    }

    #[test]
    fn magnifier_never_samples_original_pixels_under_redaction_in_either_order() {
        let image = solid(8, 4, [7, 10, 240, 255]);
        let mut magnifier = Shape::new(ShapeKind::Magnifier {
            data: Magnifier::new(
                Rect {
                    x: 4.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                Point::new(1.0, 1.0),
                2.0,
                MagnifierFrame::Rectangle,
            )
            .unwrap(),
        });
        magnifier.style.width = 0.25;
        magnifier.style.color = Color::rgba(255, 255, 255, 255);
        let redact = Shape::new(ShapeKind::Redact {
            start: Point::new(0.0, 0.0),
            end: Point::new(2.0, 2.0),
        });
        for shapes in [
            [redact.clone(), magnifier.clone()],
            [magnifier.clone(), redact.clone()],
        ] {
            let output = render_document(&image, &shapes).unwrap();
            let i = (source_offset(&image, 5, 1)) * 4;
            assert_eq!(&output.pixels[i..i + 4], &[0, 0, 0, 255]);
        }
    }

    fn source_offset(image: &Image, x: usize, y: usize) -> usize {
        y * image.width + x
    }

    #[test]
    fn preview_cache_never_reuses_unredacted_sampling_after_redaction_changes() {
        let image = Arc::new(solid(8, 4, [7, 10, 240, 255]));
        let mut lens = Shape::new(ShapeKind::Magnifier {
            data: Magnifier::new(
                Rect {
                    x: 4.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                Point::new(1.0, 1.0),
                2.0,
                MagnifierFrame::Rectangle,
            )
            .unwrap(),
        });
        lens.style.width = 0.25;
        let redaction = Shape::new(ShapeKind::Redact {
            start: Point::new(0.0, 0.0),
            end: Point::new(2.0, 2.0),
        });
        let mut moved = redaction.clone();
        moved.offset(1.0, 0.0);
        let mut cache = None;
        let no_redaction = [lens.clone()];
        assert_eq!(
            render_preview(&image, &no_redaction, 1, &mut cache).unwrap(),
            render_document(&image, &no_redaction).unwrap()
        );
        let original = [lens.clone(), redaction];
        let original_preview = render_preview(&image, &original, 1, &mut cache).unwrap();
        assert_eq!(
            original_preview,
            render_document(&image, &original).unwrap()
        );
        let redacted_sample = source_offset(&image, 5, 1) * 4;
        assert_eq!(
            &original_preview.pixels[redacted_sample..redacted_sample + 4],
            &[0, 0, 0, 255]
        );
        let changed = [lens.clone(), moved];
        assert_eq!(
            render_preview(&image, &changed, 1, &mut cache).unwrap(),
            render_document(&image, &changed).unwrap()
        );
        assert_eq!(
            render_preview(&image, &no_redaction, 1, &mut cache).unwrap(),
            render_document(&image, &no_redaction).unwrap()
        );
    }

    #[test]
    fn magnifier_zoom_and_frame_change_the_expected_source_pixels() {
        let mut image = solid(10, 10, [0, 0, 0, 255]);
        for y in 0..10 {
            for x in 0..10 {
                image.pixels[(y * 10 + x) * 4 + 2] = (x * 20) as u8;
            }
        }
        let viewport = Rect {
            x: 2.0,
            y: 2.0,
            width: 6.0,
            height: 6.0,
        };
        let mut lens = Magnifier::new(
            viewport,
            Point::new(5.0, 5.0),
            1.0,
            MagnifierFrame::Rectangle,
        )
        .unwrap();
        let style = Style {
            width: 0.25,
            color: Color::rgba(255, 255, 255, 255),
            ..Style::default()
        };
        let mut once = [1, 2, 3, 255].repeat(100);
        magnifier(&mut once, &image, &lens, &style).unwrap();
        let index = (4 * 10 + 3) * 4;
        assert_eq!(&once[index..index + 4], &[0, 0, 60, 255]);
        lens.set_zoom(3.0).unwrap();
        let mut thrice = [1, 2, 3, 255].repeat(100);
        magnifier(&mut thrice, &image, &lens, &style).unwrap();
        assert_eq!(&thrice[index..index + 4], &[0, 0, 80, 255]);
        lens.frame = MagnifierFrame::Circle;
        let mut round = [1, 2, 3, 255].repeat(100);
        magnifier(&mut round, &image, &lens, &style).unwrap();
        let corner = (2 * 10 + 2) * 4;
        assert_eq!(&round[corner..corner + 4], &[1, 2, 3, 255]);
        assert_ne!(&thrice[corner..corner + 4], &round[corner..corner + 4]);
    }

    #[test]
    fn mixed_transparent_border_and_content_use_disjoint_coverage() {
        // Eight subsamples each: alpha weights 8*128 and 8*64.
        assert_eq!(
            coverage_pixel([0, 0, 255, 128], [255, 0, 0, 64], 8, 8),
            [85, 0, 170, 96]
        );
        assert_eq!(
            coverage_pixel([255, 2, 3, 0], [9, 8, 7, 0], 8, 8),
            [0, 0, 0, 0]
        );
    }

    #[test]
    fn opaque_lens_content_and_border_do_not_leak_background_at_inner_edge() {
        let image = solid(20, 20, [0, 0, 255, 255]);
        let mut dest = [255, 0, 0, 255].repeat(400);
        let lens = Magnifier::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 6.0,
                height: 6.0,
            },
            Point::new(10.0, 10.0),
            2.0,
            MagnifierFrame::Rectangle,
        )
        .unwrap();
        let style = Style {
            width: 0.5,
            color: Color::rgba(255, 0, 0, 255),
            ..Style::default()
        };
        magnifier(&mut dest, &image, &lens, &style).unwrap();
        let i = (2 * 20) * 4;
        assert_eq!(&dest[i..i + 4], &[0, 0, 255, 255]);
    }

    #[test]
    fn translucent_lens_content_is_composited_over_existing_document() {
        let image = solid(10, 10, [0, 0, 200, 128]);
        let mut dest = [200, 0, 0, 255].repeat(100);
        let lens = Magnifier::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 6.0,
                height: 6.0,
            },
            Point::new(5.0, 5.0),
            2.0,
            MagnifierFrame::Rectangle,
        )
        .unwrap();
        let style = Style {
            width: 0.25,
            ..Style::default()
        };
        magnifier(&mut dest, &image, &lens, &style).unwrap();
        let i = (2 * 10 + 2) * 4;
        assert_eq!(
            &dest[i..i + 4],
            &source_over([200, 0, 0, 255], [0, 0, 200, 128], 255)
        );
    }

    #[test]
    fn effects_reject_invalid_destination_without_mutating_it() {
        let image = solid(4, 4, [20, 30, 40, 255]);
        let mut dest = vec![91; 7];
        let before = dest.clone();
        let effect = Spotlight::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            0.5,
        )
        .unwrap();
        assert!(spotlight(&mut dest, &image, &effect).is_err());
        assert_eq!(dest, before);
    }

    #[test]
    fn cached_preview_matches_export_for_effects() {
        let image = Arc::new(solid(8, 8, [140, 100, 60, 255]));
        let spotlight = Shape::new(ShapeKind::Spotlight {
            data: Spotlight::new(
                Rect {
                    x: 2.0,
                    y: 2.0,
                    width: 3.0,
                    height: 3.0,
                },
                0.5,
            )
            .unwrap(),
        });
        let mut magnifier = Shape::new(ShapeKind::Magnifier {
            data: Magnifier::new(
                Rect {
                    x: 2.0,
                    y: 2.0,
                    width: 4.0,
                    height: 4.0,
                },
                Point::new(4.0, 4.0),
                2.0,
                MagnifierFrame::Circle,
            )
            .unwrap(),
        });
        magnifier.style.width = 0.25;
        let shapes = [spotlight, magnifier];
        let expected = render_document(&image, &shapes).unwrap();
        let mut cache = None;
        assert_eq!(
            render_preview(&image, &shapes, 0, &mut cache).unwrap(),
            expected
        );
        assert_eq!(
            render_preview(&image, &shapes, 1, &mut cache).unwrap(),
            expected
        );
    }
}
