use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h besitzt gültigen eigenen Fensterzustand auf dem UI-Thread.
// Warum erfüllt bzw. offen: Die aufrufenden Fensterhandler verwenden nur kurzzeitige gelesene Werte; der Gesamtvertrag bleibt offen.
// Nachrichten: Keine der geprüften synchronen Fensternachrichten im Funktionskörper.
pub(super) unsafe fn image_view(h: H) -> (i32, i32, i32, i32) {
    let mut r: Rect = zeroed();
    GetClientRect(h, &mut r);
    let top = toolbar_height(h);
    let v = view::layout(
        (
            window_state(h).width.max(1) as u32,
            window_state(h).height.max(1) as u32,
        ),
        (0, top, r.right - r.left, r.bottom - r.top - top),
        window_state(h).zoom,
        (window_state(h).pan_x, window_state(h).pan_y),
    );
    (v.x, v.y, v.width, v.height)
}
// SAFETY CONTRACT:
// Voraussetzung: h erfüllt den image_view-Vertrag; l enthält gepackte Clientkoordinaten.
// Warum erfüllt bzw. offen: l wird nur als Ganzzahl dekodiert; kein Pointer daraus wird dereferenziert.
// Nachrichten: Keine der geprüften synchronen Fensternachrichten im Funktionskörper.
pub(super) unsafe fn point(h: H, l: L) -> (i32, i32) {
    let (x, y, w, height) = image_view(h);
    let px = (l as u32 as u16 as i16) as i32;
    let py = ((l as u32 >> 16) as u16 as i16) as i32;
    view::to_image(
        view::View {
            x,
            y,
            width: w,
            height,
        },
        (
            window_state(h).width.max(1) as u32,
            window_state(h).height.max(1) as u32,
        ),
        (px, py),
    )
}
// Build a temporary composited bitmap. The base image remains unchanged so
// annotation objects can be selected and undone. Caller owns the returned HBITMAP.
// SAFETY CONTRACT:
// Voraussetzung: h erfüllt den Fensterzustandsvertrag; die Bitmap wird vom Aufrufer freigegeben.
// Warum erfüllt bzw. offen: Bild, Formen und Auswahl werden vor GDI-Aufrufen kopiert; Ressourcenpfade werden geprüft.
// Nachrichten: Native GDI-Aufrufe können Reentranz nicht pauschal ausschließen.
pub(super) unsafe fn composited_bitmap(h: H, preview: bool) -> Result<H, String> {
    if window_state(h).bitmap.is_null() {
        return Err("Kein Bild geöffnet.".into());
    }
    let (crop_start, crop_end) = (window_state(h).crop_start, window_state(h).crop_end);
    // Copy editor data before rendering or making Win32 calls, which may
    // synchronously dispatch messages and borrow this window state again.
    let (source, shapes, tool, selection, stable_count) = {
        let state = editor(h);
        let mut draft = if preview { state.preview() } else { None };
        let stable_count = if draft.is_some() && state.tool() == Tool::Select {
            state.selected_index().unwrap_or(state.shapes().len())
        } else {
            state.shapes().len()
        };
        let mut shapes = Vec::with_capacity(state.shapes().len() + usize::from(draft.is_some()));
        for (i, shape) in state.shapes().iter().enumerate() {
            if draft.is_some() && state.tool() == Tool::Select && state.selected_index() == Some(i)
            {
                shapes.push(draft.take().expect("draft checked above"));
            } else {
                shapes.push(shape.clone());
            }
        }
        if let Some(shape) = draft {
            shapes.push(shape);
        }
        (
            state.image_arc(),
            shapes,
            state.tool(),
            state.selection().cloned(),
            stable_count,
        )
    };
    let rendered = if preview {
        // Take the cache out before GDI+ rendering: nested window messages may
        // paint again, so no RefCell borrow or window-state reference may span it.
        let shared = Rc::clone(&window_state(h).preview_cache);
        let mut cache = shared.borrow_mut().take();
        let result =
            shotkit::alpha_document::render_preview(&source, &shapes, stable_count, &mut cache);
        *shared.borrow_mut() = cache;
        result?
    } else {
        shotkit::alpha_document::render_document(&source, &shapes)?
    };
    // StretchBlt ignores the DIB alpha channel. Flatten only this on-screen
    // preview against a checkerboard so transparent source pixels and
    // translucent annotations are displayed using the same source-over math
    // as the document renderer. Export continues to use the original alpha.
    let display_image = if preview {
        shotkit::alpha_composite::flatten_checkerboard(&rendered)?
    } else {
        rendered
    };
    let output = create_bitmap(&display_image)?;
    if !preview || (tool != Tool::Crop && tool != Tool::Select) {
        return Ok(output);
    }
    let screen = GetDC(null_mut());
    if screen.is_null() {
        DeleteObject(output);
        return Err("Bildschirm konnte nicht geöffnet werden.".into());
    }
    let out_dc = CreateCompatibleDC(screen);
    if out_dc.is_null() {
        ReleaseDC(null_mut(), screen);
        DeleteObject(output);
        return Err("Bild konnte nicht zusammengesetzt werden.".into());
    }
    let old_out = SelectObject(out_dc, output);
    if old_out.is_null() || old_out as isize == -1 {
        DeleteDC(out_dc);
        ReleaseDC(null_mut(), screen);
        DeleteObject(output);
        return Err("Bild konnte nicht ausgewählt werden.".into());
    }
    if preview && tool == Tool::Crop {
        if let (Some(a), Some(b)) = (crop_start, crop_end) {
            let pen = CreatePen(0, 2, 0x00ffffff);
            let previous_pen = SelectObject(out_dc, pen);
            let previous_brush = SelectObject(out_dc, GetStockObject(5));
            Rectangle(
                out_dc,
                a.0.min(b.0),
                a.1.min(b.1),
                a.0.max(b.0),
                a.1.max(b.1),
            );
            SelectObject(out_dc, previous_brush);
            SelectObject(out_dc, previous_pen);
            DeleteObject(pen);
        }
    }
    if preview && tool == Tool::Select {
        if let Some(shape) = selection.as_ref() {
            let b = shape.bounds();
            let pen = CreatePen(0, 1, 0x00d77800);
            let old_pen = SelectObject(out_dc, pen);
            let old_brush = SelectObject(out_dc, GetStockObject(5));
            Rectangle(
                out_dc,
                b.x as i32 - 3,
                b.y as i32 - 3,
                (b.x + b.width) as i32 + 3,
                (b.y + b.height) as i32 + 3,
            );
            for p in shape.handles() {
                Rectangle(
                    out_dc,
                    p.x as i32 - 4,
                    p.y as i32 - 4,
                    p.x as i32 + 5,
                    p.y as i32 + 5,
                );
            }
            SelectObject(out_dc, old_brush);
            SelectObject(out_dc, old_pen);
            DeleteObject(pen);
        }
    }
    SelectObject(out_dc, old_out);
    DeleteDC(out_dc);
    ReleaseDC(null_mut(), screen);
    Ok(output)
}

