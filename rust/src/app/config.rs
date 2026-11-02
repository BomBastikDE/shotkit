use super::*;

pub(super) fn active_settings() -> Settings {
    SETTINGS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_default()
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Besitzer für mögliche Fehlerdialoge.
// Warum erfüllt: Aufruf nach erfolgreicher Erstellung des Hauptfensters.
// Nachrichten: MessageBoxW kann reentrieren; der SETTINGS-Guard der Zuweisung endet vor dem Fehlerdialog.
pub(super) unsafe fn load_settings(h: H) {
    let result = settings::settings_path().and_then(settings::load_or_default);
    match result {
        Ok(value) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
            SETTINGS_WRITABLE.store(true, Ordering::SeqCst);
        }
        Err(e) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(Settings::default());
            SETTINGS_WRITABLE.store(false, Ordering::SeqCst);
            MessageBoxW(h,wide(&format!("Einstellungen konnten nicht geladen werden: {e}\nDie Standardwerte werden vorübergehend verwendet.")).as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Dialog-Besitzerfenster.
// Warum erfüllt: Menü- und Editoraktionen übergeben ihr eigenes Fenster.
// Nachrichten: MessageBoxW kann reentrieren; bei Fehlerdialogen wird kein SETTINGS-Guard gehalten.
pub(super) unsafe fn save_settings(h: H, new: Settings) -> bool {
    if !SETTINGS_WRITABLE.load(Ordering::SeqCst) {
        MessageBoxW(h,wide("Einstellungen können nicht gespeichert werden, solange die vorhandene Datei nicht lesbar ist.").as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        return false;
    }
    let result = settings::settings_path().and_then(|path| settings::save_atomic(path, &new));
    match result {
        Ok(()) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(new);
            true
        }
        Err(e) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Einstellungen konnten nicht gespeichert werden: {e}"
                ))
                .as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            false
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h hat einen initialisierten WindowState auf dem UI-Thread.
// Warum erfüllt: Aufruf aus der Ereignisbehandlung eines eigenen Editorfensters.
// Nachrichten: ReleaseCapture kann Nachrichten auslösen; der Editor-Borrow endet vor diesem Aufruf.
pub(super) unsafe fn cancel_gesture(h: H) {
    let had_capture = window_state(h).drawing;
    window_state(h).drawing = false;
    window_state(h).crop_start = None;
    window_state(h).crop_end = None;
    if let Some(state) = (&mut window_state(h).editor).as_mut() {
        state.cancel_drag();
    }
    if had_capture {
        ReleaseCapture();
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Besitzer; UTF-16-Zeiger leben bis zum API-Rücksprung.
// Warum erfüllt: eigene Menüaktion; alle Puffer bleiben während der synchronen Aufrufe im Scope.
// Nachrichten: ShellExecuteW und MessageBoxW können reentrieren; Einstellungen, Ordner und UTF-16-Puffer sind eigene Werte.
pub(super) unsafe fn open_storage_folder(h: H) {
    let Some(folder) = active_settings().save_folder else {
        MessageBoxW(
            h,
            wide("Es ist kein Speicherordner eingestellt. Wählen Sie zuerst unter Einstellungen einen Speicherordner.").as_ptr(),
            wide("Speicherordner öffnen").as_ptr(),
            0x30,
        );
        return;
    };
    match std::fs::metadata(&folder) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            MessageBoxW(
                h,
                wide(&format!("Der eingestellte Speicherpfad ist kein Ordner:\n{}", folder.display())).as_ptr(),
                wide("Speicherordner öffnen").as_ptr(),
                0x10,
            );
            return;
        }
        Err(error) => {
            MessageBoxW(
                h,
                wide(&format!("Der eingestellte Speicherordner ist nicht verfügbar:\n{}\n\n{error}", folder.display())).as_ptr(),
                wide("Speicherordner öffnen").as_ptr(),
                0x10,
            );
            return;
        }
    }
    let path: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = ShellExecuteW(
        h,
        wide("open").as_ptr(),
        path.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        5,
    );
    if result as usize <= 32 {
        MessageBoxW(
            h,
            wide(&format!("Windows konnte den Speicherordner nicht öffnen:\n{}\n\nShellExecuteW meldete Fehlercode {}.", folder.display(), result as usize)).as_ptr(),
            wide("Speicherordner öffnen").as_ptr(),
            0x10,
        );
    }
}
