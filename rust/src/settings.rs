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
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const CURRENT_VERSION: u32 = 1;

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
            Self::NoConfigDirectory => f.write_str("neither APPDATA nor LOCALAPPDATA is available"),
        }
    }
}

impl Error for SettingsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::UnsupportedVersion { .. } | Self::NoConfigDirectory => None,
        }
    }
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
    let bytes = fs::read(path).map_err(|source| SettingsError::Io {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    // Inspect the version before interpreting version-specific fields. A newer
    // file can legitimately contain fields or enum values this build does not
    // understand; reject it as a future version without changing the file.
    #[derive(Deserialize)]
    struct VersionHeader {
        version: u32,
    }
    let header: VersionHeader =
        serde_json::from_slice(&bytes).map_err(|source| SettingsError::Json {
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
        serde_json::from_slice(&bytes).map_err(|source| SettingsError::Json {
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

/// Persist settings using a sibling temporary file and an atomic replace.
/// On any error before replacement, an existing destination remains intact.
pub fn save_atomic(path: impl AsRef<Path>, settings: &Settings) -> Result<(), SettingsError> {
    let path = path.as_ref();
    settings.validate()?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|source| SettingsError::Json {
        path: path.to_owned(),
        source,
    })?;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| SettingsError::Io {
        operation: "create settings directory for",
        path: parent.to_owned(),
        source,
    })?;

    let (temp_path, mut file) = create_temp_file(parent).map_err(|source| SettingsError::Io {
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

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let source = wide(source.as_os_str());
    let destination = wide(destination.as_os_str());
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn wide(path: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.encode_wide().chain(Some(0)).collect()
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut settings = Settings::default();
        settings.version = 2;
        assert!(matches!(
            save_atomic(&path, &settings),
            Err(SettingsError::UnsupportedVersion { found: 2, .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), existing);
        fs::remove_dir_all(dir).unwrap();
    }
}
