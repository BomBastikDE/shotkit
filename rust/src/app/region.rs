use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h bleibt über Auswahl und modal verschachtelte Nachrichten gültig.
// Warum erfüllt bzw. offen: Die Funktion wird vom Fensterhandler aufgerufen; vollständiger Reentranznachweis steht noch aus.
// Nachrichten: ShowWindow, MessageBoxW und overlay::select können reentrieren; Koordinaten und BusyGuard sind eigene Werte, kein State-Borrow bleibt aktiv.
pub(super) unsafe fn capture_region(h: H) {
    let Some(_busy) = acquire_recovery_safe_source(h) else {
        return;
    };
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    let Some((x, y, w, height)) = overlay::select() else {
        if was_visible {
            ShowWindow(h, 5);
        }
        return;
    };
    let Some(rect) = last_region::Rect::from_origin_size(x, y, w, height) else {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Die Auswahl hat ungültige Bildschirmgrenzen.").as_ptr(),
            wide("Bereichsaufnahme").as_ptr(),
            0x10,
        );
        return;
    };
    capture_region_rect(h, rect, true, was_visible);
}

// SAFETY CONTRACT:
// Voraussetzung: h und rect bleiben während der Auswahl-/Ausgabeoperation gültig.
// Warum erfüllt bzw. offen: rect ist ein besitzender Wert; die Lebensdauer von h bei Reentranz ist noch zu belegen.
// Nachrichten: ShowWindow, MessageBoxW und EnumDisplayMonitors können reentrieren; Monitorliste und LAST_REGION-Zuweisungs-Guard enden vor späteren Dialogen.
pub(super) unsafe fn capture_region_rect(
    h: H,
    rect: last_region::Rect,
    remember_on_success: bool,
    was_visible: bool,
) {
    std::thread::sleep(std::time::Duration::from_millis(200));
    if !remember_on_success {
        let monitors = match current_monitor_rects() {
            Ok(monitors) => monitors,
            Err(error) => {
                if was_visible {
                    ShowWindow(h, 5);
                }
                MessageBoxW(
                    h,
                    wide(&error).as_ptr(),
                    wide("Bereich erneut aufnehmen").as_ptr(),
                    0x10,
                );
                return;
            }
        };
        if !last_region::fully_covered_by_monitors(rect, &monitors) {
            if was_visible {
                ShowWindow(h, 5);
            }
            MessageBoxW(
                h,
                wide("Der gespeicherte Bereich liegt nicht mehr vollständig auf den aktuell angeschlossenen Bildschirmen. Wähle den Bereich erneut aus.").as_ptr(),
                wide("Bereich erneut aufnehmen").as_ptr(),
                0x30,
            );
            return;
        }
    }
    let image = scroll_windows::WindowsScrollIo::new(
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        0,
    )
    .and_then(|mut io| io.capture());
    match image.and_then(|image| handle_capture(h, image, was_visible)) {
        Ok(()) => {
            if remember_on_success {
                *LAST_REGION.lock().unwrap_or_else(|p| p.into_inner()) = Some(rect);
            }
        }
        Err(e) => {
            if was_visible {
                ShowWindow(h, 5);
            }
            MessageBoxW(
                h,
                wide(&e).as_ptr(),
                wide("Bereichsaufnahme").as_ptr(),
                0x10,
            );
        }
    }
}

pub(super) struct MonitorList {
    rects: Vec<last_region::Rect>,
    failed: bool,
}

