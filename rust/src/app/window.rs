use super::*;

// SAFETY CONTRACT:
// Voraussetzung: Windows ruft mit gültigen Nachrichtenparametern auf dem
// Besitzerthread auf; Userdata ist null oder zeigt exklusiv auf WindowState.
// Warum erfüllt bzw. offen: WM_NCCREATE installiert den Zustand, WM_NCDESTROY
// entfernt ihn vor Freigabe; Panikfreiheit über die ABI-Grenze ist nicht belegt.
// Nachrichten: Dispatch und DefWindowProcW können synchron Nachrichten auslösen.
pub(super) unsafe extern "system" fn proc(h: H, m: u32, w: W, l: L) -> L {
    if m == 0x0081 {
        // WM_NCCREATE
        let mut state = WindowState::new();
        // Our class accepts only null or a live EditorCreationProtection in
        // lpCreateParams. Clone before any Win32 call; never retain the marker
        // reference across creation messages or after CreateWindowExW returns.
        let params = (*(l as *const CreateStructParams)).create_params;
        if !params.is_null() {
            let marker = &*(params as *const EditorCreationProtection);
            marker.lifetime.set(h);
            state.initialization_lifetime = Some(marker.lifetime.clone());
            state.initialization_close_blocked = Some(
                (*(params as *const EditorCreationProtection))
                    .close_blocked
                    .clone(),
            );
        }
        let state = Box::into_raw(Box::new(state));
        SetWindowLongPtrW(h, -21, state as isize);
    }
    if GetWindowLongPtrW(h, -21) == 0 {
        return DefWindowProcW(h, m, w, l);
    }
    if m == 0x0082 {
        // WM_NCDESTROY, after child controls and WM_DESTROY
        let state = GetWindowLongPtrW(h, -21) as *mut WindowState;
        if let Some(lifetime) = (*state).initialization_lifetime.as_ref() {
            lifetime.set(null_mut());
        }
        SetWindowLongPtrW(h, -21, 0);
        drop(Box::from_raw(state));
        return DefWindowProcW(h, m, w, l);
    }
    if h == PRIMARY && m != 0 && m == TASKBAR_CREATED.load(Ordering::SeqCst) {
        // Explorer restarted, or the taskbar appeared only after ShotKit
        // started: every notification icon is gone and must be added again.
        tray::restore(h);
        return 0;
    }
    if m == 0x0011 {
        // WM_QUERYENDSESSION: sign-out, restart or shutdown. Without an answer
        // Windows ends the process and unsaved annotations or captures are
        // lost without any question. No dialog here: Windows allows only a
        // few seconds and shows the reason itself, with "cancel" for the user.
        let unsaved = unsaved_work();
        if unsaved {
            ShutdownBlockReasonCreate(
                h,
                wide("Ungespeicherte Bilder oder Aufnahmen in ShotKit").as_ptr(),
            );
        } else {
            ShutdownBlockReasonDestroy(h);
        }
        return end_session_reply(unsaved);
    }
    if m == 0x0016 && w == 0 {
        // WM_ENDSESSION with FALSE: the session continues after all.
        ShutdownBlockReasonDestroy(h);
        return 0;
    }
    if m == 0x0010
        && window_state(h)
            .initialization_close_blocked
            .as_ref()
            .is_some_and(|blocked| blocked.get())
    {
        return 0;
    }
    let input = m == toolbar::WM_COMMAND
        || (0x0100..=0x0109).contains(&m)
        || (0x0200..=0x020e).contains(&m);
    let starts_operation = input || m == TRAY || m == 0x0312 || m == DELAYED_CAPTURE;
    if (starts_operation || m == WM_COPYDATA)
        && window_state(h)
            .initialization_close_blocked
            .as_ref()
            .is_some_and(|blocked| blocked.get())
    {
        return if m == WM_COPYDATA { 3 } else { 0 };
    }
    if m == DELAYED_CAPTURE {
        let Some(ticket) = CaptureTicket::from_wire(w) else {
            return 0;
        };
        if !DELAYED_CAPTURE_STATE
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_waiting(ticket)
        {
            return 0;
        }
        if window_state(h).modal_depth > 0
            || SCROLL_BUSY.load(Ordering::SeqCst)
            || QUITTING.load(Ordering::SeqCst)
        {
            // Retry only this still-waiting generation. Stale or duplicate
            // wakeups cannot start a new selection or a newer request early.
            post_capture_after(h, ticket, std::time::Duration::from_millis(250));
            return 0;
        }
    }
    if m == WM_COPYDATA && (window_state(h).modal_depth > 0 || QUITTING.load(Ordering::SeqCst)) {
        return 3;
    }
    if (window_state(h).modal_depth > 0 && (m == 0x0010 || starts_operation))
        || (QUITTING.load(Ordering::SeqCst) && starts_operation)
    {
        return 0;
    }
    let modal_event = input
        || m == WM_COPYDATA
        || m == TRAY
        || m == 0x0312
        || m == SCROLL_DONE
        || m == DELAYED_CAPTURE;
    // Keep windows alive while handlers run nested Win32 modal loops. Paint and
    // resize remain available; another document can still be used normally.
    let modal = modal_event.then(|| ModalGuard::new(h));
    let result = dispatch_window(h, m, w, l);
    drop(modal);
    if input && h != PRIMARY && GetWindowLongPtrW(h, -21) != 0 {
        refresh_title(h);
    }
    result
}
// SAFETY CONTRACT:
// Voraussetzung: proc hat den Fensterzustand geprüft und die Nachrichtenparameter stammen von Windows.
// Warum erfüllt bzw. offen: proc ist der einzige Aufrufer; Nachricht-spezifische Verträge und Reentranz bleiben gesondert zu prüfen.
// Nachrichten: Fenster-, Dialog- und Standardprozeduraufrufe können Nachrichten verarbeiten.
// SAFETY CONTRACT:
// Voraussetzung: proc hat den Fensterzustand geprüft; Nachrichtenparameter sind gültig.
// Warum erfüllt bzw. offen: proc ist der einzige Aufrufer; alle Unterhandler
// müssen den Zustands- und Reentranzvertrag einhalten.
// Nachrichten: Unterhandler und DefWindowProcW können synchron reentrieren.
unsafe fn dispatch_window(h: H, m: u32, w: W, l: L) -> L {
    if let Some(result) = window_commands::dispatch_system(h, m, w, l) {
        return result;
    }
    if let Some(result) = window_commands::dispatch_shortcuts(h, m, w, l) {
        return result;
    }
    if let Some(result) = window_commands::dispatch_editor_keys(h, m, w, l) {
        return result;
    }
    if let Some(result) = window_mouse::dispatch_mouse_begin(h, m, w, l) {
        return result;
    }
    if let Some(result) = window_mouse::dispatch_mouse_continue(h, m, w, l) {
        return result;
    }
    if let Some(result) = window_mouse::dispatch_paint_and_close(h, m, w, l) {
        return result;
    }
    DefWindowProcW(h, m, w, l)
}

#[cfg(test)]
#[path = "window_initialization_tests.rs"]
mod initialization_tests;
