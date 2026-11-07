//! Windows capture-output helpers. The caller owns the captured image and drives
//! [`CaptureOutputState`], including the editor action and failure reporting.
use crate::{
    capture_output::{Advance, CaptureAction, CaptureOutputState},
    image_io,
    settings::Settings,
    stitch::Image,
};
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(any(test, not(windows)))]
use std::fs::OpenOptions;
#[cfg(test)]
use std::io::Write;
use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// What the caller should do after asking this module to advance the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputProgress {
    /// The next planned step is the editor; the caller must open it and report
    /// its result with `CaptureOutputState::finish_action`.
    EditorRequired,
    /// A save/copy action was run and its result was recorded in the state.
    Advanced(Advance),
    /// There are no more planned actions.
    Complete,
}

/// Run and record the next save/clipboard step. Editor creation stays with the
/// caller so it can use the application's UI framework and retain the image.
pub fn advance_output(
    state: &mut CaptureOutputState,
    settings: &Settings,
    image: &Image,
    #[cfg(windows)] clipboard_owner: *mut c_void,
) -> Result<OutputProgress, String> {
    let Some(action) = state.next_action() else {
        return Ok(OutputProgress::Complete);
    };
    if action == CaptureAction::OpenEditor {
        return Ok(OutputProgress::EditorRequired);
    }
    let result = execute_action(
        action,
        settings,
        image,
        #[cfg(windows)]
        clipboard_owner,
    )
    .map(|_| ());
    state
        .finish_action(action, result)
        .map(OutputProgress::Advanced)
        .map_err(|error| error.to_string())
}

/// Result of one non-editor output action. `saved_path` is set only for AutoSave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputResult {
    pub saved_path: Option<PathBuf>,
}

/// Execute one non-editor action. The image is borrowed and remains available
/// to the caller on both success and failure. Handle `OpenEditor` in the caller.
pub fn execute_action(
    action: CaptureAction,
    settings: &Settings,
    image: &Image,
    #[cfg(windows)] clipboard_owner: *mut c_void,
) -> Result<OutputResult, String> {
    match action {
        CaptureAction::AutoSave => {
            if !settings.auto_save {
                return Err("AutoSave wurde in den Einstellungen nicht aktiviert".into());
            }
            let folder = settings
                .save_folder
                .as_deref()
                .ok_or_else(|| "Kein Speicherordner konfiguriert".to_owned())?;
            save_png(folder, image)
                .map(|saved_path| OutputResult {
                    saved_path: Some(saved_path),
                })
                .map_err(|e| format!("PNG konnte nicht gespeichert werden: {e}"))
        }
        CaptureAction::CopyToClipboard => {
            if !settings.copy_to_clipboard {
                return Err("Clipboard-Kopie wurde in den Einstellungen nicht aktiviert".into());
            }
            #[cfg(windows)]
            {
                copy_image_to_clipboard(
                    clipboard_owner,
                    image,
                    settings.exclude_from_clipboard_history,
                )?;
                Ok(OutputResult { saved_path: None })
            }
            #[cfg(not(windows))]
            {
                let _ = image;
                Err("Die Windows-Zwischenablage ist auf diesem System nicht verfügbar".into())
            }
        }
        CaptureAction::OpenEditor => Err(
            "OpenEditor muss vom Aufrufer ausgeführt werden; das Bild bleibt dessen Eigentum"
                .into(),
        ),
    }
}

fn save_png(folder: &std::path::Path, image: &Image) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    #[cfg(windows)]
    {
        crate::atomic_save_windows::write_new_unique_with(
            folder,
            256,
            || {
                let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                folder.join(format!("ShotKit-{stamp}-{sequence}.png"))
            },
            |file| {
                image_io::write_png_file(image, file)
                    .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
            },
        )
    }

    // Create the sibling temp exclusively. Publish the final name without
    // replacement only after the complete PNG has been written and synced.
    #[cfg(not(windows))]
    {
        for _ in 0..256 {
            let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = folder.join(format!("ShotKit-{stamp}-{sequence}.png"));
            let temp_path = folder.join(format!(
                ".ShotKit-{}-{stamp}-{sequence}.tmp",
                std::process::id()
            ));
            let file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            let result = write_and_publish(&temp_path, &path, file, |file| {
                image_io::write_png_file(image, file)
                    .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))?;
                file.sync_all()
            });
            match result {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique PNG filename",
        ))
    }
}

