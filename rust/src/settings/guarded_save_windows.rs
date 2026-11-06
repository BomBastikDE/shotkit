//! Isolated Windows guarded-save primitive; application integration is separate.
//!
//! FILE SAFETY CONTRACT:
//! - The caller supplies an absolute file path in a trusted, stable parent and
//!   ancestor namespace, an explicit load baseline, and validated settings JSON.
//!   This module also checks JSON/version before creating any files. It does not
//!   create directories or defend against parent/reparse namespace attacks,
//!   privileged access, memory-mapped writers, or nonconforming filesystems.
//! - A unique sibling temporary file is created with READ | WRITE | DELETE,
//!   sharing READ only. Its complete bytes are written and flushed before an
//!   existing destination is opened, checked, or moved. Its handle remains held
//!   through publication; a failed preparation deliberately retains the temp.
//! - Existing baselines require exact byte equality and unambiguous supported
//!   JSON/version. The source is held READ | DELETE, sharing READ only, from
//!   verification through both renames. Leaf reparse points are refused. Native
//!   FileRenameInfo renames operate on these held objects and never replace an
//!   existing name. Missing is not inferred from another read/open error.
//! - Existing saves first rename the old object to a unique sibling backup,
//!   then rename the already complete temp to the destination. There IS a
//!   destination-name gap. A competing new destination wins; publication fails
//!   without changing it. This is not a compare-and-swap or an atomic transaction.
//! - No failure deletes a file, rolls back, or automatically retries publication.
//!   Errors report the target and retained or uncertain artifact paths. After a
//!   native rename error, inspect both endpoints: failure is not asserted to
//!   prove either endpoint absent. Paths describe this operation's last known
//!   state; after handles close, another actor may change those names.
//! - Equal bytes cannot prove file identity or detect change-and-change-back
//!   since loading. Identical old/new bytes still follow the same guarded save
//!   and preserve a backup. A byte baseline is not a file-identity baseline.
//! - File sync covers prepared contents, not directory-entry crash durability.
//!   Process/power failure between renames may leave only temp and backup names;
//!   no crash recovery, multi-file atomicity, or durable directory flush is
//!   promised. Callers must surface preserved paths and obtain a fresh baseline
//!   after failure rather than silently falling back to an unguarded save.
//!
//! Win32 references:
//! <https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew>
//! <https://learn.microsoft.com/windows/win32/api/winbase/ns-winbase-file_rename_info>

use super::{Settings, CURRENT_VERSION};
use serde::Deserialize;
use std::{
    error::Error,
    ffi::c_void,
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    mem::size_of,
    os::windows::{ffi::OsStrExt, fs::MetadataExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_RENAME_INFO: i32 = 3;
static NAME_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The actual load result, never a default inferred from malformed/unreadable
/// input. Existing contains the exact original file bytes, not reserialized JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Baseline {
    Missing,
    Existing(Vec<u8>),
}

/// Last completed stage. No variant authorizes cleanup or assumes the target
/// absent. A failed native rename requires inspection of both named endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveState {
    /// No file was created or moved by this operation.
    NoMutation,
    /// The temp was created (possibly partial); the target was not moved.
    TemporaryRetained { temp_path: PathBuf },
    /// Moving the original failed. Inspect target and attempted backup as well
    /// as the prepared temp; publication was not attempted.
    BackupMoveFailed {
        temp_path: PathBuf,
        attempted_backup: PathBuf,
    },
    /// Publication failed. The complete temp's rename was attempted, so inspect
    /// temp and target. A successful old-file backup, if any, is retained.
    PublicationFailed {
        temp_path: PathBuf,
        backup_path: Option<PathBuf>,
    },
}

#[derive(Debug)]
pub struct SaveError {
    pub target_path: PathBuf,
    // Keep the multi-path diagnostic out of every Result's inline error size.
    pub state: Box<SaveState>,
    pub operation: &'static str,
    pub source: io::Error,
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "guarded settings save {} at {} failed ({:?}): {}",
            self.operation,
            self.target_path.display(),
            self.state,
            self.source
        )
    }
}

impl Error for SaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
pub struct SaveSuccess {
    pub target_path: PathBuf,
    pub backup_path: Option<PathBuf>,
}

