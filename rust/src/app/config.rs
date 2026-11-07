use super::*;

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Editorfenster auf dem UI-Thread.
// Warum erfüllt: Aufruf aus der bewachten Ereignisbehandlung; der Plan ist ein
// eigener Wert und alle Editor-Borrows enden vor dem Dialog.
// Nachrichten: MessageBoxW kann reentrieren; ModalGuard blockiert Änderungen
// und Schließen dieses Editors, der Plan prüft zusätzlich den Dokumentzustand.
pub(super) unsafe fn confirm_and_commit_staged_edit(
    h: H,
) -> Result<Option<bool>, shotkit::document::DocumentError> {
    let plan = editor(h).document_mut().plan_commit_edit_with_fallback()?;
    let Some(mut plan) = plan else {
        return editor(h).document_mut().commit_edit().map(Some);
    };
    let evictions = plan.undo_entries_to_evict();
    let redo_discarded = plan.redo_entries_to_discard();
    let mode = plan.mode();
    if mode == shotkit::document::HistoryCommitMode::WithoutUndo
        || evictions > 0
        || redo_discarded > 0
    {
        // Restore the old document while the modal loop may repaint; the
        // prepared new state stays owned by the plan until approval.
        plan = editor(h).document_mut().suspend_edit_with_plan(plan)?;
    }
    let Some(approval) = prompt_history_approval(h, mode, evictions, redo_discarded) else {
        return Ok(None);
    };
    editor(h)
        .document_mut()
        .commit_edit_with_approval(plan, approval)
        .map(Some)
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Editorfenster auf dem UI-Thread.
// Warum erfüllt: Aufrufer sind die bewachten Ereignishandler; der vorbereitete
// Plan besitzt alle Daten und der Editor-Borrow endet vor dem Dialog.
// Nachrichten: MessageBoxW kann reentrieren; ModalGuard schützt den Editor,
// und die Übernahme prüft den zwischenzeitlichen Dokument- und Editorzustand.
pub(super) unsafe fn apply_annotation_edit(
    h: H,
    action: impl FnOnce(&mut EditorController),
) -> bool {
    let prepared = match editor(h).prepare_annotation_edit(action) {
        Ok(value) => value,
        Err(error) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Die Bearbeitung konnte nicht übernommen werden: {error}"
                ))
                .as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            return false;
        }
    };
    let Some(plan) = prepared else {
        return true;
    };
    let undo = plan.undo_entries_to_evict();
    let redo = plan.redo_entries_to_discard();
    let Some(approval) = prompt_history_approval(h, plan.mode(), undo, redo) else {
        return false;
    };
    match editor(h).commit_annotation_edit_with_approval(plan, approval) {
        Ok(_) => true,
        Err(error) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Die Bearbeitung konnte nicht übernommen werden: {error}"
                ))
                .as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            false
        }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Editorfenster und es wird kein
// Editor-Borrow gehalten. Warum erfüllt: beide Aufrufer besitzen nur Plandaten.
// Nachrichten: MessageBoxW kann reentrieren; ModalGuard schützt das Fenster.
unsafe fn prompt_history_approval(
    h: H,
    mode: shotkit::document::HistoryCommitMode,
    undo: usize,
    redo: usize,
) -> Option<shotkit::document::CommitApproval> {
    use shotkit::document::{CommitApproval, HistoryCommitMode};
    if mode == HistoryCommitMode::Undoable && undo == 0 && redo == 0 {
        return Some(CommitApproval::PreserveHistory);
    }
    let (message, approval) = if mode == HistoryCommitMode::WithoutUndo {
        (format!("Das Speicherlimit reicht für diese Bearbeitung nicht aus. Der gesamte Rückgängig- und Wiederholen-Verlauf ({undo} und {redo} Schritte) wird gelöscht. Diese Bearbeitung kann nicht rückgängig gemacht werden. Möchten Sie fortfahren?"), CommitApproval::CommitWithoutUndo)
    } else {
        (format!("Für diese Bearbeitung werden {undo} ältere Rückgängig-Schritte und {redo} Wiederholen-Schritte entfernt. Möchten Sie fortfahren?"), CommitApproval::DiscardOldHistory)
    };
    let _modal = ModalGuard::new(h);
    (MessageBoxW(
        h,
        wide(&message).as_ptr(),
        wide("Rückgängig-Verlauf").as_ptr(),
        0x124,
    ) == 6)
        .then_some(approval)
}

