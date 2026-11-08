//! Versioned, persistent application settings.
//!
//! Call [`load_or_default`] at startup to use defaults when no settings file
//! exists while still surfacing malformed, unreadable, or future-version files.
//! [`save_atomic`] writes a sibling temporary file and only replaces the
//! destination after the complete JSON has been flushed.

use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
#[cfg(not(windows))]
use std::{
    fs::OpenOptions,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

/// Pure fail-closed planning; not yet connected to filesystem recovery.
pub mod transaction_model;

mod recovery_version;
#[cfg(any(windows, test))]
mod reset_receipt;

#[cfg(windows)]
pub mod guarded_save_windows;
#[cfg(windows)]
pub mod recovery_windows;
#[cfg(windows)]
pub mod reset_windows;

/// Only an explicit Yes for a complete, rejected, unambiguous v1 snapshot
/// authorizes the UI reset. Closing the prompt has the same effect as No.
pub fn reset_approved(eligibility: RecoveryEligibility, dialog_result: i32) -> bool {
    eligibility == RecoveryEligibility::UnambiguousVersion1 && dialog_result == 6
}

/// Windows MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2: No is the default.
pub const RESET_CONFIRM_FLAGS: u32 = 0x134;

pub const CURRENT_VERSION: u32 = 1;
/// More than sufficient for the supported preferences and a Windows path,
/// while bounding memory even if a local file is damaged or grows during read.
const MAX_SETTINGS_BYTES: usize = 1024 * 1024;

#[cfg(not(windows))]
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Scroll direction selected for a new capture.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    #[default]
    Down,
    Right,
    Left,
}

/// Persistent user preferences. `auto_save` defaults to false to avoid
/// unattended writes; `copy_to_clipboard` also defaults to false.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    #[serde(default)]
    pub save_folder: Option<PathBuf>,
    #[serde(default = "default_open_editor")]
    pub open_editor: bool,
    #[serde(default)]
    pub copy_to_clipboard: bool,
    /// Request that Windows excludes copied images from history and cloud sync.
    #[serde(default = "default_exclude_from_clipboard_history")]
    pub exclude_from_clipboard_history: bool,
    #[serde(default)]
    pub auto_save: bool,
    #[serde(default)]
    pub scroll_direction: ScrollDirection,
    /// Reserved for "capture mouse cursor" (#36), withdrawn before 2.1.0.
    ///
    /// Only read, never written and never used: a file from the 2.1.0 test
    /// build with the option enabled stays loadable, and the next save drops
    /// the field, so the file is readable by 2.0.x again.
    #[serde(default, skip_serializing)]
    pub capture_cursor: bool,
}

fn default_open_editor() -> bool {
    true
}

fn default_exclude_from_clipboard_history() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            save_folder: None,
            open_editor: true,
            copy_to_clipboard: false,
            exclude_from_clipboard_history: true,
            auto_save: false,
            scroll_direction: ScrollDirection::Down,
            capture_cursor: false,
        }
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    TooLarge {
        path: PathBuf,
    },
    NoConfigDirectory,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                f,
                "could not {operation} settings at {}: {source}",
                path.display()
            ),
            Self::Json { path, source } => {
                write!(f, "invalid settings JSON at {}: {source}", path.display())
            }
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "settings version {found} is unsupported (this build supports version {supported})"
            ),
            Self::TooLarge { path } => write!(
                f,
                "Die Einstellungsdatei {} überschreitet das Größenlimit von 1 MiB",
                path.display()
            ),
            Self::NoConfigDirectory => f.write_str("neither APPDATA nor LOCALAPPDATA is available"),
        }
    }
}

impl Error for SettingsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::UnsupportedVersion { .. } | Self::TooLarge { .. } | Self::NoConfigDirectory => {
                None
            }
        }
    }
}