#[cfg(any(test, not(windows)))]
fn write_and_publish(
    temp_path: &std::path::Path,
    final_path: &std::path::Path,
    mut file: std::fs::File,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let result = write(&mut file);
    drop(file);
    if let Err(write_error) = result {
        return match fs::remove_file(temp_path) {
            Ok(()) => Err(write_error),
            Err(cleanup_error) => Err(io::Error::new(
                cleanup_error.kind(),
                format!(
                    "{write_error}; additionally could not remove temporary file: {cleanup_error}"
                ),
            )),
        };
    }

    match publish_no_replace(temp_path, final_path) {
        Ok(true) => Ok(()),
        Ok(false) => match fs::remove_file(temp_path) {
            Ok(()) => Ok(()),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("PNG was published but temporary file could not be removed: {error}"),
            )),
        },
        Err(publish_error) => match fs::remove_file(temp_path) {
            Ok(()) => Err(publish_error),
            Err(cleanup_error) => Err(io::Error::new(
                cleanup_error.kind(),
                format!(
                    "{publish_error}; additionally could not remove temporary file: {cleanup_error}"
                ),
            )),
        },
    }
}

#[cfg(all(test, windows))]
fn publish_no_replace(source: &std::path::Path, destination: &std::path::Path) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // Both paths are siblings, including for UNC folders, so this is a
    // same-volume rename. Omitting MOVEFILE_REPLACE_EXISTING makes collisions
    // fail atomically instead of replacing an existing capture.
    // SAFETY: Both NUL-terminated UTF-16 buffers remain alive for the call;
    // this filesystem operation does not synchronously dispatch window messages.
    let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x8) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(true)
    }
}

