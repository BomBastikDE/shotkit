//! Narrow Windows recovery primitive; ordinary saves need separate protection.
//!
//! FILE SAFETY CONTRACT:
//! - The caller supplies exact previously read bytes for explicitly approved
//!   recovery. Only an unambiguous JSON version 1 is eligible.
//! - The parent and its ancestors must be trusted, stable directories throughout
//!   the call. This is not protection against directory/reparse namespace
//!   attacks, privileged filesystem access, or nonconforming drivers.
//! - The existing source is opened READ | DELETE, sharing READ only. Its bytes
//!   and version are checked while the same handle stays open through backup
//!   rename and publication. Source leaf reparse points are refused.
//! - The backup rename uses that handle, FileRenameInfo, and no replacement.
//!   Publication uses CreateNew only. No path is unlinked or rolled back.
//! - These are two operations, not a transaction. On failure the backup and any
//!   new/partial settings or competing file remain. Success describes this
//!   operation, not future ownership or crash durability of directory entries.
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
const DELETE: u32 = 0x0001_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_RENAME_INFO: i32 = 3;
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Last completed stage, including material left behind on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryState {
    /// No mutation completed: preparation failed or all no-replace rename
    /// attempts collided with existing files.
    Unchanged,
    /// Native rename failed; no publication was attempted. Inspect both paths
    /// before retrying. This deliberately does not assert either path absent.
    BackupMoveFailed { attempted_backup: PathBuf },
    /// Rename succeeded; CreateNew did not succeed. A competing destination
    /// may exist and must be left alone.
    BackedUp { backup_path: PathBuf },
    /// CreateNew succeeded, but writing or syncing failed. The new file may
    /// be empty/partial. Both it and the backup are deliberately retained.
    ReplacementIncomplete { backup_path: PathBuf },
}

#[derive(Debug)]
pub struct RecoveryError {
    pub state: RecoveryState,
    pub operation: &'static str,
    pub source: io::Error,
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "settings recovery {} failed ({:?}): {}",
            self.operation, self.state, self.source
        )
    }
}

impl Error for RecoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
pub struct RecoverySuccess {
    pub backup_path: PathBuf,
}

fn failure(state: RecoveryState, operation: &'static str, source: io::Error) -> RecoveryError {
    RecoveryError {
        state,
        operation,
        source,
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Back up expected v1 bytes and publish validated replacement settings.
///
/// Requires an explicit caller decision and the module's file safety contract.
/// Never creates parent directories, recovers unclear/other versions, replaces
/// an existing destination, removes files, or retries partial recovery.
pub fn recover_v1(
    path: &Path,
    expected_bytes: &[u8],
    replacement: &Settings,
) -> Result<RecoverySuccess, RecoveryError> {
    let unchanged = |operation, source| failure(RecoveryState::Unchanged, operation, source);
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(unchanged(
            "validate path",
            invalid("an absolute file path is required"),
        ));
    }
    // Future CURRENT_VERSION changes must not silently broaden eligibility.
    if CURRENT_VERSION != 1 || replacement.version != 1 {
        return Err(unchanged(
            "validate replacement",
            invalid("only version 1 is supported"),
        ));
    }
    let replacement_bytes = serde_json::to_vec_pretty(replacement).map_err(|error| {
        unchanged(
            "serialize replacement",
            io::Error::new(io::ErrorKind::InvalidData, error),
        )
    })?;
    let _: Settings = serde_json::from_slice(&replacement_bytes).map_err(|error| {
        unchanged(
            "validate replacement",
            io::Error::new(io::ErrorKind::InvalidData, error),
        )
    })?;

    let mut held =
        open_protected(path).map_err(|error| unchanged("open expected source", error))?;
    verify_expected(&mut held, expected_bytes)
        .map_err(|error| unchanged("verify expected source", error))?;
    let backup_path = move_to_unique_backup(&held, path)?;
    publish_new(path, &backup_path, &replacement_bytes)?;
    // Retain original object protection even after its name becomes the backup.
    drop(held);
    Ok(RecoverySuccess { backup_path })
}

fn open_protected(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid(
            "source must be a regular file, not a reparse point",
        ));
    }
    Ok(file)
}

