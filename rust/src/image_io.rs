//! Image loading/encoding independently of Windows. Internal pixels are BGRA.
use crate::stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION};
use image::{
    codecs::jpeg::JpegEncoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits,
    RgbImage, RgbaImage,
};
use std::{
    io::{BufReader, Cursor, Read},
    path::Path,
};

const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

/// Enforce the limit on bytes actually read, even if the file changes after opening.
pub fn read_image_file(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_IMAGE_BYTES {
        return Err("Die Bilddatei ist zu groß.".into());
    }
    read_bounded(file, MAX_IMAGE_BYTES)
}

fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Die Bilddatei ist zu groß.".into());
    }
    Ok(bytes)
}

pub fn decode(data: &[u8]) -> Result<Image, String> {
    let mut reader = ImageReader::new(BufReader::new(Cursor::new(data)))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_BYTES);
    limits.max_image_width = Some(MAX_EDITABLE_IMAGE_DIMENSION as u32);
    limits.max_image_height = Some(MAX_EDITABLE_IMAGE_DIMENSION as u32);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(|error| match error {
        image::ImageError::Limits(ref limit)
            if limit.kind() == image::error::LimitErrorKind::DimensionError =>
        {
            format!("Das Bild ist zu groß: maximal {MAX_EDITABLE_IMAGE_DIMENSION} Pixel je Seite.")
        }
        _ => error.to_string(),
    })?;
    let (width, height) = decoder.dimensions();
    validate_import_dimensions(width, height)?;
    // Mirror ImageReader::decode's allocation reservation before decoding.
    limits
        .reserve(decoder.total_bytes())
        .map_err(|e| e.to_string())?;
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    decoded.apply_orientation(orientation);
    // Consume the decoded image so already-RGBA input can reuse its pixel
    // allocation instead of making another full-size copy during import.
    let rgba = decoded.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Image::new(w as usize, h as usize, pixels).map_err(str::to_owned)
}

fn validate_import_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("Die Bildabmessungen sind ungültig.".into());
    }
    if u64::from(width) > MAX_EDITABLE_IMAGE_DIMENSION as u64
        || u64::from(height) > MAX_EDITABLE_IMAGE_DIMENSION as u64
    {
        return Err(format!(
            "Das Bild ist zu groß: maximal {MAX_EDITABLE_IMAGE_DIMENSION} Pixel je Seite."
        ));
    }
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("Die Bildgröße läuft über.")?;
    if bytes > MAX_IMAGE_BYTES {
        return Err("Das dekodierte Bild überschreitet 512 MiB.".into());
    }
    Ok(())
}

pub fn encode(image: &Image, format: ImageFormat) -> Result<Vec<u8>, String> {
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp | ImageFormat::Gif
    ) {
        return Err("Bitte PNG, JPG, BMP oder GIF wählen.".into());
    }
    let w = u32::try_from(image.width).map_err(|_| "Bildbreite zu groß")?;
    let h = u32::try_from(image.height).map_err(|_| "Bildhöhe zu groß")?;
    let mut pixels = image.pixels.clone();
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let rgba = RgbaImage::from_raw(w, h, pixels).ok_or("Ungültige Bildgröße")?;
    let mut bytes = Cursor::new(Vec::new());
    if format == ImageFormat::Jpeg {
        let output = DynamicImage::ImageRgb8(
            RgbImage::from_raw(w, h, jpeg_rgb_on_white(rgba.into_raw()))
                .ok_or("Ungültige Bildgröße")?,
        );
        JpegEncoder::new_with_quality(&mut bytes, 92)
            .encode_image(&output)
            .map_err(|e| e.to_string())?;
    } else {
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut bytes, format)
            .map_err(|e| e.to_string())?;
    }
    Ok(bytes.into_inner())
}

