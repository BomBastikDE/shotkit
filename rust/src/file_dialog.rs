//! Native Windows open/save dialogs for the supported image formats.
//!
//! The common-dialog API uses UTF-16 and a generously sized buffer so paths
//! near the Windows extended-path limit are not truncated. Cancellation is
//! returned as `Ok(None)` and common-dialog failures as `Err`.

use std::{
    ffi::c_void,
    path::{Path, PathBuf},
};

#[cfg(windows)]
mod win32 {
    use super::*;
    use std::{
        mem::size_of,
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr,
    };

    type Hwnd = *mut c_void;
    type Hinstance = *mut c_void;
    type Lparam = isize;
    type Hook = Option<unsafe extern "system" fn(Hwnd, u32, usize, Lparam) -> usize>;

    #[repr(C)]
    struct OpenFileNameW {
        size: u32,
        owner: Hwnd,
        instance: Hinstance,
        filter: *const u16,
        custom_filter: *mut u16,
        max_custom_filter: u32,
        filter_index: u32,
        file: *mut u16,
        max_file: u32,
        file_title: *mut u16,
        max_file_title: u32,
        initial_dir: *const u16,
        title: *const u16,
        flags: u32,
        file_offset: u16,
        file_extension: u16,
        default_extension: *const u16,
        custom_data: Lparam,
        hook: Hook,
        template_name: *const u16,
        reserved: *mut c_void,
        reserved_word: u32,
        flags_ex: u32,
    }

    #[repr(C)]
    struct NotifyHeader {
        from: Hwnd,
        id: usize,
        code: i32,
    }

    #[repr(C)]
    struct OfNotifyW {
        header: NotifyHeader,
        ofn: *mut OpenFileNameW,
        file: *const u16,
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetOpenFileNameW(ofn: *mut OpenFileNameW) -> i32;
        fn GetSaveFileNameW(ofn: *mut OpenFileNameW) -> i32;
        fn CommDlgExtendedError() -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(owner: Hwnd, text: *const u16, title: *const u16, flags: u32) -> i32;
    }

    const OFN_HIDEREADONLY: u32 = 0x0000_0004;
    const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
    const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
    const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
    const OFN_EXPLORER: u32 = 0x0008_0000;
    const OFN_ENABLEHOOK: u32 = 0x0000_0020;
    const WM_NOTIFY: u32 = 0x004e;
    const CDN_TYPECHANGE: i32 = -607;

    // OPENFILENAMEW is capped at 32767 UTF-16 code units by the Windows path
    // model. Leave one code unit for the terminator.
    const FILE_BUFFER_LEN: usize = 32_768;

    fn filter(save: bool) -> Vec<u16> {
        // The Win32 filter is a sequence of NUL-terminated display/pattern
        // pairs and ends with an additional NUL.
        let mut v = Vec::new();
        let entries: &[&str] = if save {
            &[
                "PNG images (*.png)",
                "*.png",
                "JPEG images (*.jpg;*.jpeg)",
                "*.jpg;*.jpeg",
                "Bitmap images (*.bmp)",
                "*.bmp",
                "GIF images (*.gif)",
                "*.gif",
            ]
        } else {
            &[
                "PNG images (*.png)",
                "*.png",
                "JPEG images (*.jpg;*.jpeg)",
                "*.jpg;*.jpeg",
                "Bitmap images (*.bmp)",
                "*.bmp",
                "GIF images (*.gif)",
                "*.gif",
                "TIFF images (*.tif;*.tiff)",
                "*.tif;*.tiff",
            ]
        };
        for &part in entries {
            v.extend(part.encode_utf16());
            v.push(0);
        }
        v.push(0);
        v
    }

    static EXT_PNG: [u16; 4] = [b'p' as u16, b'n' as u16, b'g' as u16, 0];
    static EXT_JPG: [u16; 4] = [b'j' as u16, b'p' as u16, b'g' as u16, 0];
    static EXT_BMP: [u16; 4] = [b'b' as u16, b'm' as u16, b'p' as u16, 0];
    static EXT_GIF: [u16; 4] = [b'g' as u16, b'i' as u16, b'f' as u16, 0];