// SAFETY CONTRACT:
// Voraussetzung: data zeigt exklusiv auf eine lebende MonitorList.
// Warum erfüllt: current_monitor_rects übergibt die lokale Liste an den synchronen
// EnumDisplayMonitors-Aufruf; der Callback speichert den Zeiger nicht.
// Nachrichten: EnumDisplayMonitors ruft diesen Callback synchron auf.
pub(super) unsafe extern "system" fn collect_monitor(
    monitor: H,
    _dc: H,
    _rect: *mut Rect,
    data: L,
) -> i32 {
    let list = &mut *(data as *mut MonitorList);
    let mut info: MonitorInfo = zeroed();
    info.size = size_of::<MonitorInfo>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        list.failed = true;
        return 0;
    }
    list.rects.push(last_region::Rect {
        left: info.monitor.left,
        top: info.monitor.top,
        right: info.monitor.right,
        bottom: info.monitor.bottom,
    });
    1
}

// SAFETY CONTRACT:
// Voraussetzung: Der Callback-Pointer ist während EnumDisplayMonitors gültig und exklusiv.
// Warum erfüllt: Die lokale Liste lebt bis zum synchronen Rücksprung und wird nicht gespeichert.
// Nachrichten: keine der geprüften synchronen Fenster-Nachrichten.
pub(super) unsafe fn current_monitor_rects() -> Result<Vec<last_region::Rect>, String> {
    let mut list = MonitorList {
        rects: Vec::new(),
        failed: false,
    };
    let enumerated = EnumDisplayMonitors(
        null_mut(),
        std::ptr::null(),
        Some(collect_monitor),
        &mut list as *mut _ as L,
    );
    if enumerated == 0 || list.failed || list.rects.is_empty() {
        return Err(
            "Die aktuellen Monitorgrenzen konnten nicht zuverlässig gelesen werden.".into(),
        );
    }
    Ok(list.rects)
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster; gespeicherte Koordinaten sind gültig.
// Warum erfüllt bzw. offen: Der Handler übergibt h; Monitorgrenzen werden vor Wiederholung geprüft.
// Nachrichten: MessageBoxW, ShowWindow und capture_region_rect können reentrieren; LAST_REGION wird kopiert und der MutexGuard vor dem Dialog beendet.
pub(super) unsafe fn repeat_last_region(h: H) {
    let Some(_busy) = acquire_recovery_safe_source(h) else {
        return;
    };
    // Drop the mutex guard before the dialog can dispatch nested messages.
    let previous_region = *LAST_REGION.lock().unwrap_or_else(|p| p.into_inner());
    let Some(rect) = previous_region else {
        MessageBoxW(
            h,
            wide("Es wurde in dieser Sitzung noch kein Aufnahmebereich erfolgreich aufgenommen.")
                .as_ptr(),
            wide("Bereich erneut aufnehmen").as_ptr(),
            0x30,
        );
        return;
    };
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    capture_region_rect(h, rect, false, was_visible);
}

// SAFETY CONTRACT:
// Voraussetzung: h lebt während der modalen Auswahl und des Starts des Workers.
// Warum erfüllt: window::proc hält für beide Aufrufer (Hotkey und Tray) einen
// ModalGuard; dessen Close-Gate schützt h auch während synchroner Sends.
// Nachrichten: MessageBoxW, ShowWindow, SetForegroundWindow, SendMessageTimeoutW
// und overlay::select können reentrieren; kein State-Borrow bleibt dabei aktiv.
pub(super) unsafe fn start_scroll(h: H) {
    let Some(mut busy) = acquire_recovery_safe_source(h) else {
        return;
    };
    let previous_direction = active_settings().scroll_direction;
    let previous_label = match previous_direction {
        settings::ScrollDirection::Down => "nach unten",
        settings::ScrollDirection::Right => "nach rechts",
        settings::ScrollDirection::Left => "nach links",
    };
    let first_default = if previous_direction == settings::ScrollDirection::Down {
        0
    } else {
        0x0100 // MB_DEFBUTTON2: horizontal choice
    };
    let prompt = wide(&format!(
        "Scroll-Aufnahme (experimentell):\nJa = nach unten, Nein = nach rechts oder links, Abbrechen = schließen.\nZuletzt verwendet: {previous_label}.\nDanach den Inhaltsbereich mit der Maus aufziehen; Esc bricht ab."
    ));
    let choice = MessageBoxW(
        h,
        prompt.as_ptr(),
        wide("Scroll-Richtung").as_ptr(),
        0x23 | first_default,
    );
    let direction = match choice {
        6 => Direction::Down,
        7 => {
            let second_default = match previous_direction {
                settings::ScrollDirection::Left => 0x0100, // MB_DEFBUTTON2: left choice
                settings::ScrollDirection::Down | settings::ScrollDirection::Right => 0,
            };
            match MessageBoxW(
                h,
                wide("Ja = nach rechts, Nein = nach links, Abbrechen = schließen.").as_ptr(),
                wide("Scroll-Richtung").as_ptr(),
                0x23 | second_default,
            ) {
                6 => Direction::Right,
                7 => Direction::Left,
                _ => return,
            }
        }
        _ => return,
    };
    let mut prefs = active_settings();
    let selected_direction = match direction {
        Direction::Down => settings::ScrollDirection::Down,
        Direction::Right => settings::ScrollDirection::Right,
        Direction::Left => settings::ScrollDirection::Left,
    };
    if prefs.scroll_direction != selected_direction {
        prefs.scroll_direction = selected_direction;
        // A persistence error must not prevent the requested capture.
        let _ = save_settings(h, prefs);
    }
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    let Some((x, y, width, height)) = overlay::select() else {
        if was_visible {
            ShowWindow(h, 5);
        }
        return;
    };
    if width < 100
        || height < 100
        || match direction {
            Direction::Down => height < 200,
            _ => width < 200,
        }
    {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(h, wide("Der Bereich muss mindestens 100 × 100 Pixel groß und in Scrollrichtung mindestens 200 Pixel lang sein.").as_ptr(), wide("Scroll-Aufnahme").as_ptr(), 0x30);
        return;
    }
    let target = WindowFromPoint(Point {
        x: x + width / 2,
        y: y + height / 2,
    });
    let root = if target.is_null() {
        null_mut()
    } else {
        GetAncestor(target, 2)
    };
    if root.is_null() {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Zielfenster konnte nicht ermittelt werden.").as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x30,
        );
        return;
    }
    if let Err(error) = activate_scroll_target(root) {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide(error.message()).as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x30,
        );
        return;
    }
    let owner = WindowIdentity {
        handle: h as usize,
        generation: window_state(h).generation,
    };
    let ticket = SCROLL_COMPLETION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .begin(owner, was_visible);
    let Some(ticket) = ticket else {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Scroll-Aufnahme konnte nicht gestartet werden.").as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x10,
        );
        return;
    };
    // PRIMARY remains alive while SCROLL_BUSY is set. Install on its owner
    // thread before launching the worker; the original editor may later close.
    if SetTimer(PRIMARY, SCROLL_POLL_TIMER, 100, None) == 0 {
        SCROLL_COMPLETION
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancel_unstarted(ticket);
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Scroll-Aufnahme konnte den Abschlusstimer nicht starten.").as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x10,
        );
        return;
    }
    let diagnostic_folder = active_settings().save_folder;
    let coordinator = PRIMARY as usize;
    let target_window = root as usize;
    busy.disarm();
    let worker = std::thread::Builder::new()
        .name("scroll-capture".into())
        .spawn(move || {
            let result = (|| -> Result<Outcome, String> {
                let mut io =
                    scroll_windows::WindowsScrollIo::new(x, y, width, height, target_window)?;
                io.set_diagnostic_folder(diagnostic_folder);
                std::thread::sleep(std::time::Duration::from_millis(350));
                scroll_capture::run(&mut io, direction)
            })();
            scroll_completion::publish_scroll(ticket, result, || {
                PostMessageW(coordinator as H, SCROLL_DONE, ticket.to_wire(), 0)
            });
        });
    if let Err(error) = worker {
        scroll_completion::publish_scroll(
            ticket,
            Err(format!(
                "Scroll-Worker konnte nicht gestartet werden: {error}"
            )),
            || 0,
        );
        scroll_completion::finish_scroll(ticket);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollTargetActivationError {
    Unavailable,
    Unresponsive,
    Changed,
    NotForeground,
}

impl ScrollTargetActivationError {
    fn message(self) -> &'static str {
        match self {
            Self::Unavailable => {
                "Das ausgewählte Fenster ist nicht mehr verfügbar. Bitte erneut auswählen."
            }
            Self::Unresponsive => {
                "Windows konnte die Rückmeldung des ausgewählten Fensters nicht bestätigen. Bitte erneut versuchen."
            }
            Self::Changed => "Das ausgewählte Fenster hat sich geändert. Bitte erneut auswählen.",
            Self::NotForeground => {
                "Windows hat das ausgewählte Fenster nicht in den Vordergrund gebracht. Bitte erneut versuchen."
            }
        }
    }
}

