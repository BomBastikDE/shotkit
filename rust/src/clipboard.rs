//! Win32 clipboard support for transferring a GDI bitmap as a CF_DIB and
//! reading clipboard-owned CF_BITMAP data into the application's BGRA image.
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
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, memory: H) -> H;
    fn GetClipboardData(format: u32) -> H;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> H;
    fn GlobalLock(memory: H) -> *mut c_void;
    fn GlobalUnlock(memory: H) -> i32;
    fn GlobalFree(memory: H) -> H;
}

const CF_DIB: u32 = 8;
const CF_BITMAP: u32 = 2;
const GMEM_MOVEABLE: u32 = 0x0002;
const GMEM_ZEROINIT: u32 = 0x0040;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const MAX_IMAGE_DIMENSION: usize = 30_000;
const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;

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

#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(object: H, bytes: i32, out: *mut c_void) -> i32;
}

struct GlobalMemory(H);
impl Drop for GlobalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
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

/// Copies an existing, unselected GDI bitmap to the Windows clipboard as CF_DIB.
/// The caller retains ownership of `bitmap`; successful `SetClipboardData` transfers
/// ownership of the allocated DIB block to Windows.
pub unsafe fn copy_bitmap(
    owner: *mut c_void,
    bitmap: *mut c_void,
    width: i32,
    height: i32,
) -> Result<(), String> {
    if bitmap.is_null() {
        return Err("bitmap handle is null".into());
    }
    if width <= 0 || height <= 0 {
        return Err("bitmap dimensions must be positive".into());
    }

    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| "bitmap row size overflow".to_string())?;
    let pixel_bytes = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| "bitmap pixel size overflow".to_string())?;
    let pixel_bytes_u32 =
        u32::try_from(pixel_bytes).map_err(|_| "bitmap is too large for a CF_DIB".to_string())?;
    let total_bytes = std::mem::size_of::<BitmapInfoHeader>()
        .checked_add(pixel_bytes)
        .ok_or_else(|| "DIB allocation size overflow".to_string())?;

    let memory = GlobalMemory(GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total_bytes));
    if memory.0.is_null() {
        return Err("GlobalAlloc failed for CF_DIB".into());
    }
    let base = GlobalLock(memory.0).cast::<u8>();
    if base.is_null() {
        return Err("GlobalLock failed for CF_DIB".into());
    }

    let mut header = BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width,
        height, // positive height: bottom-up DIB rows
        planes: 1,
        bit_count: 32,
        compression: BI_RGB,
        size_image: pixel_bytes_u32,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };
    std::ptr::write(base.cast::<BitmapInfoHeader>(), header);
    let bits = base
        .add(std::mem::size_of::<BitmapInfoHeader>())
        .cast::<c_void>();
    let dc = ScreenDc(GetDC(null_mut()));
    if dc.0.is_null() {
        GlobalUnlock(memory.0);
        return Err("GetDC failed while reading bitmap".into());
    }
    let lines = GetDIBits(
        dc.0,
        bitmap,
        0,
        height as u32,
        bits,
        &mut header as *mut _ as *mut c_void,
        0,
    );
    if lines != height {
        GlobalUnlock(memory.0);
        return Err(format!("GetDIBits returned {lines} of {height} scan lines"));
    }
    // Ensure the serialized header matches the requested CF_DIB layout.
    std::ptr::write(base.cast::<BitmapInfoHeader>(), header);
    GlobalUnlock(memory.0);
    drop(dc);

    if OpenClipboard(owner) == 0 {
        return Err("OpenClipboard failed".into());
    }
    if EmptyClipboard() == 0 {
        CloseClipboard();
        return Err("EmptyClipboard failed".into());
    }
    if SetClipboardData(CF_DIB, memory.0).is_null() {
        CloseClipboard();
        return Err("SetClipboardData(CF_DIB) failed".into());
    }
    // Windows owns the movable block after a successful SetClipboardData.
    let transferred = memory.0;
    std::mem::forget(memory);
    if CloseClipboard() == 0 {
        // Clipboard data has already been transferred and must not be freed here.
        let _ = transferred;
        return Err("CloseClipboard failed after transferring CF_DIB".into());
    }
    Ok(())
}

