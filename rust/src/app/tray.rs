use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h und sein Zustand bleiben während Menü und Folgeaktionen gültig.
// Warum erfüllt bzw. offen: Einstellungen und Ziel sind eigene Werte; Handle-Lebensdauer bei Reentranz bleibt zu prüfen.
// Nachrichten: TrackPopupMenu und Folgeaktionen können Nachrichten verarbeiten.
pub(super) unsafe fn tray_menu(h: H, active_target: Result<ActiveWindowSnapshot, String>) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (1, "Bildschirm aufnehmen (F9)"),
        (9, "Bereich nach 3 Sekunden auswählen"),
        (10, "Bereich nach 5 Sekunden auswählen"),
        (11, "Bereich nach 10 Sekunden auswählen"),
        (2, "Bereich aufnehmen (F11)"),
        (16, "Letzten Bereich erneut aufnehmen"),
        (3, "Scroll-Aufnahme (F10)"),
        (7, "Alle Bildschirme aufnehmen"),
        (13, "Aktives Fenster aufnehmen"),
        (4, "Editor anzeigen"),
        (5, "Bild öffnen …"),
        (12, "Bild aus Zwischenablage öffnen"),
        (14, "Speicherordner öffnen"),
        (8, "Ungesicherte Aufnahme speichern …"),
        (15, "Über ShotKit"),
        (6, "Beenden"),
    ] {
        let flags = if id == 16
            && LAST_REGION
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_none()
        {
            0x0003
        } else {
            0
        };
        AppendMenuW(menu, flags, id, wide(label).as_ptr());
    }
    let prefs = active_settings();
    let submenu = CreatePopupMenu();
    if !submenu.is_null() {
        for (id, label, checked) in [
            (101, "Nach Aufnahme Editor öffnen", prefs.open_editor),
            (102, "In Zwischenablage kopieren", prefs.copy_to_clipboard),
            (103, "Automatisch als PNG speichern", prefs.auto_save),
            (
                106,
                "Aufnahmen von Zwischenablageverlauf und Cloud ausschließen",
                prefs.exclude_from_clipboard_history,
            ),
        ] {
            AppendMenuW(
                submenu,
                if checked { 0x0008 } else { 0 },
                id,
                wide(label).as_ptr(),
            );
        }
        AppendMenuW(submenu, 0, 104, wide("Speicherordner wählen …").as_ptr());
        match autostart::get() {
            Ok(enabled) => {
                AppendMenuW(
                    submenu,
                    if enabled { 0x0008 } else { 0 },
                    105,
                    wide("Mit Windows starten").as_ptr(),
                );
            }
            Err(_) => {
                AppendMenuW(
                    submenu,
                    0x0003,
                    105,
                    wide("Autostart nicht verfügbar").as_ptr(),
                );
            }
        }

        AppendMenuW(
            menu,
            0x0010,
            submenu as usize,
            wide("Einstellungen").as_ptr(),
        );
    }
    let mut pos = Point { x: 0, y: 0 };
    if GetCursorPos(&mut pos) == 0 {
        DestroyMenu(menu);
        return;
    }
    SetForegroundWindow(h);
    let choice = TrackPopupMenu(menu, 0x0100 | 0x0002, pos.x, pos.y, 0, h, std::ptr::null());
    crate::uia_picker::log_diagnostic(&format!("tray_track_popup_choice={choice}"));
    // Windows requires a posted no-op after a notification-icon menu so the
    // next invocation is not dismissed immediately.
    let _ = PostMessageW(h, 0, 0, 0); // WM_NULL
    DestroyMenu(menu);
    match choice {
        1 => capture(h),
        9 => schedule_capture(h, 3),
        10 => schedule_capture(h, 5),
        11 => schedule_capture(h, 10),
        2 => capture_region(h),
        16 => repeat_last_region(h),
        3 => start_scroll(h),
        7 => capture_rect(h, capture_targets::virtual_desktop()),
        13 => {
            let target = active_target.and_then(|saved| resolve_active_window_target(h, &saved));
            capture_rect(h, target);
        }
        4 => show_editors(),
        5 => open_image(h),
        12 => paste_image(h),
        14 => open_storage_folder(h),
        15 => about_dialog::show(h),
        8 => {
            resolve_recovery(h);
        }
        101 | 102 | 103 | 106 => {
            let mut prefs = active_settings();
            match choice {
                101 => prefs.open_editor = !prefs.open_editor,
                102 => prefs.copy_to_clipboard = !prefs.copy_to_clipboard,
                106 => prefs.exclude_from_clipboard_history = !prefs.exclude_from_clipboard_history,
                103 => {
                    if !prefs.auto_save && prefs.save_folder.is_none() {
                        match folder_dialog::choose_folder(h) {
                            Ok(Some(path)) => prefs.save_folder = Some(path),
                            Ok(None) => return,
                            Err(e) => {
                                MessageBoxW(
                                    h,
                                    wide(&e).as_ptr(),
                                    wide("Speicherordner").as_ptr(),
                                    0x10,
                                );
                                return;
                            }
                        }
                    }
                    prefs.auto_save = !prefs.auto_save;
                }
                _ => {}
            }
            let saved = save_settings(h, prefs);
            crate::uia_picker::log_diagnostic(&format!(
                "settings_save={}",
                if saved { "success" } else { "failed" }
            ));
        }
        105 => {
            let result = autostart::get().and_then(|value| autostart::set(!value));
            if let Err(e) = result {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Autostart").as_ptr(), 0x10);
            }
        }
        104 => match folder_dialog::choose_folder(h) {
            Ok(Some(path)) => {
                let mut prefs = active_settings();
                prefs.save_folder = Some(path);
                save_settings(h, prefs);
            }
            Ok(None) => {}
            Err(e) => {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherordner").as_ptr(), 0x10);
            }
        },
        6 => quit_application(h),
        _ => {}
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster und inst ein gültiges Modulhandle.
// Warum erfüllt: install und restore übergeben das Primärfenster und GetModuleHandleW; lokale Notify-Daten leben während des Aufrufs.
// Nachrichten: Shell_NotifyIconW kann Nachrichten auslösen; es wird kein Zustand gehalten.
unsafe fn add(h: H, inst: H) -> bool {
    let mut n: Notify = zeroed();
    n.size = size_of::<Notify>() as u32;
    n.hwnd = h;
    n.id = 1;
    n.flags = 1 | 2 | 4;
    n.callback = TRAY;
    // Win32 integer resource ID 1, encoded as a pointer without dereferencing.
    n.icon = LoadIconW(inst, std::ptr::without_provenance::<u16>(1));
    let tip = wide("ShotKit: F9 = Bildschirm, Klick = Bereich, Rechtsklick = Menü");
    n.tip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(0, &n) != 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupTray {
    Ready,
    /// No taskbar yet, for example when autostart runs before Explorer is up.
    /// The icon is added when "TaskbarCreated" arrives.
    WaitForTaskbar,
    Failed,
}

fn startup_tray(added: bool, taskbar_exists: bool) -> StartupTray {
    match (added, taskbar_exists) {
        (true, _) => StartupTray::Ready,
        (false, false) => StartupTray::WaitForTaskbar,
        (false, true) => StartupTray::Failed,
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster und inst ein gültiges Modulhandle.
// Warum erfüllt bzw. offen: run übergibt das neu erzeugte Fenster und GetModuleHandleW.
// Nachrichten: Shell-Aufruf und Fehlerdialog können Nachrichten auslösen.
pub(super) unsafe fn install(h: H, inst: H) -> bool {
    let added = add(h, inst);
    let taskbar_exists = !FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null()).is_null();
    match startup_tray(added, taskbar_exists) {
        StartupTray::Ready | StartupTray::WaitForTaskbar => true,
        StartupTray::Failed => {
            MessageBoxW(
                h,
                wide("Tray-Symbol konnte nicht erstellt werden.").as_ptr(),
                wide("ShotKit").as_ptr(),
                0x10,
            );
            false
        }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h ist das lebende Primärfenster auf seinem Besitzerthread.
// Warum erfüllt: proc ruft nur für PRIMARY bei der registrierten Nachricht "TaskbarCreated" auf.
// Nachrichten: Shell_NotifyIconW kann Nachrichten auslösen; kein Dialog, kein Zustand.
pub(super) unsafe fn restore(h: H) {
    // Fails harmlessly when the icon still exists (the message is also sent
    // after a DPI change of the primary display).
    add(h, GetModuleHandleW(std::ptr::null()));
}

// SAFETY CONTRACT:
// Voraussetzung: h und ID bezeichnen das zuvor registrierte Symbol.
// Warum erfüllt bzw. offen: Der Aufruf erfolgt beim WM_DESTROY des Primärfensters mit derselben ID.
// Nachrichten: Shell-Aufruf; keine pauschale Reentranzfreiheit angenommen.
pub(super) unsafe fn remove(h: H) {
    let mut n: Notify = zeroed();
    n.size = size_of::<Notify>() as u32;
    n.hwnd = h;
    n.id = 1;
    Shell_NotifyIconW(2, &n);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_waits_for_a_missing_taskbar_and_fails_only_with_one_present() {
        assert_eq!(startup_tray(true, true), StartupTray::Ready);
        assert_eq!(startup_tray(true, false), StartupTray::Ready);
        assert_eq!(startup_tray(false, false), StartupTray::WaitForTaskbar);
        assert_eq!(startup_tray(false, true), StartupTray::Failed);
    }
}
