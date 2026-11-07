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
//! - Before moving the verified source, an exclusive sibling temp is created
//!   READ | WRITE | DELETE with CreateNew and sharing disabled. The complete
//!   replacement is written and synced while both source and temp stay held.
//!   Preparation failure leaves the original name untouched and retains the temp.
//! - Both renames use those held objects, FileRenameInfo, and no replacement.
//!   Publication renames the prepared object; it never creates or writes the
//!   destination by path. No path is unlinked or rolled back; publication is
//!   never automatically retried. Backup naming alone skips up to 128 collisions.
//!   An opt-in API returns the original backup handle after success; only the
//!   explicit user-consent deletion method can then request native disposition.
//! - These are two renames, not a transaction or compare-and-swap. There is a
//!   destination-name gap; a competing creator wins without being overwritten.
//!   Errors report the prepared path and backup/attempted backup where relevant.
//!   Native rename errors require inspection of both endpoints; no endpoint is
//!   asserted absent. Artifact paths are last-known diagnostics, not authority.
//! - File sync covers prepared contents, not directory-entry crash durability.
//!   Process/power loss between renames can leave only backup and temp names.
//!   No automatic restart recovery, retention bound, or directory flush is
//!   provided. Protection ends when handles close; namespace trust remains required.
//!
//! Win32 references:
//! <https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew>
//! <https://learn.microsoft.com/windows/win32/api/winbase/ns-winbase-file_rename_info>

use super::{guarded_save_windows::HeldBackup, Settings, CURRENT_VERSION};
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
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Last completed stage, including material left behind on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryState {
    /// The original was not moved. Consult `prepared_path`: allocating backup
    /// names can fail after a complete temp was created and retained.
    Unchanged,
    /// A temp was created, but writing/syncing it failed. The source was not
    /// moved; the possibly partial temp is retained at `prepared_path`.
    PreparationIncomplete,
    /// Native rename failed; no publication was attempted. Inspect the source,
    /// attempted backup, and prepared_path. Neither rename endpoint is asserted
    /// absent; no automatic retry is performed.
    BackupMoveFailed { attempted_backup: PathBuf },
    /// Backup rename succeeded; prepared-object publication failed. Inspect
    /// the temp and destination as well as the backup. A competitor is retained.
    BackedUp { backup_path: PathBuf },
    /// Legacy state retained for callers. Prepared-object recovery never emits
    /// this state: no destination write occurs after moving the source.
    ReplacementIncomplete { backup_path: PathBuf },
}

