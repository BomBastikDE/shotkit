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
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
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
                copy_image_to_clipboard(clipboard_owner, image)?;
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
    let png = image_io::encode(image, image::ImageFormat::Png)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // create_new is the collision guard, including concurrent captures.
    for _ in 0..256 {
        let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = folder.join(format!("ShotKit-{stamp}-{sequence}.png"));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        finish_created_file(&path, file, |file| {
            file.write_all(&png)?;
            file.sync_all()
        })?;
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique PNG filename",
    ))
}

fn finish_created_file(
    path: &std::path::Path,
    mut file: std::fs::File,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let result = write(&mut file);
    drop(file);
    if let Err(write_error) = result {
        return match fs::remove_file(path) {
            Ok(()) => Err(write_error),
            Err(cleanup_error) => Err(io::Error::new(
                cleanup_error.kind(),
                format!(
                    "{write_error}; additionally could not remove partial file: {cleanup_error}"
                ),
            )),
        };
    }
    Ok(())
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
    bytes.try_reserve_exact(total).map_err(|e| format!("DIBV5-Puffer konnte nicht reserviert werden: {e}"))?;
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
    bytes.try_reserve_exact(total).map_err(|e| format!("DIB-Puffer konnte nicht reserviert werden: {e}"))?;
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
            .chunks_exact(4)
            .zip(bytes[header_size + y * row_bytes..header_size + (y + 1) * row_bytes].chunks_exact_mut(4))
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
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;
    const MAX_DIMENSION: usize = 30_000;
    const MAX_BYTES: usize = 512 * 1024 * 1024;
    if bytes.len() < 40 {
        return Err("DIB-Header ist zu kurz".into());
    }
    let read_u16 = |offset: usize| -> Result<u16, String> {
        let slice = bytes.get(offset..offset + 2).ok_or("DIB-Header ist abgeschnitten")?;
        Ok(u16::from_le_bytes([slice[0], slice[1]]))
    };
    let read_u32 = |offset: usize| -> Result<u32, String> {
        let slice = bytes.get(offset..offset + 4).ok_or("DIB-Header ist abgeschnitten")?;
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
    let profile_size = if header_size >= 124 { read_u32(116)? } else { 0 };
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
            let count = if compression == BI_ALPHABITFIELDS { 4 } else { 3 };
            let mask_bytes = count * 4;
            if bytes.len() < header_size + mask_bytes {
                return Err("DIB-Farbmasken sind abgeschnitten".into());
            }
            red_mask = read_u32(header_size)?;
            green_mask = read_u32(header_size + 4)?;
            blue_mask = read_u32(header_size + 8)?;
            if count == 4 { alpha_mask = read_u32(header_size + 12)?; }
            pixel_offset += mask_bytes;
        }
        let contiguous = |mask: u32| {
            if mask == 0 { return false; }
            let shifted = mask >> mask.trailing_zeros();
            shifted & shifted.wrapping_add(1) == 0
        };
        if !contiguous(red_mask) || !contiguous(green_mask) || !contiguous(blue_mask)
            || (alpha_mask != 0 && !contiguous(alpha_mask))
            || red_mask & green_mask != 0 || red_mask & blue_mask != 0
            || green_mask & blue_mask != 0 || alpha_mask & (red_mask | green_mask | blue_mask) != 0
        {
            return Err("DIB-Farbmasken sind ungültig".into());
        }
    }
    let bits_per_row = width.checked_mul(usize::from(bpp)).ok_or("DIB-Zeilengröße überläuft")?;
    let row_stride = bits_per_row.checked_add(31).ok_or("DIB-Zeilengröße überläuft")? / 32 * 4;
    let source_bytes = row_stride.checked_mul(height).ok_or("DIB-Pixelgröße überläuft")?;
    let output_bytes = width.checked_mul(height).and_then(|n| n.checked_mul(4)).ok_or("DIB-Pixelgröße überläuft")?;
    if output_bytes > MAX_BYTES || pixel_offset.checked_add(source_bytes).filter(|end| *end <= bytes.len()).is_none() {
        return Err("DIB-Pixeldaten sind abgeschnitten oder überschreiten 512 MiB".into());
    }
    fn component(pixel: u32, mask: u32) -> u8 {
        if mask == 0 { return 255; }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        let value = (pixel & mask) >> shift;
        ((u64::from(value) * 255 + u64::from(max) / 2) / u64::from(max)) as u8
    }
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(output_bytes).map_err(|e| format!("DIB-Bildspeicher konnte nicht reserviert werden: {e}"))?;
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
                pixels[dst + 3] = if alpha_mask == 0 { 255 } else { component(value, alpha_mask) };
            }
        }
    }
    Image::new(width, height, pixels).map_err(str::to_owned)
}

