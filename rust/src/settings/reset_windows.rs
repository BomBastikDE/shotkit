//! UI-only reset protocol around the held-object recovery primitive.
//!
//! A fixed CREATE_NEW intent is flushed before any recovery mutation. Any
//! intent, orphan, unknown, unreadable or incomplete recovery artifact blocks
//! startup and saves, including when settings.json is missing. No automatic
//! resume, deletion or rollback is offered. On success the SAME intent handle
//! becomes a no-replace completion receipt bound to the exact backup bytes.
//!
//! Trusted stable parents/ancestors are required, as by recovery_windows. The
//! marker and renames protect against ordinary competing file opens, not an
//! administrator, older running ShotKit, directory/reparse namespace attacks,
//! memory-mapped writers or nonconforming drivers. Checks are not atomic with
//! later ordinary saves. Content flushes do NOT establish power-loss durability.
//! One failed attempt retains at most intent + temp + backup; startup then
//! refuses further attempts. Each explicitly approved success retains one
//! original backup and one small receipt. There is NO total retention bound.

use super::{recovery_windows, RecoveryEligibility, RejectedSettings, Settings};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};

const DELETE: u32 = 0x0001_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const RECEIPT_LIMIT: usize = 4096;
const PROTOCOL: &str = "ShotKit explicit rejected-v1 reset/1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    protocol: String,
    target_name: String,
    backup_name: String,
    source_len: usize,
    source_sha256: [u8; 32],
    source_identity: [u32; 3],
    replacement_json: String,
    replacement_sha256: [u8; 32],
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn pending_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".reset-pending");
    path.with_file_name(name)
}

fn receipt_path(backup: &Path) -> PathBuf {
    let mut name = backup.file_name().unwrap_or_default().to_os_string();
    name.push(".complete");
    backup.with_file_name(name)
}

fn target_name(path: &Path) -> io::Result<&str> {
    if !path.is_absolute() {
        return Err(invalid("reset requires an absolute settings path"));
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("reset requires a Unicode settings file name"))
}

fn read_regular_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    read_regular_snapshot(path, limit).map(|(bytes, _)| bytes)
}

fn read_regular_snapshot(path: &Path, limit: usize) -> io::Result<(Vec<u8>, [u32; 3])> {
    // Open the leaf itself; never follow a leaf reparse point to receipt/data.
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1 | 4)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid(
            "recovery artifact is not a regular non-reparse file",
        ));
    }
    let identity = file_identity(&file)?;
    let mut bytes = Vec::new();
    (&file).take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("recovery artifact exceeds its read limit"));
    }
    Ok((bytes, identity))
}

fn check_completion(target: &Path, backup: &Path) -> io::Result<()> {
    let bytes = read_regular_bounded(&receipt_path(backup), RECEIPT_LIMIT)?;
    let record: Completion = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("invalid recovery completion receipt"))?;
    let (source, identity) = read_regular_snapshot(backup, super::MAX_SETTINGS_BYTES)?;
    let backup_name = backup.file_name().and_then(|name| name.to_str());
    // A completion belongs to exactly this target and uniquely named original,
    // and verifies consistency of all original bytes and the held original's
    // file identity. This is not actor authentication or proof against forged
    // receipts in an untrusted namespace; the directory trust contract applies.
    if record.protocol != PROTOCOL
        || record.target_name != target_name(target)?
        || Some(record.backup_name.as_str()) != backup_name
        || record.source_len != source.len()
        || record.source_len > super::MAX_SETTINGS_BYTES
        || record.source_sha256 != sha256(&source)?
        || record.source_identity != identity
        || !super::reset_receipt::valid_v1_settings(record.replacement_json.as_bytes())
        || record.replacement_sha256 != sha256(record.replacement_json.as_bytes())?
        || super::recovery_version::validate_v1_source(&source).is_err()
        || super::reset_receipt::valid_v1_settings(&source)
    {
        return Err(invalid(
            "recovery completion does not match its rejected-v1 backup",
        ));
    }
    Ok(())
}

