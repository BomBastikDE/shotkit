use super::*;

#[cfg(test)]
use shotkit::atomic_save_windows::write_atomic;
use shotkit::atomic_save_windows::write_atomic_with;

pub(super) fn write_image_atomic(
    path: &Path,
    image: &Image,
    format: image::ImageFormat,
    replace: bool,
) -> Result<(), String> {
    if format == image::ImageFormat::Png {
        write_atomic_with(path, replace, |file| {
            image_io::write_png_file(image, file).map_err(std::io::Error::other)
        })
    } else if matches!(format, image::ImageFormat::Bmp | image::ImageFormat::Jpeg) {
        write_atomic_with(path, replace, |file| {
            image_io::write_bmp_or_jpeg_file(image, file, format).map_err(std::io::Error::other)
        })
    } else if format == image::ImageFormat::Gif {
        write_atomic_with(path, replace, |file| {
            image_io::write_gif_file(image, file).map_err(std::io::Error::other)
        })
    } else {
        Err("Bitte PNG, JPG, BMP oder GIF wählen.".into())
    }
}

/// Avoid replacing an opened original when the export path cannot preserve its
/// source format faithfully. Unknown and currently unsupported formats fail
/// closed and must go through Save As as well.
fn requires_save_as(path: &Path) -> bool {
    !matches!(
        image_io::format_from_path(path),
        Ok(image::ImageFormat::Png | image::ImageFormat::Bmp)
    )
}

pub(super) fn opened_original_requires_save_as(
    path: &Path,
    format: Option<image::ImageFormat>,
) -> bool {
    requires_save_as(path)
        || !matches!(
            format,
            Some(image::ImageFormat::Png | image::ImageFormat::Bmp)
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

fn file_identity(file: &std::fs::File) -> Result<(u32, u32, u32), String> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: H, information: *mut FileInformation) -> i32;
    }
    let mut info = FileInformation::default();
    // SAFETY: file owns a valid handle and info is writable for the duration
    // of this synchronous filesystem call, which does not dispatch UI messages.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((info.volume_serial, info.index_high, info.index_low))
}

fn protect_original(path: &Path) -> Result<std::fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    // Share reads but deny writes and deletes while the protected export runs.
    // If another incompatible handle already exists, fail closed.
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1) // FILE_SHARE_READ
        .open(path)
        .map_err(|e| format!("Originaldatei konnte nicht geschützt werden: {e}"))
}

fn same_existing_path(original: &std::fs::File, right: &Path) -> Result<bool, String> {
    let target = match std::fs::canonicalize(right) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Zielpfad konnte nicht geprüft werden: {error}")),
    };
    let left_id = file_identity(original)
        .map_err(|e| format!("Originaldatei konnte nicht geprüft werden: {e}"))?;
    let target_file = std::fs::File::open(&target)
        .map_err(|e| format!("Zieldatei konnte nicht geprüft werden: {e}"))?;
    let right_id = file_identity(&target_file)
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
    let protected_guard = if force_save_as {
        let Some(original) = previous.as_deref() else {
            return false;
        };
        let guard = match protect_original(original) {
            Ok(guard) => guard,
            Err(error) => {
                MessageBoxW(
                    h,
                    wide(&error).as_ptr(),
                    wide("Original geschützt").as_ptr(),
                    0x30,
                );
                return false;
            }
        };
        let result = same_existing_path(&guard, &path);
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
        Some(guard)
    } else {
        None
    };
    let result = (|| -> Result<(), String> {
        let format = image_io::format_from_path(&path)?;
        // Keep the shared source or tracked annotation output alive through
        // encoding and atomic publication; borrowing pixels detaches no lease.
        let image = bitmap_image(h)?;
        write_image_atomic(&path, &image, format, replace_existing)
    })();
    // The original remains protected until the synchronous rename returns.
    drop(protected_guard);
    match result {
        Ok(()) => {
            editor(h).document_mut().mark_saved();
            window_state(h).current_path = Some(path.clone());
            window_state(h).protected_original = false;
            refresh_title(h);
            // The save is complete; a remnant check of a network folder (#67)
            // cannot change its result.
            temp_recovery::after_save(h, &path);
            true
        }
        Err(e) => {
            MessageBoxW(
                h,
                wide(&format!("Speichern fehlgeschlagen:\n{e}")).as_ptr(),
                wide("ShotKit").as_ptr(),
                0x10,
            );
            false
        }
    }
}