impl SettingsError {
    /// Fixed diagnostic fields only: never format error messages, paths or JSON.
    pub fn diagnostic_summary(&self) -> String {
        let (category, operation, source) = match self {
            Self::Io {
                operation, source, ..
            } => {
                // Even &'static str may contain arbitrary caller-provided text.
                let operation = match *operation {
                    "read" => "read",
                    "create settings directory for" => "create_settings_directory",
                    "create temporary file for" => "create_temporary_file",
                    "write" => "write",
                    "replace" => "replace",
                    "inspect recovery artifacts" => "inspect_recovery_artifacts",
                    "explicit reset" => "explicit_reset",
                    _ => "unknown",
                };
                ("Io", operation, Some(source))
            }
            Self::Json { .. } => ("Json", "none", None),
            Self::UnsupportedVersion { .. } => ("UnsupportedVersion", "none", None),
            Self::TooLarge { .. } => ("TooLarge", "none", None),
            Self::NoConfigDirectory => ("NoConfigDirectory", "none", None),
        };
        let kind = source.map_or_else(|| "none".to_owned(), |error| format!("{:?}", error.kind()));
        let code = source
            .and_then(original_os_code)
            .map_or_else(|| "none".to_owned(), |code| code.to_string());
        format!("category={category} operation={operation} io_kind={kind} os_code={code}")
    }
}