#[derive(Debug)]
pub struct RecoveryError {
    pub state: RecoveryState,
    /// Last-known or attempted temp name, including partial preparation and
    /// CREATE_NEW collision. A collided name belongs to another actor; this
    /// diagnostic grants no ownership or cleanup permission. After a failed
    /// publication rename inspect this AND the requested target path.
    /// None means preparation did not reach temporary-name allocation.
    pub prepared_path: Option<PathBuf>,
    pub operation: &'static str,
    pub source: io::Error,
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "settings recovery {} failed ({:?}, prepared={:?}): {}",
            self.operation, self.state, self.prepared_path, self.source
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

/// Successful recovery retaining the moved original object's handle.
#[derive(Debug)]
pub struct RetainedRecoverySuccess {
    pub backup: HeldBackup,
}

fn failure(state: RecoveryState, operation: &'static str, source: io::Error) -> RecoveryError {
    RecoveryError {
        state,
        prepared_path: None,
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
    let recovered = recover_v1_retaining_backup(path, expected_bytes, replacement)?;
    Ok(RecoverySuccess {
        backup_path: recovered.backup.last_known_path().to_owned(),
    })
}

/// Opt-in recovery retaining the original backup handle across publication and
/// return. Explicit cleanup is available only after success and caller-obtained
/// user consent. Failure artifacts remain retained; no path-based cleanup exists.
pub fn recover_v1_retaining_backup(
    path: &Path,
    expected_bytes: &[u8],
    replacement: &Settings,
) -> Result<RetainedRecoverySuccess, RecoveryError> {
    recover_with(path, expected_bytes, replacement, flush_prepared, |_, _| {})
}

fn recover_with(
    path: &Path,
    expected_bytes: &[u8],
    replacement: &Settings,
    prepare: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    before_publication: impl FnOnce(&Path, &Path),
) -> Result<RetainedRecoverySuccess, RecoveryError> {
    recover_with_temp_name(
        path,
        expected_bytes,
        replacement,
        prepare,
        before_publication,
        prepared_name,
    )
}

// Private callbacks permit deterministic fault/collision tests. Production
// always uses flush_prepared, a no-op interleave, and generated sibling names.
fn recover_with_temp_name(
    path: &Path,
    expected_bytes: &[u8],
    replacement: &Settings,
    prepare: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    before_publication: impl FnOnce(&Path, &Path),
    allocate_name: impl FnOnce(&Path) -> PathBuf,
) -> Result<RetainedRecoverySuccess, RecoveryError> {
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
    let temp_path = allocate_name(path);
    let with_prepared = |mut error: RecoveryError| {
        error.prepared_path = Some(temp_path.clone());
        error
    };
    // CREATE_NEW and no sharing: a pre-existing object is never opened or
    // truncated. Once created, the same object is held through publication.
    let mut prepared = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .share_mode(0)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&temp_path)
        .map_err(|error| with_prepared(unchanged("create prepared replacement", error)))?;
    prepare(&mut prepared, &replacement_bytes).map_err(|source| {
        with_prepared(failure(
            RecoveryState::PreparationIncomplete,
            "write/sync prepared replacement",
            source,
        ))
    })?;
    let backup_path = move_to_unique_backup(&held, path).map_err(with_prepared)?;
    before_publication(&temp_path, &backup_path);
    rename_without_replace(&prepared, path).map_err(|source| {
        with_prepared(failure(
            RecoveryState::BackedUp {
                backup_path: backup_path.clone(),
            },
            "publish prepared replacement",
            source,
        ))
    })?;
    drop(prepared);
    // Retain original object protection even after its name becomes the backup.
    Ok(RetainedRecoverySuccess {
        backup: HeldBackup::from_renamed_original(held, backup_path),
    })
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
    super::recovery_version::validate_v1_source(&actual)
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

fn prepared_name(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .expect("validated file name")
        .to_os_string();
    name.push(format!(
        ".recovery-{}-{timestamp}-{sequence}.tmp",
        std::process::id()
    ));
    path.with_file_name(name)
}

fn flush_prepared(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
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
    // FileNameLength excludes it. File owns a valid synchronous DELETE-capable
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
    fn retained_recovery_backup_survives_drop_and_blocks_foreign_deletion() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let recovered = recover_v1_retaining_backup(&path, original, &Settings::default()).unwrap();
        let backup_path = recovered.backup.last_known_path().to_owned();
        assert_eq!(fs::read(&backup_path).unwrap(), original);
        let blocked = OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(7)
            .open(&backup_path)
            .unwrap_err();
        assert_eq!(blocked.raw_os_error(), Some(32));
        drop(recovered);
        assert_eq!(fs::read(&backup_path).unwrap(), original);
        assert_eq!(super::super::load(&path).unwrap(), Settings::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retained_recovery_backup_can_be_explicitly_deleted_after_publication() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let recovered = recover_v1_retaining_backup(&path, original, &Settings::default()).unwrap();
        let backup_path = recovered.backup.last_known_path().to_owned();
        recovered.backup.delete_after_user_consent().unwrap();
        assert!(!backup_path.exists());
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
    fn temporary_name_collision_preserves_original_and_foreign_temp() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let occupied = dir.join("occupied.tmp");
        fs::write(&occupied, b"foreign temporary bytes").unwrap();
        let error = recover_with_temp_name(
            &path,
            original,
            &Settings::default(),
            |_, _| panic!("must not prepare an existing temporary file"),
            |_, _| panic!("must not move source or publish after temp collision"),
            |_| occupied.clone(),
        )
        .unwrap_err();
        assert_eq!(error.state, RecoveryState::Unchanged);
        assert_eq!(error.operation, "create prepared replacement");
        assert!(matches!(error.source.raw_os_error(), Some(80 | 183)));
        assert_eq!(error.prepared_path.as_deref(), Some(occupied.as_path()));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&occupied).unwrap(), b"foreign temporary bytes");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preparation_failure_retains_partial_temp_and_untouched_original() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let error = recover_with(
            &path,
            original,
            &Settings::default(),
            |file, _| {
                assert_eq!(fs::read(&path).unwrap(), original);
                file.write_all(b"partial")?;
                Err(io::Error::other("injected preparation failure"))
            },
            |_, _| panic!("preparation failure must not reach publication"),
        )
        .unwrap_err();
        assert_eq!(error.state, RecoveryState::PreparationIncomplete);
        assert_eq!(error.operation, "write/sync prepared replacement");
        assert_eq!(
            fs::read(error.prepared_path.as_ref().unwrap()).unwrap(),
            b"partial"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sync_failure_after_complete_write_does_not_move_original() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let error = recover_with(
            &path,
            original,
            &Settings::default(),
            |file, bytes| {
                file.write_all(bytes)?;
                Err(io::Error::other("injected sync failure"))
            },
            |_, _| panic!("sync failure must not move source or publish"),
        )
        .unwrap_err();
        assert_eq!(error.state, RecoveryState::PreparationIncomplete);
        assert_eq!(fs::read(&path).unwrap(), original);
        let bytes = fs::read(error.prepared_path.as_ref().unwrap()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Settings>(&bytes).unwrap(),
            Settings::default()
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recreated_destination_keeps_complete_prepared_object_and_backup() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let mut observed_temp = None;
        let error = recover_with(
            &path,
            original,
            &Settings::default(),
            flush_prepared,
            |temp, backup| {
                assert!(!path.exists());
                assert_eq!(fs::read(backup).unwrap(), original);
                // Temp stays exclusive even after the old object has moved.
                for access in [GENERIC_READ, GENERIC_WRITE, DELETE] {
                    let blocked = OpenOptions::new()
                        .access_mode(access)
                        .share_mode(7)
                        .open(temp)
                        .unwrap_err();
                    assert_eq!(blocked.raw_os_error(), Some(32));
                }
                observed_temp = Some(temp.to_owned());
                fs::write(&path, b"foreign settings").unwrap();
            },
        )
        .unwrap_err();
        assert_eq!(error.operation, "publish prepared replacement");
        assert!(matches!(error.source.raw_os_error(), Some(80 | 183)));
        let RecoveryState::BackedUp { backup_path } = &error.state else {
            panic!("expected preserved backup state");
        };
        assert_eq!(error.prepared_path, observed_temp);
        let prepared = fs::read(error.prepared_path.as_ref().unwrap()).unwrap();
        assert_eq!(
            prepared,
            serde_json::to_vec_pretty(&Settings::default()).unwrap()
        );
        assert_eq!(fs::read(&path).unwrap(), b"foreign settings");
        assert_eq!(fs::read(backup_path).unwrap(), original);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepared_success_holds_both_objects_until_publication() {
        let (dir, path) = fixture();
        let original = br#"{"version":1,"scroll_direction":"unknown"}"#;
        fs::write(&path, original).unwrap();
        let mut observed_temp = None;
        let recovered = recover_with(
            &path,
            original,
            &Settings::default(),
            |file, bytes| {
                // A complete flush occurs while the verified original is still
                // at its original name and held against competing writes/deletes.
                assert_eq!(fs::read(&path).unwrap(), original);
                flush_prepared(file, bytes)
            },
            |temp, backup| {
                observed_temp = Some(temp.to_owned());
                for name in [temp, backup] {
                    for access in [GENERIC_WRITE, DELETE] {
                        let blocked = OpenOptions::new()
                            .access_mode(access)
                            .share_mode(7)
                            .open(name)
                            .unwrap_err();
                        assert_eq!(blocked.raw_os_error(), Some(32));
                    }
                }
            },
        )
        .unwrap();
        assert!(!observed_temp.unwrap().exists());
        let backup_path = recovered.backup.last_known_path().to_owned();
        drop(recovered);
        assert_eq!(fs::read(backup_path).unwrap(), original);
        assert_eq!(super::super::load(&path).unwrap(), Settings::default());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
