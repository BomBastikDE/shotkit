//! Native canvas dialog. The dialog manager owns keyboard navigation and the
//! modal loop; its stack-owned state stays alive until DialogBox returns.
use super::*;
use shotkit::{document::CanvasAlignment, shapes::Color};

#[link(name = "user32")]
extern "system" {
    fn DialogBoxIndirectParamW(
        instance: H,
        template: *const u16,
        owner: H,
        proc: Option<unsafe extern "system" fn(H, u32, W, L) -> L>,
        parameter: L,
    ) -> L;
    fn SendMessageW(window: H, message: u32, wparam: W, lparam: L) -> L;
    fn EndDialog(dialog: H, result: L) -> i32;
    fn GetDlgItem(dialog: H, id: i32) -> H;
    fn SetDlgItemTextW(dialog: H, id: i32, text: *const u16) -> i32;
    fn GetDlgItemTextW(dialog: H, id: i32, text: *mut u16, capacity: i32) -> u32;
}

struct Options {
    width: usize,
    height: usize,
    horizontal: CanvasAlignment,
    vertical: CanvasAlignment,
    fill: Color,
}
struct DialogState {
    width: usize,
    height: usize,
    result: Option<Options>,
}

// Templates are WORD streams with DWORD-aligned item headers. A Vec<u32>
// provides the native template alignment without casting an unaligned buffer.
fn template() -> Vec<u32> {
    fn word(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_le_bytes());
    }
    fn dword(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }
    fn string(bytes: &mut Vec<u8>, value: &str) {
        for c in value.encode_utf16().chain(Some(0)) {
            word(bytes, c);
        }
    }
    let mut b = Vec::new();
    dword(&mut b, 0x80c8_08c0); // popup, caption, system menu, modal frame, font
    dword(&mut b, 0);
    word(&mut b, 16);
    for n in [0, 0, 285, 220] {
        word(&mut b, n);
    }
    word(&mut b, 0);
    word(&mut b, 0);
    string(&mut b, "Leinwandgröße");
    word(&mut b, 9);
    string(&mut b, "Segoe UI");
    let controls = [
        (0x82, 0, "&Breite (px):", 12, 14, 72, 12, 0),
        (0x81, 101, "", 88, 10, 72, 16, 0x0081_0000),
        (0x82, 0, "&Höhe (px):", 12, 38, 72, 12, 0),
        (0x81, 102, "", 88, 34, 72, 16, 0x0081_0000),
        (0x82, 0, "H&orizontal:", 12, 62, 72, 12, 0),
        (0x85, 103, "", 88, 58, 176, 80, 0x0021_0003),
        (0x82, 0, "&Vertikal:", 12, 86, 72, 12, 0),
        (0x85, 104, "", 88, 82, 176, 80, 0x0021_0003),
        (0x82, 0, "Neue &Fläche:", 12, 110, 72, 12, 0),
        (0x85, 105, "", 88, 106, 176, 65, 0x0021_0003),
        (0x82, 0, "Farbe (#RRGG&BB):", 12, 134, 90, 12, 0),
        (0x81, 106, "#FFFFFF", 108, 130, 90, 16, 0x0081_0000),
        (
            0x82,
            0,
            "1–30.000 px je Achse, maximal 512 MiB Bilddaten.",
            12,
            153,
            260,
            14,
            0,
        ),
        (
            0x82,
            0,
            "Bild bleibt unskaliert. Kleinere Leinwand schneidet ab.",
            12,
            168,
            260,
            14,
            0,
        ),
        (0x80, 1, "Übernehmen", 112, 193, 78, 18, 0x0001_0001),
        (0x80, 2, "Abbrechen", 196, 193, 68, 18, 0x0001_0000),
    ];
    for (class, id, text, x, y, width, height, style) in controls {
        while b.len() % 4 != 0 {
            b.push(0);
        }
        dword(&mut b, 0x5000_0000 | style);
        dword(&mut b, 0);
        for n in [x, y, width, height, id] {
            word(&mut b, n);
        }
        word(&mut b, 0xffff);
        word(&mut b, class);
        string(&mut b, text);
        word(&mut b, 0);
    }
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|v| u32::from_le_bytes(*v))
        .collect()
}

