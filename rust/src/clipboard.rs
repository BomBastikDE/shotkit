//! Win32 clipboard interop. Alpha-capable PNG/CF_DIBV5 formats are preferred;
//! CF_DIB and CF_BITMAP remain opaque compatibility fallbacks.
use std::{ffi::c_void, ptr::null_mut};

type H = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

#[link(name = "gdi32")]
extern "system" {
    fn GetDIBits(
        dc: H,
        bitmap: H,
        start: u32,
        lines: u32,
        bits: *mut c_void,
        info: *mut c_void,
        usage: u32,
    ) -> i32;
}
#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: H) -> H;
    fn ReleaseDC(hwnd: H, dc: H) -> i32;
    fn OpenClipboard(owner: H) -> i32;
    fn CloseClipboard() -> i32;
    fn GetClipboardData(format: u32) -> H;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GlobalLock(memory: H) -> *mut c_void;
    fn GlobalUnlock(memory: H) -> i32;
    fn GlobalSize(memory: H) -> usize;
}

const CF_DIB: u32 = 8;
const CF_BITMAP: u32 = 2;
const CF_DIBV5: u32 = 17;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const MAX_IMAGE_DIMENSION: usize = 30_000;
const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_DDB_COPY_BYTES: usize = 64 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Bitmap {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: *mut c_void,
}

#[repr(C)]
struct DibSection {
    bitmap: Bitmap,
    header: BitmapInfoHeader,
    bitfields: [u32; 3],
    section: H,
    offset: u32,
}

#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(object: H, bytes: i32, out: *mut c_void) -> i32;
}

struct ScreenDc(H);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ReleaseDC(null_mut(), self.0);
            }
        }
    }
}

