//! Windows capture-output helpers. The caller owns the captured image and drives
//! [`CaptureOutputState`], including the editor action and failure reporting.
use crate::{
    capture_output::{Advance, CaptureAction, CaptureOutputState},
    image_io,
    settings::Settings,
    stitch::Image,
};
#[cfg(windows)]
use std::ffi::c_void;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// What the caller should do after asking this module to advance the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputProgress {
    /// The next planned step is the editor; the caller must open it and report
    /// its result with `CaptureOutputState::finish_action`.
    EditorRequired,
    /// A save/copy action was run and its result was recorded in the state.
    Advanced(Advance),
    /// There are no more planned actions.
    Complete,
}

/// Run and record the next save/clipboard step. Editor creation stays with the
/// caller so it can use the application's UI framework and retain the image.
pub fn advance_output(
    state: &mut CaptureOutputState,
    settings: &Settings,
    image: &Image,
    #[cfg(windows)] clipboard_owner: *mut c_void,
) -> Result<OutputProgress, String> {
    let Some(action) = state.next_action() else {
        return Ok(OutputProgress::Complete);
    };
    if action == CaptureAction::OpenEditor {
        return Ok(OutputProgress::EditorRequired);
    }
    let result = execute_action(
        action,
        settings,
        image,
        #[cfg(windows)]
        clipboard_owner,
    )
    .map(|_| ());
    state
        .finish_action(action, result)
        .map(OutputProgress::Advanced)
        .map_err(|error| error.to_string())
}

/// Result of one non-editor output action. `saved_path` is set only for AutoSave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputResult {
    pub saved_path: Option<PathBuf>,
}

/// Execute one non-editor action. The image is borrowed and remains available
/// to the caller on both success and failure. Handle `OpenEditor` in the caller.
pub fn execute_action(
    action: CaptureAction,
    settings: &Settings,
    image: &Image,
    #[cfg(windows)] clipboard_owner: *mut c_void,
) -> Result<OutputResult, String> {
    match action {
        CaptureAction::AutoSave => {
            if !settings.auto_save {
                return Err("AutoSave wurde in den Einstellungen nicht aktiviert".into());
            }
            let folder = settings
                .save_folder
                .as_deref()
                .ok_or_else(|| "Kein Speicherordner konfiguriert".to_owned())?;
            save_png(folder, image)
                .map(|saved_path| OutputResult {
                    saved_path: Some(saved_path),
                })
                .map_err(|e| format!("PNG konnte nicht gespeichert werden: {e}"))
        }
        CaptureAction::CopyToClipboard => {
            if !settings.copy_to_clipboard {
                return Err("Clipboard-Kopie wurde in den Einstellungen nicht aktiviert".into());
            }
            #[cfg(windows)]
            {
                copy_image_to_clipboard(clipboard_owner, image)?;
                Ok(OutputResult { saved_path: None })
            }
            #[cfg(not(windows))]
            {
                let _ = image;
                Err("Die Windows-Zwischenablage ist auf diesem System nicht verfügbar".into())
            }
        }
        CaptureAction::OpenEditor => Err(
            "OpenEditor muss vom Aufrufer ausgeführt werden; das Bild bleibt dessen Eigentum"
                .into(),
        ),
    }
}

fn save_png(folder: &std::path::Path, image: &Image) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let png = image_io::encode(image, image::ImageFormat::Png)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // create_new is the collision guard, including concurrent captures.
    for _ in 0..256 {
        let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = folder.join(format!("ShotKit-{stamp}-{sequence}.png"));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        finish_created_file(&path, file, |file| {
            file.write_all(&png)?;
            file.sync_all()
        })?;
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique PNG filename",
    ))
}

