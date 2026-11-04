use super::*;

// SAFETY CONTRACT:
// Voraussetzung: Windows-Callbackparameter und der serialisierte Overlayzustand sind gültig.
// Warum erfüllt bzw. offen: window_proc wird von Windows auf dem Besitzerthread
// aufgerufen; Panic-/Lebensdauerverträge bleiben für alle Aufrufer zu beachten.
// Nachrichten: DestroyWindow kann reentrieren; das Magnifier-Handle wird vorher aus dem Zustand entnommen.
pub(super) unsafe fn handle_paint(hwnd: H, msg: u32, _w: W, _l: L) -> Option<L> {
    let state = std::ptr::addr_of_mut!(STATE);
    Some(match msg {
        WM_PAINT => {
            let (screen_rect, desktop) = {
                let s = &*state;
                let screen_rect = if let Some(rect) = s.keyboard_rect {
                    Some(rect)
                } else if s.down {
                    Some((
                        s.start.x.min(s.current.x),
                        s.start.y.min(s.current.y),
                        s.start.x.max(s.current.x),
                        s.start.y.max(s.current.y),
                    ))
                } else if let Some(click) = s.pending_click {
                    Some(click.rect)
                } else {
                    s.hover_rect
                };
                (screen_rect, desktop_rect(s))
            };
            let mut paint: Paint = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut client: Rect = zeroed();
            let _ = GetClientRect(hwnd, &mut client);
            let bg = CreateSolidBrush(0x0000_0000);
            let _ = FillRect(dc, &client, bg);
            let _ = DeleteObject(bg);

            let screen_rect = screen_rect.and_then(|r| intersection(r, desktop));
            if let Some(screen) = screen_rect {
                let origin = (*state).origin;
                let local = Rect {
                    left: screen.0.saturating_sub(origin.0),
                    top: screen.1.saturating_sub(origin.1),
                    right: screen.2.saturating_sub(origin.0),
                    bottom: screen.3.saturating_sub(origin.1),
                };
                let down = (*state).down;
                let brush = CreateSolidBrush(if down { 0x00ff_ffff } else { 0x0000_ff00 });
                let _ = FrameRect(dc, &local, brush);
                let _ = DeleteObject(brush);
            }
            let _ = EndPaint(hwnd, &paint);
            if !(*state).down && (*state).keyboard_rect.is_none() {
                (*state).visible_hover_rect = screen_rect;
            }
            0
        }
        WM_DESTROY => {
            let (timer_hwnd, magnifier) = {
                let s = &mut *state;
                s.done = true;
                if s.hwnd == hwnd {
                    s.hwnd = null_mut();
                }
                let magnifier = s.magnifier_hwnd;
                s.magnifier_hwnd = null_mut();
                (hwnd, magnifier)
            };
            wake_overlay_message_loop();
            let _ = KillTimer(timer_hwnd, UIA_TIMER_ID);
            if !magnifier.is_null() {
                DestroyWindow(magnifier);
            }
            0
        }
        _ => return None,
    })
}
