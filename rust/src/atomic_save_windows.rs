//! GUI export to a sibling temporary file held through publication.
//!
//! The parent/ancestor namespace is assumed stable. This does not guard
//! against privileged actors, pre-existing mappings, or replaced ancestors.
//! A failed rename retains the temp: a failed filesystem operation is not
//! proof that its source and target names are unchanged.

use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io::{self, Write},
    mem::size_of,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_RENAME_INFO: i32 = 3;
const FILE_DISPOSITION_INFO: i32 = 4;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Error phase and original operating-system error for the settings caller.
pub(crate) struct SettingsWriteError {
    pub(crate) operation: &'static str,
    pub(crate) source: io::Error,
}

#[derive(Debug)]
struct WriteContext {
    source: io::Error,
    temp: PathBuf,
    target: Option<PathBuf>,
    cleanup: Option<io::Error>,
}

impl std::fmt::Display for WriteContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}; temporäre Datei: {}",
            self.source,
            self.temp.display()
        )?;
        if let Some(target) = &self.target {
            write!(f, "; Ziel prüfen: {}", target.display())?;
        }
        if let Some(cleanup) = &self.cleanup {
            write!(f, "; Bereinigung fehlgeschlagen: {cleanup}")?;
        }
        Ok(())
    }
}

impl std::error::Error for WriteContext {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(
        handle: *mut c_void,
        class: i32,
        info: *const c_void,
        size: u32,
    ) -> i32;
}

fn create_temp(temp: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .access_mode(GENERIC_WRITE | DELETE)
        .share_mode(FILE_SHARE_READ)
        .create_new(true)
        .open(temp)
}

fn candidate(dir: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(
        ".shotkit-{}-{stamp}-{sequence}.tmp",
        std::process::id()
    ))
}

/// Write settings through the same held-file contract as image export.
/// This does not protect against concurrent replacement of the target itself.
pub(crate) fn write_settings(path: &Path, bytes: &[u8]) -> Result<(), SettingsWriteError> {
    write_settings_with(
        path,
        |file| file.write_all(bytes).and_then(|_| file.sync_all()),
        |_| {},
        delete_held,
    )
}

