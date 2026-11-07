//! Image loading/encoding independently of Windows. Internal pixels are BGRA.
use crate::resource_budget::{AdmissionClass, BudgetError, BudgetLease, ResourceBudget};
use crate::stitch::{Image, MAX_EDITABLE_IMAGE_DIMENSION};
#[cfg(test)]
use image::RgbImage;
use image::{
    codecs::jpeg::JpegEncoder, DynamicImage, GenericImageView, ImageDecoder, ImageFormat,
    ImageReader, Limits, Rgb, RgbaImage,
};
use std::{
    io::{BufRead, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write},
    path::Path,
};

const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
// Policy allowance, not a measured or enforced bound on external codec memory.
const IMPORT_WORKSPACE_ALLOWANCE: usize = 16 * 1024 * 1024;

/// A successful decode owns its pixels even if final capacity accounting fails.
/// Callers must retain this value through admission/recovery instead of turning
/// `post_decode_error` into an error that drops the image. Image is declared
/// before lease so pixels are destroyed before their accounting is released.
#[derive(Debug)]
#[must_use = "retain the decoded image and lease through handoff or explicit recovery"]
pub struct BudgetedImage {
    pub image: Image,
    pub lease: BudgetLease,
    pub post_decode_error: Option<String>,
}

impl BudgetedImage {
    /// The caller has already reserved before decoding/allocation. Finalize
    /// against actual retained capacity, preserving both values on rejection.
    /// An exceptional rejected growth keeps the original, insufficient lease;
    /// it must go to recovery, not be described as fully accounted pixels.
    pub fn from_decoded(image: Image, mut lease: BudgetLease) -> Self {
        let post_decode_error = lease
            .try_resize(image.pixels.capacity())
            .err()
            .map(|error| format!("Bildspeicher konnte nicht vollständig verbucht werden: {error}"));
        Self {
            image,
            lease,
            post_decode_error,
        }
    }

    /// Record a cleanup failure without giving up a successful decode.
    pub fn retain_error(&mut self, message: String) {
        self.post_decode_error = Some(match self.post_decode_error.take() {
            Some(previous) => format!("{previous}; {message}"),
            None => message,
        });
    }
}

fn import_budget_error(error: BudgetError) -> String {
    format!("Das Bild kann mit dem verfügbaren Importbudget nicht geöffnet werden. Schließen Sie andere importierte Bilder und versuchen Sie es erneut. Bereits geöffnete Bilder bleiben erhalten. ({error})")
}

mod bounded_reader;

/// Enforce the limit on bytes actually read, even if the file changes after opening.
pub fn read_image_file(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_IMAGE_BYTES {
        return Err("Die Bilddatei ist zu groß.".into());
    }
    read_bounded(file, MAX_IMAGE_BYTES)
}

fn read_bounded(mut reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        // Include one probe byte without overflowing even for a u64::MAX limit.
        let remaining = limit - bytes.len() as u64;
        let request = remaining.min((chunk.len() - 1) as u64) as usize + 1;
        let count = match reader.read(&mut chunk[..request]) {
            Ok(0) => return Ok(bytes),
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        };
        if count as u64 > remaining {
            return Err("Die Bilddatei ist zu groß.".into());
        }
        // Reserve fallibly before extending: extend_from_slice then cannot
        // invoke Vec's infallible growth path. Geometric 25% growth limits
        // excess address-space reservation while remaining amortized linear;
        // the allocator may still reserve more or move an existing buffer.
        let needed = bytes
            .len()
            .checked_add(count)
            .ok_or("Bilddatei benötigt zu viel Speicher.")?;
        if needed > bytes.capacity() {
            let capacity_limit = usize::try_from(limit).unwrap_or(usize::MAX);
            let growth = bytes.capacity().saturating_add(bytes.capacity() / 4);
            let capacity = needed.max(growth.min(capacity_limit));
            bytes
                .try_reserve_exact(capacity - bytes.len())
                .map_err(|_| "Nicht genügend Speicher zum Lesen der Bilddatei.".to_owned())?;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

pub fn decode(data: &[u8]) -> Result<Image, String> {
    let reader = ImageReader::new(BufReader::new(Cursor::new(data)))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    decode_reader(reader)
}

/// Decode borrowed encoded bytes using the same peak policy as file imports.
/// No complete encoded-data copy is made. The caller keeps the source stable
/// until return (for clipboard memory, hold its native lock). An error means
/// no successful Image was produced; post-decode errors travel with ownership.
pub fn decode_with_budget(data: &[u8], budget: &ResourceBudget) -> Result<BudgetedImage, String> {
    let mut lease = budget
        .try_acquire(IMPORT_WORKSPACE_ALLOWANCE, AdmissionClass::Normal)
        .map_err(import_budget_error)?;
    let reader = ImageReader::new(BufReader::new(Cursor::new(data)))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let image = decode_reader_admitted(reader, Some(&mut lease))?;
    Ok(BudgetedImage::from_decoded(image, lease))
}

/// Decode a file without retaining a second, complete copy of its encoded bytes.
/// On Windows the open handle refuses concurrent write/delete access until the
/// decoder finishes. Other platforms do not provide that guarantee here.
pub fn decode_file(path: &Path) -> Result<(Image, Option<ImageFormat>), String> {
    decode_file_inner(path, None)
}

/// Admit an import against a shared budget and return ownership of its pixels
/// together with their lease. The caller must keep that lease alive until the
/// image is disposed of (including any immutable shared owner of both values).
///
/// Policy: reserve a 16-MiB workspace allowance before opening/recognizing the
/// file; after header parsing reserve that allowance plus two decoder raster
/// buffers and one RGBA buffer for orientation/conversion before decoding. All
/// size arithmetic is checked. Successful import retains pixels.capacity().
/// Imports use Normal admission, preserving the budget's configured headroom.
/// Accounting is constant-time per admission; pixel conversion is linear in
/// pixel count. This path retains no complete encoded-file copy. Reservation
/// does not itself allocate the represented bytes.
///
/// This is admission accounting for this import, not a process memory ceiling:
/// codecs can allocate while parsing headers, potentially exceeding the initial
/// 16-MiB allowance, and their internal scratch memory is not measured or capped
/// by this allowance. Allocator overhead, other allocations and OS memory are
/// uncounted. Existing decoder/file limits remain independently enforced.
/// The returned lease does not automatically account for later image clones or
/// edits. Errors release this import's reservation through RAII.
pub fn decode_file_with_budget(
    path: &Path,
    budget: &ResourceBudget,
) -> Result<(Image, Option<ImageFormat>, BudgetLease), String> {
    let mut lease = budget
        .try_acquire(IMPORT_WORKSPACE_ALLOWANCE, AdmissionClass::Normal)
        .map_err(import_budget_error)?;
    let (image, format) = decode_file_inner(path, Some(&mut lease))?;
    // Decoder, orientation/conversion temporaries and file reader are gone.
    lease
        .try_resize(image.pixels.capacity())
        .map_err(|error| error.to_string())?;
    Ok((image, format, lease))
}

fn decode_file_inner(
    path: &Path,
    lease: Option<&mut BudgetLease>,
) -> Result<(Image, Option<ImageFormat>), String> {
    let file = open_image_file(path, MAX_IMAGE_BYTES)?;
    // Keep the same file object available for the final length check and, on
    // Windows, keep the read-only sharing contract alive until that check.
    let guard = file.try_clone().map_err(|e| e.to_string())?;
    let bounded =
        bounded_reader::BoundedReader::new(file, MAX_IMAGE_BYTES).map_err(|e| e.to_string())?;
    let reader = ImageReader::new(BufReader::new(bounded))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader.format();
    let image = decode_reader_admitted(reader, lease)?;
    if guard.metadata().map_err(|e| e.to_string())?.len() > MAX_IMAGE_BYTES {
        return Err("Die Bilddatei ist zu groß.".into());
    }
    Ok((image, format))
}

fn open_image_file(path: &Path, limit: u64) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ; refuse concurrent writes or replacement.
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err("Die Bilddatei ist zu groß.".into());
    }
    Ok(file)
}

