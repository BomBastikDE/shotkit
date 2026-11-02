use super::*;

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
            InvalidateRect(h, std::ptr::null(), 0);
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
                    InvalidateRect(h, std::ptr::null(), 0);
                    return Some(0);
                }
            }
            window_state(h).drawing = true;
            if editor(h).tool() == Tool::Crop {
                window_state(h).crop_start = Some((x, y));
                window_state(h).crop_end = Some((x, y));
                SetCapture(h);
                InvalidateRect(h, std::ptr::null(), 0);
                return Some(0);
            }
            editor(h).mouse_down(ShapePoint::new(x as f32, y as f32));
            SetCapture(h);
            InvalidateRect(h, std::ptr::null(), 0);
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
                InvalidateRect(h, std::ptr::null(), 0);
                return Some(0);
            }
            editor(h)
                .mouse_move_with_shift(ShapePoint::new(x as f32, y as f32), GetKeyState(0x10) < 0);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0202 if window_state(h).drawing => {
            let (x, y) = point(h, l);
            if let Some(start) = std::ptr::replace(&mut window_state(h).crop_start, None) {
                window_state(h).crop_end = None;
                window_state(h).drawing = false;
                ReleaseCapture();
                apply_crop(h, start, (x, y));
                InvalidateRect(h, std::ptr::null(), 1);
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
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0215 if window_state(h).drawing => {
            window_state(h).drawing = false;
            window_state(h).crop_start = None;
            window_state(h).crop_end = None;
            editor(h).cancel_drag();
            InvalidateRect(h, std::ptr::null(), 0);
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
