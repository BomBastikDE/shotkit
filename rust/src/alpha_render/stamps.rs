//! Asset-free scalable symbol rasterization, shared by preview and export.
use super::RenderError;
use crate::shapes::{Point, StampKind, Style};

pub(super) fn render(
    width: usize,
    height: usize,
    center: Point,
    kind: StampKind,
    style: &Style,
    pixels: &mut [u8],
) -> Result<(), RenderError> {
    let size = style.font_size;
    if !size.is_finite()
        || !(1.0..=512.0).contains(&size)
        || !center.x.is_finite()
        || !center.y.is_finite()
    {
        return Err(RenderError::InvalidDimensions);
    }
    // Work in f64 so finite extreme centers cannot overflow while clipping.
    let left = center.x as f64 - size as f64 / 2.0;
    let top = center.y as f64 - size as f64 / 2.0;
    let x0 = left.floor().clamp(0.0, width as f64) as usize;
    let y0 = top.floor().clamp(0.0, height as f64) as usize;
    let x1 = (left + size as f64).ceil().clamp(0.0, width as f64) as usize;
    let y1 = (top + size as f64).ceil().clamp(0.0, height as f64) as usize;
    if style.color.a == 0 {
        return Ok(());
    }
    const SAMPLES: usize = 4;
    for y in y0..y1 {
        for x in x0..x1 {
            let mut covered = 0u32;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = ((x as f64 + (sx as f64 + 0.5) / SAMPLES as f64 - left) / size as f64)
                        as f32;
                    let v = ((y as f64 + (sy as f64 + 0.5) / SAMPLES as f64 - top) / size as f64)
                        as f32;
                    covered += u32::from(kind.hit_test(Point::new(u, v), 0.0));
                }
            }
            let alpha = ((style.color.a as u32 * covered + 8) / 16) as u8;
            if alpha != 0 {
                let offset = (y * width + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[
                    style.color.b,
                    style.color.g,
                    style.color.r,
                    alpha,
                ]);
            }
        }
    }
    Ok(())
}