/// Must precede loading (especially Missing -> defaults) and each ordinary save.
/// This is a conservative inventory, not recovery or proof of lifetime locking.
/// Existing recovery primitive artifacts without receipts also require review.
pub fn ensure_clear(path: &Path) -> io::Result<()> {
    ensure_clear_except_owned_pending(path, false)
}

fn ensure_clear_except_owned_pending(path: &Path, own_pending: bool) -> io::Result<()> {
    let name = target_name(path)?;
    match fs::symlink_metadata(pending_path(path)) {
        Ok(_) if !own_pending => {
            return Err(invalid(
                "a settings reset is pending; manual inspection is required",
            ))
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound && !own_pending => {}
        Err(error) => return Err(error),
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("settings parent missing"))?;
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !own_pending => return Ok(()),
        Err(error) => return Err(error),
    };
    let prefix = format!("{name}.recovery-");
    let folded_prefix = prefix.to_ascii_lowercase();
    let pending_name = format!("{name}.reset-pending");
    for entry in entries {
        let entry = entry?;
        let entry_name = entry.file_name();
        let lossy_name = entry_name.to_string_lossy();
        if lossy_name.eq_ignore_ascii_case(&pending_name) {
            if own_pending && lossy_name == pending_name {
                continue;
            }
            return Err(invalid("pending or ambiguous settings reset intent"));
        }
        if !lossy_name.to_ascii_lowercase().starts_with(&folded_prefix) {
            continue;
        }
        let Some(text) = entry_name.to_str() else {
            return Err(invalid("unclassifiable settings recovery artifact name"));
        };
        if !text.starts_with(&prefix) {
            return Err(invalid("noncanonical settings recovery artifact name"));
        }
        if text.ends_with(".bak") {
            check_completion(path, &entry.path())?;
        } else if let Some(backup_name) = text.strip_suffix(".bak.complete") {
            // Validate the receipt and original as a pair in either inventory
            // order, including an orphan receipt after external backup removal.
            check_completion(path, &parent.join(format!("{backup_name}.bak")))?;
        } else {
            return Err(invalid("unfinished or unknown settings recovery artifact"));
        }
    }
    Ok(())
}

fn default_bytes() -> io::Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(&Settings::default()).map_err(io::Error::other)?;
    if !super::reset_receipt::valid_v1_settings(&bytes) {
        return Err(invalid(
            "current defaults require an updated reset receipt protocol",
        ));
    }
    Ok(bytes)
}

/// Requires explicit UI approval for this exact rejected snapshot. This API
/// refuses normal valid settings and unclear versions even if miscalled.
/// On ANY error its owned intent is retained; callers must disable saves. All
/// artifact names in returned errors/receipts are diagnostic, not cleanup rights.
pub fn reset_rejected(path: &Path, rejected: &RejectedSettings) -> io::Result<PathBuf> {
    reset_with(path, rejected, |path, bytes, defaults| {
        recovery_windows::recover_v1_retaining_backup(path, bytes, defaults)
            .map_err(io::Error::other)
    })
}