struct ClipboardGuard(bool);
impl ClipboardGuard {
    unsafe fn open(owner: H) -> Result<Self, String> {
        if OpenClipboard(owner) == 0 {
            Err("OpenClipboard failed".into())
        } else {
            Ok(Self(true))
        }
    }
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

/// Reads clipboard-owned `CF_BITMAP` data as a top-row-first BGRA image.
///
/// Returns `Ok(None)` when the clipboard has no `CF_BITMAP` format. The
/// clipboard remains open while its handle is queried and copied. The returned
/// HBITMAP belongs to the clipboard and is never deleted/freed here.
/// `CF_BITMAP` does not provide dependable alpha data, so every output pixel
/// gets A=255; transparent edges are neither preserved nor trimmed. The B, G,
/// and R bytes from the 32-bit `BI_RGB` conversion are kept in BGRA order.
///
/// Each dimension is limited to 30,000 pixels and decoded storage to 512 MiB.
/// The caller must pass a valid owner HWND or null. Invalid metadata and GDI
/// failures are returned as errors instead of panicking.
pub unsafe fn read_image(
    owner: *mut c_void,
) -> Result<Option<shotkit_rust_prototype::stitch::Image>, String> {
    let mut clipboard = ClipboardGuard::open(owner)?;
    if IsClipboardFormatAvailable(CF_BITMAP) == 0 {
        clipboard.close()?;
        return Ok(None);
    }
    let bitmap = GetClipboardData(CF_BITMAP);
    if bitmap.is_null() {
        return Err("GetClipboardData(CF_BITMAP) failed".into());
    }

    let mut metadata = Bitmap::default();
    let object_bytes = i32::try_from(std::mem::size_of::<Bitmap>())
        .map_err(|_| "BITMAP metadata size overflow".to_string())?;
    if GetObjectW(bitmap, object_bytes, (&mut metadata as *mut Bitmap).cast()) != object_bytes {
        return Err("GetObjectW(CF_BITMAP) returned invalid metadata".into());
    }
    if metadata.width <= 0 || metadata.height == 0 || metadata.height == i32::MIN {
        return Err("CF_BITMAP has invalid dimensions".into());
    }
    let width =
        usize::try_from(metadata.width).map_err(|_| "CF_BITMAP width is invalid".to_string())?;
    let height = usize::try_from(metadata.height.unsigned_abs())
        .map_err(|_| "CF_BITMAP height is invalid".to_string())?;
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err("CF_BITMAP exceeds the 30000-pixel dimension limit".into());
    }
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "CF_BITMAP row size overflow".to_string())?;
    let pixel_bytes = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "CF_BITMAP pixel size overflow".to_string())?;
    if pixel_bytes > MAX_IMAGE_BYTES || pixel_bytes > u32::MAX as usize {
        return Err("CF_BITMAP exceeds the 512 MiB image limit".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(pixel_bytes)
        .map_err(|e| format!("Could not allocate CF_BITMAP pixels: {e}"))?;
    pixels.resize(pixel_bytes, 0);

    let mut header = BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width: metadata.width,
        height: -(height as i32), // top-down rows match Image's row order
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
    if dc.0.is_null() {
        return Err("GetDC failed while reading CF_BITMAP".into());
    }
    let lines = GetDIBits(
        dc.0,
        bitmap,
        0,
        height as u32,
        pixels.as_mut_ptr().cast(),
        (&mut header as *mut BitmapInfoHeader).cast(),
        DIB_RGB_COLORS,
    );
    if lines != height as i32 {
        return Err(format!("GetDIBits returned {lines} of {height} scan lines"));
    }
    // BI_RGB does not define alpha for this source format.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    drop(dc);
    clipboard.close()?;

    let image = shotkit_rust_prototype::stitch::Image::new(width, height, pixels)
        .map_err(|e| format!("Invalid CF_BITMAP image: {e}"))?;
    Ok(Some(image))
}
