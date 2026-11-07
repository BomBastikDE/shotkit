//! Windows file and clipboard imports. This shared budget accounts for decoder-known
//! import buffers and retained imported, clipboard, ordinary screenshot, crop
//! frame/shadow and canvas-resize pixels, not GDI, other edits, allocator overhead or process
//! RSS. Nothing here replaces an existing editor.
use super::*;
use shotkit::{
    image_handle::ImageHandle,
    import_admission::{ImportAdmission, PendingState},
    rejected_import::{RejectedImportSlot, RejectedImportState},
    resource_budget::ResourceBudget,
};
use std::sync::{Arc, OnceLock};

static IMPORT_ADMISSION: OnceLock<ImportAdmission> = OnceLock::new();
static REJECTED_IMPORT: RejectedImportSlot = RejectedImportSlot::new();

fn imports() -> &'static ImportAdmission {
    IMPORT_ADMISSION.get_or_init(|| {
        // 896 MiB normal admission plus 128 MiB emergency headroom. The
        // constructor parameters are constants, independent of file contents.
        ImportAdmission::new(
            ResourceBudget::new(1024 * 1024 * 1024, 128 * 1024 * 1024)
                .expect("constant import budget is valid"),
        )
    })
}

/// Imports, screenshots, crop, frame/shadow and canvas output share one pool.
pub(super) fn shared_image_budget() -> ResourceBudget {
    imports().budget().clone()
}

/// In-flight transfers must block Quit before it closes any windows. Retained
/// recoverable images are instead resolved by resolve_import_recovery.
pub(super) fn import_transfer_active() -> bool {
    IMPORT_ADMISSION.get().is_some_and(|admission| {
        matches!(
            admission.pending_state(),
            PendingState::Decoding | PendingState::Initializing
        )
    }) || matches!(
        REJECTED_IMPORT.state(),
        RejectedImportState::Reserved | RejectedImportState::Recovering
    )
}

pub(super) fn import_recovery_pending() -> bool {
    IMPORT_ADMISSION
        .get()
        .is_some_and(|admission| admission.pending_state() == PendingState::Recoverable)
        || REJECTED_IMPORT.state() == RejectedImportState::Recoverable
}

struct ImportBitmap(H);
impl ImportBitmap {
    fn take(mut self) -> H {
        let bitmap = self.0;
        self.0 = null_mut();
        bitmap
    }
}
impl Drop for ImportBitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // This bitmap has never been selected in a DC or installed in an
            // HWND. Once installed, WindowState owns its normal destruction.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

/// SAFETY: owner is a live app HWND on the UI thread. The constructor must
/// preserve cleanup ownership on its own toolbar/DestroyWindow failure path.
/// Decode and controller/bitmap construction finish before window creation;
/// a registration failure hands the protected HWND to the cleanup slot.
/// No WindowState, vector or mutex borrow crosses a Windows callback. Both
/// admission and initialization guards cover registration and the full handoff.
pub(super) unsafe fn open_budgeted_image(owner: H, path: PathBuf) -> Result<(), String> {
    let _modal = ModalGuard::new(owner);
    if QUITTING.load(Ordering::SeqCst) {
        return Err("ShotKit wird gerade beendet.".into());
    }
    // Preserve Windows paths containing non-Unicode UTF-16; only an embedded
    // NUL cannot be passed safely to native file/dialog APIs.
    if path.as_os_str().encode_wide().any(|unit| unit == 0) {
        return Err("Der Dateipfad enthält ungültige Zeichen.".into());
    }
    let admission = imports();
    let decode = admission.try_reserve_decode().map_err(|_| {
        "Ein Dateiimport ist noch aktiv oder wartet auf Wiederherstellung.".to_owned()
    })?;
    // Reserve BOTH exclusive destinations before any decode allocation. A
    // validation rejection can now be retained without allocation or Busy.
    let rejected = REJECTED_IMPORT
        .try_reserve()
        .map_err(|_| "Ein abgelehnter Dateiimport wartet noch auf Wiederherstellung.".to_owned())?;
    let (image, format, lease) = image_io::decode_file_with_budget(&path, admission.budget())?;
    let initialization = match decode.try_begin(image, lease) {
        Ok(initialization) => initialization,
        Err(value) => {
            rejected.store(value);
            return Err("Das Bild konnte nicht übernommen werden. Bild und Speicherreservierung bleiben zur Wiederherstellung erhalten.".into());
        }
    };
    drop(rejected);
    let (_window, protected) = display_shared_image(
        ImageHandle::from_imported(Arc::clone(initialization.image())),
        Some((path, format)),
    )?;
    drop(initialization.complete());
    drop(protected);
    Ok(())
}

