use super::*;

// SAFETY CONTRACT:
// Voraussetzung: hdc und ausgewählter Pen sind gültig; Positionsänderung ist zulässig.
// Warum erfüllt bzw. offen: draw_icon übergibt einen geliehenen DC mit ausgewähltem Pen.
// Nachrichten: GDI-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn icon_line(hdc: Hwnd, x1: i32, y1: i32, x2: i32, y2: i32) {
    MoveToEx(hdc, x1, y1, null_mut());
    LineTo(hdc, x2, y2);
}

// SAFETY CONTRACT:
// Voraussetzung: hdc und temporäre GDI-Objekte bleiben während Auswahl und Wiederherstellung gültig.
// Warum erfüllt bzw. offen: Lokale Objekte werden zurückgesetzt und gelöscht; FFI-Fehler bei Auswahl sind nicht vollständig geprüft.
// Nachrichten: GDI-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn icon_fill_rect(hdc: Hwnd, rect: NativeRect, color: u32) {
    let brush = CreateSolidBrush(color);
    let old = SelectObject(hdc, brush);
    Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(hdc, old);
    DeleteObject(brush);
}

// SAFETY CONTRACT:
// Voraussetzung: hdc und temporäre GDI-Objekte bleiben während Auswahl und Wiederherstellung gültig.
// Warum erfüllt bzw. offen: Lokale Objekte werden zurückgesetzt und gelöscht; FFI-Fehler bei Auswahl sind nicht vollständig geprüft.
// Nachrichten: GDI-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn icon_fill_ellipse(hdc: Hwnd, rect: NativeRect, color: u32) {
    let brush = CreateSolidBrush(color);
    let old = SelectObject(hdc, brush);
    Ellipse(hdc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(hdc, old);
    DeleteObject(brush);
}

// SAFETY CONTRACT:
// Voraussetzung: hdc ist gültig und GDI-Objektauswahl wird nicht gleichzeitig geändert.
// Warum erfüllt bzw. offen: Pens werden lokal angelegt und zurückgesetzt; Auswahl- und Anlagefehler sind nicht vollständig geprüft.
// Nachrichten: GDI-/User32-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn draw_swatch(hdc: Hwnd, bounds: NativeRect, color: u32) {
    let left = bounds.left + ((bounds.right - bounds.left - 20) / 2);
    let top = bounds.top + ((bounds.bottom - bounds.top - 20) / 2);
    icon_fill_rect(
        hdc,
        NativeRect {
            left: left + 3,
            top: top + 3,
            right: left + 17,
            bottom: top + 17,
        },
        color,
    );
    let hc = high_contrast_enabled();
    let outer = if hc {
        GetSysColor(18)
    } else if color == 0 {
        0x00FFFFFF
    } else {
        0x00000000
    };
    let pen = CreatePen(0, 2, outer);
    let previous = SelectObject(hdc, pen);
    Rectangle(hdc, left + 2, top + 2, left + 18, top + 18);
    SelectObject(hdc, previous);
    DeleteObject(pen);
    if hc {
        let pen = CreatePen(0, 1, GetSysColor(15));
        let previous = SelectObject(hdc, pen);
        Rectangle(hdc, left + 4, top + 4, left + 16, top + 16);
        SelectObject(hdc, previous);
        DeleteObject(pen);
    }
}

// SAFETY CONTRACT:
// Voraussetzung: hdc ist gültig; pts lebt über den Aufruf und seine Länge passt in i32.
// Warum erfüllt bzw. offen: Aktuelle Aufrufer verwenden kleine lokale Arrays; Objektwiederherstellung bei FFI-Fehlern ist offen.
// Nachrichten: GDI-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn icon_polygon(hdc: Hwnd, pts: &[NativePoint], color: u32) {
    let brush = CreateSolidBrush(color);
    let old = SelectObject(hdc, brush);
    Polygon(hdc, pts.as_ptr(), pts.len() as i32);
    SelectObject(hdc, old);
    DeleteObject(brush);
}

// SAFETY CONTRACT:
// Voraussetzung: toolbar und zugehörige Kinder sind lebende HWNDs auf dem UI-Thread.
// Warum erfüllt bzw. offen: GetDlgItem liefert opake Handles, Nullwerte werden verworfen; Handle-Reuse bleibt Aufruferpflicht.
// Nachrichten: User32-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn invalidate_tools(toolbar: Hwnd) {
    for action in ACTIONS.iter().filter(|action| action.tool) {
        let button = GetDlgItem(toolbar, action.id as i32);
        if !button.is_null() {
            InvalidateRect(button, null(), 1);
        }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: SystemParametersInfoW erhält einen gültigen korrekt dimensionierten Ausgabepuffer.
// Warum erfüllt bzw. offen: HighContrast ist lokal vollständig initialisiert; der Ergebniszeiger wird nicht dereferenziert.
// Nachrichten: User32-Aufruf; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn high_contrast_enabled() -> bool {
    let mut settings = HighContrast {
        size: std::mem::size_of::<HighContrast>() as u32,
        flags: 0,
        default_scheme: null_mut(),
    };
    SystemParametersInfoW(
        0x0042,
        settings.size,
        &mut settings as *mut _ as *mut c_void,
        0,
    ) != 0
        && settings.flags & 1 != 0
}

