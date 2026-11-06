use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Editorfenster auf dem UI-Thread.
// Warum erfüllt bzw. offen: Der Fehler wird als eigener Wert vor dem Dialog entnommen; der Zustandsvertrag bleibt Aufruferpflicht.
// Nachrichten: MessageBoxW kann Nachrichten verarbeiten.
pub(super) unsafe fn report_editor_error(h: H) {
    if let Some(error) = editor(h).take_error() {
        MessageBoxW(
            h,
            wide(&format!(
                "Die Bearbeitung konnte nicht übernommen werden: {error:?}"
            ))
            .as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x10,
        );
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h und die temporäre Bitmap bleiben während der Clipboard-Ausgabe gültig.
// Warum erfüllt bzw. offen: Die Bitmap wird nach copy_bitmap freigegeben; der Fensterzustandsvertrag bleibt zu beachten.
// Nachrichten: Clipboard und Fehlerdialog können Nachrichten auslösen.
pub(super) unsafe fn copy_current(h: H) {
    if window_state(h).bitmap.is_null() {
        return;
    }
    let copied = composited_bitmap(h, false).and_then(|bmp| {
        let result = clipboard::copy_bitmap(
            h,
            bmp,
            window_state(h).width,
            window_state(h).height,
            active_settings().exclude_from_clipboard_history,
        );
        DeleteObject(bmp);
        result
    });
    if let Err(e) = copied {
        MessageBoxW(h, wide(&e).as_ptr(), wide("Zwischenablage").as_ptr(), 0x10);
    }
}
#[repr(C)]
pub(super) struct ChooseColor {
    size: u32,
    owner: H,
    instance: H,
    color: u32,
    custom: *mut u32,
    flags: u32,
    data: isize,
    hook: *mut c_void,
    template: *const u16,
}
#[link(name = "comdlg32")]
extern "system" {
    fn ChooseColorW(cc: *mut ChooseColor) -> i32;
}
// SAFETY CONTRACT:
// Voraussetzung: h bleibt als Dialogbesitzer gültig.
// Warum erfüllt bzw. offen: Farbe und Farbarray sind eigene Werte, die bis zur Rückkehr leben.
// Nachrichten: ChooseColorW verarbeitet Nachrichten.
pub(super) unsafe fn choose_color(h: H) -> Option<shotkit::shapes::Color> {
    let current = editor(h).current_style().color;
    let mut custom = [0u32; 16];
    let mut cc = ChooseColor {
        size: size_of::<ChooseColor>() as u32,
        owner: h,
        instance: null_mut(),
        color: current.r as u32 | ((current.g as u32) << 8) | ((current.b as u32) << 16),
        custom: custom.as_mut_ptr(),
        flags: 0x1 | 0x2,
        data: 0,
        hook: null_mut(),
        template: std::ptr::null(),
    };
    if ChooseColorW(&mut cc) == 0 {
        return None;
    }
    Some(shotkit::shapes::Color::rgba(
        cc.color as u8,
        (cc.color >> 8) as u8,
        (cc.color >> 16) as u8,
        255,
    ))
}
// SAFETY CONTRACT:
// Voraussetzung: h bleibt gültig und values lebt bis zur Rückkehr.
// Warum erfüllt bzw. offen: Das lokale Menü wird zerstört und der Ergebnisindex mit get geprüft.
// Nachrichten: TrackPopupMenu verarbeitet Nachrichten.
pub(super) unsafe fn choose_number(h: H, values: &[u32]) -> Option<f32> {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }
    for (i, value) in values.iter().enumerate() {
        AppendMenuW(menu, 0, i + 1, wide(&value.to_string()).as_ptr());
    }
    let mut p = Point { x: 0, y: 0 };
    if GetCursorPos(&mut p) == 0 {
        DestroyMenu(menu);
        return None;
    }
    let selected = TrackPopupMenu(menu, 0x0100 | 0x0002, p.x, p.y, 0, h, std::ptr::null());
    DestroyMenu(menu);
    values
        .get(selected.wrapping_sub(1) as usize)
        .map(|v| *v as f32)
}

// SAFETY CONTRACT:
// Voraussetzung: h bleibt als Menübesitzer gültig.
// Warum erfüllt bzw. offen: Das Menü ist lokal und das Ergebnis wird auf gültige IDs begrenzt.
// Nachrichten: SetForegroundWindow und TrackPopupMenu können Nachrichten auslösen.
pub(super) unsafe fn choose_frame_action(h: H) -> Option<u32> {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }
    for (id, label) in [
        (1, "Rahmen hinzufügen"),
        (2, "Schlagschatten hinzufügen"),
        (3, "Rahmen und Schlagschatten hinzufügen"),
        (4, "Leinwandgröße ändern…"),
    ] {
        AppendMenuW(menu, 0, id, wide(label).as_ptr());
    }
    let mut p = Point { x: 0, y: 0 };
    if GetCursorPos(&mut p) == 0 {
        DestroyMenu(menu);
        return None;
    }
    SetForegroundWindow(h);
    let selected = TrackPopupMenu(menu, 0x0100 | 0x0002, p.x, p.y, 0, h, std::ptr::null());
    DestroyMenu(menu);
    (1..=4).contains(&selected).then_some(selected)
}