fn reset_with(
    path: &Path,
    rejected: &RejectedSettings,
    recover: impl FnOnce(
        &Path,
        &[u8],
        &Settings,
    ) -> io::Result<recovery_windows::RetainedRecoverySuccess>,
) -> io::Result<PathBuf> {
    reset_protocol(path, rejected, recover, |_| {})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Checkpoint {
    IntentCreated,
    IntentWritten,
    IntentFlushed,
    RecoveryReturned,
    ReceiptWritten,
    ReceiptFlushed,
    ReceiptPublished,
}

fn reset_protocol(
    path: &Path,
    rejected: &RejectedSettings,
    recover: impl FnOnce(
        &Path,
        &[u8],
        &Settings,
    ) -> io::Result<recovery_windows::RetainedRecoverySuccess>,
    checkpoint: impl Fn(Checkpoint),
) -> io::Result<PathBuf> {
    if rejected.recovery_eligibility != RecoveryEligibility::UnambiguousVersion1
        || !super::reset_receipt::admit_reset_source(
            rejected.bytes(),
            super::parse_settings(path, rejected.bytes()).is_err(),
        )
    {
        return Err(invalid(
            "only complete rejected unambiguous-v1 snapshots can be reset",
        ));
    }
    ensure_clear(path)?;
    let expected_hash = sha256(rejected.bytes())?;
    let replacement = default_bytes()?;
    let pending = pending_path(path);
    let mut intent = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(0x8000_0000 | 0x4000_0000 | DELETE)
        .share_mode(0)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&pending)?;
    checkpoint(Checkpoint::IntentCreated);
    // Even a partial/empty intent blocks startup. Its handle excludes ordinary
    // competing rename/delete operations until completion or process exit.
    intent.write_all(PROTOCOL.as_bytes())?;
    checkpoint(Checkpoint::IntentWritten);
    intent.sync_all()?;
    checkpoint(Checkpoint::IntentFlushed);
    // Re-inventory while owning intent: a previous reset could have completed
    // between initial inventory and CREATE_NEW. Never stack a failed attempt.
    ensure_clear_except_owned_pending(path, true)?;
    let recovered = recover(path, rejected.bytes(), &Settings::default())?;
    checkpoint(Checkpoint::RecoveryReturned);
    let backup = recovered.backup.last_known_path().to_owned();
    let source = read_regular_bounded(&backup, super::MAX_SETTINGS_BYTES)?;
    if source != rejected.bytes()
        || read_regular_bounded(path, super::MAX_SETTINGS_BYTES)? != replacement
    {
        return Err(invalid(
            "published reset or original backup changed before completion",
        ));
    }
    let record = Completion {
        protocol: PROTOCOL.to_owned(),
        target_name: target_name(path)?.to_owned(),
        backup_name: backup
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("backup name is not Unicode"))?
            .to_owned(),
        source_len: rejected.bytes().len(),
        source_sha256: expected_hash,
        source_identity: file_identity(recovered.backup.original_file())?,
        replacement_json: String::from_utf8(replacement.clone()).map_err(io::Error::other)?,
        replacement_sha256: sha256(&replacement)?,
    };
    let receipt = serde_json::to_vec(&record).map_err(io::Error::other)?;
    if receipt.len() > RECEIPT_LIMIT {
        return Err(invalid("completion receipt exceeds its size limit"));
    }
    intent.seek(SeekFrom::Start(0))?;
    intent.write_all(&receipt)?;
    intent.set_len(receipt.len() as u64)?;
    checkpoint(Checkpoint::ReceiptWritten);
    intent.sync_all()?;
    checkpoint(Checkpoint::ReceiptFlushed);
    // Never remove or reopen a marker by path. This is the same object claimed
    // above; no-replace preserves any conflicting completion file.
    recovery_windows::rename_without_replace(&intent, &receipt_path(&backup))?;
    checkpoint(Checkpoint::ReceiptPublished);
    drop(intent);
    // The successful original's capability remains held through verification
    // and receipt publication; ordinary drop retains the backup without delete.
    drop(recovered);
    ensure_clear(path)?;
    Ok(backup)
}