fn decode_reader<R: BufRead + Seek>(reader: ImageReader<R>) -> Result<Image, String> {
    decode_reader_admitted(reader, None)
}

fn import_peak_reservation(width: u32, height: u32, decoder_bytes: u64) -> Result<usize, String> {
    let rgba = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("Die Bildgröße läuft über.")?;
    let bytes = decoder_bytes
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(rgba))
        .and_then(|bytes| bytes.checked_add(IMPORT_WORKSPACE_ALLOWANCE as u64))
        .ok_or("Die Bildgröße läuft über.")?;
    usize::try_from(bytes).map_err(|_| "Die Bildgröße läuft über.".into())
}

fn decode_reader_admitted<R: BufRead + Seek>(
    mut reader: ImageReader<R>,
    lease: Option<&mut BudgetLease>,
) -> Result<Image, String> {
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
    if let Some(lease) = lease {
        lease
            .try_resize(import_peak_reservation(
                width,
                height,
                decoder.total_bytes(),
            )?)
            .map_err(import_budget_error)?;
    }
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
    let mut bytes = Cursor::new(Vec::new());
    if matches!(format, ImageFormat::Jpeg | ImageFormat::Bmp) {
        write_bmp_or_jpeg(image, &mut bytes, format)?;
    } else if format == ImageFormat::Png {
        let expected = image
            .width
            .checked_mul(image.height)
            .and_then(|n| n.checked_mul(4))
            .ok_or("Ungültige Bildgröße")?;
        // ImageBuffer::from_raw accepts surplus bytes, but image's PNG encoder
        // asserts an exact length. Return an error instead of that panic.
        if image.pixels.len() != expected {
            return Err("Ungültige Bildgröße".into());
        }
        let stored_bound =
            png_stored_size_bound(expected, image.height).ok_or("Ungültige Bildgröße")?;
        write_png_stream(&mut bytes, w, h, &image.pixels, png::Compression::Fast)?;
        // Unlike write_image_data, png's Fast stream has no automatic stored
        // fallback and can expand noise by over 30%. Retry once in the same
        // output allocation, retaining the fast path for compressible images.
        if bytes.get_ref().len() > stored_bound {
            bytes.get_mut().clear();
            bytes.set_position(0);
            write_png_stream(
                &mut bytes,
                w,
                h,
                &image.pixels,
                png::Compression::NoCompression,
            )?;
        }
    } else {
        write_gif_stream(image, &mut bytes)?;
    }
    Ok(bytes.into_inner())
}

// GIF's underlying encoder writes its trailer in Drop and ignores errors.
// Retain every non-interrupted writer failure until after the encoder is gone.
struct StickyWriter<'a, W> {
    inner: &'a mut W,
    failure: Option<String>,
}

impl<W: Write> Write for StickyWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let result = self.inner.write(data);
        match &result {
            Ok(0) if !data.is_empty() => self
                .failure
                .get_or_insert_with(|| "GIF-Schreiben lieferte keine Daten.".into()),
            Err(error) if error.kind() != std::io::ErrorKind::Interrupted => {
                self.failure.get_or_insert_with(|| error.to_string())
            }
            _ => return result,
        };
        result
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let result = self.inner.flush();
        if let Err(error) = &result {
            if error.kind() != std::io::ErrorKind::Interrupted {
                self.failure.get_or_insert_with(|| error.to_string());
            }
        }
        result
    }
}