// SAFETY CONTRACT:
// Voraussetzung: h erfüllt den Fensterzustandsvertrag und enthält gültige Bilddaten.
// Warum erfüllt bzw. offen: Arc und Formen werden vor dem Renderer kopiert.
// Nachrichten: Native GDI-Aufrufe können Reentranz nicht pauschal ausschließen.
pub(super) unsafe fn bitmap_image(h: H) -> Result<Image, String> {
    if window_state(h).bitmap.is_null() || window_state(h).width <= 0 || window_state(h).height <= 0
    {
        return Err("Kein Bild geöffnet.".into());
    }
    // Do not retain a window-state borrow while the renderer calls GDI+.
    let (image, shapes) = {
        let state = editor(h);
        (state.image_arc(), state.shapes().to_vec())
    };
    shotkit::alpha_document::render_document(&image, &shapes)
}

// SAFETY CONTRACT:
// Voraussetzung: Bildpufferlänge entspricht der 32-Bit-DIB-Größe und der DC ist gültig.
// Warum erfüllt: validate prüft öffentliche Bildfelder; CreateDIBSection und DC
// werden auf Fehler geprüft, der neue Zielspeicher überlappt die Quelle nicht.
// Nachrichten: keine der geprüften synchronen Fensternachrichten; GDI-Aufrufe.
pub(super) unsafe fn create_bitmap(image: &Image) -> Result<H, String> {
    image.validate().map_err(str::to_owned)?;
    let width = i32::try_from(image.width).map_err(|_| "Bild zu breit".to_owned())?;
    let height = i32::try_from(image.height).map_err(|_| "Bild zu hoch".to_owned())?;
    let info = Bmi {
        header: BmiHeader {
            size: 40,
            width,
            height: -height,
            planes: 1,
            bits: 32,
            compression: 0,
            image_size: image.pixels.len() as u32,
            xppm: 0,
            yppm: 0,
            used: 0,
            important: 0,
        },
        colors: [0],
    };
    let dc = GetDC(null_mut());
    if dc.is_null() {
        return Err("Bildschirm konnte nicht geöffnet werden.".into());
    }
    let mut bits: *mut c_void = null_mut();
    let bmp = CreateDIBSection(dc, &info, 0, &mut bits, null_mut(), 0);
    ReleaseDC(null_mut(), dc);
    if bmp.is_null() || bits.is_null() {
        if !bmp.is_null() {
            DeleteObject(bmp);
        }
        return Err("Ausgabebild konnte nicht angelegt werden.".into());
    }
    std::ptr::copy_nonoverlapping(image.pixels.as_ptr(), bits.cast::<u8>(), image.pixels.len());
    Ok(bmp)
}
// SAFETY CONTRACT:
// Voraussetzung: bmp ist eine neue gültige Bitmap für image, nicht die alte oder noch in einem DC ausgewählte Bitmap.
// Warum erfüllt bzw. offen: Aktuelle Aufrufer übergeben Ergebnisse von create_bitmap; die Bedingungen sind nicht im Typ erzwungen.
// Nachrichten: Kein expliziter synchroner Fensternachrichtenversand im Funktionskörper.
pub(super) unsafe fn install_bitmap(h: H, bmp: H, image: &Image) {
    if !window_state(h).bitmap.is_null() {
        DeleteObject(window_state(h).bitmap);
    }
    window_state(h).bitmap = bmp;
    window_state(h).width = image.width as i32;
    window_state(h).height = image.height as i32;
}
// Return a hidden, initialized editor shell. Register it in EDITOR_WINDOWS only
// after its optional image has been installed successfully.
