use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiges Editorfenster mit initialisiertem Zustand.
// Warum erfüllt: Aufruf nur aus dessen Maus-Handler; das Rechteck ist lokal und
// lebt bis zur Rückkehr von InvalidateRect, ohne gehaltenen STATE-Borrow.
// Nachrichten: InvalidateRect markiert den Bereich und sendet hier keine Nachricht.
unsafe fn invalidate_image_view(h: H) {
    let (x, y, width, height) = image_view(h);
    let rect = Rect {
        left: x,
        top: y,
        right: x.saturating_add(width),
        bottom: y.saturating_add(height),
    };
    InvalidateRect(h, &rect, 0);
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiges Editorfenster; vor und nach sind besessene
// Schnappschüsse der Vorschau und bleiben während der Berechnung gültig.
// Warum erfüllt: Der Maus-Handler kopiert beide Formen und beendet Zustandszugriffe
// vor InvalidateRect; unsichere Textgrenzen fallen auf die Bildfläche zurück.
// Nachrichten: InvalidateRect markiert den Bereich und sendet hier keine Nachricht.
unsafe fn invalidate_preview_change(
    h: H,
    before: Option<&shotkit::shapes::Shape>,
    after: Option<&shotkit::shapes::Shape>,
) {
    let (x, y, width, height) = image_view(h);
    let view = view::View {
        x,
        y,
        width,
        height,
    };
    let image = (
        window_state(h).width.max(1) as u32,
        window_state(h).height.max(1) as u32,
    );
    let region = view::dirty_shape_region(view, image, before, after).unwrap_or(view);
    let rect = Rect {
        left: region.x,
        top: region.y,
        right: region.x.saturating_add(region.width),
        bottom: region.y.saturating_add(region.height),
    };
    InvalidateRect(h, &rect, 0);
}

// SAFETY CONTRACT:
// Voraussetzung: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc prüft Userdata; die Lebensdauer bei Reentranz
// und der Vertrag von window_state bleiben für jeden Aufruf zu beachten.
// Nachrichten: Textdialog und SetCapture können reentrieren; Text und Koordinaten werden vor den Aufrufen kopiert.
pub(super) unsafe fn dispatch_mouse_begin(h: H, m: u32, _w: W, l: L) -> Option<L> {
    Some(match m {
        0x0203 if !window_state(h).bitmap.is_null() => {
            let (x, y) = point(h, l);
            editor(h).set_tool(Tool::Select);
            toolbar::set_active(h, toolbar::ID_SELECT);
            editor(h).mouse_down(ShapePoint::new(x as f32, y as f32));
            editor(h).mouse_up(ShapePoint::new(x as f32, y as f32));
            let current = editor(h).selected_text().map(str::to_owned);
            if let Some(text) = current {
                if let Some(changed) = text_dialog::prompt(h, &text) {
                    if editor(h).set_selected_text(changed) {
                        window_state(h).dirty = true;
                    }
                }
            }
            report_editor_error(h);
            invalidate_image_view(h);
            0
        }
        0x0201 if !window_state(h).bitmap.is_null() => {
            let (vx, vy, vw, vh) = image_view(h);
            let sx = (l as u32 as u16 as i16) as i32;
            let sy = ((l as u32 >> 16) as u16 as i16) as i32;
            if sx < vx || sy < vy || sx >= vx + vw || sy >= vy + vh {
                return Some(0);
            }
            let (x, y) = point(h, l);
            window_state(h).pen_history = None;
            let image_point = ShapePoint::new(x as f32, y as f32);
            let text_tool = editor(h).tool() == Tool::Text;
            let handle_hit = text_tool && editor(h).selected_handle_hit(image_point);
            if text_tool {
                let hit = editor(h).text_at(image_point);
                let shape_count = editor(h).shapes().len();
                crate::uia_picker::log_diagnostic(&format!(
                    "text-click image=({x},{y}) shapes={shape_count} hit={} hit_index={} selected_handle_hit={handle_hit}",
                    hit.is_some(),
                    hit.as_ref().map_or_else(|| "none".to_owned(), |(index, _)| index.to_string()),
                ));
                if !handle_hit {
                    if let Some((index, existing)) = hit {
                        if let Some(changed) = text_dialog::prompt(h, &existing) {
                            let edited = editor(h).set_text_at(index, changed);
                            crate::uia_picker::log_diagnostic(&format!(
                                "text-edit index={index} changed={edited} shapes={}",
                                editor(h).shapes().len(),
                            ));
                            if edited {
                                window_state(h).dirty = true;
                            }
                        }
                    } else if let Some(text) = text_dialog::prompt(h, "") {
                        if !text.trim().is_empty() {
                            let placed = editor(h).place_text(image_point, text);
                            crate::uia_picker::log_diagnostic(&format!(
                                "text-place image=({x},{y}) placed={placed} shapes={}",
                                editor(h).shapes().len(),
                            ));
                            if placed {
                                window_state(h).dirty = true;
                            }
                        }
                    }
                    report_editor_error(h);
                    invalidate_image_view(h);
                    return Some(0);
                }
            }
            window_state(h).drawing = true;
            if editor(h).tool() == Tool::Crop {
                window_state(h).crop_start = Some((x, y));
                window_state(h).crop_end = Some((x, y));
                SetCapture(h);
                invalidate_image_view(h);
                return Some(0);
            }
            editor(h).mouse_down(ShapePoint::new(x as f32, y as f32));
            SetCapture(h);
            invalidate_image_view(h);
            0
        }
        _ => return None,
    })
}

// SAFETY CONTRACT:
// Voraussetzung: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc prüft Userdata; die Lebensdauer bei Reentranz
// und der Vertrag von window_state bleiben für jeden Aufruf zu beachten.
// Nachrichten: ReleaseCapture und Toolbaraktionen können reentrieren; Zustandszugriffe enden vor den Aufrufen.
pub(super) unsafe fn dispatch_mouse_continue(h: H, m: u32, _w: W, l: L) -> Option<L> {
    Some(match m {
        0x0200 if window_state(h).drawing && !window_state(h).bitmap.is_null() => {
            let (x, y) = point(h, l);
            if matches!(window_state(h).crop_start, Some(_)) {
                window_state(h).crop_end = Some((x, y));
                invalidate_image_view(h);
                return Some(0);
            }
            let before = editor(h).preview();
            let pen = editor(h).tool() == Tool::Pen;
            let (positions, anchor) = if pen {
                mouse_history::pen_samples(h, l)
            } else {
                (vec![ShapePoint::new(x as f32, y as f32)], None)
            };
            window_state(h).pen_history = anchor;
            let shift = GetKeyState(0x10) < 0;
            for position in positions {
                editor(h).mouse_move_with_shift(position, shift);
            }
            let after = editor(h).preview();
            invalidate_preview_change(h, before.as_ref(), after.as_ref());
            0
        }
        0x0202 if window_state(h).drawing => {
            window_state(h).pen_history = None;
            let (x, y) = point(h, l);
            if let Some(start) = std::ptr::replace(&mut window_state(h).crop_start, None) {
                window_state(h).crop_end = None;
                window_state(h).drawing = false;
                let (old_x, old_y, old_width, old_height) = image_view(h);
                ReleaseCapture();
                apply_crop(h, start, (x, y));
                let (new_x, new_y, new_width, new_height) = image_view(h);
                let rect = Rect {
                    left: old_x.min(new_x),
                    top: old_y.min(new_y),
                    right: old_x
                        .saturating_add(old_width)
                        .max(new_x.saturating_add(new_width)),
                    bottom: old_y
                        .saturating_add(old_height)
                        .max(new_y.saturating_add(new_height)),
                };
                InvalidateRect(h, &rect, 1);
                return Some(0);
            }
            let before = editor(h).shapes().to_vec();
            editor(h)
                .mouse_up_with_shift(ShapePoint::new(x as f32, y as f32), GetKeyState(0x10) < 0);
            if before != editor(h).shapes() {
                window_state(h).dirty = true;
            }
            window_state(h).drawing = false;
            ReleaseCapture();
            report_editor_error(h);
            invalidate_image_view(h);
            0
        }
        0x0215 if window_state(h).drawing => {
            window_state(h).drawing = false;
            window_state(h).pen_history = None;
            window_state(h).crop_start = None;
            window_state(h).crop_end = None;
            editor(h).cancel_drag();
            invalidate_image_view(h);
            0
        }
        _ => return None,
    })
}

// SAFETY CONTRACT:
// Voraussetzung: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc prüft Userdata; die Lebensdauer bei Reentranz
// und der Vertrag von window_state bleiben für jeden Aufruf zu beachten.
// Nachrichten: DestroyWindow und DefWindowProcW können reentrieren; Toolbar und Bitmap werden vor ihrer Freigabe aus dem Zustand entnommen.
pub(super) unsafe fn dispatch_paint_and_close(h: H, m: u32, _w: W, _l: L) -> Option<L> {
    Some(match m {
        0x000f => {
            let mut ps: Paint = zeroed();
            let dc = BeginPaint(h, &mut ps);
            if !window_state(h).bitmap.is_null() && !dc.is_null() {
                let mut ok = false;
                if let Ok(flattened) = composited_bitmap(h, true) {
                    let mem = CreateCompatibleDC(dc);
                    if !mem.is_null() {
                        let old = SelectObject(mem, flattened);
                        if !old.is_null() && old as isize != -1 {
                            let (x, y, w, height) = image_view(h);
                            ok = StretchBlt(
                                dc,
                                x,
                                y,
                                w,
                                height,
                                mem,
                                0,
                                0,
                                window_state(h).width,
                                window_state(h).height,
                                COPY,
                            ) != 0;
                            SelectObject(mem, old);
                        }
                        DeleteDC(mem);
                    }
                    DeleteObject(flattened);
                }
                if !ok {
                    let message = wide("Bilddarstellung fehlgeschlagen. Bitte erneut versuchen.");
                    TextOutW(
                        dc,
                        12,
                        toolbar_height(h) + 15,
                        message.as_ptr(),
                        (message.len() - 1) as i32,
                    );
                }
            }
            if window_state(h).bitmap.is_null() && !dc.is_null() {
                let message =
                    wide("Bild öffnen: Strg+O · Bild einfügen: Strg+V · Aufnahme: F9 oder F11");
                TextOutW(
                    dc,
                    12,
                    toolbar_height(h) + 15,
                    message.as_ptr(),
                    (message.len() - 1) as i32,
                );
            }
            EndPaint(h, &ps);
            0
        }
        0x0010 => {
            if h == PRIMARY {
                quit_application(h);
            } else if !QUITTING.load(Ordering::SeqCst) && confirm_replace(h) {
                DestroyWindow(h);
            }
            0
        }
        0x0002 => {
            let toolbar = window_state(h).toolbar.take();
            drop(toolbar); // DestroyWindow(child) may synchronously reenter this parent.
            let bitmap = std::mem::replace(&mut window_state(h).bitmap, null_mut());
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).retain(|window| *window != h);
            if h == PRIMARY {
                hotkeys::unregister(h);
                tray::remove(h);
                PostQuitMessage(0);
            }
            0
        }
        _ => return None,
    })
}
