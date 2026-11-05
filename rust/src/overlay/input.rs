use super::*;

const WM_CANCELMODE: u32 = 0x001f;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_RBUTTONDOWN: u32 = 0x0204;

#[derive(Debug, PartialEq)]
enum PointerGesture {
    Click,
    EmptyDrag,
    Drag((i32, i32, i32, i32)),
}

fn pointer_gesture(start: Point, current: Point) -> PointerGesture {
    let dx = (i64::from(start.x) - i64::from(current.x)).abs();
    let dy = (i64::from(start.y) - i64::from(current.y)).abs();
    if dx <= i64::from(CLICK_SLOP) && dy <= i64::from(CLICK_SLOP) {
        PointerGesture::Click
    } else if dx == 0 || dy == 0 {
        PointerGesture::EmptyDrag
    } else {
        PointerGesture::Drag((
            start.x.min(current.x),
            start.y.min(current.y),
            start.x.max(current.x),
            start.y.max(current.y),
        ))
    }
}

/// End an active pointer gesture before any API that can synchronously send
/// WM_CAPTURECHANGED. Returns whether there was a gesture to cancel.
fn end_drag_for_capture_loss(state: &mut State) -> bool {
    if !state.down {
        return false;
    }
    state.down = false;
    state.press_rect = None;
    state.pending_click = None;
    true
}

