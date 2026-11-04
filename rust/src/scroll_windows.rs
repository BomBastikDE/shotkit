//! Win32 implementation of the platform-independent scroll-capture adapter.
//!
//! Construct with a screen-space selection rectangle. The adapter owns the
//! cursor for its lifetime and restores its original position on drop.

use std::{ffi::c_void, mem::size_of, ptr::null_mut, thread, time::Duration};

use shotkit::{
    scroll_capture::{Direction, ScrollIo},
    stitch::Image,
};

type H = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    dx: i32,
    dy: i32,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

// MOUSEINPUT is the largest member of the Win32 INPUT union.
#[repr(C)]
struct Input {
    kind: u32,
    mouse: MouseInput,
}

#[repr(C)]
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

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: H) -> H;
    fn ReleaseDC(hwnd: H, dc: H) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetForegroundWindow() -> H;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: H) -> H;
    fn CreateCompatibleBitmap(dc: H, width: i32, height: i32) -> H;
    fn SelectObject(dc: H, object: H) -> H;
    fn DeleteDC(dc: H) -> i32;
    fn DeleteObject(object: H) -> i32;
    fn BitBlt(
        dst: H,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        src: H,
        sx: i32,
        sy: i32,
        rop: u32,
    ) -> i32;
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

const SRCCOPY: u32 = 0x00CC0020;
const CAPTUREBLT: u32 = 0x40000000;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_HWHEEL: u32 = 0x01000;
const VK_ESCAPE: i32 = 0x1B;
const BI_RGB: u32 = 0;

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
struct MemoryDc(H);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DeleteDC(self.0);
            }
        }
    }
}
struct GdiBitmap(H);
impl Drop for GdiBitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

/// System adapter for `scroll_capture::ScrollIo`.
///
/// `rect` is in virtual-desktop screen coordinates. Non-positive dimensions,
/// coordinates outside Win32's signed range, and allocations above the image
/// module's 512 MiB limit are rejected before capture.
pub struct WindowsScrollIo {
    rect: (i32, i32, i32, i32),
    target_window: usize,
    original_cursor: Point,
    scroll_cursor: Point,
    cursor_moved: bool,
    escape_seen: bool,
}

impl WindowsScrollIo {
    /// Create an adapter for `(x, y, width, height)` screen coordinates.
    pub fn new(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        target_window: usize,
    ) -> Result<Self, String> {
        if width <= 0 || height <= 0 {
            return Err("Aufnahmebereich muss positive Breite und Höhe haben.".into());
        }
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or("Bildgröße überläuft.")?;
        if pixels > 512 * 1024 * 1024 || pixels > u32::MAX as usize {
            return Err("Aufnahmebereich überschreitet die zulässige Bildgröße.".into());
        }
        x.checked_add(width)
            .ok_or("Aufnahmebereich überschreitet Koordinatenbereich.")?;
        y.checked_add(height)
            .ok_or("Aufnahmebereich überschreitet Koordinatenbereich.")?;
        let mut original_cursor = Point::default();
        if unsafe { GetCursorPos(&mut original_cursor) } == 0 {
            return Err("Mausposition konnte nicht gelesen werden.".into());
        }
        // Initially the expected position is where the user left the pointer;
        // scroll() moves it to the direction-specific edge before sending input.
        let scroll_cursor = original_cursor;
        Ok(Self {
            rect: (x, y, width, height),
            target_window,
            original_cursor,
            scroll_cursor,
            cursor_moved: false,
            escape_seen: false,
        })
    }

    fn current_cursor(&mut self) -> Option<Point> {
        let mut p = Point::default();
        if unsafe { GetCursorPos(&mut p) } == 0 {
            return None;
        }
        if p.x != self.scroll_cursor.x || p.y != self.scroll_cursor.y {
            self.cursor_moved = true;
        }
        Some(p)
    }
}