fn original_os_code(error: &io::Error) -> Option<i32> {
    let mut current: &(dyn Error + 'static) = error;
    // Bound traversal in case a custom error provides a cyclic source chain.
    for _ in 0..32 {
        let next = if let Some(error) = current.downcast_ref::<io::Error>() {
            if let Some(code) = error.raw_os_error() {
                return Some(code);
            }
            // io::Error::source may skip its custom payload itself. Inspect the
            // payload too so an embedded io::Error's raw code is retained.
            error.get_ref().map(|inner| inner as &(dyn Error + 'static))
        } else {
            current.source()
        };
        current = next?;
    }
    None
}

/// Return the conventional per-user Windows settings path. The caller may use
/// this with [`load_or_default`] and [`save_atomic`].
pub fn settings_path() -> Result<PathBuf, SettingsError> {
    settings_path_from(
        std::env::var_os("APPDATA"),
        std::env::var_os("LOCALAPPDATA"),
    )
}

fn settings_path_from(
    app_data: Option<std::ffi::OsString>,
    local_app_data: Option<std::ffi::OsString>,
) -> Result<PathBuf, SettingsError> {
    let base = app_data
        .or(local_app_data)
        .ok_or(SettingsError::NoConfigDirectory)?;
    Ok(PathBuf::from(base)
        .join("ShotKitRust")
        .join("settings.json"))
}

impl Settings {
    fn validate(&self) -> Result<(), SettingsError> {
        if self.version != CURRENT_VERSION {
            return Err(SettingsError::UnsupportedVersion {
                found: self.version,
                supported: CURRENT_VERSION,
            });
        }
        Ok(())
    }
}

/// Read and validate a settings file. A missing file is returned as an I/O
/// error; use [`load_or_default`] when absence should select safe defaults.
pub fn load(path: impl AsRef<Path>) -> Result<Settings, SettingsError> {
    let path = path.as_ref();
    let bytes = read_settings_file(path)?;
    parse_settings(path, &bytes)
}

/// Limit the bytes actually read, not merely metadata observed before reading.
fn read_settings_file(path: &Path) -> Result<Vec<u8>, SettingsError> {
    let file = File::open(path).map_err(|source| SettingsError::Io {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    read_settings_bounded(file, path)
}

fn read_settings_bounded(reader: impl Read, path: &Path) -> Result<Vec<u8>, SettingsError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_SETTINGS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| SettingsError::Io {
            operation: "read",
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsError::TooLarge {
            path: path.to_owned(),
        });
    }
    Ok(bytes)
}

fn parse_settings(path: &Path, bytes: &[u8]) -> Result<Settings, SettingsError> {
    // Inspect the version before interpreting version-specific fields. A newer
    // file can legitimately contain fields or enum values this build does not
    // understand; reject it as a future version without changing the file.
    #[derive(Deserialize)]
    struct VersionHeader {
        version: u32,
    }
    let header: VersionHeader =
        serde_json::from_slice(bytes).map_err(|source| SettingsError::Json {
            path: path.to_owned(),
            source,
        })?;
    if header.version != CURRENT_VERSION {
        return Err(SettingsError::UnsupportedVersion {
            found: header.version,
            supported: CURRENT_VERSION,
        });
    }
    let settings: Settings =
        serde_json::from_slice(bytes).map_err(|source| SettingsError::Json {
            path: path.to_owned(),
            source,
        })?;
    settings.validate()?;
    Ok(settings)
}

/// Load settings, returning defaults only when the file does not exist.
/// Parse errors, permission errors, and unsupported versions remain visible.
pub fn load_or_default(path: impl AsRef<Path>) -> Result<Settings, SettingsError> {
    let path = path.as_ref();
    match load(path) {
        Ok(settings) => Ok(settings),
        Err(SettingsError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(Settings::default())
        }
        Err(error) => Err(error),
    }
}

/// Load settings and retain the exact bytes parsed for a later guarded save.
/// `None` means the file was absent at the time of the read; a later creator
/// must still be protected by a no-replace publication operation.
pub fn load_snapshot(path: impl AsRef<Path>) -> Result<(Settings, Option<Vec<u8>>), SettingsError> {
    let path = path.as_ref();
    match read_settings_file(path) {
        Ok(bytes) => {
            let settings = parse_settings(path, &bytes)?;
            Ok((settings, Some(bytes)))
        }
        Err(SettingsError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok((Settings::default(), None))
        }
        Err(error) => Err(error),
    }
}

/// Source-format eligibility only. This is not approval to recover, a file
/// identity check, or a promise that Windows recovery will succeed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryEligibility {
    UnambiguousVersion1,
    Ineligible,
}

/// A complete, bounded read which failed normal settings validation.
/// Raw bytes are intentionally not included in Debug output or diagnostics.
pub struct RejectedSettings {
    bytes: Vec<u8>,
    pub error: SettingsError,
    pub recovery_eligibility: RecoveryEligibility,
}

impl RejectedSettings {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Read outcome for a caller that must distinguish missing, valid, and rejected
/// settings. Rejected bytes are retained exactly, without reparsing or encoding.
/// I/O failures and oversized reads return Err and retain no partial snapshot.
pub enum SettingsLoadOutcome {
    Missing,
    Loaded { settings: Settings, bytes: Vec<u8> },
    Rejected(RejectedSettings),
}

/// Read at most 1 MiB plus a sentinel byte. No recovery, save or default-selection
/// side effect occurs; existing load/load_snapshot behavior is unchanged.
pub fn load_outcome(path: impl AsRef<Path>) -> Result<SettingsLoadOutcome, SettingsError> {
    let path = path.as_ref();
    let bytes = match read_settings_file(path) {
        Ok(bytes) => bytes,
        Err(SettingsError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(SettingsLoadOutcome::Missing);
        }
        Err(error) => return Err(error),
    };
    Ok(match parse_settings(path, &bytes) {
        Ok(settings) => SettingsLoadOutcome::Loaded { settings, bytes },
        Err(error) => {
            let recovery_eligibility =
                if CURRENT_VERSION == 1 && recovery_version::validate_v1_source(&bytes).is_ok() {
                    RecoveryEligibility::UnambiguousVersion1
                } else {
                    RecoveryEligibility::Ineligible
                };
            SettingsLoadOutcome::Rejected(RejectedSettings {
                bytes,
                error,
                recovery_eligibility,
            })
        }
    })
}

/// Persist settings using a sibling temporary file and an atomic replace.
/// On any error before replacement, an existing destination remains intact.
pub fn save_atomic(path: impl AsRef<Path>, settings: &Settings) -> Result<(), SettingsError> {
    let path = path.as_ref();
    settings.validate()?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|source| SettingsError::Json {
        path: path.to_owned(),
        source,
    })?;

    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsError::TooLarge {
            path: path.to_owned(),
        });
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| SettingsError::Io {
        operation: "create settings directory for",
        path: parent.to_owned(),
        source,
    })?;

    #[cfg(windows)]
    {
        crate::atomic_save_windows::write_settings(path, &bytes).map_err(|error| {
            SettingsError::Io {
                operation: error.operation,
                path: path.to_owned(),
                source: error.source,
            }
        })
    }
    #[cfg(not(windows))]
    {
        let (temp_path, mut file) =
            create_temp_file(parent).map_err(|source| SettingsError::Io {
                operation: "create temporary file for",
                path: path.to_owned(),
                source,
            })?;

        let write_result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            Ok::<(), io::Error>(())
        })();
        if let Err(source) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(SettingsError::Io {
                operation: "write",
                path: path.to_owned(),
                source,
            });
        }

        if let Err(source) = replace_file(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(SettingsError::Io {
                operation: "replace",
                path: path.to_owned(),
                source,
            });
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn create_temp_file(parent: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".shotkit-settings-{}-{sequence}.tmp", std::process::id());
        let candidate = parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique settings temporary file",
    ))
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(windows)]
    use std::{fs::OpenOptions, io::Write};

    #[test]
    fn reset_requires_explicit_yes_for_the_exact_eligible_class() {
        for answer in [0, 1, 2, 5, 7, 8, -1] {
            assert!(!reset_approved(
                RecoveryEligibility::UnambiguousVersion1,
                answer
            ));
        }
        assert!(reset_approved(RecoveryEligibility::UnambiguousVersion1, 6));
        for answer in [-1, 0, 6, 7] {
            assert!(!reset_approved(RecoveryEligibility::Ineligible, answer));
        }
        assert_eq!(RESET_CONFIRM_FLAGS & 0x300, 0x100); // default button 2: No
        assert_eq!(RESET_CONFIRM_FLAGS & 0xf, 4); // Yes/No, with no implicit OK
    }

    #[test]
    fn diagnostic_fields_have_fixed_format_and_follow_nested_os_error() {
        #[derive(Debug)]
        struct Context(io::Error);
        impl fmt::Display for Context {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("secret-path-and-content")
            }
        }
        impl Error for Context {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }
        let source = io::Error::other(Context(io::Error::new(
            io::ErrorKind::PermissionDenied,
            io::Error::from_raw_os_error(5),
        )));
        assert_eq!(source.raw_os_error(), None);
        let error = SettingsError::Io {
            operation: "replace",
            path: PathBuf::from("secret-settings-path"),
            source,
        };
        assert_eq!(
            error.diagnostic_summary(),
            "category=Io operation=replace io_kind=Other os_code=5"
        );
    }

    #[test]
    fn diagnostics_never_include_paths_contents_or_unknown_operation_text() {
        let secret = "secret-path-content-token\nsettings_save=success";
        let path = PathBuf::from(secret);
        let errors = [
            (
                SettingsError::Io {
                    operation: secret,
                    path: path.clone(),
                    source: io::Error::other(secret),
                },
                "category=Io operation=unknown io_kind=Other os_code=none",
            ),
            (
                SettingsError::Json {
                    path: path.clone(),
                    source: serde_json::from_str::<Settings>(&format!("{{\"{secret}\":"))
                        .unwrap_err(),
                },
                "category=Json operation=none io_kind=none os_code=none",
            ),
            (
                SettingsError::TooLarge { path },
                "category=TooLarge operation=none io_kind=none os_code=none",
            ),
            (
                SettingsError::UnsupportedVersion {
                    found: 123456,
                    supported: 987654,
                },
                "category=UnsupportedVersion operation=none io_kind=none os_code=none",
            ),
            (
                SettingsError::NoConfigDirectory,
                "category=NoConfigDirectory operation=none io_kind=none os_code=none",
            ),
        ];
        for (error, expected) in errors {
            let diagnostic = error.diagnostic_summary();
            assert_eq!(diagnostic, expected);
            assert!(!diagnostic.contains("secret"));
            assert!(!diagnostic.contains('\n'));
        }
    }

    fn test_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("shotkit-settings-test-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn settings_read_limit_handles_boundary_and_growth_without_overwriting() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let mut at_limit = br#"{"version":1}"#.to_vec();
        at_limit.resize(MAX_SETTINGS_BYTES, b' ');
        fs::write(&path, &at_limit).unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert_eq!(load_snapshot(&path).unwrap().1.unwrap(), at_limit);

        // Keep the same open handle while the file grows after it was opened.
        let opened = File::open(&path).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b" ")
            .unwrap();
        assert!(matches!(
            read_settings_bounded(opened, &path),
            Err(SettingsError::TooLarge { .. })
        ));
        assert!(matches!(load(&path), Err(SettingsError::TooLarge { .. })));
        assert!(matches!(
            load_snapshot(&path),
            Err(SettingsError::TooLarge { .. })
        ));
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            (MAX_SETTINGS_BYTES + 1) as u64
        );
        assert_eq!(
            &fs::read(&path).unwrap()[..at_limit.len()],
            at_limit.as_slice()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_serialized_settings_leave_existing_file_intact() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let original = br#"{"version":1}"#;
        fs::write(&path, original).unwrap();
        let settings = Settings {
            save_folder: Some(PathBuf::from("a".repeat(MAX_SETTINGS_BYTES))),
            ..Settings::default()
        };
        assert!(matches!(
            save_atomic(&path, &settings),
            Err(SettingsError::TooLarge { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn snapshot_retains_exact_parsed_bytes_and_distinguishes_missing() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        assert_eq!(load_snapshot(&path).unwrap(), (Settings::default(), None));
        let original = b"{\n  \"version\": 1, \"open_editor\": false\n}";
        fs::write(&path, original).unwrap();
        let (settings, bytes) = load_snapshot(&path).unwrap();
        assert!(!settings.open_editor);
        assert_eq!(bytes.as_deref(), Some(original.as_slice()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_outcome_distinguishes_absence_valid_and_exact_rejected_bytes() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        assert!(matches!(
            load_outcome(&path).unwrap(),
            SettingsLoadOutcome::Missing
        ));
        assert!(!path.exists());

        let valid = b"{\n \"version\": 1, \"open_editor\": false\n}";
        fs::write(&path, valid).unwrap();
        match load_outcome(&path).unwrap() {
            SettingsLoadOutcome::Loaded { settings, bytes } => {
                assert!(!settings.open_editor);
                assert_eq!(bytes, valid);
            }
            _ => panic!("expected a complete valid snapshot"),
        }
        let invalid = b"{\n \"version\": 1, \"open_editor\": \"wrong type\"\n}";
        fs::write(&path, invalid).unwrap();
        match load_outcome(&path).unwrap() {
            SettingsLoadOutcome::Rejected(rejected) => {
                assert_eq!(rejected.bytes(), invalid);
                assert!(matches!(rejected.error, SettingsError::Json { .. }));
                assert_eq!(
                    rejected.recovery_eligibility,
                    RecoveryEligibility::UnambiguousVersion1
                );
            }
            _ => panic!("expected rejected snapshot"),
        }
        // Existing APIs still expose the original error instead of defaults.
        assert!(matches!(
            load_snapshot(&path),
            Err(SettingsError::Json { .. })
        ));
        assert!(matches!(
            load_or_default(&path),
            Err(SettingsError::Json { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), invalid);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejected_outcome_uses_strict_shared_recovery_version_check() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let cases: &[(&[u8], RecoveryEligibility)] = &[
            (
                br#"{"version":1,"unknown":true}"#,
                RecoveryEligibility::UnambiguousVersion1,
            ),
            (
                br#"{"version":1,"scroll_direction":"diagonal"}"#,
                RecoveryEligibility::UnambiguousVersion1,
            ),
            (
                br#"{"version":2,"unknown":true}"#,
                RecoveryEligibility::Ineligible,
            ),
            (br#"{"version":0}"#, RecoveryEligibility::Ineligible),
            (
                br#"{"unknown":{"version":1}}"#,
                RecoveryEligibility::Ineligible,
            ),
            (
                br#"{"version":1,"version":1}"#,
                RecoveryEligibility::Ineligible,
            ),
            (
                br#"{"version":1,"version":2}"#,
                RecoveryEligibility::Ineligible,
            ),
            (br#"{"version":"1"}"#, RecoveryEligibility::Ineligible),
            (br#"{"version":1.0}"#, RecoveryEligibility::Ineligible),
            (br#"{"version":-1}"#, RecoveryEligibility::Ineligible),
            (
                br#"{"version":4294967296}"#,
                RecoveryEligibility::Ineligible,
            ),
            (br#"{"version":null}"#, RecoveryEligibility::Ineligible),
            (br#"{"version":1,"bad": }"#, RecoveryEligibility::Ineligible),
            (
                br#"{"version":1} trailing"#,
                RecoveryEligibility::Ineligible,
            ),
            (br#"[]"#, RecoveryEligibility::Ineligible),
            (
                b"{\"version\":1,\"bad\":\"\xff\"}",
                RecoveryEligibility::Ineligible,
            ),
        ];
        for &(bytes, expected) in cases {
            fs::write(&path, bytes).unwrap();
            match load_outcome(&path).unwrap() {
                SettingsLoadOutcome::Rejected(rejected) => {
                    assert_eq!(rejected.bytes(), bytes);
                    assert_eq!(rejected.recovery_eligibility, expected, "input: {bytes:?}");
                    assert_eq!(
                        expected == RecoveryEligibility::UnambiguousVersion1,
                        recovery_version::validate_v1_source(bytes).is_ok()
                    );
                }
                _ => panic!("expected rejected snapshot"),
            }
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovery_source_check_refuses_arrays_even_if_deserializable_as_structs() {
        assert!(recovery_version::validate_v1_source(br#"[1]"#).is_err());
        assert!(recovery_version::validate_v1_source(br#" [1, false] "#).is_err());
    }

    #[test]
    fn rejected_outcome_retains_only_complete_bounded_reads() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let mut bytes = br#"{"version":1,"bad":true}"#.to_vec();
        bytes.resize(MAX_SETTINGS_BYTES, b' ');
        fs::write(&path, &bytes).unwrap();
        match load_outcome(&path).unwrap() {
            SettingsLoadOutcome::Rejected(rejected) => assert_eq!(rejected.bytes(), bytes),
            _ => panic!("expected rejected snapshot at limit"),
        }
        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            load_outcome(&path),
            Err(SettingsError::TooLarge { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(matches!(load_outcome(&path), Err(SettingsError::Io { .. })));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn round_trip_preserves_unicode_path_and_options() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let expected = Settings {
            save_folder: Some(dir.join("Bilder ä 日本語")),
            open_editor: false,
            copy_to_clipboard: true,
            exclude_from_clipboard_history: false,
            auto_save: true,
            scroll_direction: ScrollDirection::Left,
            ..Settings::default()
        };

        save_atomic(&path, &expected).unwrap();
        assert_eq!(load(&path).unwrap(), expected);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn defaults_avoid_automatic_copy_or_save() {
        let settings = Settings::default();
        assert_eq!(settings.version, CURRENT_VERSION);
        assert!(settings.save_folder.is_none());
        assert!(settings.open_editor);
        assert!(!settings.copy_to_clipboard);
        assert!(settings.exclude_from_clipboard_history);
        assert!(!settings.auto_save);
        assert_eq!(settings.scroll_direction, ScrollDirection::Down);
    }

    #[test]
    fn withdrawn_capture_cursor_field_is_read_but_never_written() {
        // #36 was withdrawn before 2.1.0. A test-build file with the option
        // enabled must still load; saving drops the field again, so defaults
        // and saved files stay valid for the frozen v1 reset receipt and 2.0.x.
        let defaults = serde_json::to_vec_pretty(&Settings::default()).unwrap();
        assert!(reset_receipt::valid_v1_settings(&defaults));
        let mut value: serde_json::Value = serde_json::from_slice(&defaults).unwrap();
        value["capture_cursor"] = serde_json::Value::Bool(true);
        let loaded: Settings = serde_json::from_value(value).unwrap();
        assert!(loaded.capture_cursor);
        let saved = serde_json::to_vec_pretty(&loaded).unwrap();
        assert!(!String::from_utf8_lossy(&saved).contains("capture_cursor"));
        assert!(reset_receipt::valid_v1_settings(&saved));
    }

    #[test]
    fn missing_settings_fields_use_defaults_for_older_files() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        fs::write(
            &path,
            br#"{"version":1,"open_editor":false,"copy_to_clipboard":true}"#,
        )
        .unwrap();

        let settings = load(&path).unwrap();
        assert_eq!(settings.version, CURRENT_VERSION);
        assert!(!settings.open_editor);
        assert!(settings.copy_to_clipboard);
        assert!(settings.exclude_from_clipboard_history);
        assert_eq!(settings.save_folder, None);
        assert!(!settings.auto_save);
        assert_eq!(settings.scroll_direction, ScrollDirection::Down);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_version_remains_required() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        fs::write(&path, br#"{"open_editor":true}"#).unwrap();

        assert!(matches!(load(&path), Err(SettingsError::Json { .. })));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_path_is_separate_from_legacy_shotkit_folder() {
        let base = PathBuf::from(r"C:\Users\example\AppData\Roaming");
        let path = settings_path_from(Some(base.clone().into_os_string()), None).unwrap();
        assert_eq!(path, base.join("ShotKitRust").join("settings.json"));
        assert_ne!(path, base.join("ShotKit").join("settings.json"));

        let local = PathBuf::from(r"C:\Users\example\AppData\Local");
        assert_eq!(
            settings_path_from(None, Some(local.clone().into_os_string())).unwrap(),
            local.join("ShotKitRust").join("settings.json")
        );
    }

    #[test]
    fn missing_file_uses_defaults_but_malformed_json_is_visible() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        assert_eq!(load_or_default(&path).unwrap(), Settings::default());
        fs::write(&path, b"{ not json }").unwrap();
        assert!(matches!(
            load_or_default(&path),
            Err(SettingsError::Json { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_replace_keeps_existing_target_contents() {
        let dir = test_dir();
        let destination = dir.join("settings.json");
        fs::create_dir(&destination).unwrap();
        let marker = destination.join("keep.txt");
        fs::write(&marker, b"existing data").unwrap();

        assert!(matches!(
            save_atomic(&destination, &Settings::default()),
            Err(SettingsError::Io {
                operation: "replace",
                ..
            })
        ));
        assert_eq!(fs::read(marker).unwrap(), b"existing data");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn reader_without_delete_sharing_blocks_replace_until_reader_closes() {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        let dir = test_dir();
        let path = dir.join("settings.json");
        let original = br#"{"version":1,"open_editor":true}"#;
        fs::write(&path, original).unwrap();
        let changed = Settings {
            open_editor: false,
            ..Settings::default()
        };

        // Deterministically model a reader that permits reads/writes but not
        // replacement. This does not establish which handles a past CI run held.
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let error = save_atomic(&path, &changed).unwrap_err();
        assert!(matches!(
            &error,
            SettingsError::Io {
                operation: "replace",
                ..
            }
        ));
        assert_ne!(
            original_os_code(match &error {
                SettingsError::Io { source, .. } => source,
                _ => unreachable!(),
            }),
            None
        );
        assert_eq!(fs::read(&path).unwrap(), original);

        drop(reader);
        save_atomic(&path, &changed).unwrap();
        assert_eq!(load(&path).unwrap(), changed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn reader_with_delete_sharing_still_prevents_handle_based_replace() {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        const FILE_SHARE_DELETE: u32 = 4;
        let dir = test_dir();
        let path = dir.join("settings.json");
        let original = br#"{"version":1,"open_editor":true}"#;
        fs::write(&path, original).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        let changed = Settings {
            open_editor: false,
            ..Settings::default()
        };

        // FILE_SHARE_DELETE alone does not guarantee that FileRenameInfo can
        // replace an existing open target on the Windows CI filesystem.
        let error = save_atomic(&path, &changed).unwrap_err();
        assert!(matches!(
            &error,
            SettingsError::Io {
                operation: "replace",
                ..
            }
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        let mut old_bytes = Vec::new();
        (&reader).read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, original);
        drop(reader);
        save_atomic(&path, &changed).unwrap();
        assert_eq!(load(&path).unwrap(), changed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_version_with_unknown_fields_and_values_is_not_modified() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let original =
            br#"{"version":2,"scroll_direction":"diagonal","future_option":{"enabled":true}}"#;
        fs::write(&path, original).unwrap();

        assert!(matches!(
            load_or_default(&path),
            Err(SettingsError::UnsupportedVersion { found: 2, .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_current_version_field_is_rejected_without_modification() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let original = br#"{"version":1,"unknown_option":true}"#;
        fs::write(&path, original).unwrap();

        assert!(matches!(
            load_or_default(&path),
            Err(SettingsError::Json { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_versions_are_rejected_on_load_and_save() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        fs::write(&path, br#"{"version":2,"save_folder":null,"open_editor":true,"copy_to_clipboard":false,"auto_save":false,"scroll_direction":"down"}"#).unwrap();
        let existing = fs::read(&path).unwrap();
        assert!(matches!(
            load(&path),
            Err(SettingsError::UnsupportedVersion { found: 2, .. })
        ));
        let settings = Settings {
            version: 2,
            ..Settings::default()
        };
        assert!(matches!(
            save_atomic(&path, &settings),
            Err(SettingsError::UnsupportedVersion { found: 2, .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), existing);
        fs::remove_dir_all(dir).unwrap();
    }
}