fn failure(path: &Path, state: SaveState, operation: &'static str, source: io::Error) -> SaveError {
    SaveError {
        target_path: path.to_owned(),
        state: Box::new(state),
        operation,
        source,
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Publish complete settings without overwriting a changed/foreign destination.
///
/// Requires the module's safety contract. On success, the old bytes have a
/// backup name for Existing. This does not update the caller's baseline: reload
/// deliberately before a later save. All failure artifacts are retained.
pub fn save_guarded(
    path: &Path,
    baseline: &Baseline,
    replacement_bytes: &[u8],
) -> Result<SaveSuccess, SaveError> {
    save_with(
        path,
        baseline,
        replacement_bytes,
        |file, bytes| {
            file.write_all(bytes)?;
            file.sync_all()
        },
        |_, _| {},
    )
}

// The callbacks provide deterministic write/flush and name-gap fault injection
// for native Windows tests. They are private; production always uses the writer
// and no-op callback above.
fn save_with(
    path: &Path,
    baseline: &Baseline,
    bytes: &[u8],
    prepare: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    before_publish: impl FnOnce(&Path, Option<&Path>),
) -> Result<SaveSuccess, SaveError> {
    let initial = |operation, source| failure(path, SaveState::NoMutation, operation, source);
    validate_path(path).map_err(|error| initial("validate target path", error))?;
    validate_json(bytes).map_err(|error| initial("validate replacement", error))?;
    if let Baseline::Existing(expected) = baseline {
        validate_json(expected).map_err(|error| initial("validate baseline", error))?;
    }

    let (temp_path, mut temporary) =
        create_temp(path).map_err(|error| initial("create unique temporary file", error))?;
    let prepared = |operation, source| {
        failure(
            path,
            SaveState::TemporaryRetained {
                temp_path: temp_path.clone(),
            },
            operation,
            source,
        )
    };
    regular_file(&temporary).map_err(|error| prepared("validate temporary file", error))?;
    prepare(&mut temporary, bytes)
        .map_err(|error| prepared("write/flush temporary file", error))?;

    // Keep original protection until publication has finished, even after its
    // name has changed to the backup. No path-based reopening is used.
    let original = match baseline {
        Baseline::Missing => None,
        Baseline::Existing(expected) => {
            let mut held =
                open_protected(path).map_err(|error| prepared("open expected source", error))?;
            verify_expected(&mut held, expected)
                .map_err(|error| prepared("verify expected source", error))?;
            Some(held)
        }
    };
    let backup_path = original
        .as_ref()
        .map(|held| move_to_backup(held, path, &temp_path))
        .transpose()?;
    before_publish(&temp_path, backup_path.as_deref());
    rename_without_replace(&temporary, path).map_err(|source| {
        failure(
            path,
            SaveState::PublicationFailed {
                temp_path: temp_path.clone(),
                backup_path: backup_path.clone(),
            },
            "publish prepared file",
            source,
        )
    })?;
    drop(temporary);
    drop(original);
    Ok(SaveSuccess {
        target_path: path.to_owned(),
        backup_path,
    })
}

fn validate_path(path: &Path) -> io::Result<()> {
    let name = path
        .file_name()
        .filter(|_| path.is_absolute())
        .ok_or_else(|| invalid("an absolute file path is required"))?;
    if path.as_os_str().encode_wide().any(|unit| unit == 0)
        || name.encode_wide().any(|unit| unit == u16::from(b':'))
    {
        return Err(invalid(
            "target must not contain NUL or name an alternate data stream",
        ));
    }
    Ok(())
}

fn validate_json(bytes: &[u8]) -> io::Result<()> {
    // Do not silently broaden this primitive on a future schema bump.
    if CURRENT_VERSION != 1 {
        return Err(invalid("guarded save currently supports only version 1"));
    }
    if bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        != Some(b'{')
    {
        return Err(invalid("settings must be a JSON object"));
    }
    #[derive(Deserialize)]
    struct VersionHeader {
        version: u32,
    }
    // Derived struct deserialization rejects absent, duplicate and noninteger
    // versions, unlike searching text or parsing into a generic JSON map.
    let header: VersionHeader = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if header.version != CURRENT_VERSION {
        return Err(invalid("settings version is unsupported"));
    }
    let _: Settings = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(())
}

fn sibling_name(path: &Path, suffix: &str) -> PathBuf {
    let sequence = NAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut name = path
        .file_name()
        .expect("validated target file name")
        .to_os_string();
    name.push(format!(
        ".guarded-{}-{stamp}-{sequence}.{suffix}",
        std::process::id()
    ));
    path.with_file_name(name)
}

fn create_temp(path: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..128 {
        let candidate = sibling_name(path, "tmp");
        match OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .create_new(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "all temporary names collided",
    ))
}

fn regular_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid("expected a regular file, not a reparse point"));
    }
    Ok(())
}

