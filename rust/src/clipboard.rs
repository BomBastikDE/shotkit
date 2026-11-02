//! Win32 clipboard support for transferring a GDI bitmap as a CF_DIB.
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
}
#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> H;
    fn GlobalLock(memory: H) -> *mut c_void;
    fn GlobalUnlock(memory: H) -> i32;
    fn GlobalFree(memory: H) -> H;
}

const CF_DIB: u32 = 8;
const GMEM_MOVEABLE: u32 = 0x0002;
const GMEM_ZEROINIT: u32 = 0x0040;
const BI_RGB: u32 = 0;

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
