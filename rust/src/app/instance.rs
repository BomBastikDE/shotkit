use super::*;

pub(super) const INSTANCE_MUTEX_NAME: &str =
    r"Local\ShotKitRustPrototype.RustOnly.SingleInstance.v1";
pub(super) const INSTANCE_CLASS_NAME: &str = "ShotKitRustPrototype";
pub(super) const WM_COPYDATA: u32 = 0x004a;
pub(super) const WM_INSTANCE_ACTIVATE: u32 = 0x8004;
pub(super) const WM_FINISH_QUIT: u32 = 0x8005;
pub(super) const INSTANCE_COPYDATA_TAG: usize = 0x534b_5255;
pub(super) const MAX_IPC_PATH_UNITS: usize = 32_768;
static INSTANCE_FILE_BUSY: AtomicBool = AtomicBool::new(false);

pub(super) struct InstanceFileBusyGuard;
impl Drop for InstanceFileBusyGuard {
    fn drop(&mut self) {
        INSTANCE_FILE_BUSY.store(false, Ordering::SeqCst);
    }
}

#[repr(C)]
pub(super) struct CopyData {
    tag: usize,
    bytes: u32,
    data: *const c_void,
}

pub(super) struct InstanceHandle(H);
impl Drop for InstanceHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

// SAFETY CONTRACT:
// Voraussetzung: Das zurückgegebene Handle wird genau einmal geschlossen.
// Warum erfüllt bzw. offen: Der UTF-16-Name lebt bis zur Rückkehr; der RAII-Guard übernimmt ein geprüftes Handle.
// Nachrichten: Kein expliziter synchroner Fensternachrichtenversand.
pub(super) unsafe fn create_instance_mutex() -> Result<(InstanceHandle, bool), String> {
    let name = wide(INSTANCE_MUTEX_NAME);
    SetLastError(0);
    let handle = CreateMutexW(null_mut(), 0, name.as_ptr());
    if handle.is_null() {
        return Err(format!(
            "Einzelinstanz-Sperre konnte nicht erstellt werden (Win32 {})",
            GetLastError()
        ));
    }
    let already_exists = GetLastError() == 183; // ERROR_ALREADY_EXISTS
    Ok((InstanceHandle(handle), already_exists))
}

pub(super) fn absolute_argument_path(path: PathBuf) -> Result<PathBuf, String> {
    std::path::absolute(path)
        .map_err(|error| format!("Der Dateipfad konnte nicht aufgelöst werden: {error}"))
}

fn drive_location(path: &Path) -> shotkit::network_path::DriveLocation {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    shotkit::network_path::classify_drive_utf16(&units, |root| {
        // SAFETY CONTRACT: root is a NUL-terminated UTF-16 drive root owned
        // by the caller and lives through this call. No window-state borrow is
        // held; the API is not asked to open the requested image file.
        unsafe { GetDriveTypeW(root.as_ptr()) }
    })
}

// SAFETY CONTRACT:
// Voraussetzung: Das gefundene Fensterhandle wird als Momentaufnahme behandelt.
// Warum erfüllt bzw. offen: Der Klassenname ist ein lokaler UTF-16-Puffer; zukünftige Gültigkeit und Identität sind gesondert zu prüfen.
// Nachrichten: Kein expliziter synchroner Fensternachrichtenversand im geprüften Pfad.
pub(super) unsafe fn find_primary_window(timeout_ms: u32) -> H {
    let class = wide(INSTANCE_CLASS_NAME);
    let mut waited = 0;
    while waited <= timeout_ms {
        let window = FindWindowW(class.as_ptr(), std::ptr::null());
        if !window.is_null() {
            return window;
        }
        Sleep(50);
        waited += 50;
    }
    null_mut()
}