    // SAFETY CONTRACT:
    // Voraussetzung: WM_NOTIFY enthält einen gültigen ausgerichteten OFNOTIFY-Zeiger mit beschreibbarem OFN.
    // Warum erfüllt bzw. offen: Windows ruft den registrierten Hook auf; Nullprüfungen allein beweisen Layout und Lebensdauer nicht.
    // Nachrichten: Der Hook wird synchron vom Dialog aufgerufen.
    unsafe extern "system" fn save_hook(
        _hwnd: Hwnd,
        message: u32,
        _wparam: usize,
        lparam: Lparam,
    ) -> usize {
        if message == WM_NOTIFY && lparam != 0 {
            let notification = &mut *(lparam as *mut OfNotifyW);
            if notification.header.code == CDN_TYPECHANGE && !notification.ofn.is_null() {
                let ofn = &mut *notification.ofn;
                ofn.default_extension = match ofn.filter_index {
                    2 => EXT_JPG.as_ptr(),
                    3 => EXT_BMP.as_ptr(),
                    4 => EXT_GIF.as_ptr(),
                    _ => EXT_PNG.as_ptr(),
                };
            }
        }
        0
    }

    pub(super) fn dialog(
        owner: Hwnd,
        initial_name: Option<&Path>,
        save: bool,
    ) -> Result<Option<(PathBuf, bool)>, String> {
        let mut file = vec![0u16; FILE_BUFFER_LEN];
        let mut initial_dir = Vec::new();
        if let Some(path) = initial_name {
            let name = path.file_name().unwrap_or(path.as_os_str());
            let encoded: Vec<u16> = name.encode_wide().collect();
            let n = encoded.len().min(FILE_BUFFER_LEN - 1);
            file[..n].copy_from_slice(&encoded[..n]);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                initial_dir = parent
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
            }
        }

        let filters = filter(save);
        let mut ofn = OpenFileNameW {
            size: size_of::<OpenFileNameW>() as u32,
            owner,
            instance: ptr::null_mut(),
            filter: filters.as_ptr(),
            custom_filter: ptr::null_mut(),
            max_custom_filter: 0,
            filter_index: 1,
            file: file.as_mut_ptr(),
            max_file: FILE_BUFFER_LEN as u32,
            file_title: ptr::null_mut(),
            max_file_title: 0,
            initial_dir: if initial_dir.is_empty() {
                ptr::null()
            } else {
                initial_dir.as_ptr()
            },
            title: ptr::null(),
            flags: OFN_EXPLORER
                | OFN_HIDEREADONLY
                | OFN_NOCHANGEDIR
                | OFN_PATHMUSTEXIST
                | if save {
                    OFN_ENABLEHOOK
                } else {
                    OFN_FILEMUSTEXIST
                },
            file_offset: 0,
            file_extension: 0,
            default_extension: EXT_PNG.as_ptr(),
            custom_data: 0,
            hook: if save { Some(save_hook) } else { None },
            template_name: ptr::null(),
            reserved: ptr::null_mut(),
            reserved_word: 0,
            flags_ex: 0,
        };
        let accepted = unsafe {
            if save {
                GetSaveFileNameW(&mut ofn)
            } else {
                GetOpenFileNameW(&mut ofn)
            }
        };
        if accepted == 0 {
            let error = unsafe { CommDlgExtendedError() };
            return if error == 0 {
                Ok(None)
            } else {
                Err(format!(
                    "Windows file dialog failed (CommDlgExtendedError=0x{error:08X})"
                ))
            };
        }
        let end = file.iter().position(|&u| u == 0).unwrap_or(file.len());
        let selected = PathBuf::from(std::ffi::OsString::from_wide(&file[..end]));
        if !save {
            return Ok(Some((selected, false)));
        }
        let target = shotkit::save_path::with_extension(&selected, ofn.filter_index);
        let confirmed_existing = match std::fs::symlink_metadata(&target) {
            Ok(_) => {
                let message: Vec<u16> = format!(
                    "Die Datei ist bereits vorhanden:\n\n{}\n\nMöchten Sie sie ersetzen?",
                    target.display()
                )
                .encode_utf16()
                .chain(Some(0))
                .collect();
                let title: Vec<u16> = "Datei ersetzen".encode_utf16().chain(Some(0)).collect();
                // SAFETY: owner is supplied by the caller and both UTF-16
                // strings remain alive during this synchronous dialog call.
                if unsafe { MessageBoxW(owner, message.as_ptr(), title.as_ptr(), 0x134) } != 6 {
                    return Ok(None);
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("Zieldatei konnte nicht geprüft werden: {error}")),
        };
        Ok(Some((target, confirmed_existing)))
    }
}