impl ScrollIo for WindowsScrollIo {
    fn capture(&mut self) -> Result<Image, String> {
        let (x, y, width, height) = self.rect;
        let screen = ScreenDc(unsafe { GetDC(null_mut()) });
        if screen.0.is_null() {
            return Err("Bildschirm-DC konnte nicht erstellt werden.".into());
        }
        let memory = MemoryDc(unsafe { CreateCompatibleDC(screen.0) });
        if memory.0.is_null() {
            return Err("Speicher-DC konnte nicht erstellt werden.".into());
        }
        let bitmap = GdiBitmap(unsafe { CreateCompatibleBitmap(screen.0, width, height) });
        if bitmap.0.is_null() {
            return Err("Aufnahme-Bitmap konnte nicht erstellt werden.".into());
        }

        let old = unsafe { SelectObject(memory.0, bitmap.0) };
        if old.is_null() || old as isize == -1 {
            return Err("Aufnahme-Bitmap konnte nicht ausgewählt werden.".into());
        }
        let copied = unsafe {
            BitBlt(
                memory.0,
                0,
                0,
                width,
                height,
                screen.0,
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            )
        };
        // Deselect before GetDIBits, as required by GDI; RAII then releases all
        // handles even if a later operation reports an error.
        unsafe {
            SelectObject(memory.0, old);
        }
        if copied == 0 {
            return Err("Bildschirmbereich konnte nicht kopiert werden.".into());
        }

        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or("Bildgröße überläuft.")?;
        let mut pixels = vec![0u8; len];
        let mut info = BitmapInfo {
            header: BitmapInfoHeader {
                size: size_of::<BitmapInfoHeader>() as u32,
                width,
                height: -height,
                planes: 1,
                bit_count: 32,
                compression: BI_RGB,
                size_image: len as u32,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        let rows = unsafe {
            GetDIBits(
                screen.0,
                bitmap.0,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                &mut info as *mut _ as *mut c_void,
                0,
            )
        };
        if rows != height {
            return Err(format!(
                "Bilddaten konnten nicht vollständig gelesen werden ({rows}/{height} Zeilen)."
            ));
        }
        // Screen captures are opaque; GDI does not promise a useful alpha byte.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Image::new(width as usize, height as usize, pixels).map_err(str::to_owned)
    }

    fn scroll(&mut self, direction: Direction, notches: i32) -> Result<(), String> {
        if unsafe { GetForegroundWindow() } as usize != self.target_window {
            return Err("Zielfenster ist nicht mehr aktiv; Scroll-Aufnahme beendet.".into());
        }
        let (x, y, width, height) = self.rect;
        self.scroll_cursor = match direction {
            Direction::Down => Point {
                x: x + width - 8.min(width / 4),
                y: y + height / 2,
            },
            Direction::Right | Direction::Left => Point {
                x: x + width / 2,
                y: y + height - 8.min(height / 4),
            },
        };
        if unsafe { SetCursorPos(self.scroll_cursor.x, self.scroll_cursor.y) } == 0 {
            return Err("Mauszeiger konnte nicht im Aufnahmebereich positioniert werden.".into());
        }
        let input = wheel_input(direction, notches);
        // SAFETY: input has Win32 INPUT layout and lives until SendInput returns.
        // SendInput may deliver messages; no reference to global state is held here.
        let inserted = unsafe { SendInput(1, &input, size_of::<Input>() as i32) };
        check_input_result(inserted)
    }

    fn wait_ms(&mut self, millis: u64) {
        let end = std::time::Instant::now() + Duration::from_millis(millis);
        while std::time::Instant::now() < end {
            self.cancelled();
            self.mouse_moved();
            thread::sleep(
                Duration::from_millis(10)
                    .min(end.saturating_duration_since(std::time::Instant::now())),
            );
        }
    }

    fn cancelled(&mut self) -> bool {
        if unsafe { GetAsyncKeyState(VK_ESCAPE) } < 0 {
            self.escape_seen = true;
        }
        self.escape_seen
    }

    fn mouse_moved(&mut self) -> bool {
        self.current_cursor();
        self.cursor_moved
    }

    fn progress(&mut self, _length: usize) {}
}

fn wheel_input(direction: Direction, notches: i32) -> Input {
    let (flags, sign) = match direction {
        Direction::Down => (MOUSEEVENTF_WHEEL, -1i64),
        Direction::Right => (MOUSEEVENTF_HWHEEL, 1i64),
        Direction::Left => (MOUSEEVENTF_HWHEEL, -1i64),
    };
    let delta = i64::from(notches).saturating_mul(120).saturating_mul(sign);
    Input {
        kind: 0, // INPUT_MOUSE
        mouse: MouseInput {
            dx: 0,
            dy: 0,
            mouse_data: delta as i32 as u32,
            flags,
            time: 0,
            extra_info: 0,
        },
    }
}

fn check_input_result(inserted: u32) -> Result<(), String> {
    if inserted == 1 {
        Ok(())
    } else {
        Err("Scroll-Eingabe konnte nicht an Windows übergeben werden.".into())
    }
}

impl Drop for WindowsScrollIo {
    fn drop(&mut self) {
        unsafe {
            SetCursorPos(self.original_cursor.x, self.original_cursor.y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_input_has_expected_direction_and_reports_rejection() {
        for (direction, flags, delta) in [
            (Direction::Down, MOUSEEVENTF_WHEEL, -240i32),
            (Direction::Right, MOUSEEVENTF_HWHEEL, 240),
            (Direction::Left, MOUSEEVENTF_HWHEEL, -240),
        ] {
            let input = wheel_input(direction, 2);
            assert_eq!(input.kind, 0);
            assert_eq!(input.mouse.flags, flags);
            assert_eq!(input.mouse.mouse_data as i32, delta);
        }
        assert!(check_input_result(1).is_ok());
        assert!(check_input_result(0).is_err());
    }

    #[test]
    fn input_layout_matches_win32_abi() {
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<MouseInput>(), 32);
            assert_eq!(size_of::<Input>(), 40);
        }
        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<MouseInput>(), 24);
            assert_eq!(size_of::<Input>(), 28);
        }
    }
}