fn write_settings_with(
    path: &Path,
    prepare: impl FnOnce(&mut File) -> io::Result<()>,
    before_publish: impl FnOnce(&Path),
    cleanup_temp: impl FnOnce(&File) -> io::Result<()>,
) -> Result<(), SettingsWriteError> {
    let create_error = |source| SettingsWriteError {
        operation: "create temporary file for",
        source,
    };
    let absolute = std::path::absolute(path).map_err(create_error)?;
    let dir = absolute.parent().ok_or_else(|| {
        create_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ungültiger Speicherordner",
        ))
    })?;
    let mut created = None;
    for _ in 0..128 {
        let temp = candidate(dir);
        match create_temp(&temp) {
            Ok(file) => {
                created = Some((temp, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(create_error(error)),
        }
    }
    let (temp, mut file) = created.ok_or_else(|| {
        create_error(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Kein freier temporärer Dateiname",
        ))
    })?;
    if let Err(source) = prepare(&mut file) {
        let cleanup = cleanup_temp(&file).err();
        let source = if cleanup.is_some() {
            io::Error::new(
                source.kind(),
                WriteContext {
                    source,
                    temp,
                    target: None,
                    cleanup,
                },
            )
        } else {
            source
        };
        return Err(SettingsWriteError {
            operation: "write",
            source,
        });
    }
    before_publish(&temp);
    rename_held(&file, &absolute, true).map_err(|source| {
        let kind = source.kind();
        SettingsWriteError {
            operation: "replace",
            source: io::Error::new(
                kind,
                WriteContext {
                    source,
                    temp,
                    target: Some(absolute),
                    cleanup: None,
                },
            ),
        }
    })
}

pub fn write_atomic(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    // FileRenameInfo requires a fully qualified destination; resolve relative
    // configured save folders once, before creating the sibling temporary file.
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let path = absolute.as_path();
    let dir = path.parent().ok_or("Ungültiger Speicherordner.")?;
    for _ in 0..128 {
        let temp = candidate(dir);
        match create_temp(&temp) {
            Ok(file) => return write_created(path, bytes, replace, &temp, file, |_| {}),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Kein freier temporärer Dateiname gefunden.".to_owned())
}

/// Write once to a held sibling file, then try up to `attempts` distinct
/// destinations without replacing existing files. On an ambiguous rename
/// error the prepared file is retained and its path is reported.
pub fn write_new_unique(
    folder: &Path,
    bytes: &[u8],
    attempts: usize,
    next_path: impl FnMut() -> PathBuf,
) -> io::Result<PathBuf> {
    write_unique_prepared(
        folder,
        attempts,
        next_path,
        |file| file.write_all(bytes).and_then(|_| file.sync_all()),
        |_| {},
    )
}

fn write_unique_prepared(
    folder: &Path,
    attempts: usize,
    mut next_path: impl FnMut() -> PathBuf,
    prepare: impl FnOnce(&mut File) -> io::Result<()>,
    before_attempt: impl Fn(&Path),
) -> io::Result<PathBuf> {
    let absolute_folder = std::path::absolute(folder)?;
    let mut created = None;
    for _ in 0..128 {
        let temp = candidate(&absolute_folder);
        match create_temp(&temp) {
            Ok(file) => {
                created = Some((temp, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let (temp, mut file) = created.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Kein freier temporärer Dateiname",
        )
    })?;
    if let Err(error) = prepare(&mut file) {
        return match delete_held(&file) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; temporäre Datei konnte nicht entfernt werden ({}): {cleanup}",
                    temp.display()
                ),
            )),
        };
    }
    for _ in 0..attempts {
        before_attempt(&temp);
        let path = next_path();
        let absolute_path = std::path::absolute(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("{error}; temporäre Datei: {}", temp.display()),
            )
        })?;
        if absolute_path.parent() != Some(absolute_folder.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Ziel liegt nicht im Speicherordner; temporäre Datei: {}",
                    temp.display()
                ),
            ));
        }
        match rename_held(&file, &absolute_path, false) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; Speicherzustand prüfen: Ziel {} und temporäre Datei {}",
                        path.display(),
                        temp.display()
                    ),
                ))
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("Kein freier Dateiname; temporäre Datei: {}", temp.display()),
    ))
}

fn write_created<F: FnOnce(&Path)>(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    temp: &Path,
    file: File,
    before_publish: F,
) -> Result<(), String> {
    write_prepared(path, replace, temp, file, before_publish, |file| {
        file.write_all(bytes).and_then(|_| file.sync_all())
    })
}

fn write_prepared<F: FnOnce(&Path), P: FnOnce(&mut File) -> io::Result<()>>(
    path: &Path,
    replace: bool,
    temp: &Path,
    mut file: File,
    before_publish: F,
    prepare: P,
) -> Result<(), String> {
    if let Err(error) = prepare(&mut file) {
        return match delete_held(&file) {
            Ok(()) => Err(error.to_string()),
            Err(cleanup) => Err(format!(
                "{error}; temporäre Datei konnte nicht entfernt werden ({}): {cleanup}",
                temp.display()
            )),
        };
    }
    before_publish(temp);
    rename_held(&file, path, replace).map_err(|error| {
        format!(
            "{error}; Speicherzustand prüfen: Ziel {} und temporäre Datei {}",
            path.display(),
            temp.display()
        )
    })
}

