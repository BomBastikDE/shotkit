//! Ordinary screenshot ownership. Native GDI resources, clipboard copies,
//! scroll captures and non-crop edit allocations are not charged by this integration.
use super::*;
use shotkit::{
    capture_admission,
    import_admission::{ImportAdmission, PendingState},
    rejected_import::{RejectedImportSlot, RejectedImportState},
};
use std::sync::{Arc, OnceLock};

static CAPTURE_ADMISSION: OnceLock<ImportAdmission> = OnceLock::new();
static REJECTED_CAPTURE: RejectedImportSlot = RejectedImportSlot::new();

fn captures() -> &'static ImportAdmission {
    CAPTURE_ADMISSION.get_or_init(|| ImportAdmission::new(shared_image_budget()))
}

pub(super) fn capture_recovery_pending() -> bool {
    CAPTURE_ADMISSION
        .get()
        .is_some_and(|owner| owner.pending_state() != PendingState::Empty)
        || REJECTED_CAPTURE.state() != RejectedImportState::Empty
}

/// SAFETY: the caller owns the source gate and a live modal-protected HWND.
/// Admission, native acquisition and all output callbacks hold no mutex. A
/// successful fill is protected by initialization until the entire output plan
/// finishes; every early return leaves that same image/lease recoverable.
pub(super) unsafe fn capture_ordinary_rect(
    h: H,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    was_visible: bool,
) -> Result<(), String> {
    let width_bytes = usize::try_from(width).map_err(|_| "Ungültige Aufnahmebreite.")?;
    let height_bytes = usize::try_from(height).map_err(|_| "Ungültige Aufnahmehöhe.")?;
    let initialization = capture_admission::acquire(
        captures(),
        &REJECTED_CAPTURE,
        width_bytes,
        height_bytes,
        |pixels| scroll_windows::WindowsScrollIo::new(x, y, width, height, 0)?.capture_into(pixels),
    )?;
    let prefs = active_settings();
    let mut output = CaptureOutputState::new(&prefs);
    let mut saved = None;
    let mut opened = None;
    while let Some(action) = output.next_action() {
        let result = match action {
            CaptureAction::OpenEditor => display_shared_image(
                shotkit::image_handle::ImageHandle::from_imported(Arc::clone(
                    initialization.image(),
                )),
                saved
                    .take()
                    .map(|path| (path, Some(image::ImageFormat::Png))),
            )
            .map(|window| {
                opened = Some(window);
            }),
            _ => output_windows::execute_action(action, &prefs, initialization.image().image(), h)
                .map(|result| {
                    if result.saved_path.is_some() {
                        saved = result.saved_path;
                    }
                }),
        };
        let advance = output
            .finish_action(action, result)
            .map_err(|error| error.to_string())?;
        if advance == Advance::RetainImage {
            return Err("Die Aufnahme konnte nicht im Editor geöffnet werden und bleibt im Speicher. Über das Tray-Menü kann sie gesichert werden.".into());
        }
    }
    // Retain across all Windows callbacks, including output notifications.
    if was_visible {
        ShowWindow(h, 5);
    }
    if opened.is_none() {
        let message = match &saved {
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
            .map(|failure| format!("{:?}: {}", failure.action, failure.message))
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
    if opened
        .as_ref()
        .is_some_and(|(window, protection)| !protection.still_owns(*window))
    {
        return Err("Das Editorfenster wurde während der Ausgabe geschlossen. Die Aufnahme bleibt im Speicher.".into());
    }
    drop(initialization.complete());
    drop(opened);
    Ok(())
}

/// Resolve outside all mutexes, preserving exact pixels/accounting on cancel,
/// save failure or unwind. Only explicit discard/successful save releases them.
pub(super) unsafe fn resolve_capture_recovery(owner: H) -> bool {
    if let Some(admission) = CAPTURE_ADMISSION.get() {
        match admission.pending_state() {
            PendingState::Empty => {}
            PendingState::Recoverable => {
                let Ok(guard) = admission.try_recover() else {
                    return false;
                };
                match recovery_decision(owner, guard.image().image(), true) {
                    RecoveryDecision::Keep => return false,
                    RecoveryDecision::Saved | RecoveryDecision::Discard => drop(guard.complete()),
                }
            }
            _ => return false,
        }
    }
    match REJECTED_CAPTURE.state() {
        RejectedImportState::Empty => true,
        RejectedImportState::Recoverable => {
            let Ok(guard) = REJECTED_CAPTURE.try_recover() else {
                return false;
            };
            match recovery_decision(owner, guard.image(), true) {
                RecoveryDecision::Keep => false,
                RecoveryDecision::Saved => {
                    guard.commit_saved();
                    true
                }
                RecoveryDecision::Discard => {
                    guard.discard();
                    true
                }
            }
        }
        _ => false,
    }
}
