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
