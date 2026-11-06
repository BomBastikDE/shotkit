use super::*;

// SAFETY CONTRACT:
// Voraussetzung: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc prüft Userdata; die Lebensdauer bei Reentranz
// und der Vertrag von window_state bleiben für jeden Aufruf zu beachten.
// Nachrichten: DestroyWindow, ShowWindow und Menü-/Aufnahmefunktionen können reentrieren; der Handler hält keine Zustandsreferenz über diese Aufrufe.
pub(super) unsafe fn dispatch_system(h: H, m: u32, w: W, l: L) -> Option<L> {
    if let Some(result) = scroll_completion::dispatch(h, m, w, |ticket| {
        scroll_completion::finish_scroll(ticket);
    }) {
        return Some(result);
    }
    Some(match m {
        WM_FINISH_QUIT if h == PRIMARY => {
            if take_quit_completion(&FINISH_QUIT_READY, window_state(h).modal_depth) {
                DestroyWindow(h);
            }
            0
        }
        WM_COPYDATA => handle_instance_file(h, l),
        WM_INSTANCE_ACTIVATE => {
            show_editors();
            1
        }
        0x0312 if hotkeys::dispatch(h, w) => 0,
        TRAY if l as u32 == 0x0202 => {
            capture_region(h);
            0
        }
        DELAYED_CAPTURE => {
            let Some(ticket) = CaptureTicket::from_wire(w) else {
                return Some(0);
            };
            if !DELAYED_CAPTURE_STATE
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .try_start(ticket)
            {
                return Some(0);
            }
            // Running remains pending through accepted, cancelled and failed
            // selections; duplicate messages are rejected during modal loops.
            capture_region(h);
            DELAYED_CAPTURE_STATE
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .finish(ticket);
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
        _ => return None,
    })
}

// SAFETY CONTRACT:
// Voraussetzung: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc prüft Userdata; die Lebensdauer bei Reentranz
// und der Vertrag von window_state bleiben für jeden Aufruf zu beachten.
// Nachrichten: Dateidialoge, Aufnahme und SetWindowTextW können reentrieren; verwendete Zustandswerte sind vor dem Aufruf kopiert.
pub(super) unsafe fn dispatch_shortcuts(h: H, m: u32, w: W, _l: L) -> Option<L> {
    Some(match m {
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
            toolbar_action(h, toolbar::ID_ZOOM_IN);
            0
        }
        0x0100 if GetKeyState(0x11) < 0 && (w == 0x6d || w == 0xbd) => {
            toolbar_action(h, toolbar::ID_ZOOM_OUT);
            0
        }
        0x020a if !window_state(h).bitmap.is_null() => {
            let delta = ((w >> 16) as u16 as i16) as i32;
            let action = view::wheel_action(w);
            let diagnostic = std::env::var_os("SHOTKIT_UIA_DIAGNOSTICS_PATH").is_some();
            let before = if diagnostic {
                let state = window_state(h);
                Some((state.pan_x, state.pan_y, state.zoom))
            } else {
                None
            };
            if action == view::WheelAction::Zoom {
                window_state(h).zoom = view::zoom_after_wheel(window_state(h).zoom, delta);
            } else {
                let mut client: Rect = zeroed();
                GetClientRect(h, &mut client);
                let viewport = (
                    0,
                    toolbar_height(h),
                    client.right,
                    client.bottom - toolbar_height(h),
                );
                let state = window_state(h);
                let pan = view::pan_after_wheel(
                    (state.width as u32, state.height as u32),
                    viewport,
                    state.zoom,
                    (state.pan_x, state.pan_y),
                    delta,
                    action == view::WheelAction::PanHorizontal,
                );
                state.pan_x = pan.0;
                state.pan_y = pan.1;
            }
            if let Some((pan_x, pan_y, zoom)) = before {
                let (after_x, after_y, after_zoom) = {
                    let state = window_state(h);
                    (state.pan_x, state.pan_y, state.zoom)
                };
                let mode = match action {
                    view::WheelAction::Zoom => 0,
                    view::WheelAction::PanHorizontal => 1,
                    view::WheelAction::PanVertical => 2,
                };
                crate::uia_picker::log_diagnostic(&format!(
                    "wheel hwnd={} flags={} delta={} mode={} pan_before=({}, {}) zoom_before={} pan_after=({}, {}) zoom_after={}",
                    h as usize, w & 0xffff, delta, mode, pan_x, pan_y, zoom, after_x, after_y, after_zoom
                ));
            }
            InvalidateRect(h, std::ptr::null(), 1);
            0
        }
        0x0100
            if GetKeyState(0x11) < 0
                && view::zoom_reset_key(w) == Some(view::ZoomAction::Original) =>
        {
            toolbar_action(h, toolbar::ID_ZOOM_ORIGINAL);
            0
        }
        0x0100
            if GetKeyState(0x11) < 0
                && view::zoom_reset_key(w) == Some(view::ZoomAction::Fit)
                && !window_state(h).bitmap.is_null() =>
        {
            toolbar_action(h, toolbar::ID_ZOOM_FIT);
            0
        }
        toolbar::WM_COMMAND => {
            toolbar_action(h, w & 0xffff);
            0
        }
        _ => return None,
    })
}

// SAFETY CONTRACT:
// Voraussetzung: Der Fensterzustand und die jeweiligen Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc prüft Userdata; die Lebensdauer bei Reentranz
// und der Vertrag von window_state bleiben für jeden Aufruf zu beachten.
// Nachrichten: Toolbar-Resize, SetWindowTextW und Dialoge können reentrieren; die Toolbar wird als Rc geklont und Borrows enden zuvor.
pub(super) unsafe fn dispatch_editor_keys(h: H, m: u32, w: W, _l: L) -> Option<L> {
    Some(match m {
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
            if window_state(h).drawing {
                cancel_gesture(h);
            }
            let mut changed = false;
            if apply_annotation_edit(h, |state| changed = state.delete_selected()) && changed {
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
                return Some(0);
            }
            report_editor_error(h);
            InvalidateRect(h, std::ptr::null(), 0);
            0
        }
        0x0100
            if matches!(w, 0x25..=0x28)
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
            let mut changed = false;
            if apply_annotation_edit(h, |state| changed = state.nudge_selected(dx, dy)) && changed {
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
                0x57 => Some(Tool::GaussianBlur),
                0x53 => Some(Tool::Redact),
                0x4e => Some(Tool::Step),
                0x50 => Some(Tool::Pen),
                0x43 => Some(Tool::Crop),
                _ => None,
            };
            if let Some(tool) = tool {
                activate_tool(h, tool);
            }
            0
        }
        _ => return None,
    })
}