fn write_gif_stream(image: &Image, output: &mut impl Write) -> Result<(), String> {
    let w = u32::try_from(image.width).map_err(|_| "Bildbreite zu groß")?;
    let h = u32::try_from(image.height).map_err(|_| "Bildhöhe zu groß")?;
    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Ungültige Bildgröße")?;
    if w == 0 || h == 0 || image.pixels.len() != expected {
        return Err("Ungültige Bildgröße".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(expected)
        .map_err(|_| "Nicht genügend Speicher für den Bildexport.".to_owned())?;
    pixels.extend_from_slice(&image.pixels);
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let rgba = RgbaImage::from_raw(w, h, pixels).ok_or("Ungültige Bildgröße")?;
    let mut checked = StickyWriter {
        inner: output,
        failure: None,
    };
    let result = {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut checked);
        encoder
            .encode_frame(image::Frame::new(rgba))
            .map_err(|e| e.to_string())
    };
    if let Some(failure) = checked.failure {
        return Err(failure);
    }
    result
}

/// Encode GIF into a held temporary file; caller syncs and publishes on success.
pub fn write_gif_file(image: &Image, file: &mut std::fs::File) -> Result<(), String> {
    file.set_len(0).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    write_buffered_gif(image, file)
}

fn write_buffered_gif(image: &Image, output: &mut impl Write) -> Result<(), String> {
    let mut writer = BufWriter::new(output);
    write_gif_stream(image, &mut writer)?;
    writer.flush().map_err(|e| e.to_string())
}

fn write_bmp_or_jpeg(
    image: &Image,
    output: &mut (impl Write + Seek),
    format: ImageFormat,
) -> Result<(), String> {
    let w = u32::try_from(image.width).map_err(|_| "Bildbreite zu groß")?;
    let h = u32::try_from(image.height).map_err(|_| "Bildhöhe zu groß")?;
    match format {
        ImageFormat::Jpeg => {
            let expected = image
                .width
                .checked_mul(image.height)
                .and_then(|n| n.checked_mul(4))
                .ok_or("Ungültige Bildgröße")?;
            let bgra = image.pixels.get(..expected).ok_or("Ungültige Bildgröße")?;
            let view = JpegRgbView {
                bgra,
                width: w,
                height: h,
            };
            JpegEncoder::new_with_quality(output, 92)
                .encode_image(&view)
                .map_err(|e| e.to_string())
        }
        ImageFormat::Bmp => {
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(image.pixels.len())
                .map_err(|_| "Nicht genügend Speicher für den Bildexport.".to_owned())?;
            pixels.extend_from_slice(&image.pixels);
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            let rgba = RgbaImage::from_raw(w, h, pixels).ok_or("Ungültige Bildgröße")?;
            DynamicImage::ImageRgba8(rgba)
                .write_to(output, format)
                .map_err(|e| e.to_string())
        }
        _ => Err("Dieses Format wird nicht als BMP oder JPEG kodiert.".into()),
    }
}

/// Encode into a held temporary file; the caller must sync and publish it.
pub fn write_bmp_or_jpeg_file(
    image: &Image,
    file: &mut std::fs::File,
    format: ImageFormat,
) -> Result<(), String> {
    file.set_len(0).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    write_buffered_bmp_or_jpeg(image, file, format)
}

fn write_buffered_bmp_or_jpeg(
    image: &Image,
    output: &mut (impl Write + Seek),
    format: ImageFormat,
) -> Result<(), String> {
    let mut writer = BufWriter::new(output);
    write_bmp_or_jpeg(image, &mut writer, format)?;
    writer.flush().map_err(|e| e.to_string())
}

const PNG_STREAM_CHUNK_BYTES: usize = 4096;

// Include scanline filter bytes, zlib's header/checksum, stored-block headers,
// PNG signature/IHDR/IEND, and IDAT framing. The current flate2/miniz backend
// stores at least 31 KiB per full block; budget a header per 16 KiB plus three
// blocks for rounding/SYNC/FINAL. Two extra IDAT headers cover partial chunks
// emitted by finalization as well as an empty final chunk.
fn png_stored_size_bound(pixel_bytes: usize, height: usize) -> Option<usize> {
    let filtered = pixel_bytes.checked_add(height)?;
    let block_headers = (filtered / (16 * 1024)).checked_add(3)?.checked_mul(5)?;
    let zlib = filtered.checked_add(block_headers)?.checked_add(6)?;
    let chunk_headers = (zlib / PNG_STREAM_CHUNK_BYTES)
        .checked_add(2)?
        .checked_mul(12)?;
    zlib.checked_add(chunk_headers)?.checked_add(45)
}

fn write_png_stream(
    output: &mut impl Write,
    width: u32,
    height: u32,
    bgra: &[u8],
    compression: png::Compression,
) -> Result<(), String> {
    let mut encoder = png::Encoder::new(output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(compression);
    // Match image 0.25's normal PNG defaults; stored data needs no filtering.
    encoder.set_filter(if matches!(compression, png::Compression::NoCompression) {
        png::Filter::NoFilter
    } else {
        png::Filter::Adaptive
    });
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    {
        let mut stream = writer
            .stream_writer_with_size(PNG_STREAM_CHUNK_BYTES)
            .map_err(|e| e.to_string())?;
        // Conversion storage stays bounded even for wider scanlines. The
        // encoder buffers its own scanlines and compressed IDAT chunks.
        let mut rgba = [0u8; 32 * 1024];
        for chunk in bgra.chunks(rgba.len()) {
            let converted = &mut rgba[..chunk.len()];
            for (src, dst) in chunk
                .as_chunks::<4>()
                .0
                .iter()
                .zip(converted.as_chunks_mut::<4>().0.iter_mut())
            {
                dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
            }
            stream.write_all(converted).map_err(|e| e.to_string())?;
        }
        stream.finish().map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())
}

/// Encode PNG into a caller-owned temporary file. The caller must publish the
/// file only after this function and its own durability checks succeed.
/// The file is truncated before use and again if the bounded stored fallback
/// is needed, so a previous attempt cannot leave trailing bytes.
pub fn write_png_file(image: &Image, file: &mut std::fs::File) -> Result<(), String> {
    let width = u32::try_from(image.width).map_err(|_| "Bildbreite zu groß")?;
    let height = u32::try_from(image.height).map_err(|_| "Bildhöhe zu groß")?;
    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Ungültige Bildgröße")?;
    if image.pixels.len() != expected {
        return Err("Ungültige Bildgröße".into());
    }
    let bound = png_stored_size_bound(expected, image.height).ok_or("Ungültige Bildgröße")?;
    file.set_len(0).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    write_png_stream(file, width, height, &image.pixels, png::Compression::Fast)?;
    if file.stream_position().map_err(|e| e.to_string())? > bound as u64 {
        file.set_len(0).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        write_png_stream(
            file,
            width,
            height,
            &image.pixels,
            png::Compression::NoCompression,
        )?;
    }
    Ok(())
}

/// Borrow BGRA and flatten one pixel at a time as the JPEG encoder reads it.
#[derive(Clone, Copy)]
struct JpegRgbView<'a> {
    bgra: &'a [u8],
    width: u32,
    height: u32,
}

impl GenericImageView for JpegRgbView<'_> {
    type Pixel = Rgb<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let offset = ((y as usize * self.width as usize) + x as usize) * 4;
        let pixel = &self.bgra[offset..offset + 4];
        let alpha = u16::from(pixel[3]);
        let flatten =
            |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        Rgb([flatten(pixel[2]), flatten(pixel[1]), flatten(pixel[0])])
    }
}

