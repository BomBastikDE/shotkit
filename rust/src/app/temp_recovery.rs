//! Startup-only modal prompt. No borrowed application state crosses the dialog.
use super::*;
use shotkit::temp_recovery_windows::{self as recovery, Choice};
use std::time::{SystemTime, UNIX_EPOCH};

// TASKDIALOGCONFIG and TASKDIALOG_BUTTON use the Windows SDK's pack(1).
#[repr(C, packed)]
struct Button {
    id: i32,
    text: *const u16,
}
#[repr(C, packed)]
struct Config {
    size: u32,
    parent: H,
    instance: H,
    flags: u32,
    common: u32,
    title: *const u16,
    icon: H,
    instruction: *const u16,
    content: *const u16,
    count: u32,
    buttons: *const Button,
    default: i32,
    radio_count: u32,
    radios: *const Button,
    radio_default: i32,
    verification: *const u16,
    expanded: *const u16,
    expanded_control: *const u16,
    collapsed_control: *const u16,
    footer_icon: H,
    footer: *const u16,
    callback: H,
    callback_data: isize,
    width: u32,
}
#[link(name = "comctl32")]
extern "system" {
    fn TaskDialogIndirect(
        config: *const Config,
        button: *mut i32,
        radio: *mut i32,
        verification: *mut i32,
    ) -> i32;
}

pub(super) fn check(h: H) {
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

fn prompt(h: H, content: &str) -> Choice {
    let title = wide("Temporäre Dateien nach einem Abbruch");
    let instruction = wide("Zurückgebliebene Dateien prüfen");
    let content = wide(content);
    let labels = [wide("Löschen"), wide("Ordner öffnen"), wide("Später")];
    let buttons = [
        Button {
            id: 100,
            text: labels[0].as_ptr(),
        },
        Button {
            id: 101,
            text: labels[1].as_ptr(),
        },
        Button {
            id: 102,
            text: labels[2].as_ptr(),
        },
    ];
    let config = Config {
        size: size_of::<Config>() as u32,
        parent: h,
        instance: null_mut(),
        flags: 8,
        common: 0,
        title: title.as_ptr(),
        icon: null_mut(),
        instruction: instruction.as_ptr(),
        content: content.as_ptr(),
        count: 3,
        buttons: buttons.as_ptr(),
        default: 102,
        radio_count: 0,
        radios: null_mut(),
        radio_default: 0,
        verification: null_mut(),
        expanded: null_mut(),
        expanded_control: null_mut(),
        collapsed_control: null_mut(),
        footer_icon: null_mut(),
        footer: null_mut(),
        callback: null_mut(),
        callback_data: 0,
        width: 0,
    };
    let mut button = 0;
    // SAFETY: SDK pack(1) structures, valid startup owner and all pointees live
    // until the synchronous modal call returns. No callback or borrowed state.
    let result = unsafe { TaskDialogIndirect(&config, &mut button, null_mut(), null_mut()) };
    if result < 0 {
        report(
            h,
            "Der Hinweisdialog konnte nicht geöffnet werden. Es wurde nichts gelöscht.",
        );
        return Choice::Later;
    }
    match button {
        100 => Choice::Delete,
        101 => Choice::OpenFolder,
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