// SAFETY CONTRACT:
// Voraussetzung: Ohne Pfad ist message für Nullparameter gültig; mit Pfad gilt WM_COPYDATA.
// Warum erfüllt bzw. offen: Aufrufer verwenden Aktivierung beziehungsweise WM_COPYDATA; Puffer leben bis zur synchronen Rückkehr.
// Nachrichten: SendMessageTimeoutW kann synchron Nachrichten verarbeiten.
pub(super) unsafe fn send_to_primary(message: u32, path: Option<&Path>) -> Result<(), String> {
    let window = find_primary_window(5_000);
    if window.is_null() {
        return Err("Die bereits gestartete ShotKit-Instanz hat ihr Fenster nicht rechtzeitig bereitgestellt.".into());
    }
    let mut result = 0usize;
    let mut copy_data = CopyData {
        tag: INSTANCE_COPYDATA_TAG,
        bytes: 0,
        data: null_mut(),
    };
    // Keep the backing buffer alive through SendMessageTimeoutW: WM_COPYDATA
    // borrows this memory synchronously in the receiving window procedure.
    let mut units: Option<Vec<u16>> = None;
    let (wparam, lparam) = if let Some(path) = path {
        let mut encoded: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if encoded.len() > MAX_IPC_PATH_UNITS {
            return Err("Der Dateipfad ist zu lang, um an die laufende ShotKit-Instanz weitergegeben zu werden.".into());
        }
        copy_data.bytes = u32::try_from(
            encoded
                .len()
                .checked_mul(2)
                .ok_or("Dateipfadgröße überläuft")?,
        )
        .map_err(|_| "Dateipfadgröße überläuft")?;
        copy_data.data = encoded.as_mut_ptr().cast();
        units = Some(encoded);
        (0usize, (&copy_data as *const CopyData) as isize)
    } else {
        (0usize, 0isize)
    };
    let sent = SendMessageTimeoutW(
        window,
        if path.is_some() { WM_COPYDATA } else { message },
        wparam,
        lparam,
        0x0002, // SMTO_ABORTIFHUNG
        15_000,
        &mut result,
    );
    // Make the lifetime explicit to readers and prevent an unused-variable
    // warning from obscuring the synchronous-borrow guarantee above.
    drop(units);
    if sent == 0 {
        return Err("Keine rechtzeitige Rückmeldung von der laufenden ShotKit-Instanz. Die Datei kann dort noch zur Bestätigung angezeigt werden.".into());
    }
    if path.is_some() && result != 1 {
        return Err(match result {
            2 => "Das Öffnen der Datei wurde in der laufenden ShotKit-Instanz abgebrochen.".into(),
            3 => "ShotKit bearbeitet gerade eine andere Dateianfrage. Bitte versuchen Sie es erneut.".into(),
            _ => "Die laufende ShotKit-Instanz konnte die Datei nicht öffnen (ungültige Datei oder Ein-/Ausgabefehler).".into(),
        });
    }
    Ok(())
}

// SAFETY CONTRACT:
// Voraussetzung: Die Verträge von send_to_primary gelten.
// Warum erfüllt bzw. offen: Es werden nur Aktivierung ohne Pfad und WM_COPYDATA mit besessenem Pfad gesendet.
// Nachrichten: SendMessageTimeoutW kann synchron Nachrichten verarbeiten.
pub(super) unsafe fn forward_second_instance() -> Result<(), String> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        return send_to_primary(WM_INSTANCE_ACTIVATE, None);
    }
    for path in paths {
        send_to_primary(WM_COPYDATA, Some(&absolute_argument_path(path)?))?;
    }
    Ok(())
}