/// Copies the editor's GDI bitmap to the clipboard. DIB sections produced by
/// ShotKit are read directly so their straight-alpha BGRA bytes are preserved.
/// PNG and CF_DIBV5 carry alpha; CF_DIB is an opaque white
/// compatibility fallback. The caller retains ownership of `bitmap`.
// SAFETY CONTRACT:
// Voraussetzung: bitmap bleibt gültig und entspricht den Maßen; direkter DIB-Speicher ist lesbar und korrekt orientiert.
// Warum erfüllt bzw. offen: Der Aufrufer hält die temporäre Bitmap bis zur Rückkehr; DIB-Größe und Stride werden geprüft.
// Nachrichten: Clipboard-Ausgabe und Fehlerpfade können reentrieren.
pub unsafe fn copy_bitmap(
    owner: *mut c_void,
    bitmap: *mut c_void,
    width: i32,
    height: i32,
) -> Result<(), String> {
    if bitmap.is_null() { return Err("bitmap handle is null".into()); }
    if width <= 0 || height <= 0 { return Err("bitmap dimensions must be positive".into()); }
    let w = usize::try_from(width).map_err(|_| "bitmap width is invalid")?;
    let h = usize::try_from(height).map_err(|_| "bitmap height is invalid")?;
    if w > MAX_IMAGE_DIMENSION || h > MAX_IMAGE_DIMENSION { return Err("bitmap exceeds the 30000-pixel dimension limit".into()); }
    let pixel_bytes = w.checked_mul(h).and_then(|n| n.checked_mul(4)).ok_or("bitmap size overflow")?;
    if pixel_bytes > MAX_IMAGE_BYTES { return Err("bitmap exceeds the 512 MiB image limit".into()); }

    let mut section = std::mem::MaybeUninit::<DibSection>::zeroed();
    let section_size = i32::try_from(std::mem::size_of::<DibSection>()).map_err(|_| "DIBSECTION size overflow")?;
    let got = GetObjectW(bitmap, section_size, section.as_mut_ptr().cast());
    if got == section_size {
        let section = section.assume_init();
        if section.bitmap.width != width || section.header.width != width
            || section.header.height.unsigned_abs() != height as u32
        {
            return Err("DIBSECTION dimensions do not match the requested bitmap".into());
        }
        if section.header.bit_count == 32 && section.header.compression == BI_RGB
            && !section.bitmap.bits.is_null()
        {
            let row_bytes = w.checked_mul(4).ok_or("bitmap row size overflow")?;
            let source_stride = usize::try_from(section.bitmap.width_bytes.unsigned_abs())
                .map_err(|_| "DIBSECTION row stride is invalid")?;
            if source_stride != row_bytes { return Err("DIBSECTION row stride is unexpected".into()); }
            return shotkit_rust_prototype::output_windows::copy_dib_section_to_clipboard(
                owner,
                section.bitmap.bits.cast(),
                w,
                h,
                source_stride,
                // copy_bitmap is called with the editor bitmap created by
                // main::create_bitmap: that DIB section has negative
                // biHeight and its bytes are copied from top-down BGRA rows.
                // Use that creation contract directly; GetObject's BITMAP
                // dimensions are not the row-order contract for this buffer.
                true,
            );
        }
    }

    // DDB/unsupported DIBSECTION fallback. GetDIBits does not define alpha for
    // BI_RGB, so explicitly mark these pixels opaque. A temporary DDB conversion
    // is capped at 64 MiB to bound the extra Rust buffer plus clipboard block.
    if pixel_bytes > MAX_DDB_COPY_BYTES {
        return Err("DDB clipboard fallback is limited to 64 MiB; larger DIB sections use the direct path".into());
    }
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(pixel_bytes).map_err(|e| format!("Could not allocate DDB clipboard pixels: {e}"))?;
    pixels.resize(pixel_bytes, 0);
    let mut header = BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width,
        height: -(height as i32),
        planes: 1,
        bit_count: 32,
        compression: BI_RGB,
        size_image: u32::try_from(pixel_bytes).map_err(|_| "bitmap too large for DIB")?,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };
    let dc = ScreenDc(GetDC(null_mut()));
    if dc.0.is_null() { return Err("GetDC failed while reading bitmap".into()); }
    let lines = GetDIBits(dc.0, bitmap, 0, height as u32, pixels.as_mut_ptr().cast(),
        (&mut header as *mut BitmapInfoHeader).cast(), DIB_RGB_COLORS);
    if lines != height { return Err(format!("GetDIBits returned {lines} of {height} scan lines")); }
    for pixel in pixels.chunks_exact_mut(4) { pixel[3] = 255; }
    drop(dc);
    let image = shotkit_rust_prototype::stitch::Image::new(w, h, pixels).map_err(str::to_owned)?;
    shotkit_rust_prototype::output_windows::copy_image_to_clipboard(owner, &image)
}

struct ClipboardGuard(bool);
impl ClipboardGuard {
    // SAFETY CONTRACT:
    // Voraussetzung: owner ist null oder ein gültiges Fenster; Clipboard wird auf dem richtigen Thread verwendet.
    // Warum erfüllt bzw. offen: Der Rückgabewert von OpenClipboard wird geprüft.
    // Nachrichten: Kein expliziter Nachrichtenversand; allgemeine Reentranzfreiheit ist nicht belegt.
    unsafe fn open(owner: H) -> Result<Self, String> {
        if OpenClipboard(owner) == 0 {
            Err("OpenClipboard failed".into())
        } else {
            Ok(Self(true))
        }
    }
    // SAFETY CONTRACT:
    // Voraussetzung: Dieser Guard besitzt eine erfolgreich geöffnete Clipboard-Sitzung auf demselben Thread.
    // Warum erfüllt bzw. offen: Der Guard wird nach erfolgreichem OpenClipboard erzeugt und synchron geschlossen.
    // Nachrichten: Kein expliziter Nachrichtenversand; allgemeine Reentranzfreiheit ist nicht belegt.
    unsafe fn close(&mut self) -> Result<(), String> {
        if self.0 {
            self.0 = false;
            if CloseClipboard() == 0 {
                return Err("CloseClipboard failed".into());
            }
        }
        Ok(())
    }
}
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CloseClipboard();
            }
        }
    }
}