/// SAFETY: the caller owns the source gate and owner is a live UI-thread HWND.
/// The modal owner and both recovery reservations cover delayed rendering and
/// the full protected editor handoff; no mutex crosses a Windows callback.
pub(super) unsafe fn paste_budgeted_image(owner: H) -> Result<bool, String> {
    let _modal = ModalGuard::new(owner);
    if QUITTING.load(Ordering::SeqCst) {
        return Err("ShotKit wird gerade beendet.".into());
    }
    let Some(initialization) =
        shotkit::clipboard_admission::acquire(imports(), &REJECTED_IMPORT, |budget| {
            clipboard::read_image_with_budget(owner, budget)
        })?
    else {
        return Ok(false);
    };
    let (_window, protected) = display_shared_image(
        ImageHandle::from_imported(Arc::clone(initialization.image())),
        None,
    )?;
    drop(initialization.complete());
    drop(protected);
    Ok(true)
}

/// Create a protected editor without taking the caller's recovery owner.
/// That owner must remain live until this full handoff returns successfully.
/// No mutex or WindowState borrow crosses Windows callbacks.
pub(super) unsafe fn display_shared_image(
    image: ImageHandle,
    original: Option<(PathBuf, Option<image::ImageFormat>)>,
) -> Result<(H, EditorInitializationGuard), String> {
    if QUITTING.load(Ordering::SeqCst) {
        return Err("ShotKit wird gerade beendet. Das Bild bleibt im Speicher.".into());
    }
    let bitmap = ImportBitmap(create_bitmap(&image)?);
    let controller = EditorController::with_image_handle(image)
        .map_err(|(error, _handle)| format!("Bild konnte nicht geladen werden: {error}"))?;
    let (window, protected) = create_editor_window_protected()
        .map_err(|error| format!("{error} Das Bild bleibt im Speicher."))?;
    // Creation can reenter another window and consume any earlier reservation.
    // Reserve immediately before push, with no callback between the two.
    if (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS))
        .try_reserve(1)
        .is_err()
    {
        let cleanup = abandon_protected_editor(protected);
        return Err(match cleanup {
            Ok(()) => {
                "Die Fensterverwaltung hat keinen freien Speicher. Das Bild bleibt im Speicher."
                    .to_owned()
            }
            Err(error) => format!("{error} Das Bild bleibt im Speicher."),
        });
    }
    (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(window);
    // Fresh window: no old document, bitmap or gesture can be displaced.
    let image = controller.image_arc();
    install_bitmap(window, bitmap.take(), &image);
    window_state(window).editor = Some(controller);
    if let Some((path, format)) = original {
        window_state(window).protected_original = opened_original_requires_save_as(&path, format);
        window_state(window).current_path = Some(path);
        editor(window).document_mut().mark_saved();
    }
    refresh_title(window);
    if !protected.still_owns(window) {
        return Err("Editorfenster wurde geschlossen. Das Bild bleibt im Speicher.".into());
    }
    ShowWindow(window, 5);
    if !protected.still_owns(window) {
        return Err("Editorfenster wurde geschlossen. Das Bild bleibt im Speicher.".into());
    }
    SetForegroundWindow(window);
    if !protected.still_owns(window) {
        return Err("Editorfenster wurde geschlossen. Das Bild bleibt im Speicher.".into());
    }
    InvalidateRect(window, std::ptr::null(), 1);
    if !protected.still_owns(window) {
        return Err("Editorfenster wurde geschlossen. Das Bild bleibt im Speicher.".into());
    }
    // No lease is detached: editor/history retain their ImageHandle. The slot
    // only clears after every reentrant handoff call has returned.
    Ok((window, protected))
}

