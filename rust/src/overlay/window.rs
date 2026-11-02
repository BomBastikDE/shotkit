use super::*;

// SAFETY CONTRACT:
// Voraussetzung: Windows ruft auf dem Besitzerthread mit gültigen Parametern auf;
// der initialisierte Overlayzustand lebt bis zum Ende des Callbacks.
// Warum erfüllt bzw. offen: select verwaltet Fenster und Zustand; Panikfreiheit
// über die Callback-ABI ist noch nicht vollständig belegt.
// Nachrichten: Capture, Zerstörung und DefWindowProcW können reentrieren.
pub(super) unsafe extern "system" fn window_proc(hwnd: H, msg: u32, w: W, l: L) -> L {
    if let Some(result) = input::handle_mouse(hwnd, msg, w, l) { return result; }
    if let Some(result) = input::handle_keyboard(hwnd, msg, w, l) { return result; }
    if let Some(result) = painting::handle_paint(hwnd, msg, w, l) { return result; }
    DefWindowProcW(hwnd, msg, w, l)
}

// SAFETY CONTRACT:
// Voraussetzung: Overlay-Fenster und STATE leben auf diesem Nachrichtenthread.
// Warum erfüllt bzw. offen: select ruft nach der Anlage auf; Callback-Panikfreiheit
// und alle während der Schleife möglichen Lebensdauerpfade bleiben zu prüfen.
// Nachrichten: GetMessageW und DispatchMessageW verarbeiten synchron Nachrichten.
pub(super) unsafe fn run_message_loop() -> Option<i32> {
    let mut msg: Msg = zeroed();
    let mut quit_code = None;
    loop {
        let r = GetMessageW(&mut msg, null_mut(), 0, 0);
        if r == 0 {
            (*std::ptr::addr_of_mut!(STATE)).done = true;
            quit_code = Some(msg.wparam as i32);
            break;
        }
        if r < 0 {
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if (*std::ptr::addr_of!(STATE)).done {
            break;
        }
    }
    quit_code
}
