use super::*;

pub(super) unsafe fn capture_desktop(origin: (i32, i32), size: (i32, i32)) -> (H, H, H) {
    let estimated_bytes = i64::from(size.0)
        .checked_mul(i64::from(size.1))
        .and_then(|pixels| pixels.checked_mul(4));
    if estimated_bytes.map_or(true, |bytes| bytes > MAX_MAGNIFIER_SNAPSHOT_BYTES) {
        return (null_mut(), null_mut(), null_mut());
    }
    let screen_dc = GetDC(null_mut());
    if screen_dc.is_null() {
        return (null_mut(), null_mut(), null_mut());
    }
    let memory_dc = CreateCompatibleDC(screen_dc);
    if memory_dc.is_null() {
        ReleaseDC(null_mut(), screen_dc);
        return (null_mut(), null_mut(), null_mut());
    }
    let bitmap = CreateCompatibleBitmap(screen_dc, size.0, size.1);
    if bitmap.is_null() {
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);
        return (null_mut(), null_mut(), null_mut());
    }
    let original = SelectObject(memory_dc, bitmap);
    if original.is_null()
        || original as isize == -1
        || BitBlt(
            memory_dc,
            0,
            0,
            size.0,
            size.1,
            screen_dc,
            origin.0,
            origin.1,
            SRCCOPY | CAPTUREBLT,
        ) == 0
    {
        if !original.is_null() && original as isize != -1 {
            SelectObject(memory_dc, original);
        }
        DeleteObject(bitmap);
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);
        return (null_mut(), null_mut(), null_mut());
    }
    ReleaseDC(null_mut(), screen_dc);
    (memory_dc, bitmap, original)
}

pub(super) unsafe fn release_desktop(state: *mut State) {
    let s = &mut *state;
    if !s.source_dc.is_null() {
        if !s.source_original.is_null() {
            SelectObject(s.source_dc, s.source_original);
        }
        if !s.source_bitmap.is_null() {
            DeleteObject(s.source_bitmap);
        }
        DeleteDC(s.source_dc);
    }
    s.source_dc = null_mut();
    s.source_bitmap = null_mut();
    s.source_original = null_mut();
}

pub(super) unsafe fn monitor_local_rect(point: Point, origin: (i32, i32), desktop: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
    if !monitor.is_null() {
        let mut info: MonitorInfo = zeroed();
        info.size = size_of::<MonitorInfo>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            return (
                info.monitor.left.saturating_sub(origin.0).max(desktop.0),
                info.monitor.top.saturating_sub(origin.1).max(desktop.1),
                info.monitor.right.saturating_sub(origin.0).min(desktop.2),
                info.monitor.bottom.saturating_sub(origin.1).min(desktop.3),
            );
        }
    }
    desktop
}

pub(super) unsafe fn magnifier_rect(state: &State, point: Point) -> (i32, i32, i32, i32) {
    let desktop = desktop_rect(state);
    let local_cursor = Point {
        x: point.x.saturating_sub(state.origin.0),
        y: point.y.saturating_sub(state.origin.1),
    };
    let bounds = monitor_local_rect(point, state.origin, desktop);
    let mut x = local_cursor.x.saturating_add(24);
    let mut y = local_cursor.y.saturating_add(24);
    if x.saturating_add(MAG_SIZE) > bounds.2 {
        x = local_cursor.x.saturating_sub(24 + MAG_SIZE);
    }
    if y.saturating_add(MAG_SIZE + MAG_LABEL_HEIGHT + 2) > bounds.3 {
        y = local_cursor.y.saturating_sub(24 + MAG_SIZE + MAG_LABEL_HEIGHT + 2);
    }
    let max_x = bounds.2.saturating_sub(MAG_SIZE).max(bounds.0);
    let max_y = bounds.3.saturating_sub(MAG_SIZE + MAG_LABEL_HEIGHT + 2).max(bounds.1);
    x = x.clamp(bounds.0, max_x);
    y = y.clamp(bounds.1, max_y);
    (x, y, x.saturating_add(MAG_SIZE), y.saturating_add(MAG_SIZE))
}

pub(super) fn magnifier_source_rect(state: &State, point: Point) -> (i32, i32, i32, i32) {
    let local_x = point.x.saturating_sub(state.origin.0);
    let local_y = point.y.saturating_sub(state.origin.1);
    let width = state.size.0.min(MAG_PIXELS).max(1);
    let height = state.size.1.min(MAG_PIXELS).max(1);
    let max_x = state.size.0.saturating_sub(width).max(0);
    let max_y = state.size.1.saturating_sub(height).max(0);
    let left = local_x.saturating_sub(width / 2).clamp(0, max_x);
    let top = local_y.saturating_sub(height / 2).clamp(0, max_y);
    (left, top, width, height)
}

