//! Startup-only modal prompt. No borrowed application state crosses the dialog.
use super::*;
use shotkit::temp_recovery_windows::{self as recovery, Choice};
use std::time::{SystemTime, UNIX_EPOCH};

#[link(name = "user32")]
extern "system" {
    fn DialogBoxIndirectParamW(
        instance: H,
        template: *const u16,
        parent: H,
        callback: Option<unsafe extern "system" fn(H, u32, usize, isize) -> isize>,
        parameter: isize,
    ) -> isize;
    fn EndDialog(dialog: H, result: isize) -> i32;
    fn SetDlgItemTextW(dialog: H, id: i32, text: *const u16) -> i32;
    fn GetDlgItem(dialog: H, id: i32) -> H;
}

pub(super) fn check(h: H) {
    // SAFETY: startup supplies the live initialized main window. This guard
    // blocks owner close, capture/menu operations and WM_COPYDATA reentrancy.
    let _modal = unsafe { ModalGuard::new(h) };
    let mut directories = Vec::new();
    if let Ok(path) = settings::settings_path() {
        if let Some(parent) = path.parent() {
            directories.push((parent.to_path_buf(), true));
        }
    }
    if let Some(folder) = active_settings().save_folder {
        if !directories.iter().any(|(path, _)| path == &folder) {
            directories.push((folder, false));
        }
    }
    // Each directory is presented separately, making the destination of
    // "Ordner öffnen" and the scope of confirmation unambiguous.
    for (directory, settings) in directories {
        if drive_location(&directory) != shotkit::network_path::DriveLocation::KnownDrive {
            let text = wide(&format!("Der Ordner liegt im Netzwerk oder sein Laufwerkstyp ist unbekannt:\n{}\n\nDie Prüfung kann den Server kontaktieren, Windows-Anmeldedaten übertragen und bei Nichterreichbarkeit dauern. Ordner jetzt auf temporäre Dateien prüfen?", directory.display()));
            // SAFETY: owner protected by ModalGuard; owned UTF-16 buffers.
            if unsafe {
                MessageBoxW(
                    h,
                    text.as_ptr(),
                    wide("Netzwerkordner prüfen").as_ptr(),
                    0x134,
                )
            } != 6
            {
                continue;
            }
        }
        let Ok(files) = recovery::scan(&directory, settings, SystemTime::now()) else {
            continue;
        };
        if files.is_empty() {
            continue;
        }
        let mut content = format!("{} Dateien in {}\n\n", files.len(), directory.display());
        for file in files.iter().take(10) {
            content.push_str(&format!(
                "{} — {} Bytes — {} UTC\n",
                file.path.file_name().unwrap_or_default().to_string_lossy(),
                file.size,
                date(file.modified)
            ));
        }
        if files.len() > 10 {
            content.push_str(&format!("… und {} weitere Dateien\n", files.len() - 10));
        }
        content.push_str("\nDiese Dateien können Bilddaten enthalten. Ein passender Name beweist die Herkunft nicht. „Löschen“ löscht alle oben gezählten Dateien endgültig. „Später“ fragt beim nächsten Start erneut.");
        let choice = prompt(h, &content);
        let errors = recovery::complete(files, choice);
        if choice == Choice::OpenFolder {
            // SAFETY: live startup owner; all UTF-16 temporaries survive the call.
            let result = unsafe {
                ShellExecuteW(
                    h,
                    wide("open").as_ptr(),
                    wide(&directory.to_string_lossy()).as_ptr(),
                    null_mut(),
                    null_mut(),
                    1,
                )
            };
            if result as usize <= 32 {
                report(h, "Der Ordner konnte nicht geöffnet werden.");
            }
        }
        if !errors.is_empty() {
            report(
                h,
                &format!(
                    "Einige Dateien konnten nicht gelöscht werden:\n{}",
                    errors.join("\n")
                ),
            );
        }
    }
}

