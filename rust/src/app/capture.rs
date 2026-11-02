use super::*;

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
                        if was_visible { ShowWindow(h, 5); }
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
pub(super) unsafe fn capture(h: H) {
    capture_rect(h, capture_targets::monitor_under_cursor());
}
pub(super) unsafe fn schedule_capture(h: H, seconds: u64) {
    let request = DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).begin();
    let ticket = match request {
        Ok(ticket) => ticket,
        Err(error) => {
            let message = match error {
                BeginError::Busy => "Eine verzögerte Aufnahme läuft bereits.",
                BeginError::Exhausted => "Keine weitere verzögerte Aufnahme möglich. Bitte ShotKit neu starten.",
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
        if !DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).is_waiting(ticket) {
            return;
        }
        // Completion can race with this PostMessage. The receiver therefore
        // checks the same generation and claims Waiting -> Running exactly once.
        if unsafe { PostMessageW(hwnd as H, DELAYED_CAPTURE, ticket.to_wire(), 0) } == 0 {
            DELAYED_CAPTURE_STATE.lock().unwrap_or_else(|p| p.into_inner()).cancel_waiting(ticket);
        }
    });
}
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
pub(super) unsafe fn active_window_target(
    h: H,
    window: H,
) -> Result<ActiveWindowSnapshot, String> {
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
        return Err("Windows konnte Prozess und Thread des aktiven Fensters nicht bestimmen.".into());
    }
    Ok(ActiveWindowSnapshot {
        hwnd: root,
        process_id,
        thread_id,
        class_name,
    })
}
pub(super) unsafe fn resolve_active_window_target(
    h: H,
    saved: &ActiveWindowSnapshot,
) -> Result<capture_targets::ScreenRect, String> {
    if IsWindow(saved.hwnd) == 0 {
        return Err("Das zuvor aktive Fenster wurde geschlossen, bevor die Aufnahme startete.".into());
    }
    let current = active_window_target(h, saved.hwnd).map_err(|_| {
        "Das zuvor aktive Fenster ist nicht mehr als dasselbe gültige Zielfenster verfügbar.".to_string()
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
pub(super) unsafe fn capture_region(h: H) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _busy = BusyGuard(true);
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    let Some((x, y, w, height)) = overlay::select() else {
        if was_visible {
            ShowWindow(h, 5);
        }
        return;
    };
    let Some(rect) = last_region::Rect::from_origin_size(x, y, w, height) else {
        if was_visible { ShowWindow(h, 5); }
        MessageBoxW(h, wide("Die Auswahl hat ungültige Bildschirmgrenzen.").as_ptr(), wide("Bereichsaufnahme").as_ptr(), 0x10);
        return;
    };
    capture_region_rect(h, rect, true, was_visible);
}

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
        return Err("Die aktuellen Monitorgrenzen konnten nicht zuverlässig gelesen werden.".into());
    }
    Ok(list.rects)
}

pub(super) unsafe fn repeat_last_region(h: H) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _busy = BusyGuard(true);
    let Some(rect) = *LAST_REGION.lock().unwrap_or_else(|p| p.into_inner()) else {
        MessageBoxW(
            h,
            wide("Es wurde in dieser Sitzung noch kein Aufnahmebereich erfolgreich aufgenommen.").as_ptr(),
            wide("Bereich erneut aufnehmen").as_ptr(),
            0x30,
        );
        return;
    };
    let was_visible = IsWindowVisible(h) != 0;
    ShowWindow(h, 0);
    capture_region_rect(h, rect, false, was_visible);
}

pub(super) unsafe fn start_scroll(h: H) {
    if SCROLL_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let mut busy = BusyGuard(true);
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
    prefs.scroll_direction = match direction {
        Direction::Down => settings::ScrollDirection::Down,
        Direction::Right => settings::ScrollDirection::Right,
        Direction::Left => settings::ScrollDirection::Left,
    };
    // save_settings refuses to replace unreadable or unsupported settings
    // files; a persistence error must not prevent the requested capture.
    let _ = save_settings(h, prefs);
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
    SetForegroundWindow(root);
    if GetForegroundWindow() != root {
        if was_visible {
            ShowWindow(h, 5);
        }
        MessageBoxW(
            h,
            wide("Zielfenster konnte nicht aktiviert werden. Bitte erneut versuchen.").as_ptr(),
            wide("Scroll-Aufnahme").as_ptr(),
            0x30,
        );
        return;
    }
    RESTORE_EDITOR.store(was_visible, Ordering::SeqCst);
    busy.0 = false;
    let hwnd = h as usize;
    let target_window = root as usize;
    std::thread::spawn(move || {
        let result = (|| -> Result<Outcome, String> {
            let mut io = scroll_windows::WindowsScrollIo::new(x, y, width, height, target_window)?;
            std::thread::sleep(std::time::Duration::from_millis(350));
            scroll_capture::run(&mut io, direction)
        })();
        *SCROLL_RESULT.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
        if PostMessageW(hwnd as H, SCROLL_DONE, 0, 0) == 0 {
            SCROLL_RESULT
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            SCROLL_BUSY.store(false, Ordering::SeqCst);
        }
    });
}
