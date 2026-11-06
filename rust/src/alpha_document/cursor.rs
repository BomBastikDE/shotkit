//! Composite an owned cursor over the current document without native handles.
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
                    let alpha = u64::from(p[3]);
                    if alpha == 0 {
                        continue;
                    }
                    let dst_alpha = u64::from(pixels[dst + 3]);
                    let remaining = 255 - alpha;
                    let divisor = alpha * 255 + dst_alpha * remaining;
                    for channel in 0..3 {
                        let premul = u64::from(p[2 - channel]);
                        let background = u64::from(pixels[dst + channel]);
                        let numerator = premul * 255 * 255 + background * dst_alpha * remaining;
                        pixels[dst + channel] = ((numerator + divisor / 2) / divisor) as u8;
                    }
                    pixels[dst + 3] = ((divisor + 127) / 255) as u8;
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
        assert_eq!(&dest[0..4], &[5, 10, 115, 255]);
        assert_eq!(&dest[4..], &source.pixels[4..]);
    }

    #[test]
    fn premultiplied_alpha_does_not_round_twice() {
        let source = Image::new(1, 1, vec![64, 64, 64, 255]).unwrap();
        let sprite = CursorSprite::new(
            Point::new(0.0, 0.0),
            1,
            1,
            CursorPixels::PremultipliedRgba(vec![1, 1, 1, 2]),
        )
        .unwrap();
        let mut dest = source.pixels.clone();
        cursor(&mut dest, &source, &sprite).unwrap();
        assert_eq!(dest, [64, 64, 64, 255]);
    }

    #[test]
    fn premultiplied_alpha_over_partial_destination() {
        let source = Image::new(1, 1, vec![80, 40, 20, 128]).unwrap();
        let sprite = CursorSprite::new(
            Point::new(0.0, 0.0),
            1,
            1,
            CursorPixels::PremultipliedRgba(vec![32, 0, 0, 64]),
        )
        .unwrap();
        let mut dest = source.pixels.clone();
        cursor(&mut dest, &source, &sprite).unwrap();
        assert_eq!(dest, [48, 24, 63, 160]);
    }
}
