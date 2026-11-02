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
        let flags = if id == 16 && LAST_REGION.lock().unwrap_or_else(|p| p.into_inner()).is_none() { 0x0003 } else { 0 };
        AppendMenuW(menu, flags, id, wide(label).as_ptr());
    }
    let prefs = active_settings();
    let submenu = CreatePopupMenu();
    if !submenu.is_null() {
        for (id, label, checked) in [
            (101, "Nach Aufnahme Editor öffnen", prefs.open_editor),
            (102, "In Zwischenablage kopieren", prefs.copy_to_clipboard),
            (103, "Automatisch als PNG speichern", prefs.auto_save),
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
        101 | 102 | 103 => {
            let mut prefs = active_settings();
            match choice {
                101 => prefs.open_editor = !prefs.open_editor,
                102 => prefs.copy_to_clipboard = !prefs.copy_to_clipboard,
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
            save_settings(h, prefs);
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
// Warum erfüllt bzw. offen: run übergibt das neu erzeugte Fenster und GetModuleHandleW; lokale Notify-Daten leben während des Aufrufs.
// Nachrichten: Shell-Aufruf und Fehlerdialog können Nachrichten auslösen.
pub(super) unsafe fn install(h: H, inst: H) -> bool {
    let mut n: Notify = zeroed();
        n.size = size_of::<Notify>() as u32;
        n.hwnd = h;
        n.id = 1;
        n.flags = 1 | 2 | 4;
        n.callback = TRAY;
        n.icon = LoadIconW(inst, 1usize as *const u16);
        let tip = wide("ShotKit Rust: F9 = Bildschirm, Klick = Bereich, Rechtsklick = Menü");
        n.tip[..tip.len()].copy_from_slice(&tip);
        if Shell_NotifyIconW(0, &n) == 0 {
            MessageBoxW(
                h,
                wide("Tray-Symbol konnte nicht erstellt werden.").as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            return false;
        }
    true
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