fn finish_created_file(
    path: &std::path::Path,
    mut file: std::fs::File,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let result = write(&mut file);
    drop(file);
    if let Err(write_error) = result {
        return match fs::remove_file(path) {
            Ok(()) => Err(write_error),
            Err(cleanup_error) => Err(io::Error::new(
                cleanup_error.kind(),
                format!(
                    "{write_error}; additionally could not remove partial file: {cleanup_error}"
                ),
            )),
        };
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DibInfo {
    width: i32,
    height: i32,
    pixel_bytes: u32,
}

fn validate_dib_image(image: &Image) -> Result<DibInfo, String> {
    const MAX_DIMENSION: usize = 30_000;
    const MAX_BYTES: usize = 512 * 1024 * 1024;
    if image.width == 0
        || image.height == 0
        || image.width > MAX_DIMENSION
        || image.height > MAX_DIMENSION
    {
        return Err("Bildabmessungen sind ungültig oder überschreiten 30000 Pixel".into());
    }
    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Bildgröße überläuft".to_owned())?;
    if expected != image.pixels.len() || expected > MAX_BYTES {
        return Err("BGRA-Puffergröße passt nicht zu den Bildabmessungen".into());
    }
    let pixel_bytes =
        u32::try_from(expected).map_err(|_| "Bild ist zu groß für CF_DIB".to_owned())?;
    Ok(DibInfo {
        width: i32::try_from(image.width).map_err(|_| "Bildbreite ist zu groß".to_owned())?,
        height: i32::try_from(image.height).map_err(|_| "Bildhöhe ist zu groß".to_owned())?,
        pixel_bytes,
    })
}

#[cfg(windows)]
fn copy_image_to_clipboard(owner: *mut c_void, image: &Image) -> Result<(), String> {
    type Handle = *mut c_void;

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        clr_used: u32,
        clr_important: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: Handle) -> Handle;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(memory: Handle) -> *mut c_void;
        fn GlobalUnlock(memory: Handle) -> i32;
        fn GlobalFree(memory: Handle) -> Handle;
    }

    const CF_DIB: u32 = 8;
    const GMEM_MOVEABLE: u32 = 0x0002;
    const GMEM_ZEROINIT: u32 = 0x0040;
    const BI_RGB: u32 = 0;
    let dib = validate_dib_image(image)?;
    let width = dib.width;
    let height = dib.height;
    let pixel_bytes =
        usize::try_from(dib.pixel_bytes).map_err(|_| "Bild ist zu groß für CF_DIB")?;
    let dib_bytes = std::mem::size_of::<BitmapInfoHeader>()
        .checked_add(pixel_bytes)
        .ok_or("DIB-Größe überläuft")?;
    let pixel_bytes_u32 = dib.pixel_bytes;
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, dib_bytes) };
    if memory.is_null() {
        return Err("GlobalAlloc für CF_DIB ist fehlgeschlagen".into());
    }
    struct MemoryGuard(Handle);
    impl Drop for MemoryGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    GlobalFree(self.0);
                }
            }
        }
    }
    let memory = MemoryGuard(memory);
    let base = unsafe { GlobalLock(memory.0).cast::<u8>() };
    if base.is_null() {
        return Err("GlobalLock für CF_DIB ist fehlgeschlagen".into());
    }
    let header = BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width,
        height: -height, // top-down rows match Image's BGRA storage
        planes: 1,
        bit_count: 32,
        compression: BI_RGB,
        size_image: pixel_bytes_u32,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };
    unsafe {
        std::ptr::write(base.cast::<BitmapInfoHeader>(), header);
        std::ptr::copy_nonoverlapping(
            image.pixels.as_ptr(),
            base.add(std::mem::size_of::<BitmapInfoHeader>()),
            pixel_bytes,
        );
        GlobalUnlock(memory.0);
    }

    if unsafe { OpenClipboard(owner) } == 0 {
        return Err("OpenClipboard ist fehlgeschlagen".into());
    }
    struct ClipboardGuard(bool);
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CloseClipboard();
                }
            }
        }
    }
    let mut clipboard = ClipboardGuard(true);
    if unsafe { EmptyClipboard() } == 0 {
        return Err("EmptyClipboard ist fehlgeschlagen".into());
    }
    if unsafe { SetClipboardData(CF_DIB, memory.0) }.is_null() {
        return Err("SetClipboardData(CF_DIB) ist fehlgeschlagen".into());
    }
    std::mem::forget(memory); // Windows owns the allocation after success.
    clipboard.0 = false;
    if unsafe { CloseClipboard() } == 0 {
        return Err("CloseClipboard ist nach der Übertragung fehlgeschlagen".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("shotkit-output-{id}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn image() -> Image {
        Image::new(2, 1, vec![0, 0, 255, 255, 255, 0, 0, 255]).unwrap()
    }

    fn run_action(
        action: CaptureAction,
        settings: &Settings,
        image: &Image,
    ) -> Result<OutputResult, String> {
        execute_action(
            action,
            settings,
            image,
            #[cfg(windows)]
            std::ptr::null_mut(),
        )
    }

    #[test]
    fn autosave_creates_distinct_png_files_in_configured_folder() {
        let dir = temp_dir();
        let settings = Settings {
            auto_save: true,
            save_folder: Some(dir.clone()),
            ..Settings::default()
        };
        let first = run_action(CaptureAction::AutoSave, &settings, &image()).unwrap();
        let second = run_action(CaptureAction::AutoSave, &settings, &image()).unwrap();
        let first = first.saved_path.unwrap();
        let second = second.saved_path.unwrap();
        assert_ne!(first, second);
        assert_eq!(&fs::read(first).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&fs::read(second).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_partial_write_removes_created_file() {
        let dir = temp_dir();
        let path = dir.join("partial.png");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let error = finish_created_file(&path, file, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::new(
                io::ErrorKind::Other,
                "injected write failure",
            ))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dib_validation_checks_dimensions_and_exact_bgra_length() {
        assert_eq!(
            validate_dib_image(&image()).unwrap(),
            DibInfo {
                width: 2,
                height: 1,
                pixel_bytes: 8
            }
        );
        let wrong_length = Image {
            width: 2,
            height: 1,
            pixels: vec![0; 4],
        };
        assert!(validate_dib_image(&wrong_length).is_err());
        let invalid_dimensions = Image {
            width: usize::MAX,
            height: 2,
            pixels: vec![],
        };
        assert!(validate_dib_image(&invalid_dimensions).is_err());
    }
}
