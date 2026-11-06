use super::*;

use shotkit::atomic_save_windows::write_atomic;

/// Avoid replacing an opened original when the export path cannot preserve its
/// source format faithfully. Unknown and currently unsupported formats fail
/// closed and must go through Save As as well.
fn requires_save_as(path: &Path) -> bool {
    !matches!(
        image_io::format_from_path(path),
        Ok(image::ImageFormat::Png | image::ImageFormat::Bmp)
    )
}

fn opened_original_requires_save_as(path: &Path, bytes: &[u8]) -> bool {
    requires_save_as(path)
        || !matches!(
            image::guess_format(bytes),
            Ok(image::ImageFormat::Png | image::ImageFormat::Bmp)
        )
}

fn confirm_save_as_for_original(owner: H) -> bool {
    // SAFETY: owner is the live editor HWND on its UI thread; no window-state
    // references are held across the modal call, which can dispatch messages.
    unsafe {
        MessageBoxW(
            owner,
            wide("Dieses Original kann beim direkten Speichern verändert oder unvollständig erhalten werden. Soll stattdessen unter einem neuen Namen gespeichert werden?").as_ptr(),
            wide("Original schützen").as_ptr(),
            0x24, // MB_ICONWARNING | MB_YESNO; only Yes proceeds via Save As.
        ) == 6
    }
}

#[repr(C)]
#[derive(Default)]
struct FileInformation {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

fn file_identity(path: &Path) -> Result<(u32, u32, u32), String> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: H, information: *mut FileInformation) -> i32;
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut info = FileInformation::default();
    // SAFETY: file owns a valid handle and info is writable for the duration
    // of this synchronous filesystem call, which does not dispatch UI messages.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((info.volume_serial, info.index_high, info.index_low))
}

