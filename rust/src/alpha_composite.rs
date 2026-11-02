//! Lossless-in-alpha source-over compositing for straight-alpha BGRA images.
//!
//! The calculation uses integer premultiplied weights internally, then stores
//! straight-alpha BGRA. It does not mutate either input image.

use crate::stitch::Image;

const SCALE: u64 = 255 * 255;
const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;

/// Composites `src` over `dst`; `opacity` scales the source alpha (0 is a no-op,
/// 255 is full source opacity). Pixels are `[B, G, R, A]` with straight alpha.
pub fn source_over(dst: [u8; 4], src: [u8; 4], opacity: u8) -> [u8; 4] {
    let src_weight = u64::from(src[3]) * u64::from(opacity);
    if src_weight == 0 {
        return dst;
    }
    let dst_weight = u64::from(dst[3]) * (SCALE - src_weight);
    let alpha_numerator = src_weight * 255 + dst_weight;
    if alpha_numerator == 0 {
        return [0, 0, 0, 0];
    }

    let mut out = [0; 4];
    for channel in 0..3 {
        let premultiplied =
            u64::from(src[channel]) * src_weight * 255 + u64::from(dst[channel]) * dst_weight;
        out[channel] = div_round(premultiplied, alpha_numerator) as u8;
    }
    out[3] = div_round(alpha_numerator, SCALE) as u8;
    out
}

/// Composites an equally sized straight-alpha BGRA layer over `base`.
///
/// Both public `Image` fields are revalidated because callers can construct an
/// inconsistent value directly. The dimension and byte-count checks happen
/// before allocation; the returned image is independent of both inputs.
pub fn composite_layer(base: &Image, layer: &Image, opacity: u8) -> Result<Image, &'static str> {
    let base_len = checked_image_len(base)?;
    let layer_len = checked_image_len(layer)?;
    if base.width != layer.width || base.height != layer.height || base_len != layer_len {
        return Err("Bild und Ebene haben unterschiedliche Abmessungen");
    }

    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(base_len)
        .map_err(|_| "Nicht genug Speicher für Compositing")?;
    pixels.resize(base_len, 0);
    for (i, (dst, src)) in base
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(layer.pixels.as_chunks::<4>().0.iter())
        .enumerate()
    {
        let result = source_over(
            [dst[0], dst[1], dst[2], dst[3]],
            [src[0], src[1], src[2], src[3]],
            opacity,
        );
        pixels[i * 4..i * 4 + 4].copy_from_slice(&result);
    }
    Image::new(base.width, base.height, pixels)
}

fn checked_image_len(image: &Image) -> Result<usize, &'static str> {
    let len = image
        .width
        .checked_mul(image.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Bildgröße überläuft")?;
    if image.width == 0 || image.height == 0 || len != image.pixels.len() || len > MAX_IMAGE_BYTES {
        return Err("Ungültige Bildgröße");
    }
    Ok(len)
}

fn div_round(numerator: u64, denominator: u64) -> u64 {
    (numerator + denominator / 2) / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(pixels: Vec<u8>) -> Image {
        Image::new(pixels.len() / 4, 1, pixels).unwrap()
    }

    #[test]
    fn source_over_known_half_alpha_on_opaque_pixel() {
        assert_eq!(
            source_over([10, 20, 30, 255], [110, 120, 130, 128], 255),
            [60, 70, 80, 255]
        );
    }

    #[test]
    fn transparent_and_opacity_edges_are_exact() {
        assert_eq!(
            source_over([8, 9, 10, 77], [200, 100, 50, 0], 255),
            [8, 9, 10, 77]
        );
        assert_eq!(
            source_over([8, 9, 10, 77], [200, 100, 50, 255], 0),
            [8, 9, 10, 77]
        );
        assert_eq!(
            source_over([9, 8, 7, 0], [200, 100, 50, 255], 255),
            [200, 100, 50, 255]
        );
        // A zero-alpha source is an exact no-op, including hidden RGB.
        assert_eq!(
            source_over([9, 8, 7, 0], [200, 100, 50, 0], 255),
            [9, 8, 7, 0]
        );
    }

    #[test]
    fn two_marker_layers_compose_in_order_without_losing_alpha() {
        let base = img(vec![0, 0, 0, 0]);
        let marker_a = img(vec![0, 0, 255, 105]);
        let marker_b = img(vec![0, 255, 0, 105]);
        let once = composite_layer(&base, &marker_a, 255).unwrap();
        assert_eq!(once.pixels, vec![0, 0, 255, 105]);
        let twice = composite_layer(&once, &marker_b, 255).unwrap();
        assert_eq!(twice.pixels, vec![0, 161, 94, 167]);
        assert_eq!(base.pixels, vec![0, 0, 0, 0]);
    }

    #[test]
    fn opaque_base_stays_opaque_under_translucent_layer() {
        let base = img(vec![10, 20, 30, 255]);
        let layer = img(vec![110, 120, 130, 128]);
        assert_eq!(
            composite_layer(&base, &layer, 255).unwrap().pixels,
            vec![60, 70, 80, 255]
        );
    }

    #[test]
    fn rejects_mismatched_or_invalid_images() {
        let a = Image::new(1, 1, vec![0; 4]).unwrap();
        let b = Image::new(2, 1, vec![0; 8]).unwrap();
        assert!(composite_layer(&a, &b, 255).is_err());
        let invalid = Image {
            width: 2,
            height: 1,
            pixels: vec![0; 4],
        };
        assert!(composite_layer(&invalid, &invalid, 255).is_err());
    }
}
