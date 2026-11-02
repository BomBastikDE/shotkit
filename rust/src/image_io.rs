//! Image loading/encoding independently of Windows. Internal pixels are BGRA.
use crate::stitch::Image;
use image::{DynamicImage, ImageFormat, ImageReader, Limits, RgbaImage};
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
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).to_rgb8())
    } else {
        DynamicImage::ImageRgba8(rgba)
    };
    let mut bytes = Cursor::new(Vec::new());
    output
        .write_to(&mut bytes, format)
        .map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
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
    fn invalid_data_is_rejected() {
        assert!(decode(b"not an image").is_err());
    }
    #[test]
    fn reject_unsupported_extension() {
        assert!(format_from_path(Path::new("x.webp")).is_err());
    }
}
