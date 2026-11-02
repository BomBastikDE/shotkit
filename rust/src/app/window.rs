use super::*;

// SAFETY CONTRACT:
// Voraussetzung: Windows ruft mit gültigen Nachrichtenparametern auf dem
// Besitzerthread auf; Userdata ist null oder zeigt exklusiv auf WindowState.
// Warum erfüllt bzw. offen: WM_NCCREATE installiert den Zustand, WM_NCDESTROY
// entfernt ihn vor Freigabe; Panikfreiheit über die ABI-Grenze ist nicht belegt.
// Nachrichten: Dispatch und DefWindowProcW können synchron Nachrichten auslösen.
pub(super) unsafe extern "system" fn proc(h: H, m: u32, w: W, l: L) -> L {
    if m == 0x0081 { // WM_NCCREATE
        let state = Box::into_raw(Box::new(WindowState::new()));
        SetWindowLongPtrW(h, -21, state as isize);
    }
    if GetWindowLongPtrW(h, -21) == 0 { return DefWindowProcW(h, m, w, l); }
    if m == 0x0082 { // WM_NCDESTROY, after child controls and WM_DESTROY
        let state = GetWindowLongPtrW(h, -21) as *mut WindowState;
        SetWindowLongPtrW(h, -21, 0);
        drop(Box::from_raw(state));
        return DefWindowProcW(h, m, w, l);
    }
    let input = m == toolbar::WM_COMMAND || (0x0100..=0x0109).contains(&m)
        || (0x0200..=0x020e).contains(&m);
    let starts_operation = input || m == TRAY || m == 0x0312 || m == DELAYED_CAPTURE;
    if m == DELAYED_CAPTURE {
        let Some(ticket) = CaptureTicket::from_wire(w) else { return 0; };
        if !DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).is_waiting(ticket) {
            return 0;
        }
        if window_state(h).modal_depth > 0 || SCROLL_BUSY.load(Ordering::SeqCst)
            || QUITTING.load(Ordering::SeqCst) {
            // Retry only this still-waiting generation. Stale or duplicate
            // wakeups cannot start a new selection or a newer request early.
            post_capture_after(h, ticket, std::time::Duration::from_millis(250));
            return 0;
        }
    }
    if m == WM_COPYDATA && (window_state(h).modal_depth > 0 || QUITTING.load(Ordering::SeqCst)) { return 3; }
    if (window_state(h).modal_depth > 0 && (m == 0x0010 || starts_operation))
        || (QUITTING.load(Ordering::SeqCst) && starts_operation) { return 0; }
    let modal_event = input || m == WM_COPYDATA || m == TRAY || m == 0x0312
        || m == SCROLL_DONE || m == DELAYED_CAPTURE;
    // Keep windows alive while handlers run nested Win32 modal loops. Paint and
    // resize remain available; another document can still be used normally.
    let modal = modal_event.then(|| ModalGuard::new(h));
    let result = dispatch_window(h, m, w, l);
    drop(modal);
    if input && h != PRIMARY && GetWindowLongPtrW(h, -21) != 0 { refresh_title(h); }
    result
}
// SAFETY CONTRACT:
// Voraussetzung: proc hat den Fensterzustand geprüft und die Nachrichtenparameter stammen von Windows.
// Warum erfüllt bzw. offen: proc ist der einzige Aufrufer; Nachricht-spezifische Verträge und Reentranz bleiben gesondert zu prüfen.
// Nachrichten: Fenster-, Dialog- und Standardprozeduraufrufe können Nachrichten verarbeiten.
unsafe fn dispatch_window(h: H, m: u32, w: W, l: L) -> L {
    match m {
        WM_FINISH_QUIT if h == PRIMARY => { DestroyWindow(h); 0 }
        WM_COPYDATA => handle_instance_file(h, l),
        WM_INSTANCE_ACTIVATE => { show_editors(); 1 }
        0x0312 if hotkeys::dispatch(h, w) => 0,
        TRAY if l as u32 == 0x0202 => {
            capture_region(h);
            0
        }
        SCROLL_DONE => {
            let result = SCROLL_RESULT
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            let Some(result) = result else {
                return 0;
            };
            SCROLL_BUSY.store(false, Ordering::SeqCst);
            match result {
                Ok(outcome) => {
                    let message = outcome.reason;
                    let was_visible = RESTORE_EDITOR.swap(false, Ordering::SeqCst);
                    if let Err(e) = handle_capture(h, outcome.image, was_visible) {
                        if was_visible {
                            ShowWindow(h, 5);
                        }
                        MessageBoxW(h, wide(&e).as_ptr(), wide("ShotKit Rust").as_ptr(), 0x10);
                    } else {
                        MessageBoxW(
                            h,
                            wide(&message).as_ptr(),
                            wide("Scroll-Aufnahme beendet").as_ptr(),
                            0,
                        );
                    }
                }
                Err(e) => {
                    if RESTORE_EDITOR.swap(false, Ordering::SeqCst) {
                        ShowWindow(h, 5);
                    }
                    MessageBoxW(
                        h,
                        wide(&e).as_ptr(),
                        wide("Scroll-Aufnahme fehlgeschlagen").as_ptr(),
                        0x10,
                    );
                }
            }
            0
        }
        DELAYED_CAPTURE => {
            let Some(ticket) = CaptureTicket::from_wire(w) else { return 0; };
            if !DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).try_start(ticket) {
                return 0;
            }
            // Running remains pending through accepted, cancelled and failed
            // selections; duplicate messages are rejected during modal loops.
            capture_region(h);
            DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).finish(ticket);
            0
        }
        TRAY if l as u32 == 0x0205 => {
            // Save the target before the menu activates ShotKit. The menu
            // command later uses these validated bounds without querying the
            // menu or Shell as foreground again.
            let active_target = active_window_target(h, GetForegroundWindow());
            tray::tray_menu(h, active_target);
            0
        }
        0x0100 if w == 0x56 && GetKeyState(0x11) < 0 => {
            paste_image(h);
            0
        }
        0x0100 if w == 0x4f && GetKeyState(0x11) < 0 => {
            open_image(h);
            0
        }
        0x0100 if w == 0x4e && GetKeyState(0x11) < 0 => {
            capture_region(h);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && GetKeyState(0x10) < 0 && w == 0x5a => {
            history(h, true);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x6b || w == 0xbb) => {
            window_state(h).zoom = (window_state(h).zoom * 1.25).clamp(0.05, 64.0);
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x6d || w == 0xbd) => {
            window_state(h).zoom = (window_state(h).zoom * 0.8).clamp(0.05, 64.0);
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x020a if !window_state(h).bitmap.is_null() => {
            let delta = ((w >> 16) as u16 as i16) as i32;
            if GetKeyState(0x11) < 0 {
                window_state(h).zoom = (window_state(h).zoom * if delta > 0 { 1.2 } else { 1.0 / 1.2 }).clamp(0.05, 64.0);
            } else if GetKeyState(0x10) < 0 {
                window_state(h).pan_x = window_state(h).pan_x.saturating_sub(delta);
            } else {
                window_state(h).pan_y = window_state(h).pan_y.saturating_sub(delta);
            }
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && w == 0x30 => {
            window_state(h).zoom = 1.0;
            window_state(h).pan_x = 0;
            window_state(h).pan_y = 0;
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && w == 0x39 && !window_state(h).bitmap.is_null() => {
            let v = view::layout(
                (window_state(h).width as u32, window_state(h).height as u32),
                (
                    0,
                    toolbar_height(h),
                    {
                        let mut r: Rect = zeroed();
                        GetClientRect(h, &mut r);
                        r.right
                    },
                    {
                        let mut r: Rect = zeroed();
                        GetClientRect(h, &mut r);
                        r.bottom - toolbar_height(h)
                    },
                ),
                1.0,
                (0, 0),
            );
            window_state(h).zoom = (window_state(h).width as f64 / v.width as f64).clamp(0.05, 64.0);
            window_state(h).pan_x = 0;
            window_state(h).pan_y = 0;
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        toolbar::WM_COMMAND => {
            toolbar_action(h, w & 0xffff);
            0
        }
        0x0005 => {
            // Rc keeps native controls owned during synchronous SetWindowPos
            // callbacks without retaining a borrow into the parent's state.
            let bar = window_state(h).toolbar.clone();
            if let Some(bar) = bar {
                let mut rect: Rect = zeroed();
                GetClientRect(h, &mut rect);
                bar.resize(rect.right - rect.left);
                InvalidateRect(h, std::ptr::null(), 1);
            }
            0
        }
        0x0100 if w == 0x43 && GetKeyState(0x11) < 0 => {
            copy_current(h);
            0
        }
        0x0100 if w == 0x53 && GetKeyState(0x11) < 0 => {
            save(h, GetKeyState(0x10) < 0);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x5a || w == 0x59) => {
            history(h, w == 0x59);
            0
        }
        0x0100 if w == 0x2e || w == 0x08 => {
            if editor(h).delete_selected() {
                window_state(h).dirty = true;
                InvalidateRect(h, std::ptr::null(), 0);
            }
            report_editor_error(h);
            0
        }
        0x0100 if w == 0x1b => {
            if editor(h).selected_index().is_some() {
                cancel_gesture(h);
                editor(h).clear_selection();
            } else if window_state(h).drawing {
                cancel_gesture(h);
            } else if editor(h).tool() != Tool::Select {
                cancel_gesture(h);
                editor(h).set_tool(Tool::Select);
                toolbar::set_active(h, toolbar::ID_SELECT);
            } else {
                return 0;
            }
            report_editor_error(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0100 if matches!(w, 0x25..=0x28)
            && GetKeyState(0x11) >= 0
            && GetKeyState(0x12) >= 0
            && !window_state(h).drawing
            && editor(h).selected_index().is_some() =>
        {
            let step = if GetKeyState(0x10) < 0 { 10.0 } else { 1.0 };
            let (dx, dy) = match w {
                0x25 => (-step, 0.0),
                0x26 => (0.0, -step),
                0x27 => (step, 0.0),
                _ => (0.0, step),
            };
            if editor(h).nudge_selected(dx, dy) {
                window_state(h).dirty = true;
                InvalidateRect(h, std::ptr::null(), 0);
            }
            report_editor_error(h);
            0
        }
        0x0100 if GetKeyState(0x11) >= 0 => {
            let tool = match w {
                0x56 => Some(Tool::Select),
                0x41 => Some(Tool::Arrow),
                0x4c => Some(Tool::Line),
                0x52 => Some(Tool::Rectangle),
                0x45 => Some(Tool::Ellipse),
                0x54 => Some(Tool::Text),
                0x4d => Some(Tool::Highlight),
                0x42 => Some(Tool::Blur),
                0x4e => Some(Tool::Step),
                0x50 => Some(Tool::Pen),
                0x43 => Some(Tool::Crop),
                _ => None,
            };
            if let Some(tool) = tool {
                cancel_gesture(h);
                editor(h).set_tool(tool);
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
            0
        }
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
                return 0;
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
                    return 0;
                }
            }
            window_state(h).drawing = true;
            if editor(h).tool() == Tool::Crop {
                window_state(h).crop_start = Some((x, y));
                window_state(h).crop_end = Some((x, y));
                SetCapture(h);
                InvalidateRect(h, std::ptr::null(), 0);
                return 0;
            }
            editor(h).mouse_down(ShapePoint::new(x as f32, y as f32));
            SetCapture(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0200 if window_state(h).drawing && !window_state(h).bitmap.is_null() => {
            let (x, y) = point(h, l);
            if matches!(window_state(h).crop_start, Some(_)) {
                window_state(h).crop_end = Some((x, y));
                InvalidateRect(h, std::ptr::null(), 0);
                return 0;
            }
            editor(h).mouse_move_with_shift(
                ShapePoint::new(x as f32, y as f32),
                GetKeyState(0x10) < 0,
            );
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
                return 0;
            }
            let before = editor(h).shapes().to_vec();
            editor(h).mouse_up_with_shift(
                ShapePoint::new(x as f32, y as f32),
                GetKeyState(0x10) < 0,
            );
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
                            ok = StretchBlt(dc, x, y, w, height, mem, 0, 0, window_state(h).width, window_state(h).height, COPY) != 0;
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
                let message = wide("Bild öffnen: Strg+O · Bild einfügen: Strg+V · Aufnahme: F9 oder F11");
                TextOutW(dc, 12, toolbar_height(h) + 15, message.as_ptr(), (message.len() - 1) as i32);
            }
            EndPaint(h, &ps);
            0
        }
        0x0010 => {
            if h == PRIMARY { quit_application(h); }
            else if !QUITTING.load(Ordering::SeqCst) && confirm_replace(h) { DestroyWindow(h); }
            0
        }
        0x0002 => {
            let toolbar = window_state(h).toolbar.take();
            drop(toolbar); // DestroyWindow(child) may synchronously reenter this parent.
            let bitmap = std::mem::replace(&mut window_state(h).bitmap, null_mut());
            if !bitmap.is_null() { DeleteObject(bitmap); }
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).retain(|window| *window != h);
            if h == PRIMARY {
                hotkeys::unregister(h);
                tray::remove(h);
                PostQuitMessage(0);
            }
            0
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}