/// Flatten borrowed BGRA pixels directly into RGB on white for JPEG.
/// This avoids an intermediate full-size RGBA copy of the source image.
/// Channels are rounded to the nearest integer to match ordinary source-over.
#[cfg(test)]
fn jpeg_rgb_on_white(bgra: &[u8]) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::new();
    rgb.try_reserve_exact(bgra.len() / 4 * 3)
        .map_err(|_| "Nicht genügend Speicher zum Kodieren des Bildes.".to_owned())?;
    for pixel in bgra.as_chunks::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in pixel[..3].iter().rev() {
            let value = u16::from(*channel) * alpha + 255 * (255 - alpha);
            rgb.push(((value + 127) / 255) as u8);
        }
    }
    Ok(rgb)
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
    fn bounded_read_handles_empty_and_chunk_boundaries() {
        assert!(super::read_bounded(&b""[..], 0).unwrap().is_empty());
        assert_eq!(
            super::read_bounded(&b"1"[..], 0).unwrap_err(),
            "Die Bilddatei ist zu groß."
        );
        let data = vec![42; 8193];
        assert_eq!(super::read_bounded(data.as_slice(), 8193).unwrap(), data);
        assert_eq!(
            super::read_bounded(data.as_slice(), 8192).unwrap_err(),
            "Die Bilddatei ist zu groß."
        );
        assert_eq!(super::read_bounded(&b"1"[..], u64::MAX).unwrap(), b"1");
    }
    #[test]
    fn bounded_read_preserves_short_reads_across_multiple_growths() {
        struct ShortReads<'a>(&'a [u8]);
        impl std::io::Read for ShortReads<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = output.len().min(self.0.len()).min(37);
                output[..count].copy_from_slice(&self.0[..count]);
                self.0 = &self.0[count..];
                Ok(count)
            }
        }
        let bytes: Vec<_> = (0..100_000).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            super::read_bounded(ShortReads(&bytes), bytes.len() as u64).unwrap(),
            bytes
        );
        assert_eq!(
            super::read_bounded(ShortReads(&bytes), (bytes.len() - 1) as u64).unwrap_err(),
            "Die Bilddatei ist zu groß."
        );
    }
    #[test]
    fn bounded_read_retries_interrupts_and_reports_read_errors() {
        struct InterruptedThenData(bool);
        impl std::io::Read for InterruptedThenData {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if !self.0 {
                    self.0 = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                b"1234".as_slice().read(output)
            }
        }
        // The limit probe reaches a real read failure after partial input.
        struct FailsAfterData(bool);
        impl std::io::Read for FailsAfterData {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    return Err(std::io::Error::other("image read failed"));
                }
                self.0 = true;
                output[0] = 42;
                Ok(1)
            }
        }
        let mut reader = InterruptedThenData(false).take(4);
        assert_eq!(super::read_bounded(&mut reader, 4).unwrap(), b"1234");
        assert_eq!(
            super::read_bounded(FailsAfterData(false), 1).unwrap_err(),
            "image read failed"
        );
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
    fn stream_test_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "shotkit-stream-{}-{:?}-{}-{suffix}",
            std::process::id(),
            std::thread::current().id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn file_decode_matches_byte_decode_and_detects_actual_format() {
        let image = sample();
        for format in [
            ImageFormat::Png,
            ImageFormat::Bmp,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
        ] {
            let bytes = encode(&image, format).unwrap();
            // Intentionally misleading extension: format must follow bytes.
            let path = stream_test_path("misleading.png");
            std::fs::write(&path, &bytes).unwrap();
            let (decoded, detected) = decode_file(&path).unwrap();
            assert_eq!(detected, Some(format));
            assert_eq!(decoded, decode(&bytes).unwrap());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn budgeted_file_decode_retains_exact_pixel_capacity_until_release() {
        for format in [
            ImageFormat::Png,
            ImageFormat::Bmp,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::Tiff,
        ] {
            let mut bytes = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(
                RgbaImage::from_raw(2, 1, vec![67, 45, 23, 0, 153, 122, 91, 128]).unwrap(),
            )
            .write_to(&mut bytes, format)
            .unwrap();
            let path = stream_test_path("budgeted-wrong-extension.jpg");
            std::fs::write(&path, bytes.get_ref()).unwrap();
            let budget = ResourceBudget::new(64 * 1024 * 1024, 1024).unwrap();
            let existing = budget.try_acquire(17, AdmissionClass::Reserved).unwrap();
            let (image, detected, lease) = decode_file_with_budget(&path, &budget).unwrap();
            assert_eq!(image, decode(bytes.get_ref()).unwrap());
            assert_eq!(detected, Some(format));
            assert_eq!(lease.class(), AdmissionClass::Normal);
            assert_eq!(lease.bytes(), image.pixels.capacity());
            assert_eq!(budget.snapshot().used_bytes, 17 + image.pixels.capacity());
            // Caller disposes of its pixels before releasing their accounting.
            drop(image);
            drop(lease);
            assert_eq!(budget.snapshot().used_bytes, 17);
            drop(existing);
            assert_eq!(budget.snapshot().used_bytes, 0);
            assert_eq!(std::fs::read(&path).unwrap(), bytes.into_inner());
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn borrowed_budgeted_decode_preserves_png_pixels_alpha_and_retained_capacity() {
        let original = Image::new(2, 1, vec![17, 31, 53, 0, 71, 97, 127, 128]).unwrap();
        let bytes = encode(&original, ImageFormat::Png).unwrap();
        let budget = ResourceBudget::new(64 * 1024 * 1024, 1024).unwrap();
        let existing = budget.try_acquire(37, AdmissionClass::Normal).unwrap();
        let decoded = decode_with_budget(&bytes, &budget).unwrap();
        assert_eq!(decoded.image, original);
        assert!(decoded.post_decode_error.is_none());
        assert_eq!(decoded.lease.class(), AdmissionClass::Normal);
        assert_eq!(decoded.lease.bytes(), decoded.image.pixels.capacity());
        assert_eq!(
            budget.snapshot().used_bytes,
            37 + decoded.image.pixels.capacity()
        );
        drop(decoded);
        assert_eq!(budget.snapshot().used_bytes, 37);
        drop(existing);
        assert_eq!(budget.snapshot().used_bytes, 0);
        assert_eq!(decode(&bytes).unwrap(), original); // borrowed input stayed intact
    }

    #[test]
    fn borrowed_budgeted_decode_rejects_initial_and_peak_admission_without_touching_existing_lease() {
        let bytes = encode(&sample(), ImageFormat::Png).unwrap();
        for ceiling in [IMPORT_WORKSPACE_ALLOWANCE - 1, IMPORT_WORKSPACE_ALLOWANCE] {
            let budget = ResourceBudget::new(ceiling + 37, 0).unwrap();
            let existing = budget.try_acquire(37, AdmissionClass::Normal).unwrap();
            let error = decode_with_budget(&bytes, &budget).unwrap_err();
            assert!(error.contains("cannot reserve"), "{error}");
            assert_eq!(budget.snapshot().used_bytes, existing.bytes());
        }
    }

    #[test]
    fn borrowed_budgeted_decode_errors_release_workspace() {
        let budget = ResourceBudget::new(64 * 1024 * 1024, 0).unwrap();
        assert!(decode_with_budget(b"invalid header", &budget).is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
        let mut bytes = encode(&sample(), ImageFormat::Png).unwrap();
        bytes.truncate(bytes.len() / 2);
        assert!(decode_with_budget(&bytes, &budget).is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn budgeted_import_converts_sixteen_bit_pixels_with_identical_results() {
        let source = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(
            2,
            1,
            vec![0, 32768, 65535, 0, 65535, 12345, 45678, 32768],
        )
        .unwrap();
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba16(source)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let path = stream_test_path("sixteen-bit.png");
        std::fs::write(&path, bytes.get_ref()).unwrap();
        let budget = ResourceBudget::new(64 * 1024 * 1024, 0).unwrap();
        let (image, format, lease) = decode_file_with_budget(&path, &budget).unwrap();
        assert_eq!(image, decode(bytes.get_ref()).unwrap());
        assert_eq!(format, Some(ImageFormat::Png));
        assert_eq!(lease.bytes(), image.pixels.capacity());
        drop(image);
        drop(lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn budgeted_import_rejects_initial_admission_before_file_access() {
        let path = stream_test_path("does-not-exist");
        let budget = ResourceBudget::new(IMPORT_WORKSPACE_ALLOWANCE + 37, 1).unwrap();
        let existing = budget.try_acquire(37, AdmissionClass::Normal).unwrap();
        let error = decode_file_with_budget(&path, &budget).unwrap_err();
        assert!(error.contains("cannot reserve"), "{error}");
        assert_eq!(budget.snapshot().used_bytes, existing.bytes());
    }

    #[test]
    fn budgeted_import_rejects_after_headers_before_decoding_missing_pixels() {
        let bytes = encode(&sample(), ImageFormat::Bmp).unwrap();
        let pixel_offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
        let path = stream_test_path("header-only.bmp");
        std::fs::write(&path, &bytes[..pixel_offset]).unwrap();
        let budget = ResourceBudget::new(IMPORT_WORKSPACE_ALLOWANCE + 37, 0).unwrap();
        let existing = budget.try_acquire(37, AdmissionClass::Normal).unwrap();
        let error = decode_file_with_budget(&path, &budget).unwrap_err();
        // Valid headers reach peak admission; absent pixels must not be decoded.
        assert!(error.contains("cannot reserve"), "{error}");
        assert_eq!(budget.snapshot().used_bytes, existing.bytes());
        let larger = ResourceBudget::new(64 * 1024 * 1024, 0).unwrap();
        let decode_error = decode_file_with_budget(&path, &larger).unwrap_err();
        assert!(!decode_error.contains("cannot reserve"), "{decode_error}");
        assert_eq!(larger.snapshot().used_bytes, 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn budgeted_import_releases_on_open_and_header_errors() {
        let budget = ResourceBudget::new(64 * 1024 * 1024, 0).unwrap();
        let existing = budget.try_acquire(37, AdmissionClass::Normal).unwrap();
        let path = stream_test_path("bad-import");
        assert!(decode_file_with_budget(&path, &budget).is_err());
        assert_eq!(budget.snapshot().used_bytes, existing.bytes());
        std::fs::write(&path, b"invalid image header").unwrap();
        assert!(decode_file_with_budget(&path, &budget).is_err());
        assert_eq!(budget.snapshot().used_bytes, existing.bytes());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn import_reservation_checks_decoder_bytes_and_all_arithmetic() {
        assert_eq!(
            import_peak_reservation(2, 3, 48).unwrap(),
            IMPORT_WORKSPACE_ALLOWANCE + 2 * 48 + 2 * 3 * 4
        );
        assert!(import_peak_reservation(1, 1, u64::MAX).is_err());
        assert!(import_peak_reservation(u32::MAX, u32::MAX, 0).is_err());
    }

    #[test]
    fn file_open_enforces_size_limit_and_rejects_bad_data() {
        let path = stream_test_path("limit");
        std::fs::write(&path, b"12345").unwrap();
        assert!(open_image_file(&path, 5).is_ok());
        assert!(open_image_file(&path, 4).is_err());
        assert!(decode_file(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"12345");
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn file_import_handle_blocks_writers_and_replacement_until_closed() {
        use std::os::windows::fs::OpenOptionsExt;
        let path = stream_test_path("locked.png");
        let other = stream_test_path("replacement.png");
        let bytes = encode(&sample(), ImageFormat::Png).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(&other, &bytes).unwrap();
        let reader = open_image_file(&path, MAX_IMAGE_BYTES).unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::rename(&other, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        drop(reader);
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(7)
            .open(&path)
            .unwrap();
        assert!(decode_file(&path).is_err());
        drop(writer);
        assert_eq!(decode_file(&path).unwrap().0, sample());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(other).unwrap();
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

    fn assert_png_pixels(source: &Image) {
        let original = source.clone();
        let bytes = encode(source, ImageFormat::Png).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut rgba).unwrap();
        assert_eq!(
            (info.width as usize, info.height as usize),
            (source.width, source.height)
        );
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        let expected: Vec<u8> = source.pixels[..source.width * source.height * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect();
        assert_eq!(&rgba[..info.buffer_size()], expected);
        reader.finish().unwrap();
        assert_eq!(&bytes[bytes.len() - 12..], b"\0\0\0\0IEND\xaeB`\x82");
        assert_eq!(source, &original);
    }

    #[test]
    fn png_stream_preserves_every_channel_and_alpha_value() {
        let pixels = (0..=255u8)
            .flat_map(|alpha| {
                (0..=255u8).flat_map(move |channel| {
                    [channel, channel.wrapping_add(73), 255 - channel, alpha]
                })
            })
            .collect();
        let source = Image::new(256, 256, pixels).unwrap();
        assert_png_pixels(&source);
        assert_eq!(
            decode(&encode(&source, ImageFormat::Png).unwrap()).unwrap(),
            source
        );
    }

    #[test]
    fn png_stream_crosses_conversion_blocks_and_scanlines() {
        // 8193 pixels exceed the 32-KiB block by one pixel. Subsequent rows
        // begin inside a block; 30000 also exercises the largest import width.
        for width in [8191, 8192, 8193, 30_000] {
            let pixels = (0..width * 3 * 4).map(|i| (i * 37 + i / 7) as u8).collect();
            let source = Image::new(width, 3, pixels).unwrap();
            assert_png_pixels(&source);
            assert_eq!(
                decode(&encode(&source, ImageFormat::Png).unwrap()).unwrap(),
                source
            );
        }
    }

    fn png_with_compression(source: &Image, compression: png::Compression) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        write_png_stream(
            &mut output,
            source.width as u32,
            source.height as u32,
            &source.pixels,
            compression,
        )
        .unwrap();
        output.into_inner()
    }

    fn png_via_file(source: &Image) -> Vec<u8> {
        let path = stream_test_path("encode.png");
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        // A longer previous payload makes failure to truncate observable.
        file.write_all(&vec![0xaa; source.pixels.len() + 4096])
            .unwrap();
        write_png_file(source, &mut file).unwrap();
        drop(file);
        let output = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        output
    }

    #[test]
    fn bmp_and_jpeg_file_export_matches_previous_bytes_and_truncates() {
        let source = Image::new(
            9,
            7,
            (0..9 * 7 * 4).map(|n| (n * 37 + n / 3) as u8).collect(),
        )
        .unwrap();
        let original = source.clone();
        for format in [ImageFormat::Bmp, ImageFormat::Jpeg] {
            let path = stream_test_path("buffered-image.tmp");
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(&vec![0xaa; 16_384]).unwrap();
            write_bmp_or_jpeg_file(&source, &mut file, format).unwrap();
            let end = file.stream_position().unwrap();
            drop(file);
            let actual = std::fs::read(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            assert_eq!(end, actual.len() as u64);
            let expected = if format == ImageFormat::Jpeg {
                previous_jpeg_encode(&source).unwrap()
            } else {
                let mut rgba = source.pixels.clone();
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                let mut old = Cursor::new(Vec::new());
                DynamicImage::ImageRgba8(RgbaImage::from_raw(9, 7, rgba).unwrap())
                    .write_to(&mut old, format)
                    .unwrap();
                old.into_inner()
            };
            assert_eq!(actual, expected);
            assert_eq!(source, original);
        }
    }

    #[test]
    fn bmp_and_jpeg_buffered_export_reports_write_and_flush_errors() {
        struct FailingWriter {
            cursor: Cursor<Vec<u8>>,
            fail_write: bool,
        }
        impl Write for FailingWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if self.fail_write {
                    return Err(std::io::Error::other("injected write failure"));
                }
                self.cursor.write(data)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("injected flush failure"))
            }
        }
        impl Seek for FailingWriter {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.cursor.seek(pos)
            }
        }
        for format in [ImageFormat::Bmp, ImageFormat::Jpeg] {
            for fail_write in [true, false] {
                let mut sink = FailingWriter {
                    cursor: Cursor::new(Vec::new()),
                    fail_write,
                };
                let error = write_buffered_bmp_or_jpeg(&sample(), &mut sink, format).unwrap_err();
                assert!(
                    error.contains(if fail_write {
                        "injected write failure"
                    } else {
                        "injected flush failure"
                    }),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn bmp_buffered_export_reports_failure_during_encoding() {
        struct LimitedWriter {
            cursor: Cursor<Vec<u8>>,
            remaining: usize,
        }
        impl Write for LimitedWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if self.remaining == 0 {
                    return Err(std::io::Error::other("injected midstream failure"));
                }
                let count = data.len().min(self.remaining);
                self.remaining -= count;
                self.cursor.write(&data[..count])
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl Seek for LimitedWriter {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.cursor.seek(pos)
            }
        }
        let source = Image::new(100, 100, [3, 7, 11, 255].repeat(10_000)).unwrap();
        let mut sink = LimitedWriter {
            cursor: Cursor::new(Vec::new()),
            remaining: 9_000,
        };
        let error = write_buffered_bmp_or_jpeg(&source, &mut sink, ImageFormat::Bmp).unwrap_err();
        assert!(error.contains("injected midstream failure"), "{error}");
        assert_eq!(sink.remaining, 0);
    }

    #[test]
    fn png_stream_reports_mid_write_failure_without_changing_source() {
        struct FailingSink {
            remaining: usize,
            written: usize,
        }
        impl Write for FailingSink {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if self.remaining == 0 {
                    return Err(std::io::Error::other("injected PNG write failure"));
                }
                let count = data.len().min(self.remaining);
                self.remaining -= count;
                self.written += count;
                Ok(count)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let source = sample();
        let original = source.clone();
        let mut sink = FailingSink {
            remaining: 40,
            written: 0,
        };
        let error =
            write_png_stream(&mut sink, 2, 2, &source.pixels, png::Compression::Fast).unwrap_err();
        assert!(error.contains("injected PNG write failure"));
        assert_eq!(sink.written, 40);
        assert_eq!(source, original);
    }

    #[test]
    fn png_stream_retries_incompressible_noise_with_bounded_stored_output() {
        let (width, height) = (1024, 256);
        let mut state = 0x1234_5678u32;
        let pixels = (0..width * height * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let source = Image::new(width, height, pixels).unwrap();
        let bound = png_stored_size_bound(source.pixels.len(), height).unwrap();
        let fast = png_with_compression(&source, png::Compression::Fast);
        assert!(fast.len() > bound, "noise must exercise the retry");
        drop(fast);
        let output = encode(&source, ImageFormat::Png).unwrap();
        assert!(output.len() <= bound);
        assert_eq!(decode(&output).unwrap(), source);
        assert_eq!(
            output,
            png_with_compression(&source, png::Compression::NoCompression)
        );
        assert_eq!(png_via_file(&source), output);
        assert_png_pixels(&source);
    }

    #[test]
    fn png_stream_keeps_fast_compression_for_flat_and_screenshot_like_images() {
        for screenshot in [false, true] {
            let (width, height) = (1024, 256);
            let mut pixels = Vec::with_capacity(width * height * 4);
            for y in 0..height {
                for x in 0..width {
                    let pixel = if screenshot && (32..992).contains(&x) && (24..232).contains(&y) {
                        if y % 24 < 3 && x % 160 < 130 {
                            [24, 47, 89, 255]
                        } else {
                            [230, 240, 250, 192]
                        }
                    } else {
                        [255, 255, 255, 255]
                    };
                    pixels.extend_from_slice(&pixel);
                }
            }
            let source = Image::new(width, height, pixels).unwrap();
            let fast = png_with_compression(&source, png::Compression::Fast);
            assert!(fast.len() < png_stored_size_bound(source.pixels.len(), height).unwrap());
            let output = encode(&source, ImageFormat::Png).unwrap();
            assert_eq!(
                output, fast,
                "compressible images must retain the fast path"
            );
            assert_eq!(decode(&output).unwrap(), source);
            assert_eq!(png_via_file(&source), output);
        }
    }

    #[test]
    fn png_stored_size_bound_checks_arithmetic_overflow() {
        assert!(png_stored_size_bound(usize::MAX, 1).is_none());
        assert!(png_stored_size_bound(usize::MAX, 0).is_none());
        assert!(png_stored_size_bound(usize::MAX / 2, 1).is_some());
    }

    #[test]
    fn png_stored_bound_covers_tiny_images_and_idat_chunk_boundaries() {
        let mut below_chunk = false;
        let mut above_chunk = false;
        // Tiny inputs expose fixed/finalization overhead. Widths around 1024
        // put RGBA scanlines and their stored zlib output around 4096 bytes.
        for (width, height) in [(1, 1), (1, 2), (2, 1), (3, 3)]
            .into_iter()
            .chain((1016..=1026).map(|width| (width, 1)))
            .chain([(341, 3), (1, 819), (1, 820)])
        {
            let source = Image::new(width, height, vec![42; width * height * 4]).unwrap();
            let output = png_with_compression(&source, png::Compression::NoCompression);
            let bound = png_stored_size_bound(source.pixels.len(), height).unwrap();
            assert!(
                output.len() <= bound,
                "{width}x{height}: {} > {bound}",
                output.len()
            );
            assert_eq!(decode(&output).unwrap(), source);
            let mut position = 8;
            let mut idat_bytes = 0;
            while position < output.len() {
                let length =
                    u32::from_be_bytes(output[position..position + 4].try_into().unwrap()) as usize;
                if &output[position + 4..position + 8] == b"IDAT" {
                    idat_bytes += length;
                }
                position += length + 12;
            }
            below_chunk |= idat_bytes < PNG_STREAM_CHUNK_BYTES;
            above_chunk |= idat_bytes > PNG_STREAM_CHUNK_BYTES;
        }
        assert!(
            below_chunk && above_chunk,
            "exercise both sides of the IDAT buffer size"
        );
    }

    #[test]
    fn png_stream_matches_previous_encoder_pixels() {
        let source = Image::new(
            3,
            2,
            vec![
                23, 45, 67, 0, 91, 122, 153, 128, 200, 1, 47, 255, 12, 253, 76, 1, 29, 83, 199,
                254, 65, 7, 44, 16,
            ],
        )
        .unwrap();
        let mut rgba = source.pixels.clone();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let mut previous = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_raw(3, 2, rgba).unwrap())
            .write_to(&mut previous, ImageFormat::Png)
            .unwrap();
        let streamed = encode(&source, ImageFormat::Png).unwrap();
        assert_eq!(
            decode(&streamed).unwrap(),
            decode(previous.get_ref()).unwrap()
        );
        // Streamed IDAT chunking/deflation need not produce identical bytes.
        assert_png_pixels(&source);
    }

    #[test]
    fn png_stream_rejects_surplus_instead_of_panicking_in_image_encoder() {
        for surplus in 1..=8 {
            let mut source = sample();
            source.pixels.extend(vec![79; surplus]);
            let original = source.clone();
            assert_eq!(
                encode(&source, ImageFormat::Png).unwrap_err(),
                "Ungültige Bildgröße"
            );
            assert_eq!(source, original);
        }
    }

    #[test]
    fn png_stream_rejects_invalid_public_fields_without_panicking() {
        for (width, height, pixels) in [
            (0, 1, vec![]),
            (1, 0, vec![]),
            (0, 0, vec![]),
            (1, 1, vec![0; 3]),
            (8193, 2, vec![0; 8193 * 2 * 4 - 1]),
            (u32::MAX as usize, u32::MAX as usize, vec![]),
            (usize::MAX, 1, vec![]),
            (1, usize::MAX, vec![]),
        ] {
            assert!(encode(
                &Image {
                    width,
                    height,
                    pixels
                },
                ImageFormat::Png
            )
            .is_err());
        }
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
        let path = stream_test_path("orientation.jpg");
        std::fs::write(&path, &with_exif).unwrap();
        let (file_rotated, format) = decode_file(&path).unwrap();
        assert_eq!(format, Some(ImageFormat::Jpeg));
        assert_eq!(file_rotated, rotated);
        let budget = ResourceBudget::new(64 * 1024 * 1024, 0).unwrap();
        let (admitted_rotated, admitted_format, lease) =
            decode_file_with_budget(&path, &budget).unwrap();
        assert_eq!(admitted_rotated, rotated);
        assert_eq!(admitted_format, format);
        assert_eq!(lease.bytes(), admitted_rotated.pixels.capacity());
        drop(admitted_rotated);
        drop(lease);
        assert_eq!(budget.snapshot().used_bytes, 0);
        std::fs::remove_file(path).unwrap();
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
    fn gif_export_preserves_source_and_opaque_palette_pixels() {
        let source = sample();
        let original = source.clone();
        let bytes = encode(&source, ImageFormat::Gif).unwrap();
        assert_eq!(decode(&bytes).unwrap(), original);
        assert_eq!(source, original);
    }

    #[test]
    fn gif_owned_frame_matches_previous_encoder_bytes() {
        for (width, height, many_colors) in [(1, 1, false), (17, 9, false), (33, 29, true)] {
            let pixels: Vec<u8> = (0..width * height)
                .flat_map(|n| {
                    if many_colors {
                        [
                            (n * 17) as u8,
                            (n * 31) as u8,
                            (n * 47 + n / 3) as u8,
                            (n * 13) as u8,
                        ]
                    } else {
                        [
                            if n % 2 == 0 { 255 } else { 0 },
                            42,
                            17,
                            if n % 3 == 0 { 0 } else { 255 },
                        ]
                    }
                })
                .collect();
            let source = Image::new(width, height, pixels).unwrap();
            let original = source.clone();
            let mut rgba = source.pixels.clone();
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            let old = DynamicImage::ImageRgba8(
                RgbaImage::from_raw(width as u32, height as u32, rgba).unwrap(),
            );
            let mut bytes = Cursor::new(Vec::new());
            old.write_to(&mut bytes, ImageFormat::Gif).unwrap();
            let expected = bytes.into_inner();
            assert_eq!(encode(&source, ImageFormat::Gif).unwrap(), expected);
            let path = stream_test_path("stream.gif");
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(&vec![0xaa; expected.len() + 100]).unwrap();
            write_gif_file(&source, &mut file).unwrap();
            drop(file);
            assert_eq!(std::fs::read(&path).unwrap(), expected);
            std::fs::remove_file(path).unwrap();
            assert_eq!(source, original);
        }
        let invalid = Image {
            width: 2,
            height: 2,
            pixels: vec![0; 15],
        };
        assert!(encode(&invalid, ImageFormat::Gif).is_err());
        let surplus = Image {
            width: 2,
            height: 2,
            pixels: vec![0; 17],
        };
        assert!(encode(&surplus, ImageFormat::Gif).is_err());
        let empty = Image {
            width: 0,
            height: 1,
            pixels: vec![],
        };
        assert!(encode(&empty, ImageFormat::Gif).is_err());
        let empty_height = Image {
            width: 1,
            height: 0,
            pixels: vec![],
        };
        assert!(encode(&empty_height, ImageFormat::Gif).is_err());
    }

    #[test]
    fn gif_trailer_write_failure_is_not_lost_when_writer_recovers() {
        struct FailOnce {
            data: Vec<u8>,
            limit: usize,
            failed: bool,
            mode: u8,
        }
        impl Write for FailOnce {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if !self.failed && self.data.len() == self.limit {
                    self.failed = true;
                    return match self.mode {
                        0 => Err(std::io::Error::other("injected trailer failure")),
                        1 => Ok(0),
                        _ => Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
                    };
                }
                let count = if self.failed {
                    bytes.len()
                } else {
                    bytes.len().min(self.limit - self.data.len())
                };
                self.data.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let source = sample();
        let encoded = encode(&source, ImageFormat::Gif).unwrap();
        assert_eq!(encoded.last(), Some(&0x3b));
        for mode in 0..3 {
            let mut writer = FailOnce {
                data: Vec::new(),
                limit: encoded.len() - 1,
                failed: false,
                mode,
            };
            let result = write_gif_stream(&source, &mut writer);
            assert!(writer.failed);
            if mode == 2 {
                result.unwrap(); // Interrupted must be retried by write_all.
            } else {
                assert!(
                    result.is_err(),
                    "a dropped trailer failure must not be lost"
                );
                writer.write_all(&[0x3b]).unwrap(); // failure was transient
            }
            assert_eq!(writer.data, encoded);
        }
    }

    #[test]
    fn gif_buffered_export_reports_final_flush_error() {
        struct FlushFails(Vec<u8>);
        impl Write for FlushFails {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("injected gif flush failure"))
            }
        }
        let error = write_buffered_gif(&sample(), &mut FlushFails(Vec::new())).unwrap_err();
        assert!(error.contains("injected gif flush failure"), "{error}");
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
        let bytes = fixture.into_inner();
        assert_eq!(decode(&bytes).unwrap(), src);
        let path = stream_test_path("tiff-with-png-extension.png");
        std::fs::write(&path, &bytes).unwrap();
        let (file_image, format) = decode_file(&path).unwrap();
        assert_eq!(format, Some(ImageFormat::Tiff));
        assert_eq!(file_image, src);
        std::fs::remove_file(path).unwrap();
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
    fn jpeg_pixels_flatten_transparent_and_semitransparent_bgra_onto_white() {
        let bgra = [
            56, 34, 12, 0, // Hidden RGB must not leak through a fully transparent pixel.
            200, 100, 20, 128, 9, 8, 7, 255,
        ];
        assert_eq!(
            jpeg_rgb_on_white(&bgra).unwrap(),
            vec![255, 255, 255, 137, 177, 227, 7, 8, 9]
        );
    }

    #[test]
    fn direct_jpeg_flattening_matches_previous_rgba_formula_exhaustively() {
        // Cover every channel/alpha pair with distinct B/G/R values to catch
        // both rounding regressions and a missing BGRA-to-RGB channel swap.
        let mut bgra = Vec::new();
        for alpha in 0..=255u8 {
            for channel in 0..=255u8 {
                bgra.extend_from_slice(&[channel, channel.wrapping_add(73), 255 - channel, alpha]);
            }
        }
        let original = bgra.clone();
        let mut rgba = bgra.clone();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let previous: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| {
                let alpha = u16::from(pixel[3]);
                pixel[..3].iter().map(move |channel| {
                    ((u16::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
                })
            })
            .collect();
        assert_eq!(jpeg_rgb_on_white(&bgra).unwrap(), previous);
        let view = JpegRgbView {
            bgra: &bgra,
            width: 256,
            height: 256,
        };
        let adapter: Vec<u8> = (0..256)
            .flat_map(|y| (0..256).flat_map(move |x| view.get_pixel(x, y).0))
            .collect();
        assert_eq!(adapter, previous);
        assert_eq!(bgra, original);
    }

    #[test]
    fn jpeg_borrowed_view_matches_previous_encoding_at_block_edges() {
        for (width, height) in [
            (1, 1),
            (1, 17),
            (17, 1),
            (7, 9),
            (8, 8),
            (9, 9),
            (64, 64),
            (65535, 1),
            (65536, 1),
        ] {
            let pixels = (0..width * height * 4)
                .map(|n| ((n * 73 + n / 7 + 19) % 256) as u8)
                .collect();
            let source = Image::new(width, height, pixels).unwrap();
            assert_eq!(
                encode(&source, ImageFormat::Jpeg),
                previous_jpeg_encode(&source),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn jpeg_bytes_match_previous_encoding_and_preserve_input() {
        let source = Image::new(
            4,
            1,
            vec![
                12, 34, 56, 0, 20, 100, 200, 128, 7, 8, 9, 255, 211, 53, 97, 1,
            ],
        )
        .unwrap();
        let original = source.clone();
        assert_eq!(
            encode(&source, ImageFormat::Jpeg),
            previous_jpeg_encode(&source)
        );
        assert_eq!(source, original);
    }

    // Reconstruct the previous RGBA-copy path, including ImageBuffer's handling
    // of public image fields and the encoder's dimension errors.
    fn previous_jpeg_encode(source: &Image) -> Result<Vec<u8>, String> {
        let width = u32::try_from(source.width).map_err(|_| "Bildbreite zu groß")?;
        let height = u32::try_from(source.height).map_err(|_| "Bildhöhe zu groß")?;
        let mut previous_rgba = source.pixels.clone();
        for pixel in previous_rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let previous_rgba = RgbaImage::from_raw(width, height, previous_rgba)
            .ok_or("Ungültige Bildgröße")?
            .into_raw();
        let previous_rgb = previous_rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| {
                let alpha = u16::from(pixel[3]);
                pixel[..3].iter().map(move |channel| {
                    ((u16::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
                })
            })
            .collect();
        let mut previous = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut previous, 92)
            .encode_image(&DynamicImage::ImageRgb8(
                RgbImage::from_raw(width, height, previous_rgb).ok_or("Ungültige Bildgröße")?,
            ))
            .map_err(|error| error.to_string())?;
        Ok(previous.into_inner())
    }

    #[test]
    fn jpeg_preserves_previous_surplus_buffer_and_zero_dimension_behavior() {
        for (width, height) in [(1, 1), (0, 1), (1, 0), (0, 0)] {
            for surplus in 0..=8 {
                let image = Image {
                    width,
                    height,
                    pixels: vec![79; width * height * 4 + surplus],
                };
                let original = image.clone();
                assert_eq!(
                    encode(&image, ImageFormat::Jpeg),
                    previous_jpeg_encode(&image)
                );
                assert_eq!(image, original);
            }
        }
    }

    #[test]
    fn jpeg_export_rejects_invalid_public_image_fields() {
        for image in [
            Image {
                width: 0,
                height: 1,
                pixels: vec![],
            },
            Image {
                width: 1,
                height: 0,
                pixels: vec![],
            },
            Image {
                width: 1,
                height: 1,
                pixels: vec![0; 3],
            },
            Image {
                width: usize::MAX,
                height: usize::MAX,
                pixels: vec![],
            },
        ] {
            assert!(encode(&image, ImageFormat::Jpeg).is_err());
        }
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
