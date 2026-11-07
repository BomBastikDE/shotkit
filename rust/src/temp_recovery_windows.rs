//! Startup recovery: candidate names are not proof of provenance. Only explicit
//! user approval authorizes deletion. Keep each exclusive, no-follow handle
//! from validation through disposition; never resolve its path again to delete.
//! As with atomic save, ancestors must remain stable. No recursive traversal.
use crate::temp_recovery_names::{is_settings_temp_name, parse_temp_recovery_name};
use std::{
    fs::{self, File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
const MIN_AGE: Duration = Duration::from_secs(600);

#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(
        handle: *mut std::ffi::c_void,
        class: i32,
        info: *const std::ffi::c_void,
        size: u32,
    ) -> i32;
    fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information) -> i32;
}
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

/// An eligible file held exclusively, with immutable display metadata.
pub struct Candidate {
    file: File,
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}
impl Candidate {
    /// Delete this same held object on close, only after the caller's confirmation.
    pub fn delete(self) -> io::Result<()> {
        let disposition = 1u8;
        // SAFETY: owned live DELETE-capable handle; class 4 takes a BOOLEAN.
        let ok = unsafe {
            SetFileInformationByHandle(
                self.file.as_raw_handle(),
                4,
                (&disposition as *const u8).cast(),
                size_of::<u8>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        drop(self);
        Ok(())
    }
}

/// Scan one directory. Inaccessible, locked, young and nonregular entries are
/// skipped. Settings names are accepted only in the settings directory.
pub fn scan(directory: &Path, settings: bool, now: SystemTime) -> io::Result<Vec<Candidate>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(directory)? {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if parse_temp_recovery_name(name).is_none() && !(settings && is_settings_temp_name(name)) {
            continue;
        }
        let path = entry.path();
        // No delete sharing, no read/write sharing, and do not follow the leaf.
        let Ok(file) = OpenOptions::new()
            .access_mode(0x8000_0000 | 0x0001_0000)
            .share_mode(0)
            .custom_flags(0x0020_0000)
            .open(&path)
        else {
            continue;
        };
        let Ok(metadata) = file.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.file_attributes() & (0x400 | 0x10) != 0 {
            continue;
        }
        let mut info = Information::default();
        // SAFETY: writable repr(C) BY_HANDLE_FILE_INFORMATION and owned handle.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
            || info.links != 1
        {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if !now.duration_since(modified).is_ok_and(|age| age > MIN_AGE) {
            continue;
        }
        found.try_reserve(1).map_err(io::Error::other)?;
        found.push(Candidate {
            file,
            path,
            size: metadata.len(),
            modified,
        });
    }
    Ok(found)
}

/// A closed/cancelled prompt and opening a folder leave all candidates intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Delete,
    OpenFolder,
    Later,
}

/// Consume the held set. Only Delete sets disposition; aggregate per-file errors.
pub fn complete(files: Vec<Candidate>, choice: Choice) -> Vec<String> {
    let mut errors = Vec::new();
    if choice == Choice::Delete {
        for file in files {
            let path = file.path.clone();
            if let Err(error) = file.delete() {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_names_age_scope_and_reparse_entries_are_filtered() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "shotkit-recovery-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root)?;
        let old = SystemTime::now() - Duration::from_secs(601);
        let names = [
            ".shotkit-1-2-3.tmp",
            ".ShotKit-1-2-4.tmp",
            ".shotkit-settings-1-5.tmp",
            ".shotkit-abc.tmp",
            "x.shotkit-1-2-3.tmp",
            ".shotkit-1-2-6.tmp",
        ];
        for name in names {
            let file = File::create(root.join(name))?;
            if name != ".shotkit-1-2-6.tmp" {
                file.set_times(fs::FileTimes::new().set_modified(old))?;
            }
        }
        fs::create_dir(root.join(".shotkit-1-2-7.tmp"))?;
        fs::create_dir(root.join("nested"))?;
        let nested = File::create(root.join("nested/.shotkit-1-2-8.tmp"))?;
        nested.set_times(fs::FileTimes::new().set_modified(old))?;
        drop(nested);
        // Reparse directory with exact candidate name: creation of a junction
        // does not require Developer Mode or symlink privilege on Windows CI.
        let junction = root.join(".shotkit-1-2-9.tmp");
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(root.join("nested"))
            .status()?;
        assert!(status.success(), "junction fixture creation failed");
        let link_target = root.join("link-target.bin");
        let file = File::create(&link_target)?;
        file.set_times(fs::FileTimes::new().set_modified(old))?;
        drop(file);
        let hardlink = root.join(".shotkit-1-2-10.tmp");
        fs::hard_link(&link_target, &hardlink)?;
        // Separate singly-linked, old regular target: following this link
        // would otherwise offer it, so this exercises the no-follow check.
        let symlink_target = root.join("symlink-target.bin");
        let file = File::create(&symlink_target)?;
        file.set_times(fs::FileTimes::new().set_modified(old))?;
        drop(file);
        let symlink = root.join(".shotkit-1-2-11.tmp");
        let symlink_created = match std::os::windows::fs::symlink_file(&symlink_target, &symlink) {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("FILE_SYMLINK_PRIVILEGE_UNAVAILABLE: junction and hardlink checks still mandatory");
                false
            }
            Err(error) => return Err(error),
        };
        let image_only = scan(&root, false, SystemTime::now())?;
        assert_eq!(image_only.len(), 2);
        assert!(scan(&root, true, SystemTime::now())?.iter().all(|file| file
            .path
            .file_name()
            .unwrap()
            == ".shotkit-settings-1-5.tmp"));
        complete(image_only, Choice::Later);
        let all = scan(&root, true, SystemTime::now())?;
        assert_eq!(all.len(), 3);
        assert!(complete(all, Choice::Delete).is_empty());
        // Only this test's create_new root and explicitly created fixtures.
        fs::remove_dir(&junction)?;
        assert!(link_target.is_file() && hardlink.is_file());
        if symlink_created {
            assert!(fs::symlink_metadata(&symlink)?.file_type().is_symlink());
            fs::remove_file(&symlink)?;
        }
        fs::remove_file(&hardlink)?;
        fs::remove_file(&link_target)?;
        assert!(symlink_target.is_file());
        fs::remove_file(&symlink_target)?;
        for name in [
            ".shotkit-abc.tmp",
            "x.shotkit-1-2-3.tmp",
            ".shotkit-1-2-6.tmp",
        ] {
            fs::remove_file(root.join(name))?;
        }
        fs::remove_dir(root.join(".shotkit-1-2-7.tmp"))?;
        fs::remove_file(root.join("nested/.shotkit-1-2-8.tmp"))?;
        fs::remove_dir(root.join("nested"))?;
        fs::remove_dir(root)?;
        Ok(())
    }
}