const OPTIONAL_CLIPBOARD_FORMAT_LIMIT: usize = 16 * 1024 * 1024;

fn include_optional_clipboard_formats(pixel_bytes: usize) -> bool {
    pixel_bytes <= OPTIONAL_CLIPBOARD_FORMAT_LIMIT
}

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
pub fn copy_image_to_clipboard(owner: *mut c_void, image: &Image) -> Result<(), String> {
    let _dib = validate_dib_image(image)?;
    unsafe {
        copy_bgra_clipboard(owner, image.pixels.as_ptr(), image.width, image.height,
            image.width.checked_mul(4).ok_or("Bildzeile überläuft")?, true, Some(image))
    }
}

/// Transfers an existing 32-bit BGRA DIB section without creating an Image
/// copy. The source is only read; ownership remains with the caller.
#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: pixels zeigt auf ausreichend viele initialisierte, unveränderte BGRA-Bytes mit passendem Stride und Orientierung.
// Warum erfüllt bzw. offen: Maße und Stride werden geprüft; Allokationslebensdauer und GDI-Synchronisation sind Aufruferpflicht.
// Nachrichten: Clipboard-APIs können synchron Nachrichten auslösen; Quellpixel werden zuvor kopiert.
pub unsafe fn copy_dib_section_to_clipboard(
    owner: *mut c_void,
    pixels: *const u8,
    width: usize,
    height: usize,
    source_stride: usize,
    top_down: bool,
) -> Result<(), String> {
    copy_bgra_clipboard(owner, pixels, width, height, source_stride, top_down, None)
}