fn verify_expected(held: &mut File, expected: &[u8]) -> io::Result<()> {
    // Bound reads to expected length + 1; a changed large source cannot cause
    // unbounded allocation. The freshly opened handle starts at byte zero.
    let limit = u64::try_from(expected.len())
        .ok()
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| invalid("expected bytes are too large"))?;
    let mut actual = Vec::new();
    held.take(limit).read_to_end(&mut actual)?;
    if actual != expected {
        return Err(invalid("source bytes changed since the recovery decision"));
    }
    #[derive(Deserialize)]
    struct VersionHeader {
        version: u32,
    }
    // Reject absent, duplicate, noninteger and malformed headers. A textual
    // occurrence of version 1 is never sufficient authorization.
    if actual
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        != Some(b'{')
    {
        return Err(invalid("source must be a JSON object with a version field"));
    }
    let header: VersionHeader = serde_json::from_slice(&actual)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if header.version != 1 {
        return Err(invalid("source is not unambiguously version 1"));
    }
    Ok(())
}

fn move_to_unique_backup(held: &File, path: &Path) -> Result<PathBuf, RecoveryError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..128 {
        let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut name = path
            .file_name()
            .expect("validated file name")
            .to_os_string();
        name.push(format!(
            ".recovery-{}-{timestamp}-{sequence}.bak",
            std::process::id()
        ));
        let candidate = path.with_file_name(name);
        match rename_without_replace(held, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if matches!(error.raw_os_error(), Some(80 | 183)) => continue,
            Err(source) => {
                return Err(failure(
                    RecoveryState::BackupMoveFailed {
                        attempted_backup: candidate,
                    },
                    "move source to backup",
                    source,
                ))
            }
        }
    }
    Err(failure(
        RecoveryState::Unchanged,
        "allocate backup name",
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "all backup names collided; existing files were preserved",
        ),
    ))
}

