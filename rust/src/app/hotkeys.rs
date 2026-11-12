use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes Fenster auf seinem Besitzerthread.
// Warum erfüllt bzw. offen: run registriert Hotkeys nach erfolgreicher Fensteranlage.
// Nachrichten: Fehlerdialoge verarbeiten Nachrichten.
pub(super) unsafe fn register(h: H) {
    use shotkit::hotkey_action::{registration_for_id, MOD_NOREPEAT};
    let registration = |id| registration_for_id(id).expect("registered hotkey descriptor");
    let register = |registration: shotkit::hotkey_action::Registration| unsafe {
        RegisterHotKey(
            h,
            registration.id,
            registration.modifiers | MOD_NOREPEAT,
            registration.virtual_key,
        ) != 0
    };

    if !register(registration(1)) {
        MessageBoxW(
            h,
            wide("F9 ist belegt. Bildschirmaufnahme über das Tray-Menü („Bildschirm aufnehmen“).")
                .as_ptr(),
            wide("ShotKit").as_ptr(),
            0x30,
        );
    }
    if !register(registration(2)) {
        MessageBoxW(
            h,
            wide("F10 ist belegt. Die experimentelle Scroll-Aufnahme kann nicht gestartet werden.")
                .as_ptr(),
            wide("ShotKit").as_ptr(),
            0x30,
        );
    }
    if !register(registration(3)) {
        MessageBoxW(
            h,
            wide("F11 ist belegt; Bereichsaufnahme ist derzeit nicht verfügbar.").as_ptr(),
            wide("ShotKit").as_ptr(),
            0x30,
        );
    }
    let print_ok = register(registration(4));
    let alt_ok = register(registration(5));
    if !print_ok && !alt_ok {
        MessageBoxW(h,wide("Druck und Strg+Umschalt+1 sind belegt. Für Bereichsaufnahmen F11 oder das Tray-Menü verwenden.").as_ptr(),wide("Tastenkürzel" ).as_ptr(),0x30);
    }
    let mut unavailable = Vec::new();
    for id in 6..=12 {
        let descriptor = registration(id);
        if !register(descriptor) {
            unavailable.push(descriptor.label);
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
    use shotkit::hotkey_action::{action_for_id, Action};
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
