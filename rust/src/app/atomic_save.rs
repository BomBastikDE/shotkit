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

pub(super) fn write_atomic(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
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
}