// SAFETY CONTRACT:
// Voraussetzung: h und der Editor erfüllen den Zustandsvertrag auch nach Dialogen.
// Warum erfüllt bzw. offen: Dialogwerte und Bild-Arc sind eigenständig; Bitmapfehler werden behandelt.
// Nachrichten: Dialoge und ReleaseCapture können Nachrichten auslösen.
pub(super) unsafe fn apply_frame(h: H, action: u32) {
    let width = if action == 2 {
        0
    } else {
        let Some(width) = choose_number(h, &[4, 8, 12, 16, 24, 32]) else {
            return;
        };
        width as usize
    };
    let color = if width == 0 {
        shotkit::shapes::Color::rgba(0, 0, 0, 0)
    } else {
        let Some(color) = choose_color(h) else {
            return;
        };
        color
    };
    let options = FrameOptions {
        width,
        color,
        shadow: (action != 1).then_some(ShadowOptions {
            offset_x: 8,
            offset_y: 8,
            blur_radius: 12,
            opacity: 112,
        }),
    };
    cancel_gesture(h);
    let result = (|| -> Result<(), String> {
        editor(h)
            .document_mut()
            .begin_edit()
            .map_err(|e| format!("{e:?}"))?;
        if let Err(error) = editor(h).add_frame(options) {
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{error:?}"));
        }
        let output = editor(h).image_arc();
        let bitmap = match create_bitmap(&output) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = editor(h).document_mut().abort_edit();
                return Err(error);
            }
        };
        if let Err(error) = editor(h).document_mut().commit_edit() {
            DeleteObject(bitmap);
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{error:?}"));
        }
        install_bitmap(h, bitmap, &output);
        window_state(h).zoom = 1.0;
        window_state(h).pan_x = 0;
        window_state(h).pan_y = 0;
        Ok(())
    })();
    if let Err(error) = result {
        MessageBoxW(
            h,
            wide(&format!("Rahmen/Schlagschatten fehlgeschlagen: {error}")).as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x10,
        );
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h und der Editor erfüllen den Zustandsvertrag über die Auswahl hinweg.
// Warum erfüllt bzw. offen: Der Stil wird vor dem Dialog geklont und danach neu auf den Editor zugegriffen.
// Nachrichten: Dialoge und Toolbaraufrufe können Nachrichten auslösen.
pub(super) unsafe fn edit_style(h: H, id: usize) {
    let selected = editor(h).selection().map(|s| s.style.clone());
    let mut replacement = selected.unwrap_or_else(|| editor(h).current_style().clone());
    match id {
        toolbar::ID_COLOR => {
            let Some(color) = choose_color(h) else { return };
            editor(h).set_current_color(color);
            toolbar::set_color(h, color.r, color.g, color.b);
            replacement.color = color;
        }
        toolbar::ID_LINE_WIDTH => {
            let Some(width) = choose_number(h, &[1, 2, 3, 4, 6, 8, 12, 16]) else {
                return;
            };
            editor(h).set_current_width(width);
            replacement.width = width;
        }
        toolbar::ID_FONT_SIZE => {
            let Some(size) = choose_number(h, &[14, 18, 24, 28, 36, 48, 64, 96]) else {
                return;
            };
            editor(h).set_current_font_size(size);
            replacement.font_size = size;
        }
        _ => return,
    }
    let callout = editor(h).selected_index().and_then(|index| {
        editor(h).selection().and_then(|shape| match &shape.kind {
            shotkit::shapes::ShapeKind::Callout { data } => Some((index, data.as_ref().clone())),
            _ => None,
        })
    });
    if let Some((index, mut data)) = callout {
        if replacement.font_size != data.layout.font_size {
            match shotkit::alpha_render::measure_callout_text(&data.text, replacement.font_size) {
                Ok(layout) => data.layout = layout,
                Err(error) => {
                    MessageBoxW(
                        h,
                        wide(&format!(
                            "Der Text konnte nicht vermessen werden: {error:?}"
                        ))
                        .as_ptr(),
                        wide("Sprechblase").as_ptr(),
                        0x10,
                    );
                    return;
                }
            }
        }
        editor(h).replace_callout_with_style(index, data, replacement);
    } else {
        editor(h).set_selected_style(replacement);
    }
}

pub(super) fn toolbar_id_for_tool(tool: Tool) -> usize {
    match tool {
        Tool::Select => toolbar::ID_SELECT,
        Tool::Arrow => toolbar::ID_ARROW,
        Tool::Line => toolbar::ID_LINE,
        Tool::Rectangle => toolbar::ID_RECTANGLE,
        Tool::Ellipse => toolbar::ID_ELLIPSE,
        Tool::Text => toolbar::ID_TEXT,
        Tool::Callout => toolbar::ID_CALLOUT,
        Tool::Highlight => toolbar::ID_MARKER,
        Tool::Blur => toolbar::ID_BLUR,
        Tool::GaussianBlur => toolbar::ID_GAUSSIAN_BLUR,
        Tool::Redact => toolbar::ID_REDACT,
        Tool::Step => toolbar::ID_STEP,
        Tool::Stamp => toolbar::ID_STAMP,
        Tool::Pen => toolbar::ID_PEN,
        Tool::Crop => toolbar::ID_CROP,
    }
}

/// Apply a tool selection consistently for toolbar clicks and keyboard
/// shortcuts, including marker color, dirty state, and toolbar readback.
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Editorfenster auf dem UI-Thread.
// Warum erfüllt bzw. offen: Aufrufer bearbeiten ein eigenes Fenster; vollständige Handlelebensdauer bleibt Aufruferpflicht.
// Nachrichten: SetWindowTextW und InvalidateRect können Fensterzustand aktualisieren.
pub(super) unsafe fn activate_tool(h: H, tool: Tool) {
    cancel_gesture(h);
    let old_color = editor(h).current_style().color;
    editor(h).set_tool(tool);
    let color = editor(h).current_style().color;
    if color != old_color {
        toolbar::set_color(h, color.r, color.g, color.b);
    }
    toolbar::set_active(h, toolbar_id_for_tool(tool));
    SetWindowTextW(
        h,
        wide(&format!(
            "ShotKit Rust – Werkzeug {tool:?} · Strg+S speichern · F10 Scroll"
        ))
        .as_ptr(),
    );
    InvalidateRect(h, std::ptr::null(), 0);
}

fn apply_form_style(style: &mut shotkit::shapes::Style, id: usize) -> bool {
    use shotkit::shapes::{ArrowEnds, FillMode, LineStyle, RectangleCorners};
    match id {
        toolbar::ID_FILL_NONE => style.fill = FillMode::None,
        toolbar::ID_FILL_SOLID => style.fill = FillMode::Solid,
        toolbar::ID_FILL_TRANSLUCENT => style.fill = FillMode::Translucent,
        toolbar::ID_LINE_SOLID => style.line_style = LineStyle::Solid,
        toolbar::ID_LINE_DASHED => style.line_style = LineStyle::Dashed,
        toolbar::ID_LINE_DOTTED => style.line_style = LineStyle::Dotted,
        toolbar::ID_CORNERS_SQUARE => style.rectangle_corners = RectangleCorners::Square,
        toolbar::ID_CORNERS_ROUNDED => style.rectangle_corners = RectangleCorners::Rounded,
        toolbar::ID_ARROW_SINGLE => style.arrow_ends = ArrowEnds::Single,
        toolbar::ID_ARROW_DOUBLE => style.arrow_ends = ArrowEnds::Double,
        toolbar::ID_ARROW_DOT => style.arrow_ends = ArrowEnds::Dot,
        _ => return false,
    }
    true
}

// SAFETY CONTRACT:
// Voraussetzung: h und der Editor erfüllen den Zustandsvertrag für delegierte Aktionen.
// Warum erfüllt bzw. offen: Der eigene Nachrichtenhandler ruft die Aktion mit einem eigenen Fenster auf; alle Aufrufer bleiben zu prüfen.
// Nachrichten: Fenster-, Dialog- und Aufnahmeaktionen können Nachrichten auslösen.
pub(super) unsafe fn toolbar_action(h: H, id: usize) {
    // Native popup loops run inside the parent's existing ModalGuard. Return
    // only validated ordinary commands and reuse their exact editor workflows.
    let id = if matches!(
        id,
        toolbar::ID_MORE
            | toolbar::ID_FILE
            | toolbar::ID_VIEW
            | toolbar::ID_STAMP
            | toolbar::ID_FORM_STYLE
    ) {
        let stamp_kind = editor(h).stamp_kind();
        let style = editor(h)
            .selection()
            .map(|shape| shape.style.clone())
            .unwrap_or_else(|| editor(h).current_style().clone());
        let target = toolbar::style_target(editor(h).selection().map(|shape| &shape.kind));
        let Some(selected) = toolbar::popup(h, id, stamp_kind, &style, target) else {
            SetFocus(h);
            return;
        };
        selected
    } else {
        id
    };
    if (toolbar::ID_CALLOUT_FILL..=toolbar::ID_CALLOUT_POINTER).contains(&id) {
        let selected = editor(h).selected_index().and_then(|index| {
            editor(h).selection().and_then(|shape| match &shape.kind {
                shotkit::shapes::ShapeKind::Callout { data } => {
                    Some((index, data.as_ref().clone()))
                }
                _ => None,
            })
        });
        if let Some((index, mut data)) = selected {
            match id {
                toolbar::ID_CALLOUT_FILL => {
                    let Some(color) = choose_color(h) else {
                        return;
                    };
                    data.fill_color = color;
                }
                toolbar::ID_CALLOUT_TEXT_COLOR => {
                    let Some(color) = choose_color(h) else {
                        return;
                    };
                    data.text_color = color;
                }
                _ => {
                    data.tip = if data.tip.is_some() {
                        None
                    } else {
                        Some(ShapePoint::new(
                            data.origin.x + 20.0,
                            data.origin.y + data.layout.body_height + 35.0,
                        ))
                    };
                }
            }
            editor(h).replace_callout(index, data);
            report_editor_error(h);
            InvalidateRect(h, std::ptr::null(), 0);
        }
        SetFocus(h);
        return;
    }
    if (toolbar::ID_FILL_NONE..=toolbar::ID_ARROW_DOT).contains(&id) {
        let mut current = editor(h).current_style().clone();
        if !apply_form_style(&mut current, id) {
            return;
        }
        let selected = editor(h).selection().and_then(|shape| {
            toolbar::form_style_applies(id, toolbar::style_target(Some(&shape.kind)))
                .then(|| shape.style.clone())
        });
        editor(h).set_current_style(current);
        if let Some(mut selected) = selected {
            apply_form_style(&mut selected, id);
            editor(h).set_selected_style(selected);
        }
        report_editor_error(h);
        SetFocus(h);
        InvalidateRect(h, std::ptr::null(), 0);
        return;
    }
    let stamp_kind = toolbar::stamp_kind_for_command(id);
    if let Some(kind) = stamp_kind {
        editor(h).set_stamp_kind(kind);
    }
    let tool = match id {
        toolbar::ID_SELECT => Some(Tool::Select),
        toolbar::ID_ARROW => Some(Tool::Arrow),
        toolbar::ID_LINE => Some(Tool::Line),
        toolbar::ID_RECTANGLE => Some(Tool::Rectangle),
        toolbar::ID_ELLIPSE => Some(Tool::Ellipse),
        toolbar::ID_TEXT => Some(Tool::Text),
        toolbar::ID_CALLOUT => Some(Tool::Callout),
        toolbar::ID_MARKER => Some(Tool::Highlight),
        toolbar::ID_BLUR => Some(Tool::Blur),
        toolbar::ID_GAUSSIAN_BLUR => Some(Tool::GaussianBlur),
        toolbar::ID_REDACT => Some(Tool::Redact),
        toolbar::ID_STEP => Some(Tool::Step),
        _ if stamp_kind.is_some() => Some(Tool::Stamp),
        toolbar::ID_PEN => Some(Tool::Pen),
        toolbar::ID_CROP => Some(Tool::Crop),
        _ => None,
    };
    if let Some(tool) = tool {
        activate_tool(h, tool);
    } else {
        match id {
            toolbar::ID_SWATCH_RED
            | toolbar::ID_SWATCH_BLACK
            | toolbar::ID_SWATCH_YELLOW
            | toolbar::ID_SWATCH_BLUE => {
                let (r, g, b) = match id {
                    toolbar::ID_SWATCH_RED => (230, 40, 40),
                    toolbar::ID_SWATCH_BLACK => (0, 0, 0),
                    toolbar::ID_SWATCH_YELLOW => (255, 214, 0),
                    _ => (0, 120, 215),
                };
                let color = shotkit::shapes::Color::rgba(r, g, b, 255);
                let mut replacement = editor(h)
                    .selection()
                    .map(|shape| shape.style.clone())
                    .unwrap_or_else(|| editor(h).current_style().clone());
                editor(h).set_current_color(color);
                toolbar::set_color(h, r, g, b);
                replacement.color = color;
                editor(h).set_selected_style(replacement);
            }
            toolbar::ID_COLOR | toolbar::ID_LINE_WIDTH | toolbar::ID_FONT_SIZE => edit_style(h, id),
            toolbar::ID_FRAME => {
                if window_state(h).bitmap.is_null() {
                    MessageBoxW(
                        h,
                        wide("Öffne zuerst ein Bild.").as_ptr(),
                        wide("ShotKit Rust").as_ptr(),
                        0x30,
                    );
                } else if let Some(action) = choose_frame_action(h) {
                    if action == 4 {
                        canvas::resize(h);
                    } else {
                        apply_frame(h, action);
                    }
                }
            }
            toolbar::ID_CAPTURE => capture_region(h),
            toolbar::ID_SAVE => {
                save(h, false);
            }
            toolbar::ID_SAVE_AS => {
                save(h, true);
            }
            toolbar::ID_OPEN => open_image(h),
            toolbar::ID_PASTE => paste_image(h),
            toolbar::ID_FOLDER => open_storage_folder(h),
            toolbar::ID_ZOOM_IN => apply_zoom(h, view::ZoomAction::In),
            toolbar::ID_ZOOM_OUT => apply_zoom(h, view::ZoomAction::Out),
            toolbar::ID_ZOOM_ORIGINAL => apply_zoom(h, view::ZoomAction::Original),
            toolbar::ID_ZOOM_FIT => apply_zoom(h, view::ZoomAction::Fit),
            toolbar::ID_COPY => copy_current(h),
            toolbar::ID_UNDO => history(h, false),
            toolbar::ID_REDO => history(h, true),
            _ => {}
        }
    }
    report_editor_error(h);
    SetFocus(h);
    InvalidateRect(h, std::ptr::null(), 0);
}

// SAFETY CONTRACT: h is a live editor on the UI thread. State values are copied
// before GetClientRect; no WindowState reference spans a native call.
unsafe fn apply_zoom(h: H, action: view::ZoomAction) {
    let image = (window_state(h).width as u32, window_state(h).height as u32);
    let current = window_state(h).zoom;
    let top = toolbar_height(h);
    let mut client: Rect = zeroed();
    GetClientRect(h, &mut client);
    let zoom = view::zoom_for_action(
        action,
        image,
        (0, top, client.right, client.bottom - top),
        current,
    );
    {
        let state = window_state(h);
        state.zoom = zoom;
        if matches!(action, view::ZoomAction::Original | view::ZoomAction::Fit) {
            state.pan_x = 0;
            state.pan_y = 0;
        }
    }
    // A smaller image exposes background outside the new image rectangle.
    InvalidateRect(h, std::ptr::null(), 1);
}
