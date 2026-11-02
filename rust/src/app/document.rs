use super::*;

pub(super) fn path_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("Ungültiger Speicherordner.")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = dir.join(format!(".shotkit-{}-{stamp}.tmp", std::process::id()));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if unsafe {
            MoveFileExW(
                path_wide(&temp).as_ptr(),
                path_wide(path).as_ptr(),
                0x1 | 0x8,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebender Editor auf dem UI-Thread.
// Warum erfüllt: Die Aktion wird aus der Fensterbehandlung für das eigene Fenster aufgerufen; Reentranz wird noch geprüft.
// Nachrichten: ReleaseCapture, GetSaveFileNameW und SetWindowTextW können reentrieren; Pfad und Bild sind eigene Werte, State-Borrows enden vor den Dialogen.
pub(super) unsafe fn save(h: H, save_as: bool) -> bool {
    let _modal = ModalGuard::new(h);
    cancel_gesture(h);
    if window_state(h).bitmap.is_null() {
        return false;
    }
    let previous = (&window_state(h).current_path).clone();
    let path = if save_as || previous.is_none() {
        match file_dialog::save(h, previous.as_deref()) {
            Ok(Some(p)) => p,
            Ok(None) => return false,
            Err(e) => {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherdialog").as_ptr(), 0x10);
                return false;
            }
        }
    } else {
        previous.unwrap()
    };
    let result = (|| -> Result<(), String> {
        let format = image_io::format_from_path(&path)?;
        let image = bitmap_image(h)?;
        let bytes = image_io::encode(&image, format)?;
        write_atomic(&path, &bytes)
    })();
    match result {
        Ok(()) => {
            window_state(h).current_path = Some(path);
            window_state(h).dirty = false;
            refresh_title(h);
            true
        }
        Err(e) => {
            MessageBoxW(
                h,
                wide(&format!("Speichern fehlgeschlagen:\n{e}")).as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            false
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Besitzer für synchrone Dialoge.
// Warum zu prüfen: Neben dem Tray-Ablauf gibt es WM_CLOSE; deren Reentranzschutz ist noch nicht vollständig belegt.
// Nachrichten: MessageBoxW und GetSaveFileNameW können reentrieren; RECOVERY-Guards enden vor Dialogen, beim Dateischreiben bleibt ein Guard bestehen.
pub(super) unsafe fn resolve_recovery(h: H) -> bool {
    loop {
        if RECOVERY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
        {
            return true;
        }
        let answer=MessageBoxW(h,wide("Eine Aufnahme konnte nicht im Editor geöffnet werden und ist noch im Speicher. Jetzt speichern?").as_ptr(),wide("Ungesicherte Aufnahme").as_ptr(),0x23);
        if answer == 2 {
            return false;
        }
        if answer == 7 {
            RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).pop();
            continue;
        }
        if answer != 6 {
            return false;
        }
        let path = match file_dialog::save(h, None) {
            Ok(Some(path)) => path,
            Ok(None) => return false,
            Err(e) => {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherdialog").as_ptr(), 0x10);
                return false;
            }
        };
        let result = {
            let pending = RECOVERY.lock().unwrap_or_else(|p| p.into_inner());
            let Some(image) = pending.last() else {
                continue;
            };
            image_io::format_from_path(&path)
                .and_then(|format| image_io::encode(image, format))
                .and_then(|bytes| write_atomic(&path, &bytes))
        };
        match result {
            Ok(()) => {
                RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).pop();
            }
            Err(e) => {
                MessageBoxW(
                    h,
                    wide(&e).as_ptr(),
                    wide("Speichern fehlgeschlagen").as_ptr(),
                    0x10,
                );
                return false;
            }
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h und sein Editorzustand leben während der synchronen Bestätigung.
// Warum erfüllt: Aufruf für eigene Fenster im guarded Nachrichtenhandler; Reentranz muss insgesamt geprüft bleiben.
// Nachrichten: ReleaseCapture, MessageBoxW und Speicherdialoge können reentrieren; Zustandsabfragen liefern kopierte boolesche Werte.
pub(super) unsafe fn confirm_replace(h: H) -> bool {
    let _modal = ModalGuard::new(h);
    cancel_gesture(h);
    if !window_state(h).dirty || window_state(h).bitmap.is_null() {
        return true;
    }
    match MessageBoxW(
        h,
        wide("Das Bild enthält ungespeicherte Änderungen. Jetzt speichern?").as_ptr(),
        wide("ShotKit Rust").as_ptr(),
        0x23,
    ) {
        6 => save(h, false),
        7 => true,
        _ => false,
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster auf dem UI-Thread.
// Warum erfüllt: Aufruf aus der eigenen Ereignisbehandlung; Clipboard-Ressourcen werden im Clipboard-Modul verwaltet.
// Nachrichten: GetClipboardData und Fensteranlage können reentrieren; das eingelesene Bild ist besitzend und kein Editor-Borrow wird gehalten.
pub(super) unsafe fn paste_image(h: H) {
    match clipboard::read_image(h) {
        Ok(Some(image)) => {
            if let Err(e) = display_image(h, image) {
                MessageBoxW(h, wide(&e).as_ptr(), wide("Zwischenablage").as_ptr(), 0x10);
            }
        }
        Ok(None) => {
            MessageBoxW(
                h,
                wide("Die Zwischenablage enthält kein Bild.").as_ptr(),
                wide("Zwischenablage").as_ptr(),
                0x40,
            );
        }
        Err(e) => {
            MessageBoxW(h, wide(&e).as_ptr(), wide("Zwischenablage").as_ptr(), 0x10);
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebender Besitzer für den Dateidialog.
// Warum erfüllt: Aufruf aus dem eigenen Fensterhandler, der den modalen Ablauf schützt.
// Nachrichten: GetOpenFileNameW und MessageBoxW können reentrieren; der zurückgegebene Pfad ist besitzend und kein State-Borrow wird gehalten.
pub(super) unsafe fn open_image(h: H) {
    let path = match file_dialog::open(h) {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(e) => {
            MessageBoxW(h, wide(&e).as_ptr(), wide("Öffnen-Dialog").as_ptr(), 0x10);
            return;
        }
    };
    if let Err(error) = open_image_path(h, path) {
        MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Fenster auf dem UI-Thread.
// Warum erfüllt: Aufrufer sind Einzelinstanz- und Editorabläufe; path ist ein besitzender Wert.
// Nachrichten: Fensteranlage und SetWindowTextW können reentrieren; Bild und Pfad sind besitzend, State-Zuweisungen enden vor der Titelaktualisierung.
pub(super) unsafe fn open_image_path(h: H, path: PathBuf) -> Result<(), String> {
    let result = (|| -> Result<Image, String> {
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > 512 * 1024 * 1024 {
            return Err("Die Bilddatei ist zu groß.".into());
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        image_io::decode(&bytes)
    })();
    match result {
        Ok(image) => match display_image(h, image) {
            Ok(window) => {
                window_state(window).current_path = Some(path);
                window_state(window).dirty = false;
                refresh_title(window);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    }
}
// SAFETY CONTRACT:
// Voraussetzung: Die Fensterklasse ist auf dem aktuellen UI-Thread registriert.
// Warum erfüllt: run registriert die Klasse vor dem Erzeugen; Fehler aus CreateWindow und Toolbar werden geprüft.
// Nachrichten: CreateWindowExW, SendMessageW und DestroyWindow können reentrieren; die ersetzte Toolbar wird nach Ende des State-Borrows freigegeben.
pub(super) unsafe fn create_editor_window() -> Result<H, String> {
    let class = wide("ShotKitRustEditor");
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        wide("ShotKit Rust").as_ptr(),
        0x00cf0000,
        0x80000000u32 as i32,
        0x80000000u32 as i32,
        1000,
        700,
        null_mut(),
        null_mut(),
        GetModuleHandleW(std::ptr::null()),
        null_mut(),
    );
    if h.is_null() {
        return Err("Editorfenster konnte nicht erstellt werden.".into());
    }
    let bar = match toolbar::create(h) {
        Ok(bar) => Rc::new(bar),
        Err(error) => {
            DestroyWindow(h);
            return Err(error);
        }
    };
    // Drop replaced native objects only after the mutable state borrow ends.
    let previous = window_state(h).toolbar.replace(bar);
    drop(previous);
    Ok(h)
}
// SAFETY CONTRACT:
// Voraussetzung: Aufruf erfolgt auf dem UI-Thread nach Klassenregistrierung.
// Warum erfüllt: Der Editorablauf verwaltet Fensteranlage und übergibt den Bildpuffer als eigenen Wert.
// Nachrichten: Fensteranlage, ShowWindow und DestroyWindow können reentrieren; RECOVERY- und Fensterlisten-Zugriffe enden vor diesen Aufrufen.
pub(super) unsafe fn display_image(_owner: H, image: Image) -> Result<H, String> {
    if QUITTING.load(Ordering::SeqCst) {
        RECOVERY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(image);
        return Err("ShotKit wird gerade beendet. Die Aufnahme bleibt im Speicher.".into());
    }
    let h = match create_editor_window() {
        Ok(h) => h,
        Err(error) => {
            RECOVERY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(image);
            return Err(format!("{error} Die Aufnahme bleibt im Speicher."));
        }
    };
    match initialize_image(h, image) {
        Ok(()) => {
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(h);
            refresh_title(h);
            ShowWindow(h, 5);
            SetForegroundWindow(h);
            Ok(h)
        }
        Err(error) => {
            DestroyWindow(h);
            Err(error)
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein neu erzeugtes eigenes Editorfenster.
// Warum erfüllt: create_editor_window installiert den Zustand, bevor der Aufrufer diese Funktion ausführt.
// Nachrichten: ReleaseCapture und SetWindowTextW können reentrieren; Controller und Bild-Arc sind eigene Werte, State-Zuweisungen enden vorher.
pub(super) unsafe fn initialize_image(h: H, image: Image) -> Result<(), String> {
    let bmp = match create_bitmap(&image) {
        Ok(bmp) => bmp,
        Err(error) => {
            RECOVERY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(image);
            return Err(format!("{error} Die Aufnahme bleibt im Speicher."));
        }
    };
    let state = match EditorController::with_image_recover(image) {
        Ok(state) => state,
        Err((e, image)) => {
            DeleteObject(bmp);
            RECOVERY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(image);
            return Err(format!(
                "Bild konnte nicht geladen werden: {e:?}. Die Aufnahme bleibt im Speicher."
            ));
        }
    };
    let image = state.image_arc();
    cancel_gesture(h);
    install_bitmap(h, bmp, &image);
    window_state(h).editor = Some(state);
    window_state(h).current_path = None;
    window_state(h).dirty = true;
    window_state(h).zoom = 1.0;
    window_state(h).pan_x = 0;
    window_state(h).pan_y = 0;
    SetWindowTextW(
        h,
        wide("ShotKit Rust – Strg+S speichern · F10 Scroll-Aufnahme").as_ptr(),
    );
    InvalidateRect(h, std::ptr::null(), 1);
    Ok(())
}
// SAFETY CONTRACT:
// Voraussetzung: h besitzt einen lebenden Editorzustand auf dem UI-Thread.
// Warum erfüllt: Aufruf aus dem Mausereignis des eigenen Fensters; die Bildlogik prüft die Grenzen.
// Nachrichten: Toolbar- und Titelfunktionen sowie MessageBoxW können reentrieren; das Ergebnisbild ist ein Arc und Editor-Borrows enden vor diesen Aufrufen.
pub(super) unsafe fn apply_crop(h: H, a: (i32, i32), b: (i32, i32)) {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    let width = a.0.max(b.0) - x;
    let height = a.1.max(b.1) - y;
    if width < 3 || height < 3 {
        return;
    }
    let rect = CropRect {
        x: x as usize,
        y: y as usize,
        width: width as usize,
        height: height as usize,
    };
    let result = (|| -> Result<(), String> {
        editor(h)
            .document_mut()
            .begin_edit()
            .map_err(|e| format!("{e:?}"))?;
        if let Err(e) = editor(h).crop(rect) {
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{e:?}"));
        }
        let output = editor(h).image_arc();
        let bitmap = match create_bitmap(&output) {
            Ok(bmp) => bmp,
            Err(e) => {
                let _ = editor(h).document_mut().abort_edit();
                return Err(e);
            }
        };
        if let Err(e) = editor(h).document_mut().commit_edit() {
            DeleteObject(bitmap);
            let _ = editor(h).document_mut().abort_edit();
            return Err(format!("{e:?}"));
        }
        install_bitmap(h, bitmap, &output);
        window_state(h).dirty = true;
        window_state(h).zoom = 1.0;
        window_state(h).pan_x = 0;
        window_state(h).pan_y = 0;
        editor(h).set_tool(Tool::Select);
        toolbar::set_active(h, toolbar::ID_SELECT);
        refresh_title(h);
        Ok(())
    })();
    if let Err(e) = result {
        MessageBoxW(
            h,
            wide(&format!("Zuschneiden fehlgeschlagen: {e}")).as_ptr(),
            wide("ShotKit Rust").as_ptr(),
            0x10,
        );
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h besitzt einen lebenden Editorzustand auf dem UI-Thread.
// Warum erfüllt: Aufruf aus der Nachrichtenbehandlung des eigenen Fensters; Bitmap-Fehler werden behandelt.
// Nachrichten: ReleaseCapture und MessageBoxW können reentrieren; History-Ergebnisse und Bilder sind Werte beziehungsweise eigene Arcs.
pub(super) unsafe fn history(h: H, redo: bool) {
    cancel_gesture(h);
    let before = editor(h).image_arc();
    let next = if redo {
        editor(h).document().redo_image_arc()
    } else {
        editor(h).document().undo_image_arc()
    };
    let Some(next) = next else {
        return;
    };
    // Allocate the replacement GDI bitmap before changing history. On failure,
    // both the document and the displayed image remain at their old state.
    let replacement = if std::sync::Arc::ptr_eq(&before, &next) {
        None
    } else {
        match create_bitmap(&next) {
            Ok(bitmap) => Some(bitmap),
            Err(e) => {
                MessageBoxW(
                    h,
                    wide(&e).as_ptr(),
                    wide("Rückgängig/Wiederholen").as_ptr(),
                    0x10,
                );
                return;
            }
        }
    };
    let changed = if redo {
        editor(h).redo()
    } else {
        editor(h).undo()
    };
    if changed {
        if let Some(bitmap) = replacement {
            install_bitmap(h, bitmap, &next);
        }
        window_state(h).dirty = true;
        InvalidateRect(h, std::ptr::null(), 1);
    } else if let Some(bitmap) = replacement {
        DeleteObject(bitmap);
    }
}