fn prepare_scroll_recovery(
    pending: &mut Option<PendingScrollRecovery>,
    rotate: impl FnOnce(Image, Rotation) -> Result<Image, shotkit::stitch::RotationFailure>,
) -> Result<(), &'static str> {
    let Some(recovery) = pending.take() else {
        return Ok(());
    };
    let Some(rotation) = recovery.pending_rotation else {
        *pending = Some(recovery);
        return Ok(());
    };
    match rotate(recovery.image, rotation) {
        Ok(image) => {
            // Commit the orientation before exporting: a later export failure
            // must retry the correctly oriented image without rotating it again.
            *pending = Some(PendingScrollRecovery {
                image,
                pending_rotation: None,
            });
            Ok(())
        }
        Err(failure) => {
            *pending = Some(PendingScrollRecovery {
                image: failure.image,
                pending_rotation: Some(rotation),
            });
            Err(failure.error)
        }
    }
}

fn save_scroll_recovery(
    pending: &mut Option<PendingScrollRecovery>,
    path: &Path,
    replace_existing: bool,
) -> Result<(), String> {
    prepare_scroll_recovery(pending, Image::try_rotate)
        .map_err(|error| format!("Die Rückdrehung ist erneut fehlgeschlagen: {error}. Die Aufnahme und ihre Drehrichtung bleiben im Speicher."))?;
    let Some(recovery) = pending.as_ref() else {
        return Ok(());
    };
    let format = image_io::format_from_path(path)?;
    write_image_atomic(path, &recovery.image, format, replace_existing)?;
    pending.take();
    Ok(())
}