fn report(h: H, text: &str) {
    // SAFETY: startup owner and live terminated UTF-16 buffers; no borrowed state.
    unsafe {
        MessageBoxW(
            h,
            wide(text).as_ptr(),
            wide("Temporäre Dateien").as_ptr(),
            0x10,
        );
    }
}

// A standard Win32 dialog template uses only built-in EDIT/BUTTON classes;
// it requires no Common Controls v6 activation or process-wide visual changes.
fn template() -> Vec<u32> {
    fn dword(words: &mut Vec<u16>, value: u32) {
        words.extend([value as u16, (value >> 16) as u16]);
    }
    fn text(words: &mut Vec<u16>, value: &str) {
        words.extend(wide(value));
    }
    let mut words = Vec::new();
    dword(&mut words, 0x80C8_00C0); // popup, caption, system menu, modal font dialog
    dword(&mut words, 0);
    words.extend([4, 0, 0, 360, 245]); // controls, x/y/cx/cy in dialog units
    words.extend([0, 0]); // no menu; standard dialog class
    text(&mut words, "Temporäre Dateien nach einem Abbruch");
    words.push(9);
    text(&mut words, "Segoe UI");
    let controls = [
        (200, 0x50A1_0844, [12, 12, 336, 190], 0x81, ""), // scrolling, read-only multiline edit
        (100, 0x5001_0000, [60, 215, 72, 18], 0x80, "Löschen"),
        (101, 0x5001_0000, [140, 215, 100, 18], 0x80, "Ordner öffnen"),
        (102, 0x5001_0001, [248, 215, 100, 18], 0x80, "Später"),
    ];
    for (id, style, bounds, class, label) in controls {
        if words.len() % 2 != 0 {
            words.push(0);
        } // DWORD-aligned item header
        dword(&mut words, style);
        dword(&mut words, 0);
        words.extend(bounds);
        words.extend([id, 0xFFFF, class]);
        text(&mut words, label);
        words.push(0); // no creation data
    }
    if words.len() % 2 != 0 {
        words.push(0);
    }
    words
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u32::from(w[0]) | (u32::from(w[1]) << 16))
        .collect()
}

// SAFETY: registered only for our synchronous DialogBoxIndirectParamW call.
// WM_INITDIALOG receives its live UTF-16 content pointer. No pointer is retained
// in window data; all subsequent messages use only the owned dialog HWND.
unsafe extern "system" fn dialog_proc(dialog: H, message: u32, w: usize, l: isize) -> isize {
    match message {
        0x0110 => {
            SetDlgItemTextW(dialog, 200, l as *const u16);
            SetFocus(GetDlgItem(dialog, 102));
            0 // focus already assigned to Später
        }
        0x0111 => {
            match w & 0xFFFF {
                id @ 100..=102 => {
                    EndDialog(dialog, id as isize);
                }
                2 => {
                    EndDialog(dialog, 102);
                } // Escape/IDCANCEL
                _ => {
                    return 0;
                }
            }
            1
        }
        0x0010 => {
            EndDialog(dialog, 102);
            1
        }
        _ => 0,
    }
}

fn prompt(h: H, content: &str) -> Choice {
    let template = template();
    let content = wide(content);
    // SAFETY: Vec<u32> supplies DWORD alignment for the validated SDK dialog
    // layout. Both buffers live through the synchronous modal call; its callback
    // only reads content during initialization and retains no Rust references.
    let result = unsafe {
        DialogBoxIndirectParamW(
            GetModuleHandleW(null_mut()),
            template.as_ptr().cast(),
            h,
            Some(dialog_proc),
            content.as_ptr() as isize,
        )
    };
    match result {
        100 => Choice::Delete,
        101 => Choice::OpenFolder,
        -1 => {
            report(
                h,
                "Der Hinweisdialog konnte nicht geöffnet werden. Es wurde nichts gelöscht.",
            );
            Choice::Later
        }
        _ => Choice::Later,
    }
}

// UTC civil date without another dependency. Algorithm: Gregorian era decomposition.
fn date(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}
