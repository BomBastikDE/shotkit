//! Image loading/encoding independently of Windows. Internal pixels are BGRA.
use crate::stitch::Image;
use image::{DynamicImage, ImageFormat, ImageReader, Limits, RgbImage, RgbaImage};
use std::{
    io::{BufReader, Cursor},
    path::Path,
};

const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

pub fn decode(data: &[u8]) -> Result<Image, String> {
    let mut reader = ImageReader::new(BufReader::new(Cursor::new(data)))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_BYTES);
    reader.limits(limits);
    let rgba = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Image::new(w as usize, h as usize, pixels).map_err(str::to_owned)
}

pub fn encode(image: &Image, format: ImageFormat) -> Result<Vec<u8>, String> {
    let w = u32::try_from(image.width).map_err(|_| "Bildbreite zu groß")?;
    let h = u32::try_from(image.height).map_err(|_| "Bildhöhe zu groß")?;
    let mut pixels = image.pixels.clone();
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let rgba = RgbaImage::from_raw(w, h, pixels).ok_or("Ungültige Bildgröße")?;
    let output = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(
            RgbImage::from_raw(w, h, jpeg_rgb_on_white(rgba.into_raw()))
                .ok_or("Ungültige Bildgröße")?,
        )
    } else {
        DynamicImage::ImageRgba8(rgba)
    };
    let mut bytes = Cursor::new(Vec::new());
    output
        .write_to(&mut bytes, format)
        .map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

/// Flatten RGBA pixels onto white before JPEG encoding, which cannot store alpha.
/// Channels are rounded to the nearest integer to match ordinary source-over.
fn jpeg_rgb_on_white(rgba: Vec<u8>) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &pixel[..3] {
            let value = u16::from(*channel) * alpha + 255 * (255 - alpha);
            rgb.push(((value + 127) / 255) as u8);
        }
    }
    rgb
}

pub fn format_from_path(path: &Path) -> Result<ImageFormat, String> {
    let fmt = ImageFormat::from_path(path)
        .map_err(|_| "Bitte PNG, JPG, BMP oder GIF wählen.".to_owned())?;
    if matches!(
        fmt,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp | ImageFormat::Gif
    ) {
        Ok(fmt)
    } else {
        Err("Bitte PNG, JPG, BMP oder GIF wählen.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Image {
        Image::new(
            2,
            2,
            vec![
                0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
            ],
        )
        .unwrap()
    }
    #[test]
    fn png_and_bmp_round_trip_color() {
        for fmt in [ImageFormat::Png, ImageFormat::Bmp] {
            let src = sample();
            let bytes = encode(&src, fmt).unwrap();
            let dst = decode(&bytes).unwrap();
            assert_eq!(src, dst);
        }
    }
    #[test]
    fn gif_and_jpeg_are_readable() {
        for fmt in [ImageFormat::Gif, ImageFormat::Jpeg] {
            let bytes = encode(&sample(), fmt).unwrap();
            let image = decode(&bytes).unwrap();
            assert_eq!((image.width, image.height), (2, 2));
        }
    }

    #[test]
    fn jpeg_pixels_flatten_transparent_and_semitransparent_rgba_onto_white() {
        let rgba = vec![
            12, 34, 56, 0, // Hidden RGB must not leak through a fully transparent pixel.
            20, 100, 200, 128,
            7, 8, 9, 255,
        ];
        assert_eq!(
            jpeg_rgb_on_white(rgba),
            vec![255, 255, 255, 137, 177, 227, 7, 8, 9]
        );
    }

    #[test]
    fn jpeg_encoding_of_transparent_image_decodes_as_white() {
        let transparent = Image::new(1, 1, vec![56, 34, 12, 0]).unwrap();
        let bytes = encode(&transparent, ImageFormat::Jpeg).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.pixels, vec![255, 255, 255, 255]);
    }

    #[test]
    fn invalid_data_is_rejected() {
        assert!(decode(b"not an image").is_err());
    }
    #[test]
    fn reject_unsupported_extension() {
        assert!(format_from_path(Path::new("x.webp")).is_err());
    }
}