#[cfg(not(windows))]
fn publish_no_replace(source: &std::path::Path, destination: &std::path::Path) -> io::Result<bool> {
    // rename replaces an existing destination on Unix. Creating a hard link
    // is atomic and fails if destination exists; sibling paths keep it on the
    // same filesystem. Removing the temp name after link leaves the final name
    // as the only visible name for the completed file.
    fs::hard_link(source, destination).map(|()| false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DibInfo {
    width: i32,
    height: i32,
    pixel_bytes: u32,
}

fn validate_dib_image(image: &Image) -> Result<DibInfo, String> {
    const MAX_DIMENSION: usize = 30_000;
    const MAX_BYTES: usize = 512 * 1024 * 1024;
    if image.width == 0
        || image.height == 0
        || image.width > MAX_DIMENSION
        || image.height > MAX_DIMENSION
    {
        return Err("Bildabmessungen sind ungültig oder überschreiten 30000 Pixel".into());
    }
    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Bildgröße überläuft".to_owned())?;
    if expected != image.pixels.len() || expected > MAX_BYTES {
        return Err("BGRA-Puffergröße passt nicht zu den Bildabmessungen".into());
    }
    let pixel_bytes =
        u32::try_from(expected).map_err(|_| "Bild ist zu groß für CF_DIB".to_owned())?;
    Ok(DibInfo {
        width: i32::try_from(image.width).map_err(|_| "Bildbreite ist zu groß".to_owned())?,
        height: i32::try_from(image.height).map_err(|_| "Bildhöhe ist zu groß".to_owned())?,
        pixel_bytes,
    })
}

fn dibv5_header(width: i32, height: i32, pixel_bytes: u32) -> [u8; 124] {
    let mut header = [0u8; 124];
    let put_u16 = |header: &mut [u8], offset: usize, value: u16| {
        header[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    };
    let put_u32 = |header: &mut [u8], offset: usize, value: u32| {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    put_u32(&mut header, 0, 124);
    put_u32(&mut header, 4, width as u32);
    put_u32(&mut header, 8, (-height) as u32);
    put_u16(&mut header, 12, 1);
    put_u16(&mut header, 14, 32);
    put_u32(&mut header, 16, 3); // BI_BITFIELDS
    put_u32(&mut header, 20, pixel_bytes);
    put_u32(&mut header, 40, 0x00ff_0000); // red
    put_u32(&mut header, 44, 0x0000_ff00); // green
    put_u32(&mut header, 48, 0x0000_00ff); // blue
    put_u32(&mut header, 52, 0xff00_0000); // alpha
    put_u32(&mut header, 56, 0x7352_4742); // LCS_sRGB
    header
}

/// Encode an image as top-down 32-bit CF_DIBV5 with explicit BGRA masks.
pub fn encode_dibv5(image: &Image) -> Result<Vec<u8>, String> {
    let dib = validate_dib_image(image)?;
    let total = 124usize
        .checked_add(dib.pixel_bytes as usize)
        .ok_or_else(|| "DIBV5-Größe überläuft".to_owned())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total)
        .map_err(|e| format!("DIBV5-Puffer konnte nicht reserviert werden: {e}"))?;
    bytes.resize(total, 0);
    bytes[..124].copy_from_slice(&dibv5_header(dib.width, dib.height, dib.pixel_bytes));
    bytes[124..].copy_from_slice(&image.pixels);
    Ok(bytes)
}

/// Encode a legacy CF_DIB. It is deliberately opaque and flattened on white;
/// consumers that support alpha receive the lossless DIBV5/PNG formats instead.
pub fn encode_legacy_dib(image: &Image) -> Result<Vec<u8>, String> {
    let dib = validate_dib_image(image)?;
    let header_size = 40usize;
    let total = header_size
        .checked_add(dib.pixel_bytes as usize)
        .ok_or_else(|| "DIB-Größe überläuft".to_owned())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total)
        .map_err(|e| format!("DIB-Puffer konnte nicht reserviert werden: {e}"))?;
    bytes.resize(total, 0);
    bytes[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
    bytes[4..8].copy_from_slice(&(dib.width as u32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(dib.height as u32).to_le_bytes()); // bottom-up
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
    bytes[16..20].copy_from_slice(&0u32.to_le_bytes()); // BI_RGB
    bytes[20..24].copy_from_slice(&dib.pixel_bytes.to_le_bytes());
    let row_bytes = image.width * 4;
    for y in 0..image.height {
        let source_y = image.height - 1 - y;
        for (source, target) in image.pixels[source_y * row_bytes..(source_y + 1) * row_bytes]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(
                bytes[header_size + y * row_bytes..header_size + (y + 1) * row_bytes]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut(),
            )
        {
            let alpha = u16::from(source[3]);
            for channel in 0..3 {
                let value = u16::from(source[channel]) * alpha + 255 * (255 - alpha);
                target[channel] = ((value + 127) / 255) as u8;
            }
            target[3] = 255;
        }
    }
    Ok(bytes)
}

/// Decode supported clipboard DIB/DIBV5 byte layouts into top-down BGRA.
/// Supports 24-bit BI_RGB and 32-bit BI_RGB/BI_BITFIELDS/BI_ALPHABITFIELDS;
/// unsupported compression or profiles are rejected for the caller's fallback.
pub fn decode_dib(bytes: &[u8], require_v5: bool) -> Result<Image, String> {
    decode_dib_admitted(bytes, require_v5, None)
}

/// Budget the output before allocation, including actual Vec capacity before
/// converting source pixels. A pre-conversion refusal leaves no acquired image.
/// Existing unbudgeted callers retain the same layout/alpha/fallback behavior.
pub fn decode_dib_with_budget(
    bytes: &[u8],
    require_v5: bool,
    budget: &crate::resource_budget::ResourceBudget,
) -> Result<image_io::BudgetedImage, String> {
    let mut lease = budget
        .try_acquire(0, crate::resource_budget::AdmissionClass::Normal)
        .map_err(|error| error.to_string())?;
    let image = decode_dib_admitted(bytes, require_v5, Some(&mut lease))?;
    Ok(image_io::BudgetedImage::from_decoded(image, lease))
}

fn decode_dib_admitted(
    bytes: &[u8],
    require_v5: bool,
    mut lease: Option<&mut crate::resource_budget::BudgetLease>,
) -> Result<Image, String> {
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;
    const MAX_DIMENSION: usize = 30_000;
    const MAX_BYTES: usize = 512 * 1024 * 1024;
    if bytes.len() < 40 {
        return Err("DIB-Header ist zu kurz".into());
    }
    let read_u16 = |offset: usize| -> Result<u16, String> {
        let slice = bytes
            .get(offset..offset + 2)
            .ok_or("DIB-Header ist abgeschnitten")?;
        Ok(u16::from_le_bytes([slice[0], slice[1]]))
    };
    let read_u32 = |offset: usize| -> Result<u32, String> {
        let slice = bytes
            .get(offset..offset + 4)
            .ok_or("DIB-Header ist abgeschnitten")?;
        Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    };
    let header_size = usize::try_from(read_u32(0)?).map_err(|_| "Ungültige DIB-Headergröße")?;
    if header_size < 40 || header_size > bytes.len() || (require_v5 && header_size < 124) {
        return Err("DIB-Headergröße ist ungültig".into());
    }
    let width_i = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let height_i = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if width_i <= 0 || height_i == 0 || height_i == i32::MIN {
        return Err("DIB-Abmessungen sind ungültig".into());
    }
    let width = width_i as usize;
    let height = height_i.unsigned_abs() as usize;
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("DIB überschreitet 30000 Pixel je Dimension".into());
    }
    if read_u16(12)? != 1 {
        return Err("DIB-Ebenenzahl muss 1 sein".into());
    }
    let bpp = read_u16(14)?;
    let compression = read_u32(16)?;
    if !matches!(bpp, 24 | 32)
        || !matches!(compression, BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS)
        || (bpp == 24 && compression != BI_RGB)
    {
        return Err("DIB-Format wird nicht unterstützt".into());
    }
    let profile_size = if header_size >= 124 {
        read_u32(116)?
    } else {
        0
    };
    if profile_size != 0 {
        return Err("DIB-Farbprofile werden nicht unterstützt".into());
    }

    let mut pixel_offset = header_size;
    let mut red_mask = 0x00ff_0000u32;
    let mut green_mask = 0x0000_ff00u32;
    let mut blue_mask = 0x0000_00ffu32;
    let mut alpha_mask = 0u32;
    if bpp == 32 {
        if header_size >= 52 {
            red_mask = read_u32(40)?;
            green_mask = read_u32(44)?;
            blue_mask = read_u32(48)?;
            if header_size >= 56 {
                alpha_mask = read_u32(52)?;
            }
            if compression == BI_RGB && red_mask == 0 && green_mask == 0 && blue_mask == 0 {
                red_mask = 0x00ff_0000;
                green_mask = 0x0000_ff00;
                blue_mask = 0x0000_00ff;
            }
        } else if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
            let count = if compression == BI_ALPHABITFIELDS {
                4
            } else {
                3
            };
            let mask_bytes = count * 4;
            if bytes.len() < header_size + mask_bytes {
                return Err("DIB-Farbmasken sind abgeschnitten".into());
            }
            red_mask = read_u32(header_size)?;
            green_mask = read_u32(header_size + 4)?;
            blue_mask = read_u32(header_size + 8)?;
            if count == 4 {
                alpha_mask = read_u32(header_size + 12)?;
            }
            pixel_offset += mask_bytes;
        }
        let contiguous = |mask: u32| {
            if mask == 0 {
                return false;
            }
            let shifted = mask >> mask.trailing_zeros();
            shifted & shifted.wrapping_add(1) == 0
        };
        if !contiguous(red_mask)
            || !contiguous(green_mask)
            || !contiguous(blue_mask)
            || (alpha_mask != 0 && !contiguous(alpha_mask))
            || red_mask & green_mask != 0
            || red_mask & blue_mask != 0
            || green_mask & blue_mask != 0
            || alpha_mask & (red_mask | green_mask | blue_mask) != 0
        {
            return Err("DIB-Farbmasken sind ungültig".into());
        }
    }
    let bits_per_row = width
        .checked_mul(usize::from(bpp))
        .ok_or("DIB-Zeilengröße überläuft")?;
    let row_stride = bits_per_row
        .checked_add(31)
        .ok_or("DIB-Zeilengröße überläuft")?
        / 32
        * 4;
    let source_bytes = row_stride
        .checked_mul(height)
        .ok_or("DIB-Pixelgröße überläuft")?;
    let output_bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("DIB-Pixelgröße überläuft")?;
    if output_bytes > MAX_BYTES
        || pixel_offset
            .checked_add(source_bytes)
            .filter(|end| *end <= bytes.len())
            .is_none()
    {
        return Err("DIB-Pixeldaten sind abgeschnitten oder überschreiten 512 MiB".into());
    }
    fn component(pixel: u32, mask: u32) -> u8 {
        if mask == 0 {
            return 255;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        let value = (pixel & mask) >> shift;
        ((u64::from(value) * 255 + u64::from(max) / 2) / u64::from(max)) as u8
    }
    if let Some(lease) = lease.as_mut() {
        lease
            .try_resize(output_bytes)
            .map_err(|error| format!("Kein freies Bildbudget für DIB-Pixel: {error}"))?;
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(output_bytes)
        .map_err(|e| format!("DIB-Bildspeicher konnte nicht reserviert werden: {e}"))?;
    if let Some(lease) = lease.as_mut() {
        lease
            .try_resize(pixels.capacity())
            .map_err(|error| format!("DIB-Puffer überschreitet das freie Bildbudget: {error}"))?;
    }
    pixels.resize(output_bytes, 255);
    for y in 0..height {
        let source_y = if height_i < 0 { y } else { height - 1 - y };
        let row_start = pixel_offset + source_y * row_stride;
        for x in 0..width {
            let dst = (y * width + x) * 4;
            if bpp == 24 {
                let src = row_start + x * 3;
                pixels[dst..dst + 3].copy_from_slice(&bytes[src..src + 3]);
                pixels[dst + 3] = 255;
            } else {
                let src = row_start + x * 4;
                let value = u32::from_le_bytes(bytes[src..src + 4].try_into().unwrap());
                pixels[dst] = component(value, blue_mask);
                pixels[dst + 1] = component(value, green_mask);
                pixels[dst + 2] = component(value, red_mask);
                pixels[dst + 3] = if alpha_mask == 0 {
                    255
                } else {
                    component(value, alpha_mask)
                };
            }
        }
    }
    // Layout, initialized length and capacity were checked before conversion.
    // No fallible operation after successful acquisition may discard pixels.
    Ok(Image {
        width,
        height,
        pixels,
    })
}

#[cfg(any(windows, test))]
const OPTIONAL_CLIPBOARD_FORMAT_LIMIT: usize = 16 * 1024 * 1024;

#[cfg(any(windows, test))]
fn include_optional_clipboard_formats(pixel_bytes: usize) -> bool {
    pixel_bytes <= OPTIONAL_CLIPBOARD_FORMAT_LIMIT
}

#[cfg(any(windows, test))]
fn clipboard_transfer_result(
    close_succeeded: bool,
    alpha_succeeded: bool,
    legacy_succeeded: bool,
    failures: &str,
) -> Result<(), String> {
    if !close_succeeded {
        return Err(format!("Clipboard teilweise aktualisiert: CloseClipboard schlug nach EmptyClipboard fehl; bereits übertragene Formate bleiben gesetzt. {failures}"));
    }
    if alpha_succeeded {
        return Ok(());
    }
    if legacy_succeeded {
        return Err(format!("Clipboard teilweise aktualisiert: vorheriger Inhalt wurde gelöscht; nur opakes CF_DIB gesetzt. {failures}"));
    }
    Err(format!("Clipboard teilweise aktualisiert: vorheriger Inhalt wurde gelöscht, aber kein Bildformat übertragen. {failures}"))
}

#[cfg(windows)]
mod native_clipboard;
#[cfg(windows)]
pub use native_clipboard::{copy_dib_section_to_clipboard, copy_image_to_clipboard};

#[cfg(test)]
mod tests;