#[derive(Clone, Copy)]
pub(super) enum RecoveryDecision {
    Saved,
    Discard,
    Keep,
}

/// Borrowed image is owned by an exclusive recovery guard, never by a mutex or
/// WindowState borrow. New-file atomic publication is unconditional, regardless
/// of an overwrite confirmation returned by the general save dialog.
pub(super) unsafe fn recovery_decision(owner: H, image: &Image, capture: bool) -> RecoveryDecision {
    let message = if capture {
        "Eine Aufnahme ist noch im Speicher.\n\nJa = als neue Datei speichern\nNein = Bild verwerfen\nAbbrechen = im Speicher behalten"
    } else {
        "Ein importiertes Bild ist noch im Speicher.\n\nJa = als neue Datei speichern\nNein = Bild verwerfen\nAbbrechen = im Speicher behalten"
    };
    let title = if capture {
        "Ungesicherte Aufnahme"
    } else {
        "Ungesicherter Bildimport"
    };
    match MessageBoxW(owner, wide(message).as_ptr(), wide(title).as_ptr(), 0x23) {
        7 => RecoveryDecision::Discard,
        6 => {
            let path = match file_dialog::save(owner, None) {
                Ok(Some((path, _replace))) => path,
                Ok(None) => return RecoveryDecision::Keep,
                Err(error) => {
                    MessageBoxW(
                        owner,
                        wide(&error).as_ptr(),
                        wide("Speicherdialog").as_ptr(),
                        0x10,
                    );
                    return RecoveryDecision::Keep;
                }
            };
            let result = image_io::format_from_path(&path)
                .and_then(|format| write_image_atomic(&path, image, format, false));
            match result {
                Ok(()) => RecoveryDecision::Saved,
                Err(error) => {
                    MessageBoxW(owner, wide(&format!("Speichern fehlgeschlagen:\n{error}\nBitte einen neuen Dateinamen wählen. Das Bild bleibt im Speicher.")).as_ptr(),
                        wide("Bild wiederherstellen").as_ptr(), 0x10);
                    RecoveryDecision::Keep
                }
            }
        }
        _ => RecoveryDecision::Keep,
    }
}

/// SAFETY: owner remains live on the UI thread. ModalGuard protects its life;
/// exclusive recovery guards block nested imports/Quit. All dialogs, encoding
/// and atomic file I/O run outside every mutex. Failed saves/cancel/unwind keep
/// the exact image and lease. Resolution drops pixels before their lease.
pub(super) unsafe fn resolve_import_recovery(owner: H) -> bool {
    if import_transfer_active() {
        return false;
    }
    let _modal = ModalGuard::new(owner);
    if let Some(admission) = IMPORT_ADMISSION.get() {
        if admission.pending_state() == PendingState::Recoverable {
            let Ok(guard) = admission.try_recover() else {
                return false;
            };
            match recovery_decision(owner, guard.image().image(), false) {
                RecoveryDecision::Keep => return false,
                RecoveryDecision::Saved | RecoveryDecision::Discard => drop(guard.complete()),
            }
        }
    }
    match REJECTED_IMPORT.state() {
        RejectedImportState::Empty => true,
        RejectedImportState::Recoverable => {
            let Ok(guard) = REJECTED_IMPORT.try_recover() else {
                return false;
            };
            match recovery_decision(owner, guard.image(), false) {
                RecoveryDecision::Saved => {
                    guard.commit_saved();
                    true
                }
                RecoveryDecision::Discard => {
                    guard.discard();
                    true
                }
                RecoveryDecision::Keep => false,
            }
        }
        RejectedImportState::Reserved | RejectedImportState::Recovering => false,
    }
}
