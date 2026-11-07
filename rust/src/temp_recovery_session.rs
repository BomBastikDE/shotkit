//! Once-per-session bookkeeping for the temporary-file check on network folders.
//!
//! Startup never touches a network folder. After a successful save into such a
//! folder, ShotKit has contacted the server anyway and checks that folder once
//! per session for remnants of an aborted save.

use std::path::Path;

/// Network folders already checked after a save in this session.
#[derive(Debug, Default)]
pub struct NetworkRecoveryChecks {
    checked: Vec<String>,
}

impl NetworkRecoveryChecks {
    pub const fn new() -> Self {
        Self {
            checked: Vec::new(),
        }
    }

    /// Returns `true` the first time a folder is seen and `false` afterwards.
    ///
    /// Windows paths are case-insensitive, and `/` and a trailing separator do
    /// not name a different folder, so those spellings count as the same folder.
    pub fn first_check(&mut self, folder: &Path) -> bool {
        let key = normalize(folder);
        if self.checked.contains(&key) {
            return false;
        }
        self.checked.push(key);
        true
    }
}

fn normalize(folder: &Path) -> String {
    let mut text = folder.to_string_lossy().replace('/', "\\");
    // Drop trailing separators, but keep the one of a drive root (`Z:\`).
    while text.ends_with('\\') && !text.ends_with(":\\") && text.len() > 1 {
        text.pop();
    }
    text.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::NetworkRecoveryChecks;
    use std::path::Path;

    #[test]
    fn each_folder_is_checked_once_per_session() {
        let mut checks = NetworkRecoveryChecks::new();
        assert!(checks.first_check(Path::new(r"\\server\share\Bilder")));
        assert!(!checks.first_check(Path::new(r"\\server\share\Bilder")));
        assert!(checks.first_check(Path::new(r"Z:\ShotKit")));
        assert!(!checks.first_check(Path::new(r"Z:\ShotKit")));
    }

    #[test]
    fn spelling_variants_of_the_same_folder_are_one_folder() {
        let mut checks = NetworkRecoveryChecks::new();
        assert!(checks.first_check(Path::new(r"\\Server\Share\Bilder")));
        for same in [
            r"\\server\share\bilder",
            r"\\SERVER\SHARE\BILDER\",
            "//server/share/Bilder",
            r"\\server\share\Bilder\\",
        ] {
            assert!(!checks.first_check(Path::new(same)), "{same}");
        }
    }

    #[test]
    fn different_folders_and_drive_roots_stay_distinct() {
        let mut checks = NetworkRecoveryChecks::new();
        assert!(checks.first_check(Path::new(r"Z:\")));
        assert!(!checks.first_check(Path::new("Z:\\")));
        assert!(checks.first_check(Path::new(r"Z:\Bilder")));
        assert!(checks.first_check(Path::new(r"Y:\")));
        assert!(checks.first_check(Path::new(r"\\server\share\Bilder\Alt")));
    }
}
