//! Composite an owned cursor over the current document without native handles.
use crate::alpha_composite::source_over;
use crate::cursor_snapshot::CursorPixels;
use crate::shapes::CursorSprite;
use crate::stitch::Image;

pub(super) fn cursor(
    pixels: &mut [u8],
    image: &Image,
    sprite: &CursorSprite,
) -> Result<(), String> {
    let len = image
        .width
        .checked_mul(image.height)
        .and_then(|n| n.checked_mul(4))
        .filter(|&n| {
            n == pixels.len() && n == image.pixels.len() && image.width > 0 && image.height > 0
        })
        .ok_or_else(|| "Ungültige Bilddaten für den Mauszeiger.".to_owned())?;
    let _ = len;
    if !sprite.validate() {
        return Err("Ungültige Mauszeigerdaten.".into());
    }
    let x0 = sprite.origin.x.round() as i64;
    let y0 = sprite.origin.y.round() as i64;
    for sy in 0..sprite.height as usize {
        let y = y0 + sy as i64;
        if y < 0 || y >= image.height as i64 {
            continue;
        }
        for sx in 0..sprite.width as usize {
            let x = x0 + sx as i64;
            if x < 0 || x >= image.width as i64 {
                continue;
            }
            let src = sy * sprite.width as usize + sx;
            let dst = ((y as usize * image.width) + x as usize) * 4;
            match sprite.pixels.as_ref() {
                CursorPixels::PremultipliedRgba(rgba) => {
                    let p = &rgba[src * 4..src * 4 + 4];
                    let alpha = u32::from(p[3]);
                    if alpha == 0 {
                        continue;
                    }
                    let straight = |channel: u8| {
                        ((u32::from(channel) * 255 + alpha / 2) / alpha).min(255) as u8
                    };
                    let layer = [straight(p[2]), straight(p[1]), straight(p[0]), p[3]];
                    let original: [u8; 4] = pixels[dst..dst + 4]
                        .try_into()
                        .expect("validated destination pixel");
                    pixels[dst..dst + 4].copy_from_slice(&source_over(original, layer, 255));
                }
                CursorPixels::Raster { and_mask, xor_rgb } => {
                    let mask = and_mask[src];
                    let color = &xor_rgb[src * 3..src * 3 + 3];
                    pixels[dst] = (pixels[dst] & mask) ^ color[2];
                    pixels[dst + 1] = (pixels[dst + 1] & mask) ^ color[1];
                    pixels[dst + 2] = (pixels[dst + 2] & mask) ^ color[0];
                    // Legacy raster cursors operate on opaque screen pixels.
                    // Replacement pixels are opaque; transparent/invert pixels
                    // retain the alpha of the current document underneath.
                    if mask == 0 {
                        pixels[dst + 3] = 255;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Point;

    fn image() -> Image {
        Image::new(3, 1, [10, 20, 30, 255].repeat(3)).unwrap()
    }

    #[test]
    fn raster_cursor_preserves_transparent_inverts_and_replaces() {
        let source = image();
        let sprite = CursorSprite::new(
            Point::new(0.0, 0.0),
            3,
            1,
            CursorPixels::Raster {
                and_mask: vec![255, 255, 0],
                xor_rgb: vec![0, 0, 0, 255, 255, 255, 3, 2, 1],
            },
        )
        .unwrap();
        let mut dest = source.pixels.clone();
        cursor(&mut dest, &source, &sprite).unwrap();
        assert_eq!(&dest[0..4], &[10, 20, 30, 255]);
        assert_eq!(&dest[4..8], &[245, 235, 225, 255]);
        assert_eq!(&dest[8..12], &[1, 2, 3, 255]);
    }

    #[test]
    fn alpha_cursor_composites_and_clips_negative_origin() {
        let source = image();
        let sprite = CursorSprite::new(
            Point::new(-1.0, 0.0),
            2,
            1,
            CursorPixels::PremultipliedRgba(vec![0, 0, 0, 0, 100, 0, 0, 128]),
        )
        .unwrap();
        let mut dest = source.pixels.clone();
        cursor(&mut dest, &source, &sprite).unwrap();
        let straight = ((100u32 * 255 + 64) / 128) as u8;
        assert_eq!(
            &dest[0..4],
            &source_over([10, 20, 30, 255], [0, 0, straight, 128], 255)
        );
        assert_eq!(&dest[4..], &source.pixels[4..]);
    }
}
