use super::*;

pub(super) fn valid_rect(rect: (i32, i32, i32, i32)) -> bool {
    rect.2 > rect.0 && rect.3 > rect.1
}

pub(super) fn contains(rect: (i32, i32, i32, i32), p: Point) -> bool {
    p.x >= rect.0 && p.x < rect.2 && p.y >= rect.1 && p.y < rect.3
}

pub(super) fn intersection(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let rect = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    valid_rect(rect).then_some(rect)
}

pub(super) fn cursor_screen(l: L, origin: (i32, i32)) -> Point {
    let (x, y) = unpack(l);
    Point {
        x: x.saturating_add(origin.0),
        y: y.saturating_add(origin.1),
    }
}

// SAFETY CONTRACT:
// Voraussetzung: GetKeyState wird mit einem gültigen Tastencode aufgerufen.
// Warum erfüllt bzw. offen: VK_CONTROL ist eine Konstante; keine fremden Pointer werden übergeben.
// Nachrichten: Kein expliziter Nachrichtenversand.
pub(super) unsafe fn is_control_down() -> bool {
    (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0
}

// SAFETY CONTRACT:
// Voraussetzung: GetKeyState wird mit einem gültigen Tastencode aufgerufen.
// Warum erfüllt bzw. offen: VK_SHIFT ist eine Konstante; keine fremden Pointer werden übergeben.
// Nachrichten: Kein expliziter Nachrichtenversand.
pub(super) unsafe fn is_shift_down() -> bool {
    (GetKeyState(VK_SHIFT) as u16 & 0x8000) != 0
}

pub(super) fn desktop_rect(s: &State) -> (i32, i32, i32, i32) {
    (
        s.origin.0,
        s.origin.1,
        s.origin.0.saturating_add(s.size.0),
        s.origin.1.saturating_add(s.size.1),
    )
}

pub(super) fn root_candidate(s: &State) -> Option<(i32, i32, i32, i32)> {
    s.candidates.iter().find(|c| c.is_top_level).map(|c| c.rect)
}

// SAFETY CONTRACT:
// Voraussetzung: state ist gültig, initialisiert und für die Dauer exklusiv auf dem UI-Thread.
// Warum erfüllt bzw. offen: Der Aufrufer verwendet den serialisierten Overlay-Zustand; submit übergibt Werte an den Worker.
// Nachrichten: Kein geprüfter synchroner Fensteraufruf im Borrow.
pub(super) unsafe fn update_hover(state: *mut State, point: Point) {
    let s = &mut *state;
    s.current = point;
    s.uia_rect = None;
    s.request_sequence = 0;
    s.window_confirmed_sequence = 0;
    if let Some(worker) = &s.window_picker {
        s.window_point = point;
        s.window_sequence = worker.submit(
            element_picker::ScreenPoint {
                x: point.x,
                y: point.y,
            },
            0,
        );
        // Keep the last drawn outline until the sequenced scan replaces it.
        // MouseDown still freezes only a rectangle containing the actual point,
        // then requires a fresh root-HWND confirmation before accepting it.
    } else {
        s.candidates.clear();
        s.target_hwnd = 0;
        s.hover_rect = None;
        s.visible_hover_rect = None;
    }
}

// SAFETY CONTRACT:
// Voraussetzung: state ist gültig und alle Änderungen erfolgen exklusiv auf dem Overlay-Thread.
// Warum erfüllt bzw. offen: Workerantworten sind eigene Werte; Borrows enden vor weiteren Zustandsaktionen.
// Nachrichten: Kein expliziter synchroner Fensteraufruf im geprüften Pfad.
pub(super) unsafe fn poll_window(state: *mut State) -> Option<H> {
    let scan = {
        let s = &*state;
        s.window_picker
            .as_ref()
            .and_then(|worker| worker.try_take_latest())
    }?;
    let (window_seq, point, down) = {
        let s = &*state;
        (s.window_sequence, s.window_point, s.down)
    };
    if scan.sequence != window_seq || scan.point.x != point.x || scan.point.y != point.y {
        return None;
    }
    let candidates = scan.candidates.unwrap_or_default();
    let root = candidates.iter().find(|c| c.is_top_level).copied();
    uia_picker::log_diagnostic(&format!(
        "selection window_scan seq={} point=({}, {}) root_hwnd={} root_rect={:?} candidate_count={}",
        scan.sequence,
        scan.point.x,
        scan.point.y,
        root.map(|c| c.hwnd).unwrap_or(0),
        root.map(|c| c.rect),
        candidates.len()
    ));
    let (finish_result, should_choose) = {
        let s = &mut *state;
        s.window_confirmed_sequence = scan.sequence;
        s.window_confirmed_root = root.map(|candidate| candidate.hwnd).unwrap_or(0);
        if let Some(click) = s.pending_click {
            let matching = element_picker::confirms_click(
                click.root_hwnd,
                root,
                element_picker::ScreenPoint {
                    x: click.point.x,
                    y: click.point.y,
                },
            );
            if matching {
                s.pending_click = None;
                (Some(Some(click.rect)), false)
            } else {
                s.pending_click = None;
                (Some(None), false)
            }
        } else if down {
            (None, false)
        } else {
            s.candidates = candidates;
            s.target_hwnd = root.map(|candidate| candidate.hwnd).unwrap_or(0);
            s.uia_rect = None;
            s.request_sequence = if s.target_hwnd != 0 {
                s.uia
                    .as_ref()
                    .map(|uia| {
                        uia.submit(
                            s.target_hwnd,
                            uia_picker::ScreenPoint {
                                x: point.x,
                                y: point.y,
                            },
                        )
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            (None, true)
        }
    };
    if should_choose {
        choose_hover(state);
    }
    if let Some(result) = finish_result {
        let rect = result.and_then(|r| intersection(r, desktop_rect(&*state)));
        let hwnd = mark_finished(state, rect);
        return Some(hwnd);
    }
    Some(null_mut())
}

// SAFETY CONTRACT:
// Voraussetzung: state ist gültig und exklusiv auf dem Overlay-Thread.
// Warum erfüllt bzw. offen: Nur kopierte Rechtecke und Zustandsfelder werden verwendet; GetKeyState erfolgt vorher.
// Nachrichten: Kein expliziter synchroner Fensteraufruf im geprüften Pfad.
pub(super) unsafe fn choose_hover(state: *mut State) {
    let control_down = is_control_down();
    let s = &mut *state;
    let fallback = if control_down {
        root_candidate(s)
    } else {
        s.candidates
            .iter()
            .rev()
            .find(|candidate| !candidate.is_top_level)
            .map(|candidate| candidate.rect)
            .or_else(|| root_candidate(s))
    };
    let accepted_uia = s.uia_rect.filter(|r| contains(*r, s.current));
    let rect = if !control_down {
        accepted_uia.or(fallback)
    } else {
        fallback
    };
    s.hover_rect = rect.and_then(|r| intersection(r, desktop_rect(s)));
    crate::uia_picker::log_diagnostic(&format!(
        "selection hover point=({}, {}) ctrl={} uia_rect={:?} fallback_rect={:?} chosen_rect={:?}",
        s.current.x,
        s.current.y,
        control_down,
        accepted_uia,
        fallback,
        s.hover_rect
    ));
}

// SAFETY CONTRACT:
// Voraussetzung: state ist gültig und exklusiv auf dem Overlay-Thread.
// Warum erfüllt bzw. offen: Der Worker liefert eigene Werte; der letzte Borrow endet vor choose_hover.
// Nachrichten: Kein expliziter synchroner Fensteraufruf im geprüften Pfad.
pub(super) unsafe fn poll_uia(state: *mut State) -> bool {
    let response = {
        let s = &*state;
        s.uia.as_ref().and_then(|uia| uia.try_take_latest())
    };
    let Some(response) = response else {
        return false;
    };
    let s = &mut *state;
    if response.sequence != s.request_sequence || response.target_hwnd != s.target_hwnd {
        return false;
    }
    s.uia_rect = response
        .rect
        .map(|r| (r.left, r.top, r.right, r.bottom))
        .filter(|r| valid_rect(*r) && contains(*r, s.current))
        .and_then(|r| {
            let root = root_candidate(s).unwrap_or(desktop_rect(s));
            intersection(r, root)
        });
    crate::uia_picker::log_diagnostic(&format!(
        "selection uia_response seq={} hwnd=0x{:X} rect={:?} issue={:?} visited={} accepted_rect={:?}",
        response.sequence,
        response.target_hwnd,
        response.rect.map(|r| (r.left, r.top, r.right, r.bottom)),
        response.issue,
        response.visited_nodes,
        s.uia_rect
    ));
    choose_hover(state);
    true
}

// SAFETY CONTRACT:
// Voraussetzung: state ist gültig und exklusiv auf dem Overlay-Thread.
// Warum erfüllt bzw. offen: Der Zustand wird geändert und nur ein kopiertes Handle zurückgegeben.
// Nachrichten: Kein Fensteraufruf im Funktionskörper.
pub(super) unsafe fn mark_finished(state: *mut State, result: Option<(i32, i32, i32, i32)>) -> H {
    let s = &mut *state;
    s.result = result.filter(|rect| valid_rect(*rect));
    s.done = true;
    let hwnd = s.hwnd;
    s.hwnd = null_mut();
    hwnd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_is_half_open_and_intersection_requires_area() {
        let rect = (10, 20, 30, 40);
        assert!(contains(rect, Point { x: 10, y: 20 }));
        assert!(!contains(rect, Point { x: 30, y: 40 }));
        assert_eq!(intersection(rect, (20, 30, 50, 60)), Some((20, 30, 30, 40)));
        assert_eq!(intersection(rect, (30, 20, 40, 40)), None);
    }
}