#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: Der Quellpuffer erfüllt den BGRA- und Lebensdauervertrag; owner ist für Clipboard-Veröffentlichung geeignet.
// Warum erfüllt bzw. offen: Rohpixel werden vor Clipboard-Aufrufen kopiert; HGLOBAL-Ownership wird nach Übergabe beachtet.
// Nachrichten: Clipboard-APIs können synchron Nachrichten auslösen.
unsafe fn copy_bgra_clipboard(
    owner: *mut c_void,
    pixels: *const u8,
    width: usize,
    height: usize,
    source_stride: usize,
    top_down: bool,
    image_for_png: Option<&Image>,
) -> Result<(), String> {
    use std::ptr::null_mut;
    type Handle = *mut c_void;
    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: Handle) -> Handle;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(memory: Handle) -> *mut c_void;
        fn GlobalUnlock(memory: Handle) -> i32;
        fn GlobalFree(memory: Handle) -> Handle;
    }
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;
    const GMEM_MOVEABLE: u32 = 0x0002;
    const GMEM_ZEROINIT: u32 = 0x0040;

    struct Memory(Handle);
    impl Memory {
        fn allocate(size: usize) -> Result<Self, String> {
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size) };
            if handle.is_null() { return Err("GlobalAlloc für Zwischenablage ist fehlgeschlagen".into()); }
            Ok(Self(handle))
        }
        fn raw(&self) -> Handle { self.0 }
        fn transferred(&mut self) { self.0 = null_mut(); }
        fn free(mut self) { if !self.0.is_null() { unsafe { GlobalFree(self.0); } self.0 = null_mut(); } }
    }
    impl Drop for Memory { fn drop(&mut self) { if !self.0.is_null() { unsafe { GlobalFree(self.0); } } } }
    struct Clipboard(bool);
    impl Drop for Clipboard { fn drop(&mut self) { if self.0 { unsafe { CloseClipboard(); } } } }

    const HEADER_V5: usize = 124;
    const HEADER_V3: usize = 40;
    const MAX_DIMENSION: usize = 30_000;
    const MAX_BYTES: usize = 512 * 1024 * 1024;
    if pixels.is_null() { return Err("BGRA-Quellpuffer ist null".into()); }
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("Bildabmessungen ungültig oder überschreiten 30000 Pixel".into());
    }
    let row_bytes = width.checked_mul(4).ok_or("Bildzeile überläuft")?;
    if source_stride < row_bytes { return Err("BGRA-Quellzeile ist kürzer als die Bildzeile".into()); }
    let pixel_bytes = row_bytes.checked_mul(height).ok_or("Bildgröße überläuft")?;
    let source_bytes = source_stride.checked_mul(height).ok_or("Quellpuffergröße überläuft")?;
    if pixel_bytes > MAX_BYTES || source_bytes > MAX_BYTES { return Err("Clipboard-Bild überschreitet 512 MiB".into()); }
    let total_v5 = HEADER_V5.checked_add(pixel_bytes).ok_or("DIBV5-Größe überläuft")?;
    let mut dibv5_mem = Memory::allocate(total_v5)?;
    let dibv5_base = GlobalLock(dibv5_mem.raw()).cast::<u8>();
    if dibv5_base.is_null() { return Err("GlobalLock für CF_DIBV5 ist fehlgeschlagen".into()); }
    std::ptr::copy_nonoverlapping(dibv5_header(width as i32, height as i32, pixel_bytes as u32).as_ptr(), dibv5_base, HEADER_V5);
    for y in 0..height {
        let source_y = if top_down { y } else { height - 1 - y };
        std::ptr::copy_nonoverlapping(pixels.add(source_y * source_stride), dibv5_base.add(HEADER_V5 + y * row_bytes), row_bytes);
    }
    GlobalUnlock(dibv5_mem.raw());

    // Additional representations are limited to small images. The required
    // alpha format remains one HGLOBAL up to 512 MiB; no full-size DIB/PNG Vec
    // or duplicate HGLOBAL is built for large captures.
    let mut legacy_mem = None;
    let mut png_mem = None;
    let mut png_format = 0;
    if include_optional_clipboard_formats(pixel_bytes) {
        let total_legacy = HEADER_V3.checked_add(pixel_bytes).ok_or("DIB-Größe überläuft")?;
        if let Ok(memory) = Memory::allocate(total_legacy) {
            let base = GlobalLock(memory.raw()).cast::<u8>();
            if !base.is_null() {
                let mut header = [0u8; HEADER_V3];
                header[0..4].copy_from_slice(&(HEADER_V3 as u32).to_le_bytes());
                header[4..8].copy_from_slice(&(width as i32).to_le_bytes());
                header[8..12].copy_from_slice(&(height as i32).to_le_bytes());
                header[12..14].copy_from_slice(&1u16.to_le_bytes());
                header[14..16].copy_from_slice(&32u16.to_le_bytes());
                header[16..20].copy_from_slice(&0u32.to_le_bytes());
                header[20..24].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
                std::ptr::copy_nonoverlapping(header.as_ptr(), base, HEADER_V3);
                for y in 0..height {
                    let source_y = if top_down { height - 1 - y } else { y };
                    let source_row = pixels.add(source_y * source_stride);
                    let output_row = base.add(HEADER_V3 + y * row_bytes);
                    for x in 0..width {
                        let source = source_row.add(x * 4);
                        let target = output_row.add(x * 4);
                        let alpha = u16::from(*source.add(3));
                        for channel in 0..3 {
                            let value = u16::from(*source.add(channel)) * alpha + 255 * (255 - alpha);
                            *target.add(channel) = ((value + 127) / 255) as u8;
                        }
                        *target.add(3) = 255;
                    }
                }
                GlobalUnlock(memory.raw());
                legacy_mem = Some(memory);
            } else { memory.free(); }
        }
        if let Some(image) = image_for_png {
            if let Ok(png) = encode_small_clipboard_png(image) {
                let png_name = [80u16, 78, 71, 0];
                png_format = RegisterClipboardFormatW(png_name.as_ptr());
                if png_format != 0 {
                    if let Ok(memory) = Memory::allocate(png.len()) {
                        let base = GlobalLock(memory.raw()).cast::<u8>();
                        if !base.is_null() {
                            std::ptr::copy_nonoverlapping(png.as_ptr(), base, png.len());
                            GlobalUnlock(memory.raw());
                            png_mem = Some(memory);
                        } else { memory.free(); }
                    }
                }
            }
        }
    }

    if OpenClipboard(owner) == 0 { return Err("OpenClipboard ist fehlgeschlagen; Clipboard blieb unverändert".into()); }
    let mut clipboard = Clipboard(true);
    if EmptyClipboard() == 0 { return Err("EmptyClipboard ist fehlgeschlagen; Clipboard-Inhalt konnte nicht ersetzt werden".into()); }

    let mut alpha_success = false;
    let mut legacy_success = false;
    let mut failed_dibv5 = false;
    let mut failed_png = false;
    let mut failed_legacy = false;
    if SetClipboardData(CF_DIBV5, dibv5_mem.raw()).is_null() {
        failed_dibv5 = true;
    } else {
        dibv5_mem.transferred();
        alpha_success = true;
    }
    if let Some(mut memory) = png_mem {
        if SetClipboardData(png_format, memory.raw()).is_null() {
            failed_png = true;
        } else {
            memory.transferred();
            alpha_success = true;
        }
    }
    if let Some(mut memory) = legacy_mem {
        if SetClipboardData(CF_DIB, memory.raw()).is_null() {
            failed_legacy = true;
        } else {
            memory.transferred();
            legacy_success = true;
        }
    }
    let closed = CloseClipboard() != 0;
    clipboard.0 = !closed;
    let failures = format!("CF_DIBV5 fehlgeschlagen: {failed_dibv5}, PNG fehlgeschlagen: {failed_png}, CF_DIB fehlgeschlagen: {failed_legacy}");
    clipboard_transfer_result(closed, alpha_success, legacy_success, &failures)
}

