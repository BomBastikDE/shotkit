use super::*;

// SAFETY CONTRACT:
// Voraussetzung: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig.
// Warum erfüllt bzw. offen: window_proc wird von Windows auf dem Besitzerthread
// aufgerufen; Panic-/Lebensdauerverträge bleiben für alle Aufrufer zu beachten.
// Nachrichten: Manche Zweige können synchron reentrieren; State-Borrows enden davor.
pub(super) unsafe fn handle_mouse(hwnd: H, msg: u32, _w: W, l: L) -> Option<L> {
    let state = std::ptr::addr_of_mut!(STATE);
    Some(match msg {
        WM_ERASEBKGND => 1,
        WM_LBUTTONDOWN => {
            let point = cursor_screen(l, (*state).origin);
            let should_capture = {
                let s = &mut *state;
                if s.down {
                    false
                } else {
                    // Any new pointer gesture returns to the established
                    // hover/click/drag flow and discards keyboard-only edits.
                    s.down = true;
                    s.start = point;
                    s.current = point;
                    // Freeze the rectangle that was already visible. In-flight
                    // UIA completions cannot change the accepted click target.
                    // A keyboard edit is the rectangle the user is currently
                    // looking at, even though visible_hover_rect still tracks
                    // the last pointer hover. Freeze that edited rectangle for
                    // a click before clearing keyboard mode; real drags still
                    // take the manual start/current path on mouse-up.
                    s.press_rect = s
                        .keyboard_rect
                        .or(s.visible_hover_rect)
                        .filter(|rect| contains(*rect, point));
                    s.keyboard_rect = None;
                    crate::uia_picker::log_diagnostic(&format!(
                        "selection mouse_down point=({}, {}) visible_rect={:?} frozen_rect={:?}",
                        point.x, point.y, s.visible_hover_rect, s.press_rect
                    ));
                    s.pending_click = None;
                    s.window_point = point;
                    s.window_confirmed_sequence = 0;
                    s.window_confirmed_root = 0;
                    s.window_sequence = s
                        .window_picker
                        .as_ref()
                        .map(|worker| {
                            worker.submit(
                                element_picker::ScreenPoint {
                                    x: point.x,
                                    y: point.y,
                                },
                                0,
                            )
                        })
                        .unwrap_or(0);
                    true
                }
            };
            if should_capture {
                SetCapture(hwnd);
            }
            InvalidateRect(hwnd, null(), 0);
            move_magnifier(state);
            0
        }
        WM_MOUSEMOVE => {
            let point = cursor_screen(l, (*state).origin);
            let is_down = (*state).down;
            if is_down {
                (*state).current = point;
            } else if point.x != (*state).current.x || point.y != (*state).current.y {
                // Pointer movement resumes hover selection after keyboard edit.
                (*state).keyboard_rect = None;
                update_hover(state, point);
            }
            InvalidateRect(hwnd, null(), 0);
            move_magnifier(state);
            0
        }
        WM_LBUTTONUP if (*state).down => {
            let point = cursor_screen(l, (*state).origin);
            let (result, click_pending, click_cancelled, root_hwnd, edit_manual_drag) = {
                let s = &mut *state;
                s.current = point;
                let dx = (i64::from(s.start.x) - i64::from(s.current.x)).abs();
                let dy = (i64::from(s.start.y) - i64::from(s.current.y)).abs();
                let click = dx <= i64::from(CLICK_SLOP) && dy <= i64::from(CLICK_SLOP);
                let root_hwnd = s
                    .candidates
                    .iter()
                    .find(|c| c.is_top_level)
                    .map(|c| c.hwnd)
                    .unwrap_or(0);
                let result = if click {
                    s.press_rect
                } else {
                    Some((
                        s.start.x.min(s.current.x),
                        s.start.y.min(s.current.y),
                        s.start.x.max(s.current.x),
                        s.start.y.max(s.current.y),
                    ))
                };
                s.down = false;
                // Holding Shift while releasing a real drag freezes that
                // manually drawn area for pixel correction. Without Shift,
                // drags and hover clicks still finish immediately as before.
                let editable = result.and_then(|r| intersection(r, desktop_rect(s)));
                let edit_manual_drag = !click && is_shift_down() && editable.is_some();
                if edit_manual_drag {
                    s.keyboard_rect = editable;
                }
                let fresh_scan =
                    s.window_confirmed_sequence == s.window_sequence && s.window_sequence != 0;
                let fresh = fresh_scan && s.window_confirmed_root == root_hwnd;
                let valid_click = click && result.is_some_and(|r| contains(r, point));
                let pending = !edit_manual_drag && valid_click && !fresh_scan && root_hwnd != 0;
                let cancelled = click
                    && (!valid_click || (fresh_scan && !fresh) || (!fresh_scan && root_hwnd == 0));
                if pending {
                    s.pending_click = Some(PendingClick {
                        rect: result.unwrap(),
                        point,
                        root_hwnd,
                    });
                }
                (result, pending, cancelled, root_hwnd, edit_manual_drag)
            };
            ReleaseCapture();
            move_magnifier(state);
            crate::uia_picker::log_diagnostic(&format!(
                "selection mouse_up point=({}, {}) click_pending={} cancelled={} result={:?}",
                point.x, point.y, click_pending, click_cancelled, result
            ));
            if !click_pending && !edit_manual_drag {
                let final_result = if click_cancelled { None } else { result }
                    .and_then(|r| intersection(r, desktop_rect(&*state)));
                let target = mark_finished(state, final_result);
                if !target.is_null() {
                    let _ = KillTimer(target, UIA_TIMER_ID);
                    DestroyWindow(target);
                }
            }
            if edit_manual_drag {
                InvalidateRect(hwnd, null(), 0);
            }
            let _ = root_hwnd;
            0
        }
        _ => return None,
    })
}
// SAFETY CONTRACT:
// Voraussetzung: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig.
// Warum erfüllt bzw. offen: window_proc wird von Windows auf dem Besitzerthread
// aufgerufen; Panic-/Lebensdauerverträge bleiben für alle Aufrufer zu beachten.
// Nachrichten: Manche Zweige können synchron reentrieren; State-Borrows enden davor.
pub(super) unsafe fn handle_keyboard(hwnd: H, msg: u32, w: W, _l: L) -> Option<L> {
    let state = std::ptr::addr_of_mut!(STATE);
    Some(match msg {
        WM_KEYDOWN if w == VK_ESCAPE => {
            let target = mark_finished(state, None);
            if !target.is_null() {
                let _ = KillTimer(target, UIA_TIMER_ID);
                DestroyWindow(target);
            }
            0
        }
        WM_KEYDOWN if w == VK_RETURN => {
            if (*state).down {
                return Some(0);
            }
            let rect = {
                let s = &*state;
                s.keyboard_rect.or(s.visible_hover_rect).or(s.hover_rect)
            };
            let desktop = desktop_rect(&*state);
            let target = mark_finished(state, rect.and_then(|r| intersection(r, desktop)));
            if !target.is_null() {
                let _ = KillTimer(target, UIA_TIMER_ID);
                DestroyWindow(target);
            }
            0
        }
        WM_KEYDOWN if matches!(w, VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN) => {
            if !(*state).down {
                let bounds = desktop_rect(&*state);
                let current = (*state)
                    .keyboard_rect
                    .or((*state).visible_hover_rect)
                    .or((*state).hover_rect);
                if let Some(rect) = current {
                    let (dx, dy) = match w {
                        VK_LEFT => (-1, 0),
                        VK_UP => (0, -1),
                        VK_RIGHT => (1, 0),
                        _ => (0, 1),
                    };
                    let edited = if is_shift_down() {
                        selection_rect::resize(rect, dx, dy, bounds)
                    } else {
                        selection_rect::translate(rect, dx, dy, bounds)
                    };
                    (*state).keyboard_rect = Some(edited);
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_KEYDOWN | WM_KEYUP if w == VK_CONTROL as W => {
            choose_hover(state);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if w == UIA_TIMER_ID => {
            let scan_handle = poll_window(state);
            if (*state).done {
                let target = scan_handle
                    .filter(|h| !h.is_null())
                    .unwrap_or((*state).hwnd);
                if !target.is_null() {
                    let _ = KillTimer(target, UIA_TIMER_ID);
                    DestroyWindow(target);
                }
            } else if scan_handle.is_some()
                || (!(*state).down && (*state).pending_click.is_none() && poll_uia(state))
            {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        _ => return None,
    })
}