pub(super) unsafe fn draw_magnifier(dc: H, state: &State) {
    if state.source_dc.is_null() {
        return;
    }
    let point = state.current;
    let local_cursor = Point {
        x: point.x.saturating_sub(state.origin.0),
        y: point.y.saturating_sub(state.origin.1),
    };
    let (source_x, source_y, source_width, source_height) = magnifier_source_rect(state, point);
    let left = 0;
    let top = 0;
    let right = MAG_SIZE;
    let bottom = MAG_SIZE;
    let _ = FillRect(dc, &Rect { left, top, right, bottom }, GetStockObject(4)); // BLACK_BRUSH
    SetStretchBltMode(dc, COLORONCOLOR);
    StretchBlt(
        dc,
        left,
        top,
        MAG_SIZE,
        MAG_SIZE,
        state.source_dc,
        source_x,
        source_y,
        source_width,
        source_height,
        SRCCOPY,
    );

    let white_pen = CreatePen(PS_SOLID, 1, 0x00ff_ffff);
    let old_pen = if white_pen.is_null() { null_mut() } else { SelectObject(dc, white_pen) };
    let marker_x = local_cursor.x.saturating_sub(source_x).clamp(0, source_width - 1);
    let marker_y = local_cursor.y.saturating_sub(source_y).clamp(0, source_height - 1);
    let center_x = marker_x * MAG_SIZE / source_width;
    let center_y = marker_y * MAG_SIZE / source_height;
    let marker_width = (MAG_SIZE / source_width).max(1);
    let marker_height = (MAG_SIZE / source_height).max(1);
    Rectangle(dc, left + center_x, top + center_y, left + center_x + marker_width + 1, top + center_y + marker_height + 1);
    let accent_pen = CreatePen(PS_SOLID, 2, 0x00ff_9600);
    if !accent_pen.is_null() {
        let previous_pen = SelectObject(dc, accent_pen);
        Rectangle(dc, left, top, right, bottom);
        if !previous_pen.is_null() && previous_pen as isize != -1 {
            SelectObject(dc, previous_pen);
        }
        DeleteObject(accent_pen);
    }
    if !old_pen.is_null() && old_pen as isize != -1 {
        SelectObject(dc, old_pen);
    }
    if !white_pen.is_null() {
        DeleteObject(white_pen);
    }

    let pixel = GetPixel(state.source_dc, local_cursor.x, local_cursor.y);
    let label = if pixel == 0xffff_ffff {
        format!("{}, {}", point.x, point.y)
    } else {
        format!("{}, {}  #{:02X}{:02X}{:02X}", point.x, point.y, pixel & 0xff, (pixel >> 8) & 0xff, (pixel >> 16) & 0xff)
    };
    let wide: Vec<u16> = label.encode_utf16().collect();
    let text_rect = Rect { left, top: MAG_SIZE + 2, right: MAG_SIZE, bottom: MAG_WINDOW_HEIGHT };
    let dark = CreateSolidBrush(0x0014_1414);
    if !dark.is_null() {
        FillRect(dc, &text_rect, dark);
        DeleteObject(dark);
    }
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let old_font = if !font.is_null() { SelectObject(dc, font) } else { null_mut() };
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, 0x00ff_ffff);
    TextOutW(dc, 3, MAG_SIZE + 4, wide.as_ptr(), wide.len() as i32);
    if !old_font.is_null() { SelectObject(dc, old_font); }
}

pub(super) unsafe fn move_magnifier(state: *mut State) {
    // Copy everything needed before MoveWindow, which can synchronously call
    // the window procedure again; no reference into STATE crosses that call.
    let (hwnd, left, top) = {
        let s = &*state;
        if s.magnifier_hwnd.is_null() {
            return;
        }
        let (left, top, _, _) = magnifier_rect(s, s.current);
        (s.magnifier_hwnd, left.saturating_add(s.origin.0), top.saturating_add(s.origin.1))
    };
    MoveWindow(hwnd, left, top, MAG_SIZE, MAG_WINDOW_HEIGHT, 0);
    InvalidateRect(hwnd, null(), 0);
}

// SAFETY CONTRACT: Windows invokes this on the owning UI thread; WM_PAINT
// pairs BeginPaint and EndPaint and the overlay remains alive for the callback.
// Callback code must not unwind across the system ABI.
pub(super) unsafe extern "system" fn magnifier_proc(hwnd: H, msg: u32, w: W, l: L) -> L {
    match msg {
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => HTTRANSPARENT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE,
        WM_PAINT => {
            let mut paint: Paint = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            draw_magnifier(dc, &*std::ptr::addr_of!(STATE));
            EndPaint(hwnd, &paint);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