fn verify_scroll_target_identity(
    expected_thread: u32,
    expected_process: u32,
    current_thread: u32,
    current_process: u32,
) -> Result<(), ScrollTargetActivationError> {
    if current_thread != expected_thread || current_process != expected_process {
        Err(ScrollTargetActivationError::Changed)
    } else {
        Ok(())
    }
}

// SAFETY CONTRACT:
// Voraussetzung: Aufruf auf dem UI-Thread, ohne gehaltene Zustands-Borrows oder
// Locks. root ist ein fremdes HWND und wird nie als Rust-Zeiger dereferenziert.
// Warum erfüllt: start_scroll hält nur lokale Werte; sein eigener Owner bleibt
// durch den ModalGuard des Nachrichtenhandlers am Leben. Fehlgeschlagener Send
// oder Thread-/PID-Wechsel des fremden HWND brechen ab; Wiederverwendung im
// selben Thread ist dadurch nicht ausgeschlossen. Vor Eingaben prüft zusätzlich
// WindowsScrollIo das exakte Vordergrundfenster.
// Nachrichten: SetForegroundWindow und SendMessageTimeoutW können synchron
// Nachrichten verarbeiten.
unsafe fn activate_scroll_target(root: H) -> Result<(), ScrollTargetActivationError> {
    activate_scroll_target_with_foreground(root, || GetForegroundWindow())
}

