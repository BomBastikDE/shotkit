//! Best-effort diagnostic bundle for a final ambiguous scroll match.
use crate::stitch::Image;
use std::{
    fs,
    io::{self, BufWriter, Write},
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
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(name))?;
            let mut writer = BufWriter::new(file);
            write_png(&mut writer, image)?;
            writer.flush()?;
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

// Convert a bounded chunk at a time; the PNG encoder also buffers scanlines.
fn write_png(out: impl Write, image: &Image) -> io::Result<()> {
    image
        .validate()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let width = u32::try_from(image.width).map_err(io::Error::other)?;
    let height = u32::try_from(image.height).map_err(io::Error::other)?;
    let mut encoder = png::Encoder::new(out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(png_error)?;
    {
        let mut stream = writer.stream_writer().map_err(png_error)?;
        let mut rgba = [0u8; 32 * 1024];
        for chunk in image.pixels.chunks(rgba.len()) {
            let converted = &mut rgba[..chunk.len()];
            for (src, dst) in chunk
                .as_chunks::<4>()
                .0
                .iter()
                .zip(converted.as_chunks_mut::<4>().0.iter_mut())
            {
                dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
            }
            stream.write_all(converted)?;
        }
        stream.finish().map_err(png_error)?;
    }
    writer.finish().map_err(png_error)
}

fn png_error(error: png::EncodingError) -> io::Error {
    match error {
        png::EncodingError::IoError(error) => error,
        other => io::Error::other(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_io;
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
    fn streaming_round_trip_across_blocks_and_rows_preserves_alpha() {
        let pixels: Vec<u8> = (0..(9000 * 3 * 4))
            .map(|i| (i * 37 + i / 7) as u8)
            .collect();
        let original = Image::new(9000, 3, pixels).unwrap();
        let mut encoded = Vec::new();
        write_png(&mut encoded, &original).unwrap();
        assert_eq!(image_io::decode(&encoded).unwrap(), original);
    }

    #[test]
    fn compresses_repetitive_pixels() {
        let original = Image::new(512, 512, vec![0; 512 * 512 * 4]).unwrap();
        let mut encoded = Vec::new();
        write_png(&mut encoded, &original).unwrap();
        assert!(encoded.len() < original.pixels.len() / 10);
        assert_eq!(image_io::decode(&encoded).unwrap(), original);
    }

    #[test]
    fn writer_errors_and_invalid_images_are_reported() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "test"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let image = Image::new(1, 1, vec![3, 4, 5, 0]).unwrap();
        assert_eq!(
            write_png(FailingWriter, &image).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        let invalid = Image {
            width: 1,
            height: 1,
            pixels: vec![],
        };
        assert_eq!(
            write_png(Vec::new(), &invalid).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
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