struct RecoveryResolveGuard;
impl Drop for RecoveryResolveGuard {
    fn drop(&mut self) {
        RECOVERY_RESOLVING.store(false, Ordering::SeqCst);
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Besitzer für synchrone Dialoge.
// Warum erfüllt: ModalGuard schützt den Besitzer, RecoveryResolveGuard verhindert
// einen zweiten Recovery-Ablauf während einer verschachtelten Nachrichtenpumpe.
// Nachrichten: MessageBoxW und GetSaveFileNameW können reentrieren; alle Mutex-
// Guards enden vor Dialogen. Rotation, Encoding und Dateischreiben pumpen nicht.
pub(super) unsafe fn resolve_recovery(h: H) -> bool {
    let Ok(_source_gate) = recovery_admission::acquire_resolution(&SCROLL_BUSY) else {
        return false;
    };
    if RECOVERY_RESOLVING.swap(true, Ordering::SeqCst) {
        return false;
    }
    let _resolving = RecoveryResolveGuard;
    let _modal = ModalGuard::new(h);
    loop {
        if import_transfer_active() {
            return false;
        }
        let scroll_pending = scroll_recovery_pending();
        if !scroll_pending
            && RECOVERY
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        {
            return resolve_capture_recovery(h) && resolve_import_recovery(h);
        }
        let message = if scroll_pending {
            "Eine horizontale Scroll-Aufnahme ist noch im Speicher. Vor dem Speichern wird eine noch ausstehende Rückdrehung erneut versucht.\n\nJa = speichern\nNein = Aufnahme verwerfen\nAbbrechen = im Speicher behalten"
        } else {
            "Eine Aufnahme konnte nicht im Editor geöffnet werden und ist noch im Speicher. Jetzt speichern?"
        };
        let answer = MessageBoxW(
            h,
            wide(message).as_ptr(),
            wide("Ungesicherte Aufnahme").as_ptr(),
            0x23,
        );
        if answer == 2 {
            return false;
        }
        if answer == 7 {
            if scroll_pending {
                SCROLL_RECOVERY
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
            } else {
                RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).pop();
            }
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
        let result = if scroll_pending {
            let mut pending = SCROLL_RECOVERY.lock().unwrap_or_else(|p| p.into_inner());
            save_scroll_recovery(&mut pending, &path, replace_existing)
        } else {
            let pending = RECOVERY.lock().unwrap_or_else(|p| p.into_inner());
            let Some(image) = pending.last() else {
                continue;
            };
            image_io::format_from_path(&path)
                .and_then(|format| write_image_atomic(&path, image, format, replace_existing))
        };
        match result {
            Ok(()) => {
                if !scroll_pending {
                    RECOVERY.lock().unwrap_or_else(|p| p.into_inner()).pop();
                }
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
        wide("ShotKit").as_ptr(),
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
    let _modal = ModalGuard::new(h);
    let Some(_busy) = acquire_recovery_safe_source(h) else {
        return;
    };
    match paste_budgeted_image(h) {
        Ok(true) => {}
        Ok(false) => {
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
    open_budgeted_image(h, path)
}
// SAFETY CONTRACT:
// Voraussetzung: Die Fensterklasse ist auf dem aktuellen UI-Thread registriert.
// Warum erfüllt: run registriert die Klasse vor dem Erzeugen; Fehler aus CreateWindow und Toolbar werden geprüft.
// Nachrichten: CreateWindowExW, SendMessageW und DestroyWindow können reentrieren; die ersetzte Toolbar wird nach Ende des State-Borrows freigegeben.
pub(super) unsafe fn create_editor_window() -> Result<H, String> {
    create_editor_window_with_protection(None)
}

// The returned guard must cover ALL initialization, registration and handoff
// calls that can synchronously dispatch WM_CLOSE. On initialization failure,
// consume it with abandon_protected_editor; ordinary Drop commits the handoff.
pub(super) unsafe fn create_editor_window_protected(
) -> Result<(H, EditorInitializationGuard), String> {
    if QUITTING.load(Ordering::SeqCst) {
        return Err("ShotKit wird gerade beendet.".into());
    }
    let protection = begin_protected_editor_creation()?;
    let result = create_editor_window_with_protection(Some(&protection));
    finish_protected_editor_creation(&protection, result.is_ok());
    result.map(|h| (h, EditorInitializationGuard(Some(protection))))
}

// No WindowState/toolbar reference crosses a Win32 call. The optional marker
// lives through CreateWindowExW; its per-window token also covers toolbar setup.
unsafe fn create_editor_window_with_protection(
    protection: Option<&EditorCreationProtection>,
) -> Result<H, String> {
    let class = wide("ShotKitRustEditor");
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        wide("ShotKit").as_ptr(),
        0x00cf0000 | 0x02000000, // WS_CLIPCHILDREN keeps toolbar children out of parent repaint.
        0x80000000u32 as i32,
        0x80000000u32 as i32,
        1000,
        700,
        null_mut(),
        null_mut(),
        GetModuleHandleW(std::ptr::null()),
        protection.map_or(null_mut(), |marker| {
            std::ptr::from_ref(marker).cast_mut().cast()
        }),
    );
    if h.is_null() {
        return Err("Editorfenster konnte nicht erstellt werden.".into());
    }
    if protection.is_some_and(|marker| marker.lifetime.get() != h) {
        return Err("Editorfenster wurde während der Anlage geschlossen.".into());
    }
    let bar = match toolbar::create(h) {
        Ok(bar) => Rc::new(bar),
        Err(error) => {
            // Child setup may have destroyed the original HWND reentrantly.
            // Never destroy a recycled numeric handle on the protected path.
            let live = protection.map_or(h, |marker| marker.lifetime.get());
            if !live.is_null() {
                DestroyWindow(live);
            }
            return Err(error);
        }
    };
    if protection.is_some_and(|marker| marker.lifetime.get() != h) {
        return Err("Editorfenster wurde während der Toolbaranlage geschlossen.".into());
    }
    // Drop replaced native objects only after the mutable state borrow ends.
    let previous = window_state(h).toolbar.replace(bar);
    drop(previous);
    if protection.is_some_and(|marker| marker.lifetime.get() != h) {
        return Err("Editorfenster wurde während der Toolbaranlage geschlossen.".into());
    }
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
                "Bild konnte nicht geladen werden: {e}. Die Aufnahme bleibt im Speicher."
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
        wide("ShotKit – Strg+S speichern · F10 Scroll-Aufnahme").as_ptr(),
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
            .map_err(|e| e.to_string())?;
        if let Err(e) = editor(h).crop_with_budget(rect, &shared_image_budget()) {
            let _ = editor(h).document_mut().abort_edit();
            return Err(e.to_string());
        }
        let output = editor(h).image_arc();
        let bitmap = match create_bitmap(&output) {
            Ok(bmp) => bmp,
            Err(e) => {
                let _ = editor(h).document_mut().abort_edit();
                return Err(e);
            }
        };
        match confirm_and_commit_staged_edit(h) {
            Ok(Some(_)) => {}
            Ok(None) => {
                DeleteObject(bitmap);
                let _ = editor(h).document_mut().abort_edit();
                return Ok(());
            }
            Err(e) => {
                DeleteObject(bitmap);
                let _ = editor(h).document_mut().abort_edit();
                return Err(e.to_string());
            }
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
            wide("ShotKit").as_ptr(),
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
    let admission = if redo {
        editor(h).document().check_redo()
    } else {
        editor(h).document().check_undo()
    };
    match admission {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            MessageBoxW(
                h,
                wide(&error.to_string()).as_ptr(),
                wide("Rückgängig/Wiederholen").as_ptr(),
                0x10,
            );
            return;
        }
    }
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
    let replacement = if before.ptr_eq(&next) {
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
    } else {
        if let Some(bitmap) = replacement {
            DeleteObject(bitmap);
        }
        report_editor_error(h);
    }
}

#[cfg(test)]
mod scroll_recovery_tests {
    use super::*;

    #[test]
    fn failed_rotation_keeps_pixels_allocation_and_direction_for_retry() {
        for rotation in [Rotation::Clockwise, Rotation::Counterclockwise] {
            let image = Image::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
            let source = image.pixels.as_ptr();
            let expected = image.clone();
            let mut pending = Some(PendingScrollRecovery {
                image,
                pending_rotation: Some(rotation),
            });
            let error = prepare_scroll_recovery(&mut pending, |image, direction| {
                assert_eq!(direction, rotation);
                Err(shotkit::stitch::RotationFailure {
                    image,
                    error: "injected allocation failure",
                })
            });
            assert_eq!(error, Err("injected allocation failure"));
            let retained = pending.as_ref().unwrap();
            assert_eq!(retained.image, expected);
            assert_eq!(retained.image.pixels.as_ptr(), source);
            assert_eq!(retained.pending_rotation, Some(rotation));
            prepare_scroll_recovery(&mut pending, Image::try_rotate).unwrap();
            assert_eq!(pending.as_ref().unwrap().pending_rotation, None);
            assert_eq!(pending.as_ref().unwrap().image.width, 1);
            assert_eq!(pending.as_ref().unwrap().image.height, 2);
        }
    }

    #[test]
    fn failed_export_retains_correct_orientation_and_retry_saves_it_once() {
        let root = std::env::temp_dir().join(format!(
            "shotkit-scroll-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (name, rotation) in [
            ("right", Rotation::Counterclockwise),
            ("left", Rotation::Clockwise),
        ] {
            let image =
                Image::new(2, 3, (1..=6).flat_map(|value| [value, 0, 0, 255]).collect()).unwrap();
            let expected = image.clone().try_rotate(rotation).unwrap();
            let mut pending = Some(PendingScrollRecovery {
                image,
                pending_rotation: Some(rotation),
            });
            let path = root.join(name).join("capture.png");
            // The absent directory makes the actual atomic file export fail
            // after rotation/encoding, without simulating a successful save.
            assert!(save_scroll_recovery(&mut pending, &path, false).is_err());
            let retained = pending.as_ref().unwrap();
            assert_eq!(retained.image, expected);
            assert_eq!(retained.pending_rotation, None);
            prepare_scroll_recovery(&mut pending, |_, _| {
                panic!("an export retry must not rotate again")
            })
            .unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            save_scroll_recovery(&mut pending, &path, false).unwrap();
            assert!(pending.is_none());
            let decoded = image_io::decode(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(decoded, expected);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod save_safety_tests {
    use super::{
        opened_original_requires_save_as, protect_original, requires_save_as, same_existing_path,
        write_atomic, write_image_atomic,
    };
    use std::path::Path;

    #[test]
    fn failed_streamed_image_export_preserves_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "shotkit-stream-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let invalid = shotkit::stitch::Image {
            width: 2,
            height: 2,
            pixels: vec![0; 15],
        };
        let valid = shotkit::stitch::Image::new(2, 2, [10, 20, 30, 255].repeat(4)).unwrap();
        for (name, format) in [
            ("test.bmp", image::ImageFormat::Bmp),
            ("test.jpg", image::ImageFormat::Jpeg),
            ("test.gif", image::ImageFormat::Gif),
        ] {
            let path = root.join(name);
            std::fs::write(&path, b"existing").unwrap();
            assert!(write_image_atomic(&path, &invalid, format, true).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"existing");
            write_image_atomic(&path, &valid, format, true).unwrap();
            let output = std::fs::read(&path).unwrap();
            assert_eq!(image::guess_format(&output).unwrap(), format);
            let decoded = shotkit::image_io::decode(&output).unwrap();
            assert_eq!((decoded.width, decoded.height), (2, 2));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

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
            image::guess_format(&png).ok()
        ));
        assert!(opened_original_requires_save_as(
            Path::new("image.png"),
            image::guess_format(&jpeg).ok()
        ));
        assert!(opened_original_requires_save_as(
            Path::new("image.jpg"),
            image::guess_format(&png).ok()
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
        let alias = path.with_extension("hardlink.jpg");
        std::fs::hard_link(&path, &alias).unwrap();
        let guard = protect_original(&path).unwrap();
        assert!(same_existing_path(&guard, &path).unwrap());
        assert!(same_existing_path(&guard, &alias).unwrap());
        assert!(!same_existing_path(
            &guard,
            &path.with_file_name("shotkit-save-safety-different.jpg")
        )
        .unwrap());
        drop(guard);
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(protect_original(&path).is_err());
    }

    #[test]
    fn protected_original_handle_blocks_replacement_until_released() {
        let dir = std::env::temp_dir().join(format!(
            "shotkit-protected-original-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let original = dir.join("original.jpg");
        let independent = dir.join("other.jpg");
        std::fs::write(&original, b"original").unwrap();
        std::fs::write(&independent, b"existing").unwrap();
        let guard = protect_original(&original).unwrap();
        assert!(write_atomic(&original, b"replacement", true).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        write_atomic(&independent, b"replacement", true).unwrap();
        assert_eq!(std::fs::read(&independent).unwrap(), b"replacement");
        drop(guard);
        write_atomic(&original, b"after release", true).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"after release");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn protected_original_guard_refuses_an_existing_writer() {
        use std::os::windows::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!(
            "shotkit-protected-writer-{}-{}.jpg",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"original").unwrap();
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(7) // read, write and delete sharing for the existing writer
            .open(&path)
            .unwrap();
        assert!(protect_original(&path).is_err());
        drop(writer);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn protected_original_survives_target_junction_switch_after_identity_check() {
        fn junction(link: &Path, target: &Path) {
            let status = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:SHOTKIT_TEST_LINK -Target $env:SHOTKIT_TEST_TARGET | Out-Null",
                ])
                .env("SHOTKIT_TEST_LINK", link)
                .env("SHOTKIT_TEST_TARGET", target)
                .status()
                .unwrap();
            assert!(status.success(), "junction creation failed: {status}");
        }
        let root = std::env::temp_dir().join(format!(
            "shotkit-junction-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let original_dir = root.join("original");
        let safe_dir = root.join("safe");
        let link = root.join("link");
        std::fs::create_dir_all(&original_dir).unwrap();
        std::fs::create_dir_all(&safe_dir).unwrap();
        let original = original_dir.join("image.jpg");
        let safe = safe_dir.join("image.jpg");
        let selected = link.join("image.jpg");
        std::fs::write(&original, b"protected original").unwrap();
        std::fs::write(&safe, b"other target").unwrap();
        junction(&link, &safe_dir);
        let guard = protect_original(&original).unwrap();
        assert!(!same_existing_path(&guard, &selected).unwrap());
        std::fs::remove_dir(&link).unwrap();
        junction(&link, &original_dir);
        assert!(same_existing_path(&guard, &selected).unwrap());
        assert!(write_atomic(&selected, b"new image", true).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"protected original");
        assert_eq!(std::fs::read(&safe).unwrap(), b"other target");
        drop(guard);
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(root).unwrap();
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
