//! Best-effort diagnostic bundle for a final ambiguous scroll match.
use crate::{image_io, stitch::Image};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn save(folder: &Path, before: &Image, after: &Image, info: &str) -> io::Result<PathBuf> {
    // Do not create a missing configured storage root by accident.
    if !folder.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Speicherordner nicht verfügbar",
        ));
    }
    let base = folder.join("Diagnose");
    fs::create_dir_all(&base)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut dir = None;
    for suffix in 0..1000 {
        let candidate = base.join(format!("Scroll_{stamp}_{suffix}"));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                dir = Some(candidate);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let dir = dir.ok_or_else(|| {
        io::Error::new(io::ErrorKind::AlreadyExists, "Kein freier Diagnoseordner")
    })?;
    let result = (|| {
        for (name, image) in [("vorher.png", before), ("nachher.png", after)] {
            let bytes =
                image_io::encode(image, image::ImageFormat::Png).map_err(io::Error::other)?;
            fs::write(dir.join(name), bytes)?;
        }
        fs::write(dir.join("info.txt"), info)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundles_are_unique_and_preserve_exact_pixels() {
        let root = std::env::temp_dir().join(format!(
            "shotkit-diag-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let a = Image::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        let b = Image::new(1, 1, vec![4, 5, 6, 255]).unwrap();
        let first = save(&root, &a, &b, "test").unwrap();
        let second = save(&root, &b, &a, "test2").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            image_io::decode(&fs::read(first.join("vorher.png")).unwrap()).unwrap(),
            a
        );
        assert_eq!(
            image_io::decode(&fs::read(first.join("nachher.png")).unwrap()).unwrap(),
            b
        );
        assert_eq!(fs::read_to_string(first.join("info.txt")).unwrap(), "test");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_root_is_not_created() {
        let root = std::env::temp_dir().join(format!(
            "shotkit-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let image = Image::new(1, 1, vec![0, 0, 0, 255]).unwrap();
        assert!(save(&root, &image, &image, "").is_err());
        assert!(!root.exists());
    }
}