// Same contract as activate_scroll_target. The reader runs only after the real
// message barrier and identity checks; tests can supply an observed HWND without
// depending on desktop-wide foreground policy or changing another window's focus.
unsafe fn activate_scroll_target_with_foreground(
    root: H,
    foreground: impl FnOnce() -> H,
) -> Result<(), ScrollTargetActivationError> {
    let mut process = 0;
    let thread = GetWindowThreadProcessId(root, &mut process);
    if thread == 0 {
        return Err(ScrollTargetActivationError::Unavailable);
    }
    SetForegroundWindow(root);
    // Cross-thread activation is asynchronous. Let the target process its
    // activation notification before checking it; never start scrolling merely
    // because SetForegroundWindow accepted the request. A hung target remains
    // bounded and no input queues are attached.
    let mut result = 0;
    if SendMessageTimeoutW(
        root,
        0, // WM_NULL: wait for the activation notification, without input.
        0,
        0,
        0x0002, // SMTO_ABORTIFHUNG
        2_000,
        &mut result,
    ) == 0
    {
        return Err(ScrollTargetActivationError::Unresponsive);
    }
    let mut current_process = 0;
    let current_thread = GetWindowThreadProcessId(root, &mut current_process);
    verify_scroll_target_identity(thread, process, current_thread, current_process)?;
    if foreground() != root {
        return Err(ScrollTargetActivationError::NotForeground);
    }
    Ok(())
}

#[cfg(test)]
#[path = "region_tests.rs"]
mod tests;