fn parse_size(width: &str, height: &str) -> Result<(usize, usize), &'static str> {
    let error = "Bitte ganze Zahlen von 1 bis 30.000 eingeben; maximal 512 MiB Bilddaten.";
    let w = width.trim().parse::<usize>().map_err(|_| error)?;
    let h = height.trim().parse::<usize>().map_err(|_| error)?;
    if !(1..=30_000).contains(&w)
        || !(1..=30_000).contains(&h)
        || w.checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .is_none_or(|n| n > 512 * 1024 * 1024)
    {
        return Err(error);
    }
    Ok((w, h))
}
fn parse_color(text: &str) -> Result<Color, &'static str> {
    let s = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if s.len() != 6 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Bitte eine deckende Farbe als #RRGGBB eingeben, z. B. #FFFFFF.");
    }
    let value = u32::from_str_radix(s, 16).map_err(|_| "Ungültige Farbe.")?;
    Ok(Color::rgba(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
        255,
    ))
}
unsafe fn text(dialog: H, id: i32) -> String {
    let mut b = [0u16; 64];
    let len = GetDlgItemTextW(dialog, id, b.as_mut_ptr(), b.len() as i32) as usize;
    String::from_utf16_lossy(&b[..len])
}
unsafe fn alignment(dialog: H, id: i32) -> Result<CanvasAlignment, &'static str> {
    match SendMessageW(GetDlgItem(dialog, id), 0x147, 0, 0) {
        0 => Ok(CanvasAlignment::Start),
        1 => Ok(CanvasAlignment::Center),
        2 => Ok(CanvasAlignment::End),
        _ => Err("Bitte eine Ausrichtung wählen."),
    }
}
// SAFETY: the modal dialog retains only the pointer passed by prompt; prompt's
// stack frame and template outlive all callbacks. No editor borrow enters it.
unsafe extern "system" fn dialog_proc(h: H, m: u32, w: W, l: L) -> L {
    match m {
        0x110 => {
            SetWindowLongPtrW(h, -21, l);
            let state = l as *mut DialogState;
            SetDlgItemTextW(h, 101, wide(&(*state).width.to_string()).as_ptr());
            SetDlgItemTextW(h, 102, wide(&(*state).height.to_string()).as_ptr());
            for (id, items) in [
                (103, &["Links", "Mitte", "Rechts"][..]),
                (104, &["Oben", "Mitte", "Unten"][..]),
                (105, &["Transparent", "Deckende Farbe"][..]),
            ] {
                for item in items {
                    SendMessageW(GetDlgItem(h, id), 0x143, 0, wide(item).as_ptr() as L);
                }
                SendMessageW(GetDlgItem(h, id), 0x14e, if id == 105 { 0 } else { 1 }, 0);
            }
            1
        }
        0x111 if w & 0xffff == 2 => {
            EndDialog(h, 0);
            1
        }
        0x10 => {
            EndDialog(h, 0);
            1
        }
        0x111 if w & 0xffff == 1 => {
            let result = (|| {
                let (width, height) = parse_size(&text(h, 101), &text(h, 102))?;
                let fill = match SendMessageW(GetDlgItem(h, 105), 0x147, 0, 0) {
                    0 => Color::rgba(0, 0, 0, 0),
                    1 => parse_color(&text(h, 106))?,
                    _ => return Err("Bitte die neue Fläche wählen."),
                };
                Ok(Options {
                    width,
                    height,
                    horizontal: alignment(h, 103)?,
                    vertical: alignment(h, 104)?,
                    fill,
                })
            })();
            match result {
                Ok(options) => {
                    let state = GetWindowLongPtrW(h, -21) as *mut DialogState;
                    if !state.is_null() {
                        (*state).result = Some(options);
                        EndDialog(h, 1);
                    }
                }
                Err(error) => {
                    MessageBoxW(
                        h,
                        wide(error).as_ptr(),
                        wide("Leinwandgröße").as_ptr(),
                        0x30,
                    );
                }
            }
            1
        }
        _ => 0,
    }
}

