//! Owned cursor pixels sampled near the screen-capture operation.
//!
//! Positions are physical virtual-desktop pixels. Call `capture` in the same
//! physical coordinate space as the screen capture; sampling cursor shape and
//! position is best effort, not an atomic screen/cursor transaction. No native
//! handle escapes this module. Hidden/suppressed cursors produce `Ok(None)`.
//! The displayed frame of animated cursors is not established by this code.
//!
//! Win32's legacy cursor operation is `(destination AND mask) XOR color`.
//! Inversion needs the destination and cannot be represented by an RGBA image.
//! Keep `Raster` for an exact renderer, or call `to_rgba` and handle Unsupported.
//! Do not substitute a stock arrow when capture or conversion fails.

/// Bound native dimensions; each individual pixel allocation is at most 16 MiB.
pub const MAX_CURSOR_SIDE: u32 = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    Native(&'static str),
    Invalid(&'static str),
    Unsupported(&'static str),
    Allocation,
    CoordinateOverflow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorPixels {
    /// Top-down RGBA bytes, with premultiplied RGB, as supplied by Windows.
    PremultipliedRgba(Vec<u8>),
    /// Top-down pixels: each mask byte is 0 or 255; XOR bytes are RGB triples.
    /// For every destination channel: `(dst & and_mask[p]) ^ xor_rgb[3*p+c]`.
    Raster { and_mask: Vec<u8>, xor_rgb: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub width: u32,
    pub height: u32,
    /// Physical screen position of the cursor hotspot at sampling time.
    pub screen_hotspot: (i64, i64),
    pub hotspot: (u32, u32),
    pub pixels: CursorPixels,
}

impl CursorSnapshot {
    /// Top-left relative to the physical screen capture's origin. Negative
    /// origins and cursor parts outside the capture remain valid; clip later.
    pub fn top_left_in_capture(&self, origin: (i64, i64)) -> Result<(i64, i64), SnapshotError> {
        let axis = |position: i64, hotspot: u32, origin: i64| {
            position
                .checked_sub(i64::from(hotspot))
                .and_then(|value| value.checked_sub(origin))
                .ok_or(SnapshotError::CoordinateOverflow)
        };
        Ok((
            axis(self.screen_hotspot.0, self.hotspot.0, origin.0)?,
            axis(self.screen_hotspot.1, self.hotspot.1, origin.1)?,
        ))
    }

    /// Copy into straight-alpha RGBA suitable for an ordinary editable image.
    /// Destination-dependent raster pixels return an explicit error.
    pub fn to_rgba(&self) -> Result<Vec<u8>, SnapshotError> {
        let count = pixel_count(self.width, self.height)?;
        let mut rgba = zeroed(count.checked_mul(4).ok_or(SnapshotError::Allocation)?)?;
        match &self.pixels {
            CursorPixels::PremultipliedRgba(bytes) => {
                if bytes.len() != rgba.len() {
                    return Err(SnapshotError::Invalid("RGBA length"));
                }
                let (src_pixels, remainder) = bytes.as_chunks::<4>();
                if !remainder.is_empty() {
                    return Err(SnapshotError::Invalid("RGBA length"));
                }
                for (src, dst) in src_pixels
                    .iter()
                    .zip(rgba.as_chunks_mut::<4>().0.iter_mut())
                {
                    let alpha = u32::from(src[3]);
                    if src[..3].iter().any(|&channel| u32::from(channel) > alpha) {
                        return Err(SnapshotError::Unsupported("non-premultiplied alpha cursor"));
                    }
                    for c in 0..3 {
                        dst[c] = ((u32::from(src[c]) * 255 + alpha / 2)
                            .checked_div(alpha)
                            .unwrap_or(0)) as u8;
                    }
                    dst[3] = src[3];
                }
            }
            CursorPixels::Raster { and_mask, xor_rgb } => {
                if and_mask.len() != count || xor_rgb.len() != count * 3 {
                    return Err(SnapshotError::Invalid("raster length"));
                }
                let (xor_pixels, xor_remainder) = xor_rgb.as_chunks::<3>();
                if !xor_remainder.is_empty() {
                    return Err(SnapshotError::Invalid("raster length"));
                }
                for ((&mask, rgb), dst) in and_mask
                    .iter()
                    .zip(xor_pixels)
                    .zip(rgba.as_chunks_mut::<4>().0.iter_mut())
                {
                    match mask {
                        0 => {
                            dst[..3].copy_from_slice(rgb);
                            dst[3] = 255;
                        }
                        255 if *rgb == [0, 0, 0] => {}
                        255 => {
                            return Err(SnapshotError::Unsupported(
                                "destination-dependent XOR pixel",
                            ))
                        }
                        _ => return Err(SnapshotError::Invalid("AND mask byte")),
                    }
                }
            }
        }
        Ok(rgba)
    }
}

fn pixel_count(width: u32, height: u32) -> Result<usize, SnapshotError> {
    if width == 0 || height == 0 || width > MAX_CURSOR_SIDE || height > MAX_CURSOR_SIDE {
        return Err(SnapshotError::Unsupported("cursor dimensions"));
    }
    (width as usize)
        .checked_mul(height as usize)
        .ok_or(SnapshotError::Allocation)
}

fn zeroed(length: usize) -> Result<Vec<u8>, SnapshotError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| SnapshotError::Allocation)?;
    bytes.resize(length, 0);
    Ok(bytes)
}

/// Decode DWORD-aligned, top-down, one-bit DIB rows (most significant bit first).
#[cfg(any(windows, test))]
fn unpack_mask(bits: &[u8], width: u32, height: u32) -> Result<Vec<u8>, SnapshotError> {
    let count = pixel_count(width, height)?;
    let stride = (width as usize).div_ceil(32) * 4;
    if bits.len() != stride * height as usize {
        return Err(SnapshotError::Invalid("mask DIB length"));
    }
    let mut result = zeroed(count)?;
    for y in 0..height as usize {
        for x in 0..width as usize {
            result[y * width as usize + x] = if bits[y * stride + x / 8] & (0x80 >> (x % 8)) != 0 {
                255
            } else {
                0
            };
        }
    }
    Ok(result)
}

#[cfg(windows)]
pub use native::capture;

#[cfg(not(windows))]
pub fn capture() -> Result<Option<CursorSnapshot>, SnapshotError> {
    Err(SnapshotError::Unsupported(
        "native cursor capture requires Windows",
    ))
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        mem::{size_of, zeroed as zeroed_struct},
        ptr::null_mut,
    };
    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct CursorInfo {
        size: u32,
        flags: u32,
        cursor: Handle,
        position: Point,
    }
    #[repr(C)]
    struct IconInfo {
        is_icon: i32,
        hotspot_x: u32,
        hotspot_y: u32,
        mask: Handle,
        color: Handle,
    }
    #[repr(C)]
    struct Bitmap {
        kind: i32,
        width: i32,
        height: i32,
        width_bytes: i32,
        planes: u16,
        bits_per_pixel: u16,
        bits: *mut c_void,
    }
    #[repr(C)]
    struct DibHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_ppm: i32,
        y_ppm: i32,
        colors_used: u32,
        colors_important: u32,
    }
    #[repr(C)]
    struct DibInfo {
        header: DibHeader,
        colors: [u32; 2],
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetCursorInfo(info: *mut CursorInfo) -> i32;
        fn GetPhysicalCursorPos(point: *mut Point) -> i32;
        fn CopyIcon(icon: Handle) -> Handle;
        fn DestroyCursor(cursor: Handle) -> i32;
        fn GetIconInfo(icon: Handle, info: *mut IconInfo) -> i32;
        fn GetDC(window: Handle) -> Handle;
        fn ReleaseDC(window: Handle, dc: Handle) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn GetObjectW(object: Handle, size: i32, output: *mut c_void) -> i32;
        fn GetDIBits(
            dc: Handle,
            bitmap: Handle,
            start: u32,
            lines: u32,
            bits: *mut c_void,
            info: *mut DibInfo,
            usage: u32,
        ) -> i32;
        fn DeleteObject(object: Handle) -> i32;
    }

    struct OwnedCursor(Handle);
    impl Drop for OwnedCursor {
        fn drop(&mut self) {
            // SAFETY CONTRACT: CopyIcon (the CopyCursor macro implementation)
            // returned this unique non-null owned HCURSOR. It remains live until
            // this sole owner drops; no borrowed
            // GetCursorInfo handle is destroyed. No messages/callbacks installed.
            if !self.0.is_null() {
                unsafe {
                    DestroyCursor(self.0);
                }
            }
        }
    }
    struct OwnedBitmaps {
        mask: Handle,
        color: Handle,
    }
    impl Drop for OwnedBitmaps {
        fn drop(&mut self) {
            // SAFETY CONTRACT: GetIconInfo allocated these bitmaps for this
            // owner. They are never selected into any DC, exposed or retained.
            // Clean up on every return; guard aliasing even for malformed output.
            // DeleteObject does not dispatch application messages.
            unsafe {
                if !self.mask.is_null() {
                    DeleteObject(self.mask);
                }
                if !self.color.is_null() && self.color != self.mask {
                    DeleteObject(self.color);
                }
            }
        }
    }
    struct ScreenDc(Handle);
    impl Drop for ScreenDc {
        fn drop(&mut self) {
            // SAFETY CONTRACT: matched GetDC(NULL), same calling thread and
            // window argument; local owner cannot escape. No message pump needed.
            unsafe {
                ReleaseDC(null_mut(), self.0);
            }
        }
    }

    pub fn capture() -> Result<Option<CursorSnapshot>, SnapshotError> {
        let mut cursor = CursorInfo {
            size: size_of::<CursorInfo>() as u32,
            flags: 0,
            cursor: null_mut(),
            position: Point::default(),
        };
        // SAFETY CONTRACT: correct repr(C) writable CURSORINFO, cbSize set;
        // pointer is valid through this synchronous call. No application messages
        // or callbacks are relied upon, and no references to output are live.
        if unsafe { GetCursorInfo(&mut cursor) } == 0 {
            return Err(SnapshotError::Native("GetCursorInfo"));
        }
        if cursor.flags & 1 == 0 || cursor.flags & 2 != 0 {
            return Ok(None);
        }
        if cursor.cursor.is_null() {
            return Err(SnapshotError::Invalid("null visible cursor"));
        }
        // SAFETY CONTRACT: GetCursorInfo supplies a borrowed system HCURSOR.
        // CopyIcon accepts cursors and produces an independent owned handle;
        // failure/races are reported. Never destroy the system's borrowed handle.
        // This synchronous API does not require dispatching application messages.
        let icon = OwnedCursor(unsafe { CopyIcon(cursor.cursor) });
        if icon.0.is_null() {
            return Err(SnapshotError::Native("CopyCursor"));
        }
        let mut position = Point::default();
        // SAFETY CONTRACT: valid writable POINT; synchronous call, no callbacks
        // and no message handling. Physical pixels match the capture contract.
        if unsafe { GetPhysicalCursorPos(&mut position) } == 0 {
            return Err(SnapshotError::Native("GetPhysicalCursorPos"));
        }
        // The status/handle and physical position above are separate Win32
        // samples. Re-sample both before reading the shape so a hide, cursor
        // replacement, or observed movement cannot silently combine unrelated
        // state. This detects many races, but is not an atomic OS transaction.
        let mut confirm = CursorInfo {
            size: size_of::<CursorInfo>() as u32,
            flags: 0,
            cursor: null_mut(),
            position: Point::default(),
        };
        // SAFETY CONTRACT: same valid CURSORINFO contract as the first query.
        if unsafe { GetCursorInfo(&mut confirm) } == 0 {
            return Err(SnapshotError::Native("GetCursorInfo confirmation"));
        }
        if confirm.flags & 1 == 0
            || confirm.flags & 2 != 0
            || confirm.cursor.is_null()
            || confirm.cursor != cursor.cursor
        {
            return Err(SnapshotError::Invalid(
                "cursor status changed during sampling",
            ));
        }
        let mut confirmed_position = Point::default();
        // SAFETY CONTRACT: same valid writable POINT contract as above.
        if unsafe { GetPhysicalCursorPos(&mut confirmed_position) } == 0 {
            return Err(SnapshotError::Native("GetPhysicalCursorPos confirmation"));
        }
        if position.x != confirmed_position.x || position.y != confirmed_position.y {
            return Err(SnapshotError::Invalid(
                "cursor position changed during sampling",
            ));
        }
        let mut info = IconInfo {
            is_icon: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            mask: null_mut(),
            color: null_mut(),
        };
        // SAFETY CONTRACT: icon is the live owned CopyIcon result; correctly
        // sized writable ICONINFO remains valid. On success the documented
        // output HBITMAPs transfer to the cleanup guard.
        // Synchronous GetIconInfo needs no application message dispatch.
        let ok = unsafe { GetIconInfo(icon.0, &mut info) };
        if ok == 0 {
            return Err(SnapshotError::Native("GetIconInfo"));
        }
        let bitmaps = OwnedBitmaps {
            mask: info.mask,
            color: info.color,
        };
        if info.is_icon != 0 {
            return Err(SnapshotError::Unsupported("icon instead of cursor"));
        }
        if bitmaps.mask.is_null() {
            return Err(SnapshotError::Invalid("missing cursor mask"));
        }
        // SAFETY CONTRACT: NULL requests the screen DC; call and ReleaseDC
        // remain on this thread, no selections/messages/callbacks are introduced.
        let dc = ScreenDc(unsafe { GetDC(null_mut()) });
        if dc.0.is_null() {
            return Err(SnapshotError::Native("GetDC"));
        }
        let mask_meta = metadata(bitmaps.mask)?;
        if mask_meta.planes != 1 || mask_meta.bits_per_pixel != 1 {
            return Err(SnapshotError::Unsupported("non-monochrome AND mask"));
        }
        let (width, height, pixels) = if bitmaps.color.is_null() {
            if mask_meta.height <= 0 || mask_meta.height % 2 != 0 {
                return Err(SnapshotError::Invalid("monochrome double-height mask"));
            }
            let width = positive(mask_meta.width)?;
            let height = positive(mask_meta.height / 2)?;
            let count = pixel_count(width, height)?;
            // Monochrome ICONINFO stores the AND plane above the XOR plane.
            let packed = read_dib(&dc, bitmaps.mask, width, height * 2, 1)?;
            let stride = (width as usize).div_ceil(32) * 4;
            let split = stride * height as usize;
            let and_mask = unpack_mask(&packed[..split], width, height)?;
            let xor_mask = unpack_mask(&packed[split..], width, height)?;
            let mut xor_rgb = zeroed(count * 3)?;
            for (pixel, value) in xor_rgb.as_chunks_mut::<3>().0.iter_mut().zip(xor_mask) {
                pixel.fill(value);
            }
            (width, height, CursorPixels::Raster { and_mask, xor_rgb })
        } else {
            let meta = metadata(bitmaps.color)?;
            let width = positive(meta.width)?;
            let height = positive(meta.height)?;
            let count = pixel_count(width, height)?;
            if mask_meta.width != meta.width || mask_meta.height != meta.height || meta.planes != 1
            {
                return Err(SnapshotError::Unsupported("color/mask geometry or planes"));
            }
            // Only known BI_RGB-convertible depths; no packed 16-bit/bitfield
            // interpretation is guessed. GDI performs palette conversion.
            if !matches!(meta.bits_per_pixel, 1 | 4 | 8 | 24 | 32) {
                return Err(SnapshotError::Unsupported("color bitmap depth"));
            }
            // BI_RGB does not prove whether the high byte carries alpha. A
            // legacy XOR bitmap can look premultiplied, so do not guess.
            if meta.bits_per_pixel == 32 {
                return Err(SnapshotError::Unsupported("ambiguous 32-bit cursor bitmap"));
            }
            let bgra = read_dib(&dc, bitmaps.color, width, height, 32)?;
            let (bgra_pixels, bgra_remainder) = bgra.as_chunks::<4>();
            if !bgra_remainder.is_empty() {
                return Err(SnapshotError::Invalid("color DIB length"));
            }
            let packed = read_dib(&dc, bitmaps.mask, width, height, 1)?;
            let and_mask = unpack_mask(&packed, width, height)?;
            let mut xor_rgb = zeroed(count * 3)?;
            for (src, dst) in bgra_pixels
                .iter()
                .zip(xor_rgb.as_chunks_mut::<3>().0.iter_mut())
            {
                dst.copy_from_slice(&[src[2], src[1], src[0]]);
            }
            let pixels = CursorPixels::Raster { and_mask, xor_rgb };
            (width, height, pixels)
        };
        if info.hotspot_x >= width || info.hotspot_y >= height {
            return Err(SnapshotError::Invalid("hotspot outside cursor"));
        }
        Ok(Some(CursorSnapshot {
            width,
            height,
            screen_hotspot: (i64::from(position.x), i64::from(position.y)),
            hotspot: (info.hotspot_x, info.hotspot_y),
            pixels,
        }))
    }

    fn positive(value: i32) -> Result<u32, SnapshotError> {
        if value <= 0 {
            return Err(SnapshotError::Invalid("nonpositive bitmap dimension"));
        }
        Ok(value as u32)
    }

    fn metadata(bitmap: Handle) -> Result<Bitmap, SnapshotError> {
        // SAFETY CONTRACT: all-zero BITMAP fields are valid integers/raw pointers;
        // no Rust references, enums, destructors or invalid zero niches exist.
        let mut info: Bitmap = unsafe { zeroed_struct() };
        // SAFETY CONTRACT: live GetIconInfo-owned HBITMAP; writable repr(C)
        // BITMAP and exact byte size. Synchronous, no application messages.
        if unsafe {
            GetObjectW(
                bitmap,
                size_of::<Bitmap>() as i32,
                (&mut info as *mut Bitmap).cast(),
            )
        } != size_of::<Bitmap>() as i32
        {
            return Err(SnapshotError::Native("GetObjectW(BITMAP)"));
        }
        Ok(info)
    }

    fn read_dib(
        dc: &ScreenDc,
        bitmap: Handle,
        width: u32,
        height: u32,
        depth: u16,
    ) -> Result<Vec<u8>, SnapshotError> {
        // Double height is permitted only for the monochrome AND/XOR stack.
        if width == 0
            || width > MAX_CURSOR_SIDE
            || height == 0
            || height > MAX_CURSOR_SIDE * 2
            || !matches!(depth, 1 | 32)
            || (depth == 32 && height > MAX_CURSOR_SIDE)
        {
            return Err(SnapshotError::Unsupported("DIB dimensions or depth"));
        }
        let stride = (width as usize * depth as usize).div_ceil(32) * 4;
        let mut bytes = zeroed(
            stride
                .checked_mul(height as usize)
                .ok_or(SnapshotError::Allocation)?,
        )?;
        let mut info = DibInfo {
            header: DibHeader {
                size: size_of::<DibHeader>() as u32,
                width: width as i32,
                height: -(height as i32),
                planes: 1,
                bit_count: depth,
                compression: 0,
                size_image: bytes.len() as u32,
                x_ppm: 0,
                y_ppm: 0,
                colors_used: if depth == 1 { 2 } else { 0 },
                colors_important: 0,
            },
            colors: [0, 0x00ff_ffff],
        };
        // SAFETY CONTRACT: live DC and unselected GetIconInfo-owned bitmap.
        // A negative BI_RGB height requests top-down output. Bounded dimensions,
        // stride and initialized allocation cover every requested scanline;
        // BITMAPINFO has both RGBQUAD entries needed for 1-bit output. Pointers
        // remain exclusive/live until return. No app messages/callbacks needed.
        let read = unsafe {
            GetDIBits(
                dc.0,
                bitmap,
                0,
                height,
                bytes.as_mut_ptr().cast(),
                &mut info,
                0,
            )
        };
        if read != height as i32 {
            return Err(SnapshotError::Native("GetDIBits"));
        }
        // For the source monochrome bitmap, preserve bit meaning: a reversed
        // palette would reverse AND/XOR semantics. Reject unexpected conversion.
        if depth == 1 && info.colors != [0, 0x00ff_ffff] {
            return Err(SnapshotError::Unsupported(
                "unexpected monochrome DIB palette",
            ));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(mask: Vec<u8>, xor: Vec<u8>) -> CursorSnapshot {
        CursorSnapshot {
            width: mask.len() as u32,
            height: 1,
            screen_hotspot: (-1915, -20),
            hotspot: (3, 0),
            pixels: CursorPixels::Raster {
                and_mask: mask,
                xor_rgb: xor,
            },
        }
    }

    #[test]
    fn physical_negative_origin_and_hotspot() {
        let snapshot = raster(vec![0], vec![0; 3]);
        assert_eq!(snapshot.top_left_in_capture((-1920, -100)), Ok((2, 80)));
        assert_eq!(snapshot.top_left_in_capture((0, 0)), Ok((-1918, -20)));
    }

    #[test]
    fn coordinate_overflow_is_explicit() {
        let mut snapshot = raster(vec![0], vec![0; 3]);
        snapshot.screen_hotspot.0 = i64::MIN;
        assert_eq!(
            snapshot.top_left_in_capture((0, 0)),
            Err(SnapshotError::CoordinateOverflow)
        );
        snapshot.screen_hotspot.0 = i64::MAX;
        assert_eq!(
            snapshot.top_left_in_capture((-100, 0)),
            Err(SnapshotError::CoordinateOverflow)
        );
    }

    #[test]
    fn masks_use_msb_order_dword_padding_and_top_down_rows() {
        let bits = [0b1000_0001, 0b1000_0000, 0xff, 0xff, 0, 0, 0xff, 0xff];
        assert_eq!(
            unpack_mask(&bits, 9, 2).unwrap(),
            vec![255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert!(unpack_mask(&bits[..7], 9, 2).is_err());
    }

    #[test]
    fn monochrome_truth_table_preserves_inversion() {
        assert_eq!(
            raster(vec![0, 0, 255], vec![0, 0, 0, 255, 255, 255, 0, 0, 0])
                .to_rgba()
                .unwrap(),
            vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 0]
        );
        assert!(matches!(
            raster(vec![255], vec![255; 3]).to_rgba(),
            Err(SnapshotError::Unsupported(_))
        ));
        assert!(matches!(
            raster(vec![255], vec![12, 0, 0]).to_rgba(),
            Err(SnapshotError::Unsupported(_))
        ));
    }

    #[test]
    fn alpha_unpremultiplies_and_rejects_ambiguous_channels() {
        let mut snapshot = raster(vec![0], vec![0; 3]);
        snapshot.pixels = CursorPixels::PremultipliedRgba(vec![64, 32, 0, 128]);
        assert_eq!(snapshot.to_rgba().unwrap(), vec![128, 64, 0, 128]);
        snapshot.pixels = CursorPixels::PremultipliedRgba(vec![1, 0, 0, 0]);
        assert!(matches!(
            snapshot.to_rgba(),
            Err(SnapshotError::Unsupported(_))
        ));
    }

    #[test]
    fn owned_snapshot_survives_source_drop_and_clone_mutation() {
        let snapshot = {
            let mask = vec![0];
            raster(mask.clone(), vec![20, 30, 40])
        };
        let mut copy = snapshot.clone();
        if let CursorPixels::Raster { xor_rgb, .. } = &mut copy.pixels {
            xor_rgb[0] = 99;
        }
        assert_eq!(snapshot.to_rgba().unwrap(), vec![20, 30, 40, 255]);
    }

    #[test]
    fn invalid_geometry_and_lengths_are_rejected() {
        assert!(pixel_count(0, 1).is_err());
        assert!(pixel_count(MAX_CURSOR_SIDE + 1, 1).is_err());
        assert!(raster(vec![0], vec![]).to_rgba().is_err());
        assert!(raster(vec![7], vec![0; 3]).to_rgba().is_err());
    }
}