fn sha256(bytes: &[u8]) -> io::Result<[u8; 32]> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptOpenAlgorithmProvider(
            algorithm: *mut *mut c_void,
            id: *const u16,
            implementation: *const u16,
            flags: u32,
        ) -> i32;
        fn BCryptHash(
            algorithm: *mut c_void,
            secret: *const u8,
            secret_len: u32,
            input: *const u8,
            input_len: u32,
            output: *mut u8,
            output_len: u32,
        ) -> i32;
        fn BCryptCloseAlgorithmProvider(algorithm: *mut c_void, flags: u32) -> i32;
    }
    let len = u32::try_from(bytes.len()).map_err(|_| invalid("hash input too large"))?;
    let id: Vec<u16> = "SHA256\0".encode_utf16().collect();
    let mut provider = std::ptr::null_mut();
    // SAFETY: all buffers and out-pointers live for these synchronous calls;
    // successful Open owns exactly one provider, closed on every later path.
    let status =
        unsafe { BCryptOpenAlgorithmProvider(&mut provider, id.as_ptr(), std::ptr::null(), 0) };
    if status < 0 {
        return Err(io::Error::other("could not open Windows SHA-256 provider"));
    }
    let mut output = [0; 32];
    let hashed = unsafe {
        BCryptHash(
            provider,
            std::ptr::null(),
            0,
            bytes.as_ptr(),
            len,
            output.as_mut_ptr(),
            32,
        )
    };
    let closed = unsafe { BCryptCloseAlgorithmProvider(provider, 0) };
    if hashed < 0 || closed < 0 {
        return Err(io::Error::other("Windows SHA-256 failed"));
    }
    Ok(output)
}