// SAFETY: invoked for the live editor on its UI thread. The ModalGuard blocks
// destructive reentrancy; native bitmap creation completes before committing
// the single image-and-shapes transaction. Failure aborts the entire edit.
pub(super) unsafe fn resize(h: H) {
    let _modal = ModalGuard::new(h);
    cancel_gesture(h);
    let image = editor(h).image_arc();
    let mut state = DialogState {
        width: image.width,
        height: image.height,
        result: None,
    };
    let template = template();
    let result = DialogBoxIndirectParamW(
        GetModuleHandleW(std::ptr::null()),
        template.as_ptr().cast(),
        h,
        Some(dialog_proc),
        &mut state as *mut DialogState as L,
    );
    if result == -1 {
        MessageBoxW(
            h,
            wide("Der Leinwanddialog konnte nicht geöffnet werden.").as_ptr(),
            wide("ShotKit").as_ptr(),
            0x10,
        );
        return;
    }
    let Some(options) = state.result else {
        return;
    };
    if (options.width, options.height) == (image.width, image.height) {
        return;
    }
    let result = (|| -> Result<(), String> {
        editor(h)
            .document_mut()
            .begin_edit()
            .map_err(|e| e.to_string())?;
        if let Err(error) = editor(h).document_mut().resize_canvas_with_budget(
            options.width,
            options.height,
            options.horizontal,
            options.vertical,
            options.fill,
            &shared_image_budget(),
        ) {
            let _ = editor(h).document_mut().abort_edit();
            return Err(error.to_string());
        }
        let output = editor(h).image_arc();
        let bitmap = match create_bitmap(&output) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = editor(h).document_mut().abort_edit();
                return Err(error);
            }
        };
        match confirm_and_commit_staged_edit(h) {
            Ok(Some(_)) => {}
            Ok(None) => {
                DeleteObject(bitmap);
                let _ = editor(h).document_mut().abort_edit();
                return Ok(());
            }
            Err(error) => {
                DeleteObject(bitmap);
                let _ = editor(h).document_mut().abort_edit();
                return Err(error.to_string());
            }
        }
        install_bitmap(h, bitmap, &output);
        window_state(h).pan_x = 0;
        window_state(h).pan_y = 0;
        editor(h).clear_selection();
        refresh_title(h);
        InvalidateRect(h, std::ptr::null(), 1);
        Ok(())
    })();
    if let Err(error) = result {
        MessageBoxW(
            h,
            wide(&format!(
                "Leinwandgröße konnte nicht geändert werden: {error}"
            ))
            .as_ptr(),
            wide("ShotKit").as_ptr(),
            0x10,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_and_over_budget_sizes() {
        for (w, h) in [
            ("0", "1"),
            ("-1", "2"),
            ("30001", "1"),
            ("30000", "30000"),
            ("1.5", "2"),
        ] {
            assert!(parse_size(w, h).is_err());
        }
        assert_eq!(parse_size("30000", "1"), Ok((30000, 1)));
        assert_eq!(parse_size("16384", "8192"), Ok((16384, 8192)));
        assert!(parse_size("16384", "8193").is_err());
    }
    #[test]
    fn colors_are_opaque_and_hex_only() {
        assert_eq!(parse_color("#1122FF"), Ok(Color::rgba(17, 34, 255, 255)));
        for input in ["#123", "#11223300", "ZZZZZZ", "##112233"] {
            assert!(parse_color(input).is_err());
        }
    }
}