/// Flatten RGBA pixels onto white before JPEG encoding, which cannot store alpha.
/// Channels are rounded to the nearest integer to match ordinary source-over.
fn jpeg_rgb_on_white(rgba: Vec<u8>) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.as_chunks::<4>().0 {
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
    #[test]
    fn bounded_read_checks_actual_bytes_not_metadata() {
        assert_eq!(super::read_bounded(&b"1234"[..], 4).unwrap(), b"1234");
        assert!(super::read_bounded(&b"12345"[..], 4).is_err());
    }
    #[test]
    fn bounded_file_read_rejects_growth_after_metadata_check() {
        use std::io::Write;
        let unique = format!(
            "shotkit-growing-image-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let path = std::env::temp_dir().join(unique);
        let mut writer = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        writer.write_all(b"1234").unwrap();
        writer.flush().unwrap();
        let reader = std::fs::File::open(&path).unwrap();
        assert_eq!(reader.metadata().unwrap().len(), 4);
        writer.write_all(b"5").unwrap();
        writer.flush().unwrap();
        assert_eq!(reader.metadata().unwrap().len(), 5);
        assert_eq!(
            super::read_bounded(reader, 4).unwrap_err(),
            "Die Bilddatei ist zu groß."
        );
        drop(writer);
        std::fs::remove_file(path).unwrap();
    }
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
    fn png_import_preserves_transparent_and_semitransparent_pixels() {
        let source = Image::new(2, 1, vec![23, 45, 67, 0, 91, 122, 153, 128]).unwrap();
        let bytes = encode(&source, ImageFormat::Png).unwrap();
        assert_eq!(decode(&bytes).unwrap(), source);
    }
    #[test]
    fn jpeg_exif_orientation_is_applied_on_load() {
        let source = Image::new(
            2,
            3,
            vec![
                0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 0, 255, 255, 255, 255, 0, 255, 255,
                255, 255, 0, 255,
            ],
        )
        .unwrap();
        let jpeg = encode(&source, ImageFormat::Jpeg).unwrap();
        let unrotated = decode(&jpeg).unwrap();
        let exif = [
            b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0,
            0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut with_exif = jpeg[..2].to_vec();
        with_exif.extend_from_slice(&[0xff, 0xe1, 0, (exif.len() + 2) as u8]);
        with_exif.extend_from_slice(&exif);
        with_exif.extend_from_slice(&jpeg[2..]);
        let rotated = decode(&with_exif).unwrap();
        assert_eq!((rotated.width, rotated.height), (3, 2));
        for y in 0..unrotated.height {
            for x in 0..unrotated.width {
                let src = (y * unrotated.width + x) * 4;
                let dst = (x * rotated.width + (rotated.width - 1 - y)) * 4;
                assert_eq!(
                    &unrotated.pixels[src..src + 4],
                    &rotated.pixels[dst..dst + 4]
                );
            }
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
    fn tiff_round_trip_preserves_pixels_without_enabling_tiff_export_paths() {
        let src = sample();
        let mut rgba = src.pixels.clone();
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        let tiff_image = DynamicImage::ImageRgba8(RgbaImage::from_raw(2, 2, rgba).unwrap());
        let mut fixture = Cursor::new(Vec::new());
        tiff_image
            .write_to(&mut fixture, ImageFormat::Tiff)
            .unwrap();
        assert_eq!(decode(&fixture.into_inner()).unwrap(), src);
        assert!(encode(&src, ImageFormat::Tiff).is_err());
        assert!(format_from_path(Path::new("capture.tif")).is_err());
        assert!(format_from_path(Path::new("capture.tiff")).is_err());
    }

    #[test]
    fn jpeg_export_uses_quality_92_relative_to_quality_75() {
        let mut pixels = Vec::with_capacity(64 * 64 * 4);
        for y in 0..64usize {
            for x in 0..64usize {
                let r = ((x * 37 + y * 13 + (x * y) % 71) & 255) as u8;
                let g = ((x * 11 + y * 43 + (x * y) % 97) & 255) as u8;
                let b = ((x * 53 + y * 7 + (x * y) % 113) & 255) as u8;
                pixels.extend_from_slice(&[b, g, r, 255]);
            }
        }
        let src = Image::new(64, 64, pixels).unwrap();
        let quality_92 = decode(&encode(&src, ImageFormat::Jpeg).unwrap()).unwrap();

        let mut rgb = Vec::with_capacity(64 * 64 * 3);
        for px in src.pixels.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let rgb = RgbImage::from_raw(64, 64, rgb).unwrap();
        let mut baseline = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut baseline, 75)
            .encode_image(&DynamicImage::ImageRgb8(rgb))
            .unwrap();
        let quality_75 = decode(&baseline.into_inner()).unwrap();

        let mean_squared_error = |decoded: &Image| -> u64 {
            src.pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(decoded.pixels.as_chunks::<4>().0.iter())
                .flat_map(|(a, b)| {
                    (0..3).map(move |channel| {
                        let delta = i64::from(a[channel]) - i64::from(b[channel]);
                        (delta * delta) as u64
                    })
                })
                .sum()
        };
        assert!(
            mean_squared_error(&quality_92) < mean_squared_error(&quality_75),
            "quality 92 should retain more source detail than quality 75"
        );
    }

    #[test]
    fn jpeg_pixels_flatten_transparent_and_semitransparent_rgba_onto_white() {
        let rgba = vec![
            12, 34, 56, 0, // Hidden RGB must not leak through a fully transparent pixel.
            20, 100, 200, 128, 7, 8, 9, 255,
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
    fn import_dimension_boundaries_match_document_rendering() {
        for (width, height) in [(30_000, 1), (1, 30_000)] {
            let source = Image::new(width, height, vec![42; width * height * 4]).unwrap();
            let bytes = encode(&source, ImageFormat::Png).unwrap();
            let decoded = decode(&bytes).unwrap();
            assert_eq!(decoded, source);
            assert_eq!(
                crate::alpha_document::render_document(&decoded, &[]).unwrap(),
                source
            );
        }
        for (width, height) in [(30_001, 1), (1, 30_001)] {
            let source = Image::new(width, height, vec![42; width * height * 4]).unwrap();
            let bytes = encode(&source, ImageFormat::Png).unwrap();
            assert!(decode(&bytes).unwrap_err().contains("30000 Pixel"));
        }
    }

    #[test]
    fn import_checks_bgra_size_before_decoding() {
        assert!(validate_import_dimensions(16_384, 8_192).is_ok());
        assert!(validate_import_dimensions(16_385, 8_192)
            .unwrap_err()
            .contains("512 MiB"));
        assert!(validate_import_dimensions(0, 1).is_err());
        assert!(validate_import_dimensions(u32::MAX, u32::MAX).is_err());
    }
    #[test]
    fn reject_unsupported_extension() {
        assert!(format_from_path(Path::new("x.webp")).is_err());
    }
}