fn file_identity(file: &File) -> io::Result<[u32; 3]> {
    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut c_void, information: *mut Information) -> i32;
    }
    let mut info = Information::default();
    // SAFETY: the file owns the live handle; info has the native C layout and
    // remains writable throughout this synchronous non-UI call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.index_high == 0 && info.index_low == 0 {
        return Err(invalid(
            "filesystem did not provide a usable recovery file identity",
        ));
    }
    Ok([info.volume, info.index_high, info.index_low])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    const ORIGINAL: &[u8] = b"{\n  \"version\": 1, \"open_editor\": \"rejected\"\n}\n";
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "shotkit-reset-test-{}-{nonce}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> PathBuf {
            self.0.join("settings.json")
        }
        fn rejected(&self) -> RejectedSettings {
            match super::super::load_outcome(self.path()).unwrap() {
                super::super::SettingsLoadOutcome::Rejected(value) => value,
                _ => panic!("fixture must be a rejected snapshot"),
            }
        }
        fn files(&self) -> usize {
            fs::read_dir(&self.0).unwrap().count()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn assert_exact_original_preserved(fixture: &Fixture, stage: &str) {
        let path = fixture.path();
        let mut originals = Vec::new();
        for entry in fs::read_dir(&fixture.0).unwrap() {
            let candidate = entry.unwrap().path();
            if candidate == path || candidate.extension().is_some_and(|ext| ext == "bak") {
                let bytes = fs::read(&candidate).unwrap();
                if bytes == ORIGINAL {
                    originals.push(candidate);
                }
            }
        }
        assert_eq!(
            originals.len(),
            1,
            "exactly one complete original must survive at {stage}"
        );
        if [
            "IntentCreated",
            "IntentWritten",
            "IntentFlushed",
            "Prepared",
        ]
        .contains(&stage)
        {
            assert_eq!(
                originals[0], path,
                "source must remain original before backup at {stage}"
            );
        } else {
            assert_ne!(
                originals[0], path,
                "moved original must survive in a backup at {stage}"
            );
            assert_eq!(originals[0].extension().unwrap(), "bak");
        }
    }

    #[test]
    fn success_preserves_exact_original_and_ordinary_saves_make_no_more_artifacts() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let backup = reset_rejected(&path, &fixture.rejected()).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), ORIGINAL);
        assert_eq!(super::super::load(&path).unwrap(), Settings::default());
        assert!(!pending_path(&path).exists());
        ensure_clear(&path).unwrap();
        assert_eq!(fixture.files(), 3); // settings + one original + one receipt
        for open_editor in [false, true, false] {
            ensure_clear(&path).unwrap();
            super::super::save_atomic(
                &path,
                &Settings {
                    open_editor,
                    ..Settings::default()
                },
            )
            .unwrap();
            ensure_clear(&path).unwrap();
            assert_eq!(fixture.files(), 3);
        }
        assert_eq!(fs::read(backup).unwrap(), ORIGINAL);
    }

    #[test]
    fn changed_source_is_preserved_and_blocks_another_reset() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let rejected = fixture.rejected();
        fs::write(&path, b"{\"version\":2}").unwrap();
        assert!(reset_rejected(&path, &rejected).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{\"version\":2}");
        assert!(pending_path(&path).exists());
        let count = fixture.files();
        assert!(reset_rejected(&path, &rejected).is_err());
        assert_eq!(fixture.files(), count); // no growing retry artifacts
    }

    #[test]
    fn pending_marker_blocks_even_with_missing_target_or_empty_marker() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(pending_path(&path), b"").unwrap();
        assert!(ensure_clear(&path).is_err());
        assert!(!path.exists());
        fs::remove_file(pending_path(&path)).unwrap();
        fs::create_dir(pending_path(&path)).unwrap();
        assert!(ensure_clear(&path).is_err());
    }

    #[test]
    fn completion_requires_original_identity_bytes_and_both_artifacts() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let backup = reset_rejected(&path, &fixture.rejected()).unwrap();
        let receipt = receipt_path(&backup);
        let good_receipt = fs::read(&receipt).unwrap();
        for field in [
            "protocol",
            "target_name",
            "backup_name",
            "source_len",
            "source_sha256",
            "source_identity",
            "replacement_json",
            "replacement_sha256",
            "extra",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&good_receipt).unwrap();
            match field {
                "protocol" | "target_name" | "backup_name" | "replacement_json" => {
                    value[field] = "foreign".into()
                }
                "source_len" => value[field] = (ORIGINAL.len() + 1).into(),
                "source_sha256" | "source_identity" | "replacement_sha256" => {
                    value[field][0] = (value[field][0].as_u64().unwrap() ^ 1).into()
                }
                _ => value[field] = serde_json::Value::Null,
            }
            fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(ensure_clear(&path).is_err(), "must reject changed {field}");
        }
        fs::write(&receipt, &good_receipt).unwrap();
        fs::write(&backup, b"{\"version\":1,\"foreign\":true}").unwrap();
        assert!(ensure_clear(&path).is_err());
        fs::write(&backup, ORIGINAL).unwrap();
        ensure_clear(&path).unwrap();
        // Equal bytes in a different file do not establish original ownership.
        let moved_original = fixture.0.join("retained-original-object");
        fs::rename(&backup, &moved_original).unwrap();
        fs::write(&backup, ORIGINAL).unwrap();
        assert!(ensure_clear(&path).is_err());
        fs::remove_file(&backup).unwrap();
        assert!(ensure_clear(&path).is_err()); // orphan receipt
        fs::rename(&moved_original, &backup).unwrap();
        ensure_clear(&path).unwrap();
        fs::remove_file(&receipt).unwrap();
        assert!(ensure_clear(&path).is_err()); // lost completion evidence
        assert_eq!(fs::read(&backup).unwrap(), ORIGINAL);
    }

    #[test]
    fn missing_parent_is_first_run_but_ineligible_or_oversized_input_never_creates_intent() {
        let fixture = Fixture::new();
        let path = fixture.0.join("missing-parent").join("settings.json");
        ensure_clear(&path).unwrap();
        assert!(!path.parent().unwrap().exists());
        let path = fixture.path();
        for original in [
            b"{\"version\":2}".as_slice(),
            b"{\"version\":1,\"version\":1}",
            b"{broken",
        ] {
            fs::write(&path, original).unwrap();
            assert!(reset_rejected(&path, &fixture.rejected()).is_err());
            assert!(!pending_path(&path).exists());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        fs::write(&path, vec![b' '; super::super::MAX_SETTINGS_BYTES + 1]).unwrap();
        assert!(super::super::load_outcome(&path).is_err());
        assert!(!pending_path(&path).exists());
    }

    #[test]
    fn explicit_successful_resets_do_not_impose_a_lifetime_quota_or_delete_old_originals() {
        let fixture = Fixture::new();
        let path = fixture.path();
        let mut backups = Vec::new();
        for _ in 0..3 {
            fs::write(&path, ORIGINAL).unwrap();
            backups.push(reset_rejected(&path, &fixture.rejected()).unwrap());
            ensure_clear(&path).unwrap();
        }
        assert_eq!(fixture.files(), 7);
        for backup in backups {
            assert_eq!(fs::read(backup).unwrap(), ORIGINAL);
        }
    }

    #[test]
    fn historical_replacement_receipt_uses_its_own_frozen_v1_bytes() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let backup = reset_rejected(&path, &fixture.rejected()).unwrap();
        let receipt = receipt_path(&backup);
        let mut record: Completion = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
        // Model a historical v1 default differing from this build. Receipt
        // consistency is checked against the recorded bytes, not today's
        // defaults, evolving Settings schema, or the current target contents.
        record.replacement_json = r#"{"version":1,"open_editor":false}"#.to_owned();
        record.replacement_sha256 = sha256(record.replacement_json.as_bytes()).unwrap();
        fs::write(&receipt, serde_json::to_vec(&record).unwrap()).unwrap();
        ensure_clear(&path).unwrap();
        assert_eq!(super::super::load(&path).unwrap(), Settings::default());
    }

    #[test]
    fn foreign_or_ambiguous_recovery_names_cannot_become_first_run() {
        for name in [
            "settings.json.recovery-foreign.tmp",
            "SETTINGS.JSON.RECOVERY-foreign.bak",
            "settings.json.recovery-foreign.unknown",
        ] {
            let fixture = Fixture::new();
            let path = fixture.path();
            fs::write(fixture.0.join(name), b"foreign").unwrap();
            assert!(ensure_clear(&path).is_err());
            assert!(!path.exists());
            assert_eq!(fixture.files(), 1);
        }
    }

    #[test]
    fn native_sha256_matches_a_known_vector() {
        assert_eq!(
            sha256(b"abc").unwrap(),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn foreign_target_in_publication_gap_is_not_overwritten_or_adopted() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let rejected = fixture.rejected();
        let error = reset_with(&path, &rejected, |path, bytes, defaults| {
            recovery_windows::recover_with(
                path,
                bytes,
                defaults,
                |file, bytes| {
                    file.write_all(bytes)?;
                    file.sync_all()
                },
                |_, _| {
                    fs::write(path, b"foreign target").unwrap();
                },
            )
            .map_err(io::Error::other)
        });
        assert!(error.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign target");
        assert!(ensure_clear(&path).is_err());
        assert_eq!(fixture.files(), 4); // foreign target + pending + temp + backup
        let backup = fs::read_dir(&fixture.0)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|ext| ext == "bak"))
            .unwrap();
        assert_eq!(fs::read(backup).unwrap(), ORIGINAL);
    }

    #[test]
    fn conflicting_completion_is_preserved_and_owned_pending_remains() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let result = reset_protocol(
            &path,
            &fixture.rejected(),
            |path, bytes, defaults| {
                recovery_windows::recover_v1_retaining_backup(path, bytes, defaults)
                    .map_err(io::Error::other)
            },
            |point| {
                if point == Checkpoint::ReceiptFlushed {
                    let backup = fs::read_dir(&fixture.0)
                        .unwrap()
                        .map(|e| e.unwrap().path())
                        .find(|p| p.extension().is_some_and(|ext| ext == "bak"))
                        .unwrap();
                    fs::write(receipt_path(&backup), b"foreign receipt").unwrap();
                }
            },
        );
        assert!(result.is_err());
        assert!(pending_path(&path).exists());
        assert!(ensure_clear(&path).is_err());
        let receipt = fs::read_dir(&fixture.0)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|ext| ext == "complete"))
            .unwrap();
        assert_eq!(fs::read(receipt).unwrap(), b"foreign receipt");
    }

    #[test]
    fn unreadable_completion_blocks_and_no_other_file_is_changed() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, ORIGINAL).unwrap();
        let backup = reset_rejected(&path, &fixture.rejected()).unwrap();
        let receipt = receipt_path(&backup);
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&receipt)
            .unwrap();
        assert!(ensure_clear(&path).is_err());
        assert_eq!(fs::read(&backup).unwrap(), ORIGINAL);
        drop(held);
        ensure_clear(&path).unwrap();
    }

    // The parent kills this process via exit(77) at each actual protocol
    // boundary. Rust stack destructors are skipped; Windows closes the handles.
    // This exercises process termination, NOT power-loss durability.
    #[test]
    fn process_exit_child() {
        let Some(dir) = std::env::var_os("SHOTKIT_RESET_TEST_CHILD_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        assert!(dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("shotkit-reset-test-"));
        let stage = std::env::var("SHOTKIT_RESET_TEST_CHILD_STAGE").unwrap();
        let path = dir.join("settings.json");
        let rejected = match super::super::load_outcome(&path).unwrap() {
            super::super::SettingsLoadOutcome::Rejected(value) => value,
            _ => panic!("child must receive rejected fixture"),
        };
        reset_protocol(
            &path,
            &rejected,
            |path, bytes, defaults| {
                recovery_windows::recover_with(
                    path,
                    bytes,
                    defaults,
                    |file, bytes| {
                        file.write_all(bytes)?;
                        file.sync_all()?;
                        if stage == "Prepared" {
                            std::process::exit(77);
                        }
                        Ok(())
                    },
                    |_, _| {
                        if stage == "BackedUp" {
                            std::process::exit(77);
                        }
                    },
                )
                .map_err(io::Error::other)
            },
            |point| {
                if stage == format!("{point:?}") {
                    std::process::exit(77);
                }
            },
        )
        .unwrap();
        panic!("unrecognized interruption stage");
    }

    #[test]
    fn abrupt_exit_at_every_transition_blocks_until_verified_completion() {
        for stage in [
            "IntentCreated",
            "IntentWritten",
            "IntentFlushed",
            "Prepared",
            "BackedUp",
            "RecoveryReturned",
            "ReceiptWritten",
            "ReceiptFlushed",
            "ReceiptPublished",
        ] {
            let fixture = Fixture::new();
            let path = fixture.path();
            fs::write(&path, ORIGINAL).unwrap();
            let rejected = fixture.rejected();
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "settings::reset_windows::tests::process_exit_child",
                    "--nocapture",
                ])
                .env("SHOTKIT_RESET_TEST_CHILD_DIR", &fixture.0)
                .env("SHOTKIT_RESET_TEST_CHILD_STAGE", stage)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(77), "stage {stage}");
            assert_exact_original_preserved(&fixture, stage);
            if stage == "ReceiptPublished" {
                ensure_clear(&path).unwrap();
                assert_eq!(super::super::load(&path).unwrap(), Settings::default());
                continue;
            }
            assert!(ensure_clear(&path).is_err(), "must block at {stage}");
            let before = fixture.files();
            assert!(reset_rejected(&path, &rejected).is_err());
            assert_eq!(fixture.files(), before);
            assert_exact_original_preserved(&fixture, stage);
            if [
                "Prepared",
                "BackedUp",
                "RecoveryReturned",
                "ReceiptWritten",
                "ReceiptFlushed",
            ]
            .contains(&stage)
            {
                // Even loss of the intent cannot turn orphan temp/backup into
                // a writable first-run load or an automatically accepted reset.
                fs::remove_file(pending_path(&path)).unwrap();
                assert!(ensure_clear(&path).is_err(), "orphan must block at {stage}");
                assert_exact_original_preserved(&fixture, stage);
            }
        }
    }
}