fn open_protected(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    regular_file(&file)?;
    Ok(file)
}

fn verify_expected(held: &mut File, expected: &[u8]) -> io::Result<()> {
    let limit = u64::try_from(expected.len())
        .ok()
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| invalid("baseline is too large"))?;
    let mut actual = Vec::new();
    held.take(limit).read_to_end(&mut actual)?;
    if actual != expected {
        return Err(invalid("source bytes changed since loading"));
    }
    validate_json(&actual)
}

fn move_to_backup(held: &File, path: &Path, temp_path: &Path) -> Result<PathBuf, SaveError> {
    for _ in 0..128 {
        let candidate = sibling_name(path, "bak");
        match rename_without_replace(held, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if matches!(error.raw_os_error(), Some(80 | 183)) => continue,
            Err(source) => {
                return Err(failure(
                    path,
                    SaveState::BackupMoveFailed {
                        temp_path: temp_path.to_owned(),
                        attempted_backup: candidate,
                    },
                    "move original to backup",
                    source,
                ));
            }
        }
    }
    Err(failure(
        path,
        SaveState::TemporaryRetained {
            temp_path: temp_path.to_owned(),
        },
        "allocate backup name",
        io::Error::new(io::ErrorKind::AlreadyExists, "all backup names collided"),
    ))
}

