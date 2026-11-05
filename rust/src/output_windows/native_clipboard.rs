use super::*;

#[cfg(windows)]
pub fn copy_image_to_clipboard(
    owner: *mut c_void,
    image: &Image,
    exclude_from_history: bool,
) -> Result<(), String> {
    let _dib = validate_dib_image(image)?;
    unsafe {
        copy_bgra_clipboard(
            owner,
            image.pixels.as_ptr(),
            image.width,
            image.height,
            image.width.checked_mul(4).ok_or("Bildzeile überläuft")?,
            true,
            Some(image),
            exclude_from_history,
        )
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
    exclude_from_history: bool,
) -> Result<(), String> {
    copy_bgra_clipboard(
        owner,
        pixels,
        width,
        height,
        source_stride,
        top_down,
        None,
        exclude_from_history,
    )
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
    exclude_from_history: bool,
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
            if handle.is_null() {
                return Err("GlobalAlloc für Zwischenablage ist fehlgeschlagen".into());
            }
            Ok(Self(handle))
        }
        fn raw(&self) -> Handle {
            self.0
        }
        fn transferred(&mut self) {
            self.0 = null_mut();
        }
        fn free(mut self) {
            if !self.0.is_null() {
                unsafe {
                    GlobalFree(self.0);
                }
                self.0 = null_mut();
            }
        }
    }
    impl Drop for Memory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    GlobalFree(self.0);
                }
            }
        }
    }
    struct Clipboard(bool);
    impl Drop for Clipboard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CloseClipboard();
                }
            }
        }
    }

    const HEADER_V5: usize = 124;
    const HEADER_V3: usize = 40;
    const MAX_DIMENSION: usize = 30_000;
    const MAX_BYTES: usize = 512 * 1024 * 1024;
    if pixels.is_null() {
        return Err("BGRA-Quellpuffer ist null".into());
    }
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("Bildabmessungen ungültig oder überschreiten 30000 Pixel".into());
    }
    let row_bytes = width.checked_mul(4).ok_or("Bildzeile überläuft")?;
    if source_stride < row_bytes {
        return Err("BGRA-Quellzeile ist kürzer als die Bildzeile".into());
    }
    let pixel_bytes = row_bytes.checked_mul(height).ok_or("Bildgröße überläuft")?;
    let source_bytes = source_stride
        .checked_mul(height)
        .ok_or("Quellpuffergröße überläuft")?;
    if pixel_bytes > MAX_BYTES || source_bytes > MAX_BYTES {
        return Err("Clipboard-Bild überschreitet 512 MiB".into());
    }
    let total_v5 = HEADER_V5
        .checked_add(pixel_bytes)
        .ok_or("DIBV5-Größe überläuft")?;
    let mut dibv5_mem = Memory::allocate(total_v5)?;
    let dibv5_base = GlobalLock(dibv5_mem.raw()).cast::<u8>();
    if dibv5_base.is_null() {
        return Err("GlobalLock für CF_DIBV5 ist fehlgeschlagen".into());
    }
    std::ptr::copy_nonoverlapping(
        dibv5_header(width as i32, height as i32, pixel_bytes as u32).as_ptr(),
        dibv5_base,
        HEADER_V5,
    );
    for y in 0..height {
        let source_y = if top_down { y } else { height - 1 - y };
        std::ptr::copy_nonoverlapping(
            pixels.add(source_y * source_stride),
            dibv5_base.add(HEADER_V5 + y * row_bytes),
            row_bytes,
        );
    }
    GlobalUnlock(dibv5_mem.raw());

    // Additional representations are limited to small images. The required
    // alpha format remains one HGLOBAL up to 512 MiB; no full-size DIB/PNG Vec
    // or duplicate HGLOBAL is built for large captures.
    let mut legacy_mem = None;
    let mut png_mem = None;
    let mut png_format = 0;
    if include_optional_clipboard_formats(pixel_bytes) {
        let total_legacy = HEADER_V3
            .checked_add(pixel_bytes)
            .ok_or("DIB-Größe überläuft")?;
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
                            let value =
                                u16::from(*source.add(channel)) * alpha + 255 * (255 - alpha);
                            *target.add(channel) = ((value + 127) / 255) as u8;
                        }
                        *target.add(3) = 255;
                    }
                }
                GlobalUnlock(memory.raw());
                legacy_mem = Some(memory);
            } else {
                memory.free();
            }
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
                        } else {
                            memory.free();
                        }
                    }
                }
            }
        }
    }

    // Register and prepare both DWORD markers before modifying the clipboard.
    // A zero DWORD asks Windows not to retain or synchronize this item.
    let mut privacy_markers = Vec::new();
    if exclude_from_history {
        for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let format = RegisterClipboardFormatW(wide.as_ptr());
            if format == 0 {
                return Err(format!("Zwischenablage-Schutzformat {name} konnte nicht registriert werden; Zwischenablage blieb unverändert"));
            }
            let memory = Memory::allocate(std::mem::size_of::<u32>())?;
            // GMEM_ZEROINIT supplies the required DWORD zero.
            privacy_markers.push((format, memory));
        }
    }

    if !crate::clipboard_retry::open_with_retry(|| OpenClipboard(owner) != 0) {
        return Err("OpenClipboard ist fehlgeschlagen; Clipboard blieb unverändert".into());
    }
    let mut clipboard = Clipboard(true);
    if EmptyClipboard() == 0 {
        return Err(
            "EmptyClipboard ist fehlgeschlagen; Clipboard-Inhalt konnte nicht ersetzt werden"
                .into(),
        );
    }

    for (format, mut memory) in privacy_markers {
        if SetClipboardData(format, memory.raw()).is_null() {
            // No image has been published. Remove any marker already transferred.
            let cleared = EmptyClipboard() != 0;
            return Err(format!("Zwischenablage-Schutz konnte nicht gesetzt werden; kein Bild kopiert (Bereinigung: {})", if cleared { "erfolgreich" } else { "fehlgeschlagen" }));
        }
        memory.transferred();
    }

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
    rgba.try_reserve_exact(pixel_bytes)
        .map_err(|e| format!("PNG-RGBA-Puffer konnte nicht reserviert werden: {e}"))?;
    rgba.extend_from_slice(&image.pixels);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let width = u32::try_from(image.width).map_err(|_| "PNG width is too large")?;
    let height = u32::try_from(image.height).map_err(|_| "PNG height is too large")?;
    let reserve = pixel_bytes
        .checked_add(pixel_bytes / 50)
        .and_then(|n| n.checked_add(image.height.checked_mul(8)?))
        .and_then(|n| n.checked_add(64 * 1024))
        .ok_or("PNG output reservation overflow")?;
    let mut png = Vec::new();
    png.try_reserve_exact(reserve)
        .map_err(|e| format!("PNG-Ausgabepuffer konnte nicht reserviert werden: {e}"))?;
    PngEncoder::new(&mut png)
        .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG clipboard encode failed: {e}"))?;
    Ok(png)
}