#[cfg(windows)]
fn encode_small_clipboard_png(image: &Image) -> Result<Vec<u8>, String> {
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    let pixel_bytes = validate_dib_image(image)?.pixel_bytes as usize;
    if pixel_bytes > OPTIONAL_CLIPBOARD_FORMAT_LIMIT {
        return Err("PNG clipboard encoding is limited to 16 MiB source pixels".into());
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(pixel_bytes).map_err(|e| format!("PNG-RGBA-Puffer konnte nicht reserviert werden: {e}"))?;
    rgba.extend_from_slice(&image.pixels);
    for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
    let width = u32::try_from(image.width).map_err(|_| "PNG width is too large")?;
    let height = u32::try_from(image.height).map_err(|_| "PNG height is too large")?;
    let reserve = pixel_bytes.checked_add(pixel_bytes / 50)
        .and_then(|n| n.checked_add(image.height.checked_mul(8)?))
        .and_then(|n| n.checked_add(64 * 1024))
        .ok_or("PNG output reservation overflow")?;
    let mut png = Vec::new();
    png.try_reserve_exact(reserve).map_err(|e| format!("PNG-Ausgabepuffer konnte nicht reserviert werden: {e}"))?;
    PngEncoder::new(&mut png).write_image(&rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG clipboard encode failed: {e}"))?;
    Ok(png)
}

#[cfg(test)]
mod tests;