fn rename_without_replace(held: &File, destination: &Path) -> io::Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
    }
    let name: Vec<u16> = destination.as_os_str().encode_wide().collect();
    if !destination.is_absolute() || name.contains(&0) {
        return Err(invalid("rename destination must be absolute without NUL"));
    }
    // FILE_RENAME_INFO: BOOLEAN/flags union, aligned HANDLE, DWORD, WCHAR[].
    // Zero initialization selects ReplaceIfExists=FALSE and RootDirectory=NULL.
    let root_offset = size_of::<usize>();
    let length_offset = root_offset + size_of::<usize>();
    let name_offset = length_offset + size_of::<u32>();
    let name_bytes = name
        .len()
        .checked_mul(2)
        .ok_or_else(|| invalid("rename path is too long"))?;
    let size = name_offset
        .checked_add(name_bytes)
        .and_then(|size| size.checked_add(2))
        .ok_or_else(|| invalid("rename path is too long"))?;
    let native_size = u32::try_from(size).map_err(|_| invalid("rename path is too long"))?;
    let native_name_bytes =
        u32::try_from(name_bytes).map_err(|_| invalid("rename path is too long"))?;
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    let buffer = storage.as_mut_ptr().cast::<u8>();
    // SAFETY: zeroed usize storage gives HANDLE alignment on both pointer widths
    // and at least `size` bytes. Checked offsets and lengths keep both payload
    // writes inside it. FileNameLength excludes the trailing zero WCHAR. File
    // holds a synchronous DELETE-capable handle throughout this call. Class 3
    // is FILE_RENAME_INFO, whose ABI layout is described above. No pointer escapes.
    let result = unsafe {
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
            held.as_raw_handle(),
            FILE_RENAME_INFO,
            buffer.cast(),
            native_size,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const OLD: &[u8] = br#"{"version":1,"open_editor":true}"#;
    const NEW: &[u8] = br#"{"version":1,"open_editor":false}"#;

    fn fixture() -> (PathBuf, PathBuf) {
        let dir = sibling_name(&std::env::temp_dir().join("shotkit-save-test"), "dir");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("settings.json");
        (dir, path)
    }

    fn flush(file: &mut File, bytes: &[u8]) -> io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    }

    fn retained_temp(error: &SaveError) -> &Path {
        match error.state.as_ref() {
            SaveState::TemporaryRetained { temp_path }
            | SaveState::BackupMoveFailed { temp_path, .. }
            | SaveState::PublicationFailed { temp_path, .. } => temp_path,
            SaveState::NoMutation => panic!("expected retained temporary file"),
        }
    }

    #[test]
    fn missing_success_publishes_exact_bytes_without_backup() {
        let (dir, path) = fixture();
        let saved = save_guarded(&path, &Baseline::Missing, NEW).unwrap();
        assert_eq!(saved.target_path, path);
        assert!(saved.backup_path.is_none());
        assert_eq!(fs::read(&path).unwrap(), NEW);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_and_identical_saves_preserve_exact_backup_bytes() {
        for replacement in [NEW, OLD] {
            let (dir, path) = fixture();
            fs::write(&path, OLD).unwrap();
            let saved =
                save_guarded(&path, &Baseline::Existing(OLD.to_vec()), replacement).unwrap();
            assert_eq!(fs::read(saved.backup_path.unwrap()).unwrap(), OLD);
            assert_eq!(fs::read(&path).unwrap(), replacement);
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn changed_or_disappeared_source_keeps_prepared_temp_and_never_publishes() {
        for present in [false, true] {
            let (dir, path) = fixture();
            if present {
                fs::write(&path, NEW).unwrap();
            }
            let error = save_guarded(&path, &Baseline::Existing(OLD.to_vec()), NEW).unwrap_err();
            assert!(matches!(
                error.state.as_ref(),
                SaveState::TemporaryRetained { .. }
            ));
            assert_eq!(fs::read(retained_temp(&error)).unwrap(), NEW);
            if present {
                assert_eq!(fs::read(&path).unwrap(), NEW);
            } else {
                assert!(!path.exists());
            }
            assert_eq!(
                fs::read_dir(&dir).unwrap().count(),
                if present { 2 } else { 1 }
            );
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn invalid_baselines_and_replacements_create_nothing() {
        for invalid_bytes in [
            &br#"[]"#[..],
            &br#"[1]"#[..],
            &br#"{"version":2}"#[..],
            &br#"{"version":0}"#[..],
            &br#"{"version":1,"version":1}"#[..],
            &br#"{"version":"1"}"#[..],
            &br#"{"open_editor":true}"#[..],
            &br#"{"version":1,"unknown":1}"#[..],
            &br#"{"version":1,"scroll_direction":"future"}"#[..],
            &b"{broken"[..],
        ] {
            let (dir, path) = fixture();
            fs::write(&path, invalid_bytes).unwrap();
            let error =
                save_guarded(&path, &Baseline::Existing(invalid_bytes.to_vec()), NEW).unwrap_err();
            assert_eq!(error.state.as_ref(), &SaveState::NoMutation);
            let error = save_guarded(&path, &Baseline::Missing, invalid_bytes).unwrap_err();
            assert_eq!(error.state.as_ref(), &SaveState::NoMutation);
            assert_eq!(fs::read(&path).unwrap(), invalid_bytes);
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn preparation_failure_retains_partial_temp_and_original() {
        let (dir, path) = fixture();
        fs::write(&path, OLD).unwrap();
        let error = save_with(
            &path,
            &Baseline::Existing(OLD.to_vec()),
            NEW,
            |file, _| {
                file.write_all(b"partial")?;
                Err(io::Error::other("injected write/flush failure"))
            },
            |_, _| panic!("must not reach publication"),
        )
        .unwrap_err();
        assert_eq!(error.operation, "write/flush temporary file");
        assert_eq!(fs::read(retained_temp(&error)).unwrap(), b"partial");
        assert_eq!(fs::read(&path).unwrap(), OLD);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn foreign_destination_before_missing_save_is_preserved() {
        let (dir, path) = fixture();
        fs::write(&path, b"foreign").unwrap();
        let error = save_guarded(&path, &Baseline::Missing, NEW).unwrap_err();
        assert!(matches!(
            error.state.as_ref(),
            SaveState::PublicationFailed {
                backup_path: None,
                ..
            }
        ));
        assert_eq!(fs::read(&path).unwrap(), b"foreign");
        assert_eq!(fs::read(retained_temp(&error)).unwrap(), NEW);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn foreign_arrival_at_publication_keeps_foreign_temp_and_optional_backup() {
        for existing in [false, true] {
            let (dir, path) = fixture();
            let baseline = if existing {
                fs::write(&path, OLD).unwrap();
                Baseline::Existing(OLD.to_vec())
            } else {
                Baseline::Missing
            };
            let error = save_with(&path, &baseline, NEW, flush, |_, backup| {
                assert_eq!(backup.is_some(), existing);
                assert!(!path.exists());
                fs::write(&path, b"foreign arrival").unwrap();
            })
            .unwrap_err();
            let SaveState::PublicationFailed { backup_path, .. } = error.state.as_ref() else {
                panic!("expected publication failure");
            };
            assert_eq!(fs::read(&path).unwrap(), b"foreign arrival");
            assert_eq!(fs::read(retained_temp(&error)).unwrap(), NEW);
            assert_eq!(backup_path.is_some(), existing);
            if let Some(backup) = backup_path {
                assert_eq!(fs::read(backup).unwrap(), OLD);
            }
            assert_eq!(
                fs::read_dir(&dir).unwrap().count(),
                if existing { 3 } else { 2 }
            );
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn held_temp_and_backup_block_writes_and_deletes_until_publication() {
        let (dir, path) = fixture();
        fs::write(&path, OLD).unwrap();
        save_with(
            &path,
            &Baseline::Existing(OLD.to_vec()),
            NEW,
            flush,
            |temp, backup| {
                for name in [temp, backup.unwrap()] {
                    // A new reader must itself share the held DELETE/WRITE access.
                    let mut reader = OpenOptions::new()
                        .read(true)
                        .share_mode(7)
                        .open(name)
                        .unwrap();
                    let mut bytes = Vec::new();
                    reader.read_to_end(&mut bytes).unwrap();
                    assert_eq!(bytes, if name == temp { NEW } else { OLD });
                    for access in [GENERIC_WRITE, DELETE] {
                        let error = OpenOptions::new()
                            .access_mode(access)
                            .share_mode(7)
                            .open(name)
                            .unwrap_err();
                        assert_eq!(error.raw_os_error(), Some(32));
                    }
                }
            },
        )
        .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_writer_prevents_source_open_and_preserves_both_files() {
        let (dir, path) = fixture();
        fs::write(&path, OLD).unwrap();
        let writer = OpenOptions::new()
            .write(true)
            .share_mode(7)
            .open(&path)
            .unwrap();
        let error = save_guarded(&path, &Baseline::Existing(OLD.to_vec()), NEW).unwrap_err();
        assert_eq!(error.source.raw_os_error(), Some(32));
        assert_eq!(error.operation, "open expected source");
        assert_eq!(fs::read(retained_temp(&error)).unwrap(), NEW);
        drop(writer);
        assert_eq!(fs::read(&path).unwrap(), OLD);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_collision_and_error_preserve_held_object() {
        let (dir, path) = fixture();
        fs::write(&path, OLD).unwrap();
        let occupied = dir.join("occupied.bak");
        fs::write(&occupied, b"other backup").unwrap();
        let held = open_protected(&path).unwrap();
        let error = rename_without_replace(&held, &occupied).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(80 | 183)));
        assert!(rename_without_replace(&held, &dir.join("missing").join("backup.bak")).is_err());
        drop(held);
        assert_eq!(fs::read(&path).unwrap(), OLD);
        assert_eq!(fs::read(&occupied).unwrap(), b"other backup");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directories_and_relative_paths_are_not_settings_files() {
        let (dir, path) = fixture();
        let error = save_guarded(Path::new("settings.json"), &Baseline::Missing, NEW).unwrap_err();
        assert_eq!(error.state.as_ref(), &SaveState::NoMutation);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), b"marker").unwrap();
        let error = save_guarded(&path, &Baseline::Existing(OLD.to_vec()), NEW).unwrap_err();
        assert!(matches!(
            error.state.as_ref(),
            SaveState::TemporaryRetained { .. }
        ));
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"marker");
        assert_eq!(fs::read(retained_temp(&error)).unwrap(), NEW);
        fs::remove_dir_all(dir).unwrap();
    }
}