// SAFETY CONTRACT:
// Voraussetzung: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig.
// Warum erfüllt bzw. offen: window_proc wird von Windows auf dem Besitzerthread
// aufgerufen; Panic-/Lebensdauerverträge bleiben für alle Aufrufer zu beachten.
// Nachrichten: SetCapture, ReleaseCapture und MoveWindow können reentrieren; die Zustandsblöcke enden vor diesen Aufrufen.
pub(super) unsafe fn handle_mouse(hwnd: H, msg: u32, _w: W, l: L) -> Option<L> {
    let state = std::ptr::addr_of_mut!(STATE);
    Some(match msg {
        WM_RBUTTONDOWN => {
            // Match the legacy overlay: right-click aborts the capture.
            // Clear gesture state before releasing capture because that can
            // synchronously reenter through WM_CAPTURECHANGED.
            if end_drag_for_capture_loss(&mut *state) {
                ReleaseCapture();
            }
            let target = mark_finished(state, None);
            if !target.is_null() {
                let _ = KillTimer(target, UIA_TIMER_ID);
                DestroyWindow(target);
            }
            0
        }
        WM_CAPTURECHANGED | WM_CANCELMODE if (*state).down => {
            // Clear `down` before ReleaseCapture: that call may reenter this
            // callback with WM_CAPTURECHANGED. The reentrant notification
            // then becomes a harmless no-op instead of finishing twice.
            let cancelled = end_drag_for_capture_loss(&mut *state);
            if msg == WM_CANCELMODE && cancelled {
                ReleaseCapture();
            }
            let target = mark_finished(state, None);
            if !target.is_null() {
                let _ = KillTimer(target, UIA_TIMER_ID);
                DestroyWindow(target);
            }
            0
        }
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
            let (result, click_pending, click_cancelled, root_hwnd, edit_manual_drag, retry_drag) = {
                let s = &mut *state;
                s.current = point;
                let gesture = pointer_gesture(s.start, s.current);
                let click = gesture == PointerGesture::Click;
                let retry_drag = gesture == PointerGesture::EmptyDrag;
                let root_hwnd = s
                    .candidates
                    .iter()
                    .find(|c| c.is_top_level)
                    .map(|c| c.hwnd)
                    .unwrap_or(0);
                let result = match gesture {
                    PointerGesture::Click => s.press_rect,
                    PointerGesture::EmptyDrag => None,
                    PointerGesture::Drag(rect) => Some(rect),
                };
                s.down = false;
                if retry_drag {
                    // Discard this empty gesture, leaving the overlay ready
                    // for another drag. Clear `down` before ReleaseCapture
                    // so its reentrant WM_CAPTURECHANGED cannot cancel it.
                    s.press_rect = None;
                    s.pending_click = None;
                }
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
                (
                    result,
                    pending,
                    cancelled,
                    root_hwnd,
                    edit_manual_drag,
                    retry_drag,
                )
            };
            ReleaseCapture();
            // ReleaseCapture may synchronously destroy the overlay or start a
            // new gesture. Never resume this gesture against a changed state.
            if retry_drag {
                if (*state).done
                    || (*state).hwnd != hwnd
                    || (*state).down
                    || (*state).current.x != point.x
                    || (*state).current.y != point.y
                {
                    return Some(0);
                }
                // The old outline may contain the release point (for example,
                // a parent window around a child). Do not freeze it on a click
                // while the new endpoint scan is still pending.
                (*state).candidates.clear();
                (*state).target_hwnd = 0;
                (*state).hover_rect = None;
                (*state).visible_hover_rect = None;
                update_hover(state, point);
            }
            move_magnifier(state);
            // MoveWindow in move_magnifier can also reenter and finish or
            // replace this gesture, so validate again before touching STATE.
            if retry_drag
                && ((*state).done
                    || (*state).hwnd != hwnd
                    || (*state).down
                    || (*state).current.x != point.x
                    || (*state).current.y != point.y)
            {
                return Some(0);
            }
            crate::uia_picker::log_diagnostic(&format!(
                "selection mouse_up point=({}, {}) click_pending={} cancelled={} result={:?}",
                point.x, point.y, click_pending, click_cancelled, result
            ));
            if !click_pending && !edit_manual_drag && !retry_drag {
                let final_result = if click_cancelled { None } else { result }
                    .and_then(|r| intersection(r, desktop_rect(&*state)));
                let target = mark_finished(state, final_result);
                if !target.is_null() {
                    let _ = KillTimer(target, UIA_TIMER_ID);
                    DestroyWindow(target);
                }
            }
            if edit_manual_drag || retry_drag {
                InvalidateRect(hwnd, null(), 0);
            }
            let _ = root_hwnd;
            0
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_dimensional_drags_are_empty_in_either_direction() {
        let start = Point { x: 100, y: 100 };
        for current in [
            Point { x: 200, y: 100 },
            Point { x: 0, y: 100 },
            Point { x: 100, y: 200 },
            Point { x: 100, y: 0 },
        ] {
            assert_eq!(pointer_gesture(start, current), PointerGesture::EmptyDrag);
        }
    }

    #[test]
    fn clicks_keep_the_existing_slop_including_zero_dimensions() {
        let start = Point { x: 100, y: 100 };
        for dx in -CLICK_SLOP..=CLICK_SLOP {
            for dy in -CLICK_SLOP..=CLICK_SLOP {
                assert_eq!(
                    pointer_gesture(
                        start,
                        Point {
                            x: 100 + dx,
                            y: 100 + dy
                        }
                    ),
                    PointerGesture::Click
                );
            }
        }
    }

    #[test]
    fn real_rectangles_keep_normalized_bounds_including_thin_drags() {
        let start = Point { x: 100, y: 100 };
        for (current, rect) in [
            (Point { x: 200, y: 150 }, (100, 100, 200, 150)),
            (Point { x: 0, y: 50 }, (0, 50, 100, 100)),
            (Point { x: 101, y: 200 }, (100, 100, 101, 200)),
            (Point { x: 200, y: 101 }, (100, 100, 200, 101)),
        ] {
            assert_eq!(pointer_gesture(start, current), PointerGesture::Drag(rect));
        }
        assert_eq!(
            pointer_gesture(Point { x: i32::MIN, y: 0 }, Point { x: i32::MAX, y: 1 }),
            PointerGesture::Drag((i32::MIN, 0, i32::MAX, 1))
        );
    }

    #[test]
    fn capture_loss_ends_drag_and_discards_incomplete_click_state() {
        let mut state = State {
            down: true,
            press_rect: Some((1, 2, 3, 4)),
            pending_click: Some(PendingClick {
                rect: (1, 2, 3, 4),
                point: Point { x: 2, y: 3 },
                root_hwnd: 1,
            }),
            ..State::default()
        };

        assert!(end_drag_for_capture_loss(&mut state));
        assert!(!state.down);
        assert_eq!(state.press_rect, None);
        assert!(state.pending_click.is_none());
    }

    #[test]
    fn capture_loss_cleanup_is_idempotent_for_releasecapture_reentrancy() {
        let mut state = State {
            down: true,
            ..State::default()
        };

        assert!(end_drag_for_capture_loss(&mut state));
        assert!(!end_drag_for_capture_loss(&mut state));
        assert!(!state.down);
    }
}
// SAFETY CONTRACT:
// Voraussetzung: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig.
// Warum erfüllt bzw. offen: window_proc wird von Windows auf dem Besitzerthread
// aufgerufen; Panic-/Lebensdauerverträge bleiben für alle Aufrufer zu beachten.
// Nachrichten: DestroyWindow kann reentrieren; Rechtecke und Handles werden vorher kopiert.
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