/// Reads clipboard images, preferring alpha-preserving registered PNG and
/// CF_DIBV5, then legacy CF_DIB and CF_BITMAP (which is necessarily opaque).
/// DIB paths accept only 24/32-bit uncompressed layouts within 30000 pixels per
/// dimension and 512 MiB decoded size. The clipboard remains open while its
/// global-memory handles are copied; Windows retains ownership of those handles.
// SAFETY CONTRACT:
// Voraussetzung: owner ist null oder ein gültiges Fenster; Clipboard bleibt beim Lesen geöffnet.
// Warum erfüllt bzw. offen: Ein Guard verwaltet die Sitzung; Formate und Decoderergebnisse werden geprüft.
// Nachrichten: GetClipboardData kann verzögertes Rendering und Nachrichten auslösen.
pub unsafe fn read_image(
    owner: *mut c_void,
) -> Result<Option<shotkit_rust_prototype::stitch::Image>, String> {
    let mut clipboard = ClipboardGuard::open(owner)?;
    let png_name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
    let png_format = RegisterClipboardFormatW(png_name.as_ptr());
    let formats = [
        (png_format, "PNG"),
        (CF_DIBV5, "CF_DIBV5"),
        (CF_DIB, "CF_DIB"),
        (CF_BITMAP, "CF_BITMAP"),
    ];
    let available = formats.iter().any(|(format, _)| *format != 0 && IsClipboardFormatAvailable(*format) != 0);
    if !available {
        clipboard.close()?;
        return Ok(None);
    }

    let mut failures = Vec::new();
    if png_format != 0 && IsClipboardFormatAvailable(png_format) != 0 {
        match read_png_format(png_format) {
            Ok(image) => { clipboard.close()?; return Ok(Some(image)); }
            Err(e) => failures.push(format!("PNG: {e}")),
        }
    }
    if IsClipboardFormatAvailable(CF_DIBV5) != 0 {
        match read_dib_format(CF_DIBV5, true) {
            Ok(image) => { clipboard.close()?; return Ok(Some(image)); }
            Err(e) => failures.push(format!("CF_DIBV5: {e}")),
        }
    }
    if IsClipboardFormatAvailable(CF_DIB) != 0 {
        match read_dib_format(CF_DIB, false) {
            Ok(image) => { clipboard.close()?; return Ok(Some(image)); }
            Err(e) => failures.push(format!("CF_DIB: {e}")),
        }
    }
    if IsClipboardFormatAvailable(CF_BITMAP) != 0 {
        match read_bitmap(GetClipboardData(CF_BITMAP)) {
            Ok(image) => { clipboard.close()?; return Ok(Some(image)); }
            Err(e) => failures.push(format!("CF_BITMAP: {e}")),
        }
    }
    clipboard.close()?;
    Err(format!("Clipboard image formats were present but unreadable: {}", failures.join("; ")))
}