fn same_existing_path(left: &Path, right: &Path) -> Result<bool, String> {
    let original = std::fs::canonicalize(left)
        .map_err(|e| format!("Originalpfad konnte nicht geprüft werden: {e}"))?;
    let target = match std::fs::canonicalize(right) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Zielpfad konnte nicht geprüft werden: {error}")),
    };
    if original
        .to_string_lossy()
        .eq_ignore_ascii_case(&target.to_string_lossy())
    {
        return Ok(true);
    }
    let left_id = file_identity(&original)
        .map_err(|e| format!("Originaldatei konnte nicht geprüft werden: {e}"))?;
    let right_id = file_identity(&target)
        .map_err(|e| format!("Zieldatei konnte nicht geprüft werden: {e}"))?;
    Ok(left_id == right_id)
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
    let previous = window_state(h).current_path.clone();
    let protected_original = window_state(h).protected_original;
    let force_save_as = !save_as && protected_original;
    if force_save_as && !confirm_save_as_for_original(h) {
        return false;
    }
    let (path, replace_existing) = match (save_as || force_save_as, previous.as_ref()) {
        (false, Some(path)) => (path.clone(), true),
        _ => {
            // A protected Save As starts without the original filename, and the
            // post-dialog check below prevents accepting that same file anyway.
            match file_dialog::save(
                h,
                if force_save_as {
                    None
                } else {
                    previous.as_deref()
                },
            ) {
                Ok(Some(selection)) => selection,
                Ok(None) => return false,
                Err(e) => {
                    MessageBoxW(h, wide(&e).as_ptr(), wide("Speicherdialog").as_ptr(), 0x10);
                    return false;
                }
            }
        }
    };
    if force_save_as {
        let Some(original) = previous.as_deref() else {
            return false;
        };
        let result = same_existing_path(original, &path);
        let message = match result {
            Ok(false) => None,
            Ok(true) => Some(
                "Bitte wählen Sie einen anderen Dateinamen, damit das Original erhalten bleibt."
                    .to_owned(),
            ),
            Err(error) => Some(error),
        };
        if let Some(message) = message {
            MessageBoxW(
                h,
                wide(&message).as_ptr(),
                wide("Original geschützt").as_ptr(),
                0x30,
            );
            return false;
        }
    }
    let result = (|| -> Result<(), String> {
        let format = image_io::format_from_path(&path)?;
        let image = bitmap_image(h)?;
        let bytes = image_io::encode(&image, format)?;
        write_atomic(&path, &bytes, replace_existing)
    })();
    match result {
        Ok(()) => {
            editor(h).document_mut().mark_saved();
            window_state(h).current_path = Some(path);
            window_state(h).protected_original = false;
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
        let (path, replace_existing) = match file_dialog::save(h, None) {
            Ok(Some(selection)) => selection,
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
                .and_then(|bytes| write_atomic(&path, &bytes, replace_existing))
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
    if window_state(h).bitmap.is_null() || !editor(h).document().is_dirty() {
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
    let result = (|| -> Result<(Image, bool), String> {
        let bytes = image_io::read_image_file(&path)?;
        let protected_original = opened_original_requires_save_as(&path, &bytes);
        Ok((image_io::decode(&bytes)?, protected_original))
    })();
    match result {
        Ok((image, protected_original)) => match display_image(h, image) {
            Ok(window) => {
                editor(window).document_mut().mark_saved();
                window_state(window).current_path = Some(path);
                window_state(window).protected_original = protected_original;
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
        0x00cf0000 | 0x02000000, // WS_CLIPCHILDREN keeps toolbar children out of parent repaint.
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
    window_state(h).protected_original = false;
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
    let image = editor(h).image_arc();
    let Some(rect) =
        shotkit::image_edit::crop_rect_from_points(a, b, (image.width as u32, image.height as u32))
    else {
        return;
    };
    if rect.width < 3 || rect.height < 3 {
        return;
    }
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
        InvalidateRect(h, std::ptr::null(), 1);
    } else if let Some(bitmap) = replacement {
        DeleteObject(bitmap);
    }
}

#[cfg(test)]
mod save_safety_tests {
    use super::{
        opened_original_requires_save_as, requires_save_as, same_existing_path, write_atomic,
    };
    use std::path::Path;

    #[test]
    fn png_and_bmp_can_be_overwritten_directly() {
        assert!(!requires_save_as(Path::new("capture.png")));
        assert!(!requires_save_as(Path::new("capture.bmp")));
    }

    #[test]
    fn lossy_and_unsupported_originals_require_save_as() {
        for path in [
            "capture.jpg",
            "capture.jpeg",
            "capture.gif",
            "capture.tif",
            "capture.tiff",
            "capture.webp",
            "capture",
        ] {
            assert!(requires_save_as(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn source_format_is_checked_even_when_extension_is_misleading() {
        let pixels = shotkit::stitch::Image::new(1, 1, vec![0, 0, 255, 255]).unwrap();
        let png = shotkit::image_io::encode(&pixels, image::ImageFormat::Png).unwrap();
        let jpeg = shotkit::image_io::encode(&pixels, image::ImageFormat::Jpeg).unwrap();
        assert!(!opened_original_requires_save_as(
            Path::new("image.png"),
            &png
        ));
        assert!(opened_original_requires_save_as(
            Path::new("image.png"),
            &jpeg
        ));
        assert!(opened_original_requires_save_as(
            Path::new("image.jpg"),
            &png
        ));
    }

    #[test]
    fn protected_save_as_rejects_the_original_path() {
        let path = std::env::temp_dir().join(format!(
            "shotkit-save-safety-{}-{}.jpg",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"fixture").unwrap();
        assert!(same_existing_path(&path, &path).unwrap());
        let alias = path.with_extension("hardlink.jpg");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(same_existing_path(&path, &alias).unwrap());
        std::fs::remove_file(&alias).unwrap();
        assert!(!same_existing_path(
            &path,
            &path.with_file_name("shotkit-save-safety-different.jpg")
        )
        .unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(same_existing_path(&path, &path).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn save_as_absent_target_collision_is_preserved_but_confirmed_target_replaces() {
        let dir = std::env::temp_dir().join(format!(
            "shotkit-atomic-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("image.png");
        let collision = b"created after the save dialog observed an absent target";
        let replacement = b"new encoded image";

        // Models another process creating the path between the dialog's
        // absence check and the atomic rename.
        std::fs::write(&path, collision).unwrap();
        assert!(write_atomic(&path, replacement, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), collision);

        // A dialog-confirmed existing target, and ordinary Ctrl+S, retain
        // their explicit replace behavior.
        write_atomic(&path, replacement, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), replacement);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