pub(super) fn active_settings() -> Settings {
    SETTINGS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_default()
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Besitzer für mögliche Fehlerdialoge.
// Warum erfüllt: Aufruf nach erfolgreicher Erstellung des Hauptfensters.
// Nachrichten: MessageBoxW kann reentrieren; der SETTINGS-Guard der Zuweisung endet vor dem Fehlerdialog.
pub(super) unsafe fn load_settings(h: H) {
    // A modal prompt can dispatch menu messages. Keep writes disabled until
    // loading, explicit recovery and completion verification have all finished.
    SETTINGS_WRITABLE.store(false, Ordering::SeqCst);
    *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(Settings::default());
    let result = settings::settings_path().and_then(|path| {
        settings::reset_windows::ensure_clear(&path).map_err(|source| settings::SettingsError::Io {
            operation: "inspect recovery artifacts",
            path: path.clone(),
            source,
        })?;
        match settings::load_outcome(&path)? {
            settings::SettingsLoadOutcome::Missing => Ok(Settings::default()),
            settings::SettingsLoadOutcome::Loaded { settings, .. } => Ok(settings),
            settings::SettingsLoadOutcome::Rejected(rejected) => {
                if rejected.recovery_eligibility != settings::RecoveryEligibility::UnambiguousVersion1 {
                    return Err(rejected.error);
                }
                let answer = MessageBoxW(
                    h,
                    wide("Die vorhandenen Einstellungen sind ungültig, enthalten aber eindeutig Version 1. Möchten Sie sie ausdrücklich auf Standardwerte zurücksetzen?\n\nDas unveränderte Original und ein kleiner Abschlussbeleg bleiben im Einstellungsordner erhalten. Jeder bestätigte Reset kann ein weiteres Backup erzeugen; es gibt keine automatische Bereinigung.\n\nBei Nein bleiben die Datei unverändert und das Speichern deaktiviert. Nach einer unterbrochenen Wiederherstellung ist eine manuelle Prüfung nötig.").as_ptr(),
                    wide("Einstellungen zurücksetzen?").as_ptr(),
                    settings::RESET_CONFIRM_FLAGS,
                );
                if !settings::reset_approved(rejected.recovery_eligibility, answer) {
                    crate::uia_picker::log_diagnostic("settings_reset_stage=declined");
                    return Err(rejected.error);
                }
                crate::uia_picker::log_diagnostic("settings_reset_stage=approved");
                let backup = settings::reset_windows::reset_rejected(&path, &rejected)
                    .map_err(|source| settings::SettingsError::Io {
                        operation: "explicit reset",
                        path: path.clone(),
                        source,
                    })?;
                crate::uia_picker::log_diagnostic("settings_reset_stage=success");
                MessageBoxW(
                    h,
                    wide(&format!("Die Einstellungen wurden auf Standardwerte zurückgesetzt. Das unveränderte Original bleibt erhalten:\n{}\n\nDer zugehörige .complete-Beleg gehört zum Backup. Beide Dateien werden zur Prüfung beim nächsten Start benötigt. Aufbewahrung und spätere Entfernung müssen manuell gemeinsam geprüft werden.", backup.display())).as_ptr(),
                    wide("Einstellungen zurückgesetzt").as_ptr(),
                    0x40,
                );
                // Recheck after the modal dialog before enabling writes; no
                // SETTINGS guard or file handle spans the user-facing dialog.
                settings::reset_windows::ensure_clear(&path).map_err(|source| settings::SettingsError::Io {
                    operation: "inspect recovery artifacts",
                    path: path.clone(),
                    source,
                })?;
                settings::load(&path)
            }
        }
    });
    match result {
        Ok(value) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
            SETTINGS_WRITABLE.store(true, Ordering::SeqCst);
        }
        Err(e) => {
            crate::uia_picker::log_diagnostic(&format!(
                "settings_load_stage=failed {}",
                e.diagnostic_summary()
            ));
            MessageBoxW(h,wide(&format!("Einstellungen konnten nicht geladen werden: {e}\nDie Standardwerte werden vorübergehend verwendet. Speichern der Einstellungen ist deaktiviert. Vorhandene Dateien bleiben erhalten; bei einer unterbrochenen Wiederherstellung müssen Einstellungsdatei, .reset-pending-Datei sowie Recovery-Backups, Belege und temporäre Dateien vor einem weiteren Versuch manuell geprüft werden.")).as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes eigenes Dialog-Besitzerfenster.
// Warum erfüllt: Menü- und Editoraktionen übergeben ihr eigenes Fenster.
// Nachrichten: MessageBoxW kann reentrieren; bei Fehlerdialogen wird kein SETTINGS-Guard gehalten.
pub(super) unsafe fn save_settings(h: H, new: Settings) -> bool {
    crate::uia_picker::log_diagnostic("settings_save_stage=attempt");
    if !SETTINGS_WRITABLE.load(Ordering::SeqCst) {
        crate::uia_picker::log_diagnostic(
            "settings_save_stage=blocked reason=settings_not_writable",
        );
        MessageBoxW(h,wide("Einstellungen können nicht gespeichert werden, solange die Einstellungsdatei oder eine Wiederherstellung ungeklärt ist.").as_ptr(),wide("ShotKit Rust").as_ptr(),0x30);
        return false;
    }
    let result = settings::settings_path().and_then(|path| {
        settings::reset_windows::ensure_clear(&path).map_err(|source| {
            SETTINGS_WRITABLE.store(false, Ordering::SeqCst);
            settings::SettingsError::Io {
                operation: "inspect recovery artifacts",
                path: path.clone(),
                source,
            }
        })?;
        // Ordinary saves retain their existing atomic-save implementation and
        // create no backup or receipt. Recovery is exclusively an approved UI action.
        settings::save_atomic(path, &new)
    });
    match result {
        Ok(()) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Some(new);
            crate::uia_picker::log_diagnostic("settings_save_stage=success");
            true
        }
        Err(e) => {
            crate::uia_picker::log_diagnostic(&format!(
                "settings_save_stage=failed {}",
                e.diagnostic_summary()
            ));
            MessageBoxW(
                h,
                wide(&format!(
                    "Einstellungen konnten nicht gespeichert werden: {e}"
                ))
                .as_ptr(),
                wide("ShotKit Rust").as_ptr(),
                0x10,
            );
            false
        }
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h hat einen initialisierten WindowState auf dem UI-Thread.
// Warum erfüllt: Aufruf aus der Ereignisbehandlung eines eigenen Editorfensters.
// Nachrichten: ReleaseCapture kann Nachrichten auslösen; der Editor-Borrow endet vor diesem Aufruf.
pub(super) unsafe fn cancel_gesture(h: H) {
    let had_capture = window_state(h).drawing;
    window_state(h).drawing = false;
    window_state(h).crop_start = None;
    window_state(h).crop_end = None;
    if let Some(state) = window_state(h).editor.as_mut() {
        state.cancel_drag();
    }
    if had_capture {
        ReleaseCapture();
    }
}
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiger Besitzer; UTF-16-Zeiger leben bis zum API-Rücksprung.
// Warum erfüllt: eigene Menüaktion; alle Puffer bleiben während der synchronen Aufrufe im Scope.
// Nachrichten: ShellExecuteW und MessageBoxW können reentrieren; Einstellungen, Ordner und UTF-16-Puffer sind eigene Werte.
pub(super) unsafe fn open_storage_folder(h: H) {
    let Some(folder) = active_settings().save_folder else {
        MessageBoxW(
            h,
            wide("Es ist kein Speicherordner eingestellt. Wählen Sie zuerst unter Einstellungen einen Speicherordner.").as_ptr(),
            wide("Speicherordner öffnen").as_ptr(),
            0x30,
        );
        return;
    };
    match std::fs::metadata(&folder) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Der eingestellte Speicherpfad ist kein Ordner:\n{}",
                    folder.display()
                ))
                .as_ptr(),
                wide("Speicherordner öffnen").as_ptr(),
                0x10,
            );
            return;
        }
        Err(error) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Der eingestellte Speicherordner ist nicht verfügbar:\n{}\n\n{error}",
                    folder.display()
                ))
                .as_ptr(),
                wide("Speicherordner öffnen").as_ptr(),
                0x10,
            );
            return;
        }
    }
    let path: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = ShellExecuteW(
        h,
        wide("open").as_ptr(),
        path.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        5,
    );
    if result as usize <= 32 {
        MessageBoxW(
            h,
            wide(&format!("Windows konnte den Speicherordner nicht öffnen:\n{}\n\nShellExecuteW meldete Fehlercode {}.", folder.display(), result as usize)).as_ptr(),
            wide("Speicherordner öffnen").as_ptr(),
            0x10,
        );
    }
}