struct LockedGlobal(H);
impl Drop for LockedGlobal {
    fn drop(&mut self) {
        unsafe { GlobalUnlock(self.0); }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: handle bezeichnet lesbaren Global-Memory-Speicher, der während decode gültig und unverändert bleibt.
// Warum erfüllt bzw. offen: Größe und GlobalLock werden geprüft; der Decoder läuft synchron unter dem Lock.
// Nachrichten: Kein expliziter Nachrichtenversand; der Decoder darf das Handle nicht invalidieren.
unsafe fn with_global_bytes<T>(handle: H, decode: impl FnOnce(&[u8]) -> Result<T, String>) -> Result<T, String> {
    if handle.is_null() { return Err("GetClipboardData returned a null handle".into()); }
    let size = GlobalSize(handle);
    if size == 0 || size > MAX_IMAGE_BYTES + 1024 * 1024 {
        return Err("clipboard global-memory size is zero or exceeds the supported limit".into());
    }
    let base = GlobalLock(handle).cast::<u8>();
    if base.is_null() { return Err("GlobalLock failed while reading clipboard data".into()); }
    let _lock = LockedGlobal(handle);
    let bytes = std::slice::from_raw_parts(base, size);
    decode(bytes)
}

// SAFETY CONTRACT:
// Voraussetzung: Clipboard ist offen und das Format liefert gültigen HGLOBAL-Speicher.
// Warum erfüllt bzw. offen: Der Aufruf erfolgt unter dem Clipboard-Guard; Dekodierung bleibt innerhalb des Locks.
// Nachrichten: GetClipboardData kann verzögertes Rendering auslösen.
unsafe fn read_png_format(format: u32) -> Result<shotkit_rust_prototype::stitch::Image, String> {
    with_global_bytes(GetClipboardData(format), |bytes| {
        shotkit_rust_prototype::image_io::decode(bytes).map_err(|e| format!("PNG decode failed: {e}"))
    })
}

// SAFETY CONTRACT:
// Voraussetzung: Clipboard ist offen und das Format liefert gültigen HGLOBAL-DIB-Speicher.
// Warum erfüllt bzw. offen: Der Aufruf erfolgt unter dem Clipboard-Guard; Dekodierung bleibt innerhalb des Locks.
// Nachrichten: GetClipboardData kann verzögertes Rendering auslösen.
unsafe fn read_dib_format(format: u32, require_v5: bool) -> Result<shotkit_rust_prototype::stitch::Image, String> {
    with_global_bytes(GetClipboardData(format), |bytes| {
        shotkit_rust_prototype::output_windows::decode_dib(bytes, require_v5)
    })
}

// SAFETY CONTRACT:
// Voraussetzung: bitmap bleibt gültig und ist für GetDIBits geeignet.
// Warum erfüllt bzw. offen: Das Handle stammt aus dem geöffneten Clipboard; der DC-Auswahlzustand ist nicht vollständig geprüft.
// Nachrichten: Kein expliziter Fensternachrichtenversand im Funktionskörper.
unsafe fn read_bitmap(bitmap: H) -> Result<shotkit_rust_prototype::stitch::Image, String> {
    if bitmap.is_null() { return Err("GetClipboardData(CF_BITMAP) returned null".into()); }
    let mut metadata = Bitmap::default();
    let object_bytes = i32::try_from(std::mem::size_of::<Bitmap>()).map_err(|_| "BITMAP metadata size overflow")?;
    if GetObjectW(bitmap, object_bytes, (&mut metadata as *mut Bitmap).cast()) != object_bytes {
        return Err("GetObjectW(CF_BITMAP) returned invalid metadata".into());
    }
    if metadata.width <= 0 || metadata.height == 0 || metadata.height == i32::MIN {
        return Err("CF_BITMAP has invalid dimensions".into());
    }
    let width = usize::try_from(metadata.width).map_err(|_| "CF_BITMAP width is invalid")?;
    let height = usize::try_from(metadata.height.unsigned_abs()).map_err(|_| "CF_BITMAP height is invalid")?;
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err("CF_BITMAP exceeds the 30000-pixel dimension limit".into());
    }
    let pixel_bytes = width.checked_mul(height).and_then(|n| n.checked_mul(4)).ok_or("CF_BITMAP size overflow")?;
    if pixel_bytes > MAX_IMAGE_BYTES || pixel_bytes > u32::MAX as usize {
        return Err("CF_BITMAP exceeds the 512 MiB image limit".into());
    }
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(pixel_bytes).map_err(|e| format!("Could not allocate CF_BITMAP pixels: {e}"))?;
    pixels.resize(pixel_bytes, 0);
    let mut header = BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width: metadata.width,
        height: -(height as i32),
        planes: 1,
        bit_count: 32,
        compression: BI_RGB,
        size_image: pixel_bytes as u32,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };
    let dc = ScreenDc(GetDC(null_mut()));
    if dc.0.is_null() { return Err("GetDC failed while reading CF_BITMAP".into()); }
    let lines = GetDIBits(dc.0, bitmap, 0, height as u32, pixels.as_mut_ptr().cast(),
        (&mut header as *mut BitmapInfoHeader).cast(), DIB_RGB_COLORS);
    if lines != height as i32 { return Err(format!("GetDIBits returned {lines} of {height} scan lines")); }
    for pixel in pixels.chunks_exact_mut(4) { pixel[3] = 255; }
    drop(dc);
    shotkit_rust_prototype::stitch::Image::new(width, height, pixels).map_err(str::to_owned)
}