/// Shows the Windows Open dialog and returns the selected image path.
///
/// Returns `Ok(None)` when the dialog is cancelled and `Err` when Windows
/// reports a common-dialog error.
/// `owner` may be null for an unowned dialog. The caller must pass a valid HWND
/// or null and must satisfy the usual Win32 thread-affinity requirements.
#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread.
// Warum erfüllt bzw. offen: Puffer leben bis zur Rückkehr; Windows- und Nicht-Windows-Varianten haben getrennte Implementierungen.
// Nachrichten: Der Windows-Dateidialog verarbeitet Nachrichten; die andere Variante nicht.
pub unsafe fn open(owner: *mut c_void) -> Result<Option<PathBuf>, String> {
    win32::dialog(owner, None, false).map(|selected| selected.map(|(path, _)| path))
}

/// Shows the Windows Save dialog and returns the selected image path.
///
/// `initial_name` optionally supplies an initial filename and directory. The
/// extension follows the selected filter when the user omits one, and
/// existing files trigger an overwrite confirmation for that final
/// path. Returns `Ok(None)` on cancel and `Err` on Win32 error.
#[cfg(windows)]
// SAFETY CONTRACT:
// Voraussetzung: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread; Hook-Vertrag gilt.
// Warum erfüllt bzw. offen: Puffer leben bis zur Rückkehr; Windows- und Nicht-Windows-Varianten sind getrennt.
// Nachrichten: Der Windows-Dateidialog und Hook verarbeiten Nachrichten; die andere Variante nicht.
pub unsafe fn save(
    owner: *mut c_void,
    initial_name: Option<&Path>,
) -> Result<Option<(PathBuf, bool)>, String> {
    win32::dialog(owner, initial_name, true)
}

// Keep the module importable by host-side tools while making its platform
// limitation explicit through the only result channel in the requested API.
#[cfg(not(windows))]
// SAFETY CONTRACT:
// Voraussetzung: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread.
// Warum erfüllt bzw. offen: Puffer leben bis zur Rückkehr; Windows- und Nicht-Windows-Varianten haben getrennte Implementierungen.
// Nachrichten: Der Windows-Dateidialog verarbeitet Nachrichten; die andere Variante nicht.
pub unsafe fn open(_owner: *mut c_void) -> Result<Option<PathBuf>, String> {
    Err("Windows file dialogs are only available on Windows".into())
}

#[cfg(not(windows))]
// SAFETY CONTRACT:
// Voraussetzung: owner ist null oder ein gültiges Fenster auf dem passenden UI-Thread; Hook-Vertrag gilt.
// Warum erfüllt bzw. offen: Puffer leben bis zur Rückkehr; Windows- und Nicht-Windows-Varianten sind getrennt.
// Nachrichten: Der Windows-Dateidialog und Hook verarbeiten Nachrichten; die andere Variante nicht.
pub unsafe fn save(
    _owner: *mut c_void,
    _initial_name: Option<&Path>,
) -> Result<Option<(PathBuf, bool)>, String> {
    Err("Windows file dialogs are only available on Windows".into())
}
