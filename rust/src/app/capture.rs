use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h lebt während der Ausgabe und erzeugte Fenster besitzen initialisierten Zustand.
// Warum erfüllt bzw. offen: display_image prüft die Fensteranlage; Reentranz bei modalen Fehlerdialogen bleibt zu prüfen.
// Nachrichten: display_image, Clipboard, ShowWindow und MessageBoxW können reentrieren; Bild/Einstellungen sind eigene Werte und kurze Zustandszuweisungen enden vorher.
pub(super) unsafe fn handle_capture(h: H, image: Image, was_visible: bool) -> Result<(), String> {
    let prefs = active_settings();
    let mut output = CaptureOutputState::new(&prefs);
    let mut image = Some(image);
    let mut saved = None;
    let mut opened = false;
    while let Some(action) = output.next_action() {
        let result = match action {
            CaptureAction::OpenEditor => {
                let owned = image.take().ok_or("Aufnahme ist nicht mehr verfügbar")?;
                let result = display_image(h, owned);
                match result {
                    Ok(window) => {
                        opened = true;
                        if let Some(path) = saved.take() {
                            window_state(window).current_path = Some(path);
                            window_state(window).dirty = false;
                            refresh_title(window);
                        }
                        if was_visible {
                            ShowWindow(h, 5);
                        }
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            _ => {
                let result = output_windows::execute_action(
                    action,
                    &prefs,
                    image.as_ref().ok_or("Aufnahme ist nicht mehr verfügbar")?,
                    h,
                );
                match result {
                    Ok(result) => {
                        if result.saved_path.is_some() {
                            saved = result.saved_path;
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        };
        let advance = match output.finish_action(action, result) {
            Ok(value) => value,
            Err(error) => {
                if let Some(image) = image.take() {
                    RECOVERY
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(image);
                }
                return Err(format!(
                    "Ausgabe konnte nicht abgeschlossen werden: {error}"
                ));
            }
        };
        if advance == Advance::RetainImage {
            return Err("Die Aufnahme konnte nicht im Editor geöffnet werden und bleibt im Speicher. Über das Tray-Menü kann sie gesichert werden.".into());
        }
    }
    if !opened {
        if was_visible {
            ShowWindow(h, 5);
        }
        let message = match saved {
            Some(path) => format!("Aufnahme gespeichert: {}", path.display()),
            None => "Aufnahme in die Zwischenablage kopiert.".into(),
        };
        MessageBoxW(
            h,
            wide(&message).as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x40,
        );
    }
    if !output.failures().is_empty() {
        let details = output
            .failures()
            .iter()
            .map(|f| format!("{:?}: {}", f.action, f.message))
            .collect::<Vec<_>>()
            .join("\n");
        MessageBoxW(
            h,
            wide(&format!(
                "Ein Teil der Ausgabe ist fehlgeschlagen:\n{details}"
            ))
            .as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x30,
        );
    }
    Ok(())
}
// SAFETY CONTRACT:
// Voraussetzung: h bleibt während Aufnahme und möglichen Dialogen gültig.
// Warum erfüllt bzw. offen: BusyGuard serialisiert Aufnahmen; die Lebensdauer bei Reentranz ist noch zu belegen.
// Nachrichten: ShowWindow, MessageBoxW und handle_capture können reentrieren; es wird keine WindowState-Referenz gehalten, die HWND-Lebensdauer bleibt offen.
pub(super) unsafe fn capture_rect(h: H, target: Result<capture_targets::ScreenRect, String>) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _busy = BusyGuard(true);
    let rect = match target {
        Ok(r) => r,
        Err(e) => {
            MessageBoxW(h, wide(&e).as_ptr(), wide("Aufnahme").as_ptr(), 0x10);
            return;
        }
    };
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let image = scroll_windows::WindowsScrollIo::new(
        rect.left(),
        rect.top(),
        rect.width(),
        rect.height(),
        0,
    )
    .and_then(|mut io| io.capture());
    match image.and_then(|image| handle_capture(h, image, was_visible)) {
        Ok(()) => {}
        Err(e) => {
            if was_visible {
                ShowWindow(h, 5);
            }
            MessageBoxW(
                h,
                wide(&e).as_ptr(),
                wide("Bildschirmaufnahme").as_ptr(),
                0x10,
            );
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster auf dem UI-Thread.
// Warum erfüllt: Der Nachrichtenhandler übergibt sein Fenster; capture_rect hält es während der Aufnahme.
// Nachrichten: capture_rect kann ShowWindow und Dialoge auslösen; h und das Zielergebnis sind Werte ohne gehaltenen State-Borrow.
pub(super) unsafe fn capture(h: H) {
    capture_rect(h, capture_targets::monitor_under_cursor());
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Dialogbesitzer auf dem UI-Thread.
// Warum erfüllt: Die Nachrichtenbehandlung übergibt das eigene Fenster; der Worker postet nur eine Nachricht.
// Nachrichten: MessageBoxW kann reentrieren; der DELAYED_CAPTURE_STATE-Guard endet vor dem Dialog, der Worker postet später eine Nachricht.
pub(super) unsafe fn schedule_capture(h: H, seconds: u64) {
    let request = DELAYED_CAPTURE_STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .begin();
    let ticket = match request {
        Ok(ticket) => ticket,
        Err(error) => {
            let message = match error {
                BeginError::Busy => "Eine verzögerte Aufnahme läuft bereits.",
                BeginError::Exhausted => {
                    "Keine weitere verzögerte Aufnahme möglich. Bitte ShotKit neu starten."
                }
            };
            MessageBoxW(h, wide(message).as_ptr(), wide("Aufnahme").as_ptr(), 0x30);
            return;
        }
    };
    post_capture_after(h, ticket, std::time::Duration::from_secs(seconds));
}
pub(super) fn post_capture_after(h: H, ticket: CaptureTicket, delay: std::time::Duration) {
    let hwnd = h as isize;
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if !DELAYED_CAPTURE_STATE
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_waiting(ticket)
        {
            return;
        }
        // Completion can race with this PostMessage. The receiver therefore
        // checks the same generation and claims Waiting -> Running exactly once.
        if unsafe { PostMessageW(hwnd as H, DELAYED_CAPTURE, ticket.to_wire(), 0) } == 0 {
            DELAYED_CAPTURE_STATE
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .cancel_waiting(ticket);
        }
    });
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein eigenes Fenster; window ist ein Windows-Fensterhandle.
// Warum erfüllt bzw. offen: active_window_target und resolve_active_window_target überprüfen Identität vor der Aufnahme.
// Nachrichten: capture_rect kann Fenster und Dialoge aufrufen; Snapshot und Ziel sind eigene Werte ohne gehaltenen State-Borrow.
pub(super) unsafe fn capture_active(h: H, window: H) {
    let target = match active_window_target(h, window) {
        Ok(snapshot) => resolve_active_window_target(h, &snapshot),
        Err(error) => Err(error),
    };
    capture_rect(h, target);
}
pub(super) struct ActiveWindowSnapshot {
    hwnd: H,
    process_id: u32,
    thread_id: u32,
    class_name: String,
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein eigenes Fenster; window ist nur ein von Windows geliefertes Handle.
// Warum erfüllt: IsWindow prüft window, alle Win32-Ausgabepuffer liegen für die Dauer des Aufrufs auf dem Stack.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
pub(super) unsafe fn active_window_target(h: H, window: H) -> Result<ActiveWindowSnapshot, String> {
    if window.is_null() || IsWindow(window) == 0 {
        return Err("Windows hat kein gültiges aktives Fenster geliefert.".into());
    }
    let root = GetAncestor(window, 2);
    let root = if root.is_null() { window } else { root };
    if root == h || editor_windows().iter().any(|editor| *editor == root) {
        return Err("ShotKit war das aktive Fenster. Bitte zuerst das gewünschte Fenster aktivieren und erneut wählen.".into());
    }
    let mut class_name = [0u16; 256];
    let class_len = GetClassNameW(root, class_name.as_mut_ptr(), class_name.len() as i32);
    if class_len <= 0 {
        return Err("Windows konnte die Klasse des aktiven Fensters nicht ermitteln.".into());
    }
    let class_name = String::from_utf16_lossy(&class_name[..class_len as usize]);
    let shell = GetShellWindow();
    if root == shell
        || matches!(
            class_name.as_str(),
            "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "TrayNotifyWnd"
                | "NotifyIconOverflowWindow"
                | "TopLevelWindowForOverflowXamlIsland"
                | "Progman"
                | "WorkerW"
        )
    {
        return Err("Die Windows-Shell war im Vordergrund; ein Zielfenster lässt sich daraus nicht sicher bestimmen. Bitte gewünschtes Fenster aktivieren und erneut wählen.".into());
    }
    let mut process_id = 0;
    let thread_id = GetWindowThreadProcessId(root, &mut process_id);
    if thread_id == 0 || process_id == 0 {
        return Err(
            "Windows konnte Prozess und Thread des aktiven Fensters nicht bestimmen.".into(),
        );
    }
    Ok(ActiveWindowSnapshot {
        hwnd: root,
        process_id,
        thread_id,
        class_name,
    })
}
// SAFETY CONTRACT:
// Voraussetzung: saved stammt aus active_window_target und h ist das eigene Fenster.
// Warum erfüllt: Handle, Prozess, Thread und Klasse werden vor Verwendung erneut verglichen.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
pub(super) unsafe fn resolve_active_window_target(
    h: H,
    saved: &ActiveWindowSnapshot,
) -> Result<capture_targets::ScreenRect, String> {
    if IsWindow(saved.hwnd) == 0 {
        return Err(
            "Das zuvor aktive Fenster wurde geschlossen, bevor die Aufnahme startete.".into(),
        );
    }
    let current = active_window_target(h, saved.hwnd).map_err(|_| {
        "Das zuvor aktive Fenster ist nicht mehr als dasselbe gültige Zielfenster verfügbar."
            .to_string()
    })?;
    if current.hwnd != saved.hwnd
        || current.process_id != saved.process_id
        || current.thread_id != saved.thread_id
        || current.class_name != saved.class_name
    {
        return Err("Das zuvor aktive Fenster wurde geschlossen oder sein HWND inzwischen wiederverwendet. Bitte den Tray-Befehl erneut wählen.".into());
    }
    capture_targets::saved_window(saved.hwnd as isize)
}