fn publish_new(path: &Path, backup_path: &Path, bytes: &[u8]) -> Result<(), RecoveryError> {
    publish_new_with(path, backup_path, |file| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn publish_new_with(
    path: &Path,
    backup_path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), RecoveryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|source| {
            failure(
                RecoveryState::BackedUp {
                    backup_path: backup_path.to_owned(),
                },
                "create replacement",
                source,
            )
        })?;
    write(&mut file).map_err(|source| {
        failure(
            RecoveryState::ReplacementIncomplete {
                backup_path: backup_path.to_owned(),
            },
            "write/sync replacement",
            source,
        )
    })
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
    if name.contains(&0) {
        return Err(invalid("backup path contains a NUL"));
    }
    // FILE_RENAME_INFO: BOOLEAN/flags union, aligned HANDLE, DWORD, WCHAR[].
    // Zero initialization means ReplaceIfExists=FALSE and RootDirectory=NULL.
    let root_offset = size_of::<usize>();
    let length_offset = root_offset + size_of::<usize>();
    let name_offset = length_offset + size_of::<u32>();
    let name_bytes = name
        .len()
        .checked_mul(2)
        .ok_or_else(|| invalid("backup path is too long"))?;
    let size = name_offset
        .checked_add(name_bytes)
        .and_then(|size| size.checked_add(2))
        .ok_or_else(|| invalid("backup path is too long"))?;
    let native_size = u32::try_from(size).map_err(|_| invalid("backup path is too long"))?;
    let native_name_bytes =
        u32::try_from(name_bytes).map_err(|_| invalid("backup path is too long"))?;
    // usize storage supplies HANDLE alignment on both Windows pointer widths.
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    let buffer = storage.as_mut_ptr().cast::<u8>();
    // SAFETY: allocation is zeroed, correctly aligned, and at least size bytes.
    // Both checked payload writes stay inside it. The UTF-16 NUL remains zero;
    // FileNameLength excludes it. File owns a valid synchronous READ|DELETE
    // handle during the call. Pointers do not escape. Class 3 selects the
    // documented FILE_RENAME_INFO layout.
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

    fn fixture() -> (PathBuf, PathBuf) {
        let id = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "shotkit-recovery-{}-{stamp}-{id}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("settings.json");
        (dir, path)
    }

    #[test]
    fn recovery_keeps_exact_original_and_publishes_valid_settings() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let recovered = recover_v1(&path, original, &Settings::default()).unwrap();
        assert_eq!(fs::read(&recovered.backup_path).unwrap(), original);
        assert_eq!(super::super::load(&path).unwrap(), Settings::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn changed_bytes_are_not_moved() {
        let (dir, path) = fixture();
        let actual = br#"{"version":1,"unknown":true}"#;
        fs::write(&path, actual).unwrap();
        let error = recover_v1(&path, br#"{"version":1}"#, &Settings::default()).unwrap_err();
        assert_eq!(error.state, RecoveryState::Unchanged);
        assert_eq!(fs::read(&path).unwrap(), actual);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unclear_and_other_versions_are_not_moved() {
        for original in [
            &br#"[1]"#[..],
            &br#"{"version":2,"future":true}"#[..],
            &br#"{"version":0}"#[..],
            &br#"{"unknown":true}"#[..],
            &br#"{"version":1,"version":2}"#[..],
            &br#"{"version":1,"version":1}"#[..],
            &br#"{"version":"1"}"#[..],
            &br#"{"version":1,"broken":"#[..],
        ] {
            let (dir, path) = fixture();
            fs::write(&path, original).unwrap();
            let error = recover_v1(&path, original, &Settings::default()).unwrap_err();
            assert_eq!(error.state, RecoveryState::Unchanged);
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn protected_handle_blocks_writers_and_deleters_and_backup_collision() {
        let (dir, path) = fixture();
        let original = br#"{"version":1}"#;
        fs::write(&path, original).unwrap();
        let backup = dir.join("existing.bak");
        fs::write(&backup, b"other backup").unwrap();
        let mut held = open_protected(&path).unwrap();
        verify_expected(&mut held, original).unwrap();
        for access in [0x4000_0000, DELETE] {
            let error = OpenOptions::new()
                .access_mode(access)
                .share_mode(7)
                .open(&path)
                .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(32));
        }
        let error = rename_without_replace(&held, &backup).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(80 | 183)));
        drop(held);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&backup).unwrap(), b"other backup");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn partial_write_failure_preserves_backup_and_partial_file() {
        let (dir, path) = fixture();
        let original = br#"{"version":1}"#;
        fs::write(&path, original).unwrap();
        let held = open_protected(&path).unwrap();
        let backup = move_to_unique_backup(&held, &path).unwrap();
        let error = publish_new_with(&path, &backup, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        })
        .unwrap_err();
        assert_eq!(
            error.state,
            RecoveryState::ReplacementIncomplete {
                backup_path: backup.clone()
            }
        );
        drop(held);
        assert_eq!(fs::read(&path).unwrap(), b"partial");
        assert_eq!(fs::read(&backup).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recreated_destination_and_backup_survive_failed_publication() {
        let (dir, path) = fixture();
        let original = br#"{"version":1}"#;
        fs::write(&path, original).unwrap();
        let held = open_protected(&path).unwrap();
        let backup = move_to_unique_backup(&held, &path).unwrap();
        fs::write(&path, b"foreign settings").unwrap();
        let error = publish_new(&path, &backup, b"replacement").unwrap_err();
        assert_eq!(
            error.state,
            RecoveryState::BackedUp {
                backup_path: backup.clone()
            }
        );
        drop(held);
        assert_eq!(fs::read(&path).unwrap(), b"foreign settings");
        assert_eq!(fs::read(&backup).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }
}