// SAFETY CONTRACT:
// Voraussetzung: l zeigt auf eine lesbare, ausgerichtete WM_COPYDATA-Struktur
// mit lesbarem, für u16 ausgerichtetem Inhalt; h erfüllt den Fensterzustandsvertrag.
// Warum erfüllt bzw. offen: Windows liefert die Daten synchron; Länge und Null
// und Payload-Ausrichtung werden geprüft; Lesbarkeit und alle Aufrufer bleiben offen.
// Nachrichten: Bestätigungs- und Fehlerdialoge verarbeiten Nachrichten.
pub(super) unsafe fn handle_instance_file(h: H, l: L) -> L {
    if l == 0 {
        return 0;
    }
    let packet = &*(l as *const CopyData);
    if packet.tag != INSTANCE_COPYDATA_TAG
        || packet.bytes < 2
        || packet.bytes as usize > MAX_IPC_PATH_UNITS * 2
        || !packet.bytes.is_multiple_of(2)
        || packet.data.is_null()
        || !(packet.data as usize).is_multiple_of(std::mem::align_of::<u16>())
    {
        return 0;
    }
    let count = packet.bytes as usize / 2;
    let units = std::slice::from_raw_parts(packet.data.cast::<u16>(), count);
    if units.last() != Some(&0) || units[..count - 1].contains(&0) {
        return 0;
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(&units[..count - 1]));
    if INSTANCE_FILE_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return 3;
    }
    let _busy_guard = InstanceFileBusyGuard;
    let path = match absolute_argument_path(path) {
        Ok(path) => path,
        Err(error) => {
            MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
            return 0;
        }
    };
    use shotkit::network_path::DriveLocation;
    let location = drive_location(&path);
    let prompt = match location {
        DriveLocation::Network => format!(
            "Warnung: Diese Datei liegt auf einem Netzwerkserver:\n\n{}\n\nWenn Sie fortfahren, kontaktiert ShotKit den Server. Windows kann dabei Ihre Anmeldeinformationen automatisch zur Authentifizierung verwenden.\n\nDatei jetzt öffnen?",
            path.display()
        ),
        DriveLocation::Unknown => format!(
            "Der Laufwerkstyp dieser Datei konnte nicht sicher bestimmt werden:\n\n{}\n\nBeim Öffnen ist möglicherweise ein Netzwerkzugriff mit Windows-Authentifizierung erforderlich.\n\nDatei jetzt öffnen?",
            path.display()
        ),
        DriveLocation::KnownDrive => format!(
            "Eine zweite ShotKit-Instanz möchte diese Datei öffnen:\n\n{}\n\nSoll sie jetzt geöffnet werden?",
            path.display()
        ),
    };
    if MessageBoxW(
        h,
        wide(&prompt).as_ptr(),
        wide(match location {
            DriveLocation::Network => "Netzwerkdatei aus zweiter Instanz",
            DriveLocation::Unknown => "Datei mit unbekanntem Laufwerkstyp",
            DriveLocation::KnownDrive => "Datei aus zweiter Instanz",
        })
        .as_ptr(),
        0x24,
    ) != 6
    {
        return 2;
    }
    match open_image_path(h, path) {
        Ok(()) => 1,
        Err(error) => {
            MessageBoxW(h, wide(&error).as_ptr(), wide("Bild öffnen").as_ptr(), 0x10);
            0
        }
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn argument_paths_are_resolved_without_opening_the_file() {
        let current = std::env::current_dir().unwrap();
        let relative =
            absolute_argument_path(PathBuf::from("shotkit-nonexistent-image.png")).unwrap();
        assert_eq!(relative, current.join("shotkit-nonexistent-image.png"));
        let drive_relative =
            absolute_argument_path(PathBuf::from(r"C:shotkit-nonexistent-image.png")).unwrap();
        assert!(drive_relative.is_absolute());
        assert!(drive_relative.ends_with("shotkit-nonexistent-image.png"));
        let verbatim = PathBuf::from(r"\\?\C:\shotkit-nonexistent-image.png");
        assert_eq!(absolute_argument_path(verbatim.clone()).unwrap(), verbatim);
    }

    #[test]
    fn actual_system_drive_is_recognized_as_a_known_drive() {
        let path = PathBuf::from(std::env::var_os("SystemRoot").expect("Windows SystemRoot"));
        assert_eq!(
            drive_location(&path),
            shotkit::network_path::DriveLocation::KnownDrive
        );
    }
}