fn delete_held(file: &File) -> io::Result<()> {
    let delete_file: u8 = 1;
    // SAFETY: File owns a live DELETE-capable Windows handle, and delete_file
    // is a valid FILE_DISPOSITION_INFO (one BOOLEAN) for this synchronous call.
    // No UI messages are dispatched. Only the held file is marked for deletion.
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FILE_DISPOSITION_INFO,
            (&delete_file as *const u8).cast(),
            1,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn rename_held(file: &File, destination: &Path, replace: bool) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "Ungültiger Zielpfad");
    let name: Vec<u16> = destination.as_os_str().encode_wide().collect();
    if !destination.is_absolute() || name.contains(&0) {
        return Err(invalid());
    }
    // FILE_RENAME_INFO has a BOOLEAN, aligned HANDLE, DWORD and WCHAR array.
    let root_offset = size_of::<usize>();
    let length_offset = root_offset + size_of::<usize>();
    let name_offset = length_offset + size_of::<u32>();
    let name_bytes = name.len().checked_mul(2).ok_or_else(invalid)?;
    let size = name_offset
        .checked_add(name_bytes)
        .and_then(|n| n.checked_add(2))
        .ok_or_else(invalid)?;
    let native_size = u32::try_from(size).map_err(|_| invalid())?;
    let native_name_bytes = u32::try_from(name_bytes).map_err(|_| invalid())?;
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    let buffer = storage.as_mut_ptr().cast::<u8>();
    // SAFETY: aligned storage covers all checked offsets; the filename bytes
    // and length are in bounds. The handle is live and DELETE-capable, and is
    // held throughout this synchronous FileRenameInfo call; no UI messages
    // are dispatched. The first BOOLEAN selects the requested replacement policy.
    let ok = unsafe {
        buffer.write(u8::from(replace));
        buffer
            .add(length_offset)
            .cast::<u32>()
            .write(native_name_bytes);
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            buffer.add(name_offset).cast::<u16>(),
            name.len(),
        );
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FILE_RENAME_INFO,
            buffer.cast(),
            native_size,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn directory() -> PathBuf {
        let dir = candidate(&std::env::temp_dir());
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn preexisting_temp_collision_preserves_foreign_bytes() {
        let dir = directory();
        let temp = dir.join("foreign.tmp");
        fs::write(&temp, b"foreign").unwrap();
        assert_eq!(
            create_temp(&temp).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&temp).unwrap(), b"foreign");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn held_temp_blocks_replacement_and_publishes_own_bytes() {
        let dir = directory();
        let temp = dir.join("temp");
        let target = dir.join("image.png");
        let file = create_temp(&temp).unwrap();
        write_created(&target, b"own image", false, &temp, file, |name| {
            assert!(fs::remove_file(name).is_err());
            assert!(fs::rename(name, name.with_extension("stolen")).is_err());
            assert!(fs::write(name, b"foreign").is_err());
        })
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"own image");
        assert!(!temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_replace_preserves_collision_and_complete_temporary_image() {
        let dir = directory();
        let temp = dir.join("temp");
        let target = dir.join("image.png");
        let file = create_temp(&temp).unwrap();
        let result = write_created(&target, b"own image", false, &temp, file, |_| {
            fs::write(&target, b"foreign").unwrap();
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"foreign");
        assert_eq!(fs::read(&temp).unwrap(), b"own image");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replace_overwrites_confirmed_target() {
        let dir = directory();
        let temp = dir.join("temp");
        let target = dir.join("image.png");
        fs::write(&target, b"old").unwrap();
        write_created(
            &target,
            b"new",
            true,
            &temp,
            create_temp(&temp).unwrap(),
            |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_preparation_deletes_only_held_temp_and_preserves_target() {
        let dir = directory();
        let temp = dir.join("temp");
        let target = dir.join("image.png");
        fs::write(&target, b"old").unwrap();
        let result = write_prepared(
            &target,
            true,
            &temp,
            create_temp(&temp).unwrap(),
            |_| panic!("must not publish an incomplete image"),
            |file| {
                file.write_all(b"partial")?;
                Err(io::Error::other("injected write or sync failure"))
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert!(!temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replace_fails_safely_when_destination_disallows_delete_sharing() {
        let dir = directory();
        let temp = dir.join("temp");
        let target = dir.join("image.png");
        fs::write(&target, b"old").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&target)
            .unwrap();
        assert!(write_created(
            &target,
            b"new",
            true,
            &temp,
            create_temp(&temp).unwrap(),
            |_| {}
        )
        .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read(&temp).unwrap(), b"new");
        drop(held);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_destination_is_supported() {
        let name = candidate(Path::new("."));
        fs::create_dir(&name).unwrap();
        let target = name.join("image.png");
        write_atomic(&target, b"image", false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"image");
        fs::remove_dir_all(name).unwrap();
    }

    #[test]
    fn unicode_destination_is_supported() {
        let dir = directory();
        let target = dir.join("Aufnahme-Ж-é.png");
        write_atomic(&target, b"image", false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"image");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_output_retries_collision_without_replacing_or_leaking_temps() {
        let dir = directory();
        let first = dir.join("existing.png");
        let second = dir.join("fresh.png");
        fs::write(&first, b"foreign").unwrap();
        let mut names = [first.clone(), second.clone()].into_iter();
        let published = write_new_unique(&dir, b"capture", 2, || names.next().unwrap()).unwrap();
        assert_eq!(published, second);
        assert_eq!(fs::read(first).unwrap(), b"foreign");
        assert_eq!(fs::read(&published).unwrap(), b"capture");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn exhausted_unique_output_retains_only_one_complete_temporary_file() {
        let dir = directory();
        let target = dir.join("existing.png");
        fs::write(&target, b"foreign").unwrap();
        let error = write_new_unique(&dir, b"capture", 3, || target.clone()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), b"foreign");
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(Result::unwrap).collect();
        assert_eq!(files.len(), 2);
        let temp = files
            .iter()
            .find(|entry| entry.path() != target)
            .unwrap()
            .path();
        assert_eq!(fs::read(&temp).unwrap(), b"capture");
        assert!(error.to_string().contains(&temp.display().to_string()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_output_is_complete_and_temp_is_protected_during_retry() {
        let dir = directory();
        let first = dir.join("existing.png");
        let second = dir.join("fresh.png");
        fs::write(&first, b"foreign").unwrap();
        let mut names = [first.clone(), second.clone()].into_iter();
        let path = write_unique_prepared(
            &dir,
            2,
            || names.next().unwrap(),
            |file| {
                file.write_all(b"complete capture")?;
                file.sync_all()
            },
            |temp| {
                assert!(!second.exists());
                assert_eq!(fs::read(temp).unwrap(), b"complete capture");
                assert!(fs::remove_file(temp).is_err());
                assert!(fs::rename(temp, temp.with_extension("other")).is_err());
                assert!(fs::write(temp, b"foreign").is_err());
            },
        )
        .unwrap();
        assert_eq!(path, second);
        assert_eq!(fs::read(first).unwrap(), b"foreign");
        assert_eq!(fs::read(second).unwrap(), b"complete capture");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_output_partial_write_failure_removes_held_temp_without_publish() {
        let dir = directory();
        let target = dir.join("capture.png");
        let error = write_unique_prepared(
            &dir,
            1,
            || target.clone(),
            |file| {
                file.write_all(b"partial")?;
                Err(io::Error::other("injected write failure"))
            },
            |_| panic!("partial output must not be published"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_output_supports_relative_folder() {
        let dir = candidate(Path::new("."));
        fs::create_dir(&dir).unwrap();
        let target = dir.join("capture.png");
        let path = write_new_unique(&dir, b"capture", 1, || target.clone()).unwrap();
        assert_eq!(path, target);
        assert_eq!(fs::read(&path).unwrap(), b"capture");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_output_rejects_foreign_directory_and_retains_complete_temp() {
        let dir = directory();
        let other = directory();
        let foreign = other.join("capture.png");
        let error = write_new_unique(&dir, b"capture", 1, || foreign.clone()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!foreign.exists());
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(Result::unwrap).collect();
        assert_eq!(files.len(), 1);
        assert_eq!(fs::read(files[0].path()).unwrap(), b"capture");
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn settings_temp_is_complete_and_protected_until_publication() {
        let dir = directory();
        let target = dir.join("settings.json");
        fs::write(&target, b"old").unwrap();
        write_settings_with(
            &target,
            |file| {
                file.write_all(b"new settings")?;
                file.sync_all()
            },
            |temp| {
                assert_eq!(fs::read(temp).unwrap(), b"new settings");
                assert!(fs::remove_file(temp).is_err());
                assert!(fs::rename(temp, temp.with_extension("other")).is_err());
                assert!(fs::write(temp, b"foreign").is_err());
                assert_eq!(fs::read(&target).unwrap(), b"old");
            },
            delete_held,
        )
        .unwrap_or_else(|error| panic!("{}: {}", error.operation, error.source));
        assert_eq!(fs::read(&target).unwrap(), b"new settings");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_preparation_failures_preserve_original_and_cleanup_owned_temp() {
        for fail_after_write in [false, true] {
            let dir = directory();
            let target = dir.join("settings.json");
            fs::write(&target, b"old").unwrap();
            let error = write_settings_with(
                &target,
                |file| {
                    if fail_after_write {
                        file.write_all(b"partial")?;
                    }
                    Err(io::Error::from_raw_os_error(5))
                },
                |_| panic!("failed preparation must not publish"),
                delete_held,
            )
            .err()
            .unwrap();
            assert_eq!(error.operation, "write");
            assert_eq!(error.source.raw_os_error(), Some(5));
            assert_eq!(fs::read(&target).unwrap(), b"old");
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn settings_failed_replace_retains_complete_temp_and_error_source() {
        use std::error::Error;
        let dir = directory();
        let target = dir.join("settings.json");
        fs::write(&target, b"old").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&target)
            .unwrap();
        let error = write_settings(&target, b"new").err().unwrap();
        assert_eq!(error.operation, "replace");
        let context = error
            .source
            .get_ref()
            .unwrap()
            .downcast_ref::<WriteContext>()
            .unwrap();
        assert_eq!(context.source.kind(), error.source.kind());
        assert!(context.source.raw_os_error().is_some());
        assert!(context.source.source().is_none());
        assert_eq!(
            error
                .source
                .source()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .and_then(io::Error::raw_os_error),
            context.source.raw_os_error()
        );
        assert_eq!(fs::read(&target).unwrap(), b"old");
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(Result::unwrap).collect();
        assert_eq!(files.len(), 2);
        let temp = files
            .iter()
            .find(|entry| entry.path() != target)
            .unwrap()
            .path();
        assert_eq!(fs::read(&temp).unwrap(), b"new");
        assert!(error
            .source
            .to_string()
            .contains(&temp.display().to_string()));
        drop(held);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_cleanup_failure_retains_temp_and_original_error_context() {
        use std::error::Error;
        let dir = directory();
        let target = dir.join("settings.json");
        fs::write(&target, b"old").unwrap();
        let error = write_settings_with(
            &target,
            |file| {
                file.write_all(b"partial")?;
                Err(io::Error::from_raw_os_error(5))
            },
            |_| panic!("failed preparation must not publish"),
            |_| Err(io::Error::other("injected cleanup failure")),
        )
        .err()
        .unwrap();
        assert_eq!(error.operation, "write");
        assert_eq!(error.source.kind(), io::ErrorKind::PermissionDenied);
        let context = error
            .source
            .get_ref()
            .unwrap()
            .downcast_ref::<WriteContext>()
            .unwrap();
        assert_eq!(context.source.raw_os_error(), Some(5));
        assert_eq!(
            error
                .source
                .source()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .and_then(io::Error::raw_os_error),
            Some(5)
        );
        assert!(context
            .cleanup
            .as_ref()
            .unwrap()
            .to_string()
            .contains("injected cleanup failure"));
        assert_eq!(fs::read(&context.temp).unwrap(), b"partial");
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert!(error
            .source
            .to_string()
            .contains(&context.temp.display().to_string()));
        fs::remove_dir_all(dir).unwrap();
    }
}