/// Render the original 20-unit icon geometry at four times its output size,
/// then downsample it. The background is included so GDI can smooth the edges.
// SAFETY CONTRACT:
// Voraussetzung: hwnd und hdc gehören zum laufenden WM_DRAWITEM; bounds ist das
// zugehörige Rechteck und alle Win32-GDI-Handles bleiben bis zur Rückkehr gültig.
// Warum erfüllt: Der Toolbar-Callback übergibt seine aktuellen Parameter; alle
// temporären Objekte werden vor der Rückkehr zurückgesetzt und freigegeben.
// Nachrichten: GDI und GetDpiForWindow senden keine expliziten Fensternachrichten;
// draw_icon kann SystemParameter abfragen und hält keinen STATE-Borrow.
pub(super) unsafe fn draw_smoothed_icon(
    hwnd: Hwnd,
    hdc: Hwnd,
    bounds: NativeRect,
    icon: Icon,
    color: u32,
    current_color: u32,
    background: u32,
) {
    const LOGICAL: i32 = 20;
    const SAMPLE: i32 = LOGICAL * 4;
    let dpi = GetDpiForWindow(hwnd).clamp(96, 384) as i32;
    let available = (bounds.right - bounds.left).min(bounds.bottom - bounds.top) - 8;
    let size = (LOGICAL * dpi / 96).min(available).max(1);
    let left = bounds.left + (bounds.right - bounds.left - size) / 2;
    let top = bounds.top + (bounds.bottom - bounds.top - size) / 2;
    let mem = CreateCompatibleDC(hdc);
    if mem.is_null() {
        draw_icon(hdc, bounds, icon, color, current_color);
        return;
    }
    let bitmap = CreateCompatibleBitmap(hdc, SAMPLE, SAMPLE);
    if bitmap.is_null() {
        DeleteDC(mem);
        draw_icon(hdc, bounds, icon, color, current_color);
        return;
    }
    let old_bitmap = SelectObject(mem, bitmap);
    if old_bitmap.is_null() || old_bitmap as isize == -1 {
        DeleteObject(bitmap);
        DeleteDC(mem);
        draw_icon(hdc, bounds, icon, color, current_color);
        return;
    }
    let bg = CreateSolidBrush(background);
    let area = NativeRect {
        left: 0,
        top: 0,
        right: SAMPLE,
        bottom: SAMPLE,
    };
    let ready = !bg.is_null() && FillRect(mem, &area, bg) != 0
        && SetMapMode(mem, 8) != 0 // MM_ANISOTROPIC
        && SetWindowExtEx(mem, LOGICAL, LOGICAL, null_mut()) != 0
        && SetViewportExtEx(mem, SAMPLE, SAMPLE, null_mut()) != 0;
    if !bg.is_null() {
        DeleteObject(bg);
    }
    let mut drawn = false;
    if ready {
        let logical_bounds = NativeRect {
            left: 0,
            top: 0,
            right: LOGICAL,
            bottom: LOGICAL,
        };
        draw_icon(mem, logical_bounds, icon, color, current_color);
        // Restore device coordinates for the source bitmap before StretchBlt.
        let source_ready = SetMapMode(mem, 1) != 0; // MM_TEXT
        let old_mode = if source_ready {
            SetStretchBltMode(hdc, 4)
        } else {
            0
        }; // HALFTONE
        if old_mode != 0 {
            let mut old_origin = NativePoint { x: 0, y: 0 };
            let origin_set = SetBrushOrgEx(hdc, 0, 0, &mut old_origin) != 0;
            drawn = StretchBlt(
                hdc, left, top, size, size, mem, 0, 0, SAMPLE, SAMPLE, 0x00CC0020,
            ) != 0; // SRCCOPY
            if origin_set {
                SetBrushOrgEx(hdc, old_origin.x, old_origin.y, null_mut());
            }
            SetStretchBltMode(hdc, old_mode);
        }
    }
    SelectObject(mem, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(mem);
    if !drawn {
        draw_icon(hdc, bounds, icon, color, current_color);
    }
}

/// Draw the same small 20-unit geometry used by the C# toolbar.
// SAFETY CONTRACT:
// Voraussetzung: hdc und ausgewählte Objekte bleiben während der Zeichnung gültig.
// Warum erfüllt bzw. offen: DC stammt aus WM_DRAWITEM; Objektanlage und Wiederherstellung bei allen Fehlern sind nicht vollständig geprüft.
// Nachrichten: GDI-/User32-Aufrufe; allgemeine Reentranzfreiheit nicht belegt.
pub(super) unsafe fn draw_icon(
    hdc: Hwnd,
    bounds: NativeRect,
    icon: Icon,
    color: u32,
    current_color: u32,
) {
    let ox = bounds.left + ((bounds.right - bounds.left - 20) / 2);
    let oy = bounds.top + ((bounds.bottom - bounds.top - 20) / 2);
    let p = |x, y| NativePoint {
        x: ox + x,
        y: oy + y,
    };
    let rect = |x, y, r, b| NativeRect {
        left: ox + x,
        top: oy + y,
        right: ox + r,
        bottom: oy + b,
    };
    let pen = CreatePen(0, 2, color);
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(5)); // HOLLOW_BRUSH

    match icon {
        Icon::More => {
            for shift in [0, 7] {
                icon_line(hdc, ox + 5 + shift, oy + 5, ox + 10 + shift, oy + 10);
                icon_line(hdc, ox + 10 + shift, oy + 10, ox + 5 + shift, oy + 15);
            }
        }
        Icon::File => {
            Rectangle(hdc, ox + 3, oy + 3, ox + 17, oy + 18);
            for y in [7, 10, 13] {
                icon_line(hdc, ox + 6, oy + y, ox + 14, oy + y);
            }
        }
        Icon::View => {
            Rectangle(hdc, ox + 2, oy + 3, ox + 18, oy + 17);
            icon_line(hdc, ox + 10, oy + 6, ox + 10, oy + 14);
            icon_line(hdc, ox + 6, oy + 10, ox + 14, oy + 10);
        }
        Icon::Select => icon_polygon(
            hdc,
            &[
                p(4, 2),
                p(4, 16),
                p(8, 12),
                p(11, 18),
                p(14, 17),
                p(11, 11),
                p(17, 11),
            ],
            color,
        ),
        Icon::Arrow => {
            let accent = if high_contrast_enabled() {
                color
            } else {
                0x000000FF
            };
            let accent_pen = CreatePen(0, 2, accent);
            let previous = SelectObject(hdc, accent_pen);
            icon_line(hdc, ox + 4, oy + 16, ox + 12, oy + 8);
            icon_polygon(hdc, &[p(9, 4), p(18, 2), p(15, 11)], accent);
            SelectObject(hdc, previous);
            DeleteObject(accent_pen);
        }
        Icon::Line => icon_line(hdc, ox + 3, oy + 17, ox + 17, oy + 3),
        Icon::Rectangle => {
            Rectangle(hdc, ox + 3, oy + 5, ox + 17, oy + 15);
        }
        Icon::Ellipse => {
            Ellipse(hdc, ox + 3, oy + 4, ox + 18, oy + 16);
        }
        Icon::Text => {
            icon_line(hdc, ox + 4, oy + 4, ox + 16, oy + 4);
            icon_line(hdc, ox + 10, oy + 4, ox + 10, oy + 17);
        }
        Icon::Marker => {
            let (fill, ink) = if high_contrast_enabled() {
                (GetSysColor(13), GetSysColor(14))
            } else {
                (0x0000D6FF, 0x00000000)
            }; // yellow marker, black edge
            let marker_pen = CreatePen(0, 2, ink);
            let previous = SelectObject(hdc, marker_pen);
            icon_polygon(hdc, &[p(3, 14), p(12, 4), p(17, 9), p(8, 18)], fill);
            icon_line(hdc, ox + 2, oy + 18, ox + 8, oy + 18);
            SelectObject(hdc, previous);
            DeleteObject(marker_pen);
        }
        Icon::Pixelate => {
            for y in 0..3 {
                for x in 0..3 {
                    let shade = if (x + y) % 2 == 0 {
                        GetSysColor(18)
                    } else {
                        GetSysColor(15)
                    };
                    icon_fill_rect(hdc, rect(2 + x * 5, 2 + y * 5, 7 + x * 5, 7 + y * 5), shade);
                }
            }
        }
        Icon::GaussianBlur => {
            // Concentric soft bands distinguish blur from the pixel grid.
            let high_contrast = high_contrast_enabled();
            for (inset, gray) in [(1, 210u32), (4, 155), (7, 100)] {
                let color = if high_contrast {
                    GetSysColor(18)
                } else {
                    gray | (gray << 8) | (gray << 16)
                };
                icon_fill_ellipse(hdc, rect(inset, inset, 20 - inset, 20 - inset), color);
            }
        }
        Icon::Redact => {
            icon_fill_rect(hdc, rect(2, 5, 18, 15), 0x00000000);
        }
        Icon::Step => {
            let hc = high_contrast_enabled();
            let circle = if hc { GetSysColor(13) } else { 0x000000FF };
            let numeral = if hc { GetSysColor(14) } else { 0x00FFFFFF };
            icon_fill_ellipse(hdc, rect(2, 2, 18, 18), circle);
            let number_pen = CreatePen(0, 2, numeral);
            let previous = SelectObject(hdc, number_pen);
            icon_line(hdc, ox + 8, oy + 7, ox + 10, oy + 5);
            icon_line(hdc, ox + 10, oy + 5, ox + 10, oy + 14);
            icon_line(hdc, ox + 7, oy + 14, ox + 13, oy + 14);
            SelectObject(hdc, previous);
            DeleteObject(number_pen);
        }
        Icon::Pen => {
            let curve = [p(3, 15), p(6, 3), p(12, 20), p(17, 5)];
            PolyBezier(hdc, curve.as_ptr(), curve.len() as u32);
        }
        Icon::Crop => {
            icon_line(hdc, ox + 6, oy + 2, ox + 6, oy + 14);
            icon_line(hdc, ox + 6, oy + 14, ox + 18, oy + 14);
            icon_line(hdc, ox + 2, oy + 6, ox + 14, oy + 6);
            icon_line(hdc, ox + 14, oy + 6, ox + 14, oy + 18);
        }
        Icon::Color => {
            draw_swatch(hdc, bounds, current_color);
        }
        Icon::SwatchRed => draw_swatch(hdc, bounds, 0x002828E6), // RGB 230, 40, 40
        Icon::SwatchBlack => draw_swatch(hdc, bounds, 0x00000000),
        Icon::SwatchYellow => draw_swatch(hdc, bounds, 0x0000D6FF), // RGB 255, 214, 0
        Icon::SwatchBlue => draw_swatch(hdc, bounds, 0x00D77800),   // RGB 0, 120, 215
        Icon::LineWidth => {
            icon_line(hdc, ox + 3, oy + 5, ox + 17, oy + 5);
            icon_line(hdc, ox + 3, oy + 10, ox + 17, oy + 10);
            icon_line(hdc, ox + 3, oy + 15, ox + 17, oy + 15);
        }
        Icon::FontSize => {
            icon_line(hdc, ox + 3, oy + 16, ox + 9, oy + 4);
            icon_line(hdc, ox + 9, oy + 4, ox + 15, oy + 16);
            icon_line(hdc, ox + 6, oy + 11, ox + 12, oy + 11);
        }
        Icon::Frame => {
            Rectangle(hdc, ox + 2, oy + 3, ox + 15, oy + 16);
            Rectangle(hdc, ox + 6, oy + 6, ox + 18, oy + 18);
        }
        Icon::Capture => {
            Rectangle(hdc, ox + 2, oy + 6, ox + 18, oy + 17);
            icon_line(hdc, ox + 6, oy + 6, ox + 8, oy + 3);
            icon_line(hdc, ox + 8, oy + 3, ox + 13, oy + 3);
            icon_line(hdc, ox + 13, oy + 3, ox + 15, oy + 6);
            Ellipse(hdc, ox + 7, oy + 8, ox + 14, oy + 15);
        }
        Icon::Copy => {
            Rectangle(hdc, ox + 3, oy + 3, ox + 12, oy + 13);
            Rectangle(hdc, ox + 8, oy + 8, ox + 17, oy + 18);
        }
        Icon::Save => {
            Rectangle(hdc, ox + 3, oy + 2, ox + 17, oy + 18);
            Rectangle(hdc, ox + 6, oy + 3, ox + 14, oy + 8);
            Rectangle(hdc, ox + 6, oy + 12, ox + 14, oy + 18);
            icon_line(hdc, ox + 8, oy + 4, ox + 8, oy + 7);
        }
        Icon::Undo | Icon::Redo => {
            let mirror = matches!(icon, Icon::Redo);
            let x = |value| ox + if mirror { 20 - value } else { value };
            let curve = [p(15, 15), p(17, 6), p(9, 5), p(5, 8)];
            let curve: Vec<_> = curve
                .iter()
                .map(|pt| NativePoint {
                    x: if mirror { ox + 20 - (pt.x - ox) } else { pt.x },
                    y: pt.y,
                })
                .collect();
            PolyBezier(hdc, curve.as_ptr(), curve.len() as u32);
            icon_polygon(
                hdc,
                &[
                    NativePoint { x: x(3), y: oy + 8 },
                    NativePoint { x: x(9), y: oy + 4 },
                    NativePoint {
                        x: x(9),
                        y: oy + 11,
                    },
                ],
                color,
            );
        }
    }

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    DeleteObject(pen);
}
