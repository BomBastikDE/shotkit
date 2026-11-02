use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes Fenster auf seinem Besitzerthread.
// Warum erfüllt bzw. offen: run registriert Hotkeys nach erfolgreicher Fensteranlage.
// Nachrichten: Fehlerdialoge verarbeiten Nachrichten.
pub(super) unsafe fn register(h: H) {
    if RegisterHotKey(h, 1, 0x4000, 0x78) == 0 {
        MessageBoxW(
            h,
            wide("F9 ist belegt. Bereichsaufnahme über Einzelklick auf das Tray-Symbol.").as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x30,
        );
    }
    if RegisterHotKey(h, 2, 0x4000, 0x79) == 0 {
        MessageBoxW(
            h,
            wide("F10 ist belegt. Die experimentelle Scroll-Aufnahme kann nicht gestartet werden.")
                .as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x30,
        );
    }
    if RegisterHotKey(h, 3, 0x4000, 0x7a) == 0 {
        MessageBoxW(
            h,
            wide("F11 ist belegt; Bereichsaufnahme ist derzeit nicht verfügbar.").as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x30,
        );
    }
    let print_ok = RegisterHotKey(h, 4, 0x4000, 0x2c) != 0;
    let alt_ok = RegisterHotKey(h, 5, 0x4000 | 0x0002 | 0x0004, 0x31) != 0;
    if !print_ok && !alt_ok {
        MessageBoxW(h,wide("Druck und Strg+Umschalt+1 sind belegt. Für Bereichsaufnahmen F11 oder das Tray-Menü verwenden.").as_ptr(),wide("Tastenkürzel" ).as_ptr(),0x30);
    }
    let mut unavailable = Vec::new();
    for (id, mods, key, label) in [
        (6, 2, 0x2c, "Strg+Druck"),
        (7, 6, 0x2c, "Strg+Umschalt+Druck"),
        (8, 4, 0x2c, "Umschalt+Druck"),
        (9, 3, 0x2c, "Strg+Alt+Druck"),
        (10, 6, 0x32, "Strg+Umschalt+2"),
        (11, 6, 0x33, "Strg+Umschalt+3"),
        (12, 6, 0x34, "Strg+Umschalt+4"),
    ] {
        if RegisterHotKey(h, id, mods | 0x4000, key) == 0 {
            unavailable.push(label);
        }
    }
    if !unavailable.is_empty() {
        MessageBoxW(
            h,
            wide(&format!(
                "Nicht verfügbare Tastenkürzel: {}",
                unavailable.join(", ")
            ))
            .as_ptr(),
            wide("Tastenkürzel").as_ptr(),
            0x30,
        );
    }
}

// SAFETY CONTRACT:
// Voraussetzung: Die IDs wurden für h auf diesem Thread registriert.
// Warum erfüllt bzw. offen: Der Aufruf erfolgt beim WM_DESTROY des Primärfensters mit denselben IDs.
// Nachrichten: Kein expliziter synchroner Nachrichtenversand.
pub(super) unsafe fn unregister(h: H) {
    for id in 1..=12 {
        UnregisterHotKey(h, id);
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h erfüllt die Verträge der gewählten Aufnahmeaktion.
// Warum erfüllt bzw. offen: Der eigene Fensterhandler übergibt h; unbekannte IDs werden verworfen.
// Nachrichten: Aufnahmeaktionen und Dialoge können Nachrichten auslösen.
pub(super) unsafe fn dispatch(h: H, id: W) -> bool {
    use shotkit_rust_prototype::hotkey_action::{action_for_id, Action};
    match action_for_id(id) {
        Some(Action::Screen) => capture(h),
        Some(Action::AllScreens) => capture_rect(h, capture_targets::virtual_desktop()),
        Some(Action::ActiveWindow) => capture_active(h, GetForegroundWindow()),
        Some(Action::Scroll) => start_scroll(h),
        Some(Action::Region) => capture_region(h),
        None => return false,
    }
    true
}
