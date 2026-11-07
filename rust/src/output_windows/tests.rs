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
fn optional_clipboard_formats_are_bounded_to_16_mib() {
    assert!(include_optional_clipboard_formats(16 * 1024 * 1024));
    assert!(!include_optional_clipboard_formats(16 * 1024 * 1024 + 1));
}

#[test]
fn clipboard_errors_report_partial_update_after_empty() {
    let error = clipboard_transfer_result(false, true, true, "legacy failed").unwrap_err();
    assert!(error.contains("teilweise aktualisiert"));
    assert!(clipboard_transfer_result(true, true, false, "legacy failed").is_ok());
    let legacy_only = clipboard_transfer_result(true, false, true, "DIBV5 failed").unwrap_err();
    assert!(legacy_only.contains("vorheriger Inhalt wurde gelöscht"));
    assert!(legacy_only.contains("nur opakes CF_DIB"));
    let none = clipboard_transfer_result(true, false, false, "all formats failed").unwrap_err();
    assert!(none.contains("kein Bildformat übertragen"));
}

#[test]
fn dibv5_round_trip_preserves_alpha_and_top_down_rows() {
    let original = Image::new(
        2,
        2,
        vec![
            7, 11, 19, 0, 23, 31, 47, 64, 53, 71, 89, 128, 101, 131, 173, 255,
        ],
    )
    .unwrap();
    let encoded = encode_dibv5(&original).unwrap();
    assert_eq!(u32::from_le_bytes(encoded[0..4].try_into().unwrap()), 124);
    assert_eq!(i32::from_le_bytes(encoded[8..12].try_into().unwrap()), -2);
    assert_eq!(decode_dib(&encoded, true).unwrap(), original);
}

#[test]
fn png_clipboard_representation_round_trips_transparent_pixels() {
    let original = Image::new(2, 1, vec![1, 2, 3, 0, 7, 11, 19, 128]).unwrap();
    let png = image_io::encode(&original, image::ImageFormat::Png).unwrap();
    assert_eq!(image_io::decode(&png).unwrap(), original);
}

#[test]
fn dibv5_bi_rgb_without_alpha_mask_is_opaque_compatibility_data() {
    let original = Image::new(1, 1, vec![10, 20, 30, 0]).unwrap();
    let mut dib = encode_dibv5(&original).unwrap();
    dib[16..20].copy_from_slice(&0u32.to_le_bytes());
    dib[40..56].fill(0);
    let decoded = decode_dib(&dib, true).unwrap();
    assert_eq!(decoded.pixels, vec![10, 20, 30, 255]);
}

#[test]
fn legacy_dib_is_bottom_up_and_opaque_on_white() {
    let original = Image::new(1, 2, vec![20, 40, 60, 0, 100, 120, 140, 128]).unwrap();
    let encoded = encode_legacy_dib(&original).unwrap();
    assert_eq!(i32::from_le_bytes(encoded[8..12].try_into().unwrap()), 2);
    let decoded = decode_dib(&encoded, false).unwrap();
    assert_eq!(decoded.pixels[0..4], [255, 255, 255, 255]);
    assert_eq!(decoded.pixels[4..8], [177, 187, 197, 255]);
}

#[test]
fn dib_decoder_rejects_truncation_unsupported_bpp_and_invalid_masks() {
    assert!(decode_dib(&[0; 20], false).is_err());
    let mut unsupported = encode_dibv5(&image()).unwrap();
    unsupported[14..16].copy_from_slice(&16u16.to_le_bytes());
    assert!(decode_dib(&unsupported, true).is_err());
    let mut invalid_masks = encode_dibv5(&image()).unwrap();
    invalid_masks[52..56].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
    assert!(decode_dib(&invalid_masks, true).is_err());
}

#[test]
fn budgeted_dib_and_dibv5_match_existing_pixels_and_capacity_accounting() {
    use crate::resource_budget::{AdmissionClass, ResourceBudget};
    let original = Image::new(1, 2, vec![20, 40, 60, 0, 100, 120, 140, 128]).unwrap();
    for (bytes, require_v5) in [
        (encode_dibv5(&original).unwrap(), true),
        (encode_legacy_dib(&original).unwrap(), false),
    ] {
        let expected = decode_dib(&bytes, require_v5).unwrap();
        let budget = ResourceBudget::new(1024, 16).unwrap();
        let existing = budget.try_acquire(37, AdmissionClass::Normal).unwrap();
        let decoded = decode_dib_with_budget(&bytes, require_v5, &budget).unwrap();
        assert_eq!(decoded.image, expected);
        assert!(decoded.post_decode_error.is_none());
        assert_eq!(decoded.lease.bytes(), decoded.image.pixels.capacity());
        assert_eq!(
            budget.snapshot().used_bytes,
            37 + decoded.image.pixels.capacity()
        );
        drop(decoded);
        assert_eq!(budget.snapshot().used_bytes, existing.bytes());
    }
}

#[test]
fn budgeted_dib_rejects_before_conversion_and_preserves_other_images() {
    use crate::resource_budget::{AdmissionClass, ResourceBudget};
    let bytes = encode_dibv5(&image()).unwrap();
    let budget = ResourceBudget::new(11, 0).unwrap();
    let existing = budget.try_acquire(4, AdmissionClass::Normal).unwrap();
    let error = decode_dib_with_budget(&bytes, true, &budget).unwrap_err();
    assert!(error.contains("Kein freies Bildbudget"), "{error}");
    assert_eq!(budget.snapshot().used_bytes, existing.bytes());
    assert!(decode_dib_with_budget(&bytes[..125], true, &budget).is_err());
    assert_eq!(budget.snapshot().used_bytes, existing.bytes());
}

#[test]
fn budgeted_24_bit_dib_preserves_padded_rows_and_orientation() {
    use crate::resource_budget::ResourceBudget;
    // A one-pixel row has three color bytes plus one padding byte.
    for height in [2i32, -2] {
        let mut bytes = vec![0; 48];
        bytes[0..4].copy_from_slice(&40u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&height.to_le_bytes());
        bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&24u16.to_le_bytes());
        bytes[40..48].copy_from_slice(&[1, 2, 3, 99, 5, 7, 11, 101]);
        let expected = if height > 0 {
            [5, 7, 11, 255, 1, 2, 3, 255]
        } else {
            [1, 2, 3, 255, 5, 7, 11, 255]
        };
        let budget = ResourceBudget::new(32, 0).unwrap();
        let decoded = decode_dib_with_budget(&bytes, false, &budget).unwrap();
        assert_eq!(decoded.image.pixels, expected);
        assert!(decoded.post_decode_error.is_none());
        drop(decoded);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
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
    assert_eq!(
        image_io::decode(&fs::read(first).unwrap()).unwrap(),
        image()
    );
    assert_eq!(
        image_io::decode(&fs::read(second).unwrap()).unwrap(),
        image()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn autosave_only_plan_executes_and_completes_with_readable_png() {
    let dir = temp_dir();
    let settings = Settings {
        open_editor: false,
        auto_save: true,
        save_folder: Some(dir.clone()),
        ..Settings::default()
    };
    let mut plan = CaptureOutputState::new(&settings);

    assert_eq!(
        advance_output(
            &mut plan,
            &settings,
            &image(),
            #[cfg(windows)]
            std::ptr::null_mut()
        )
        .unwrap(),
        OutputProgress::Advanced(Advance::Complete)
    );
    assert_eq!(
        advance_output(
            &mut plan,
            &settings,
            &image(),
            #[cfg(windows)]
            std::ptr::null_mut()
        )
        .unwrap(),
        OutputProgress::Complete
    );

    let mut entries = fs::read_dir(&dir).unwrap();
    let saved_path = entries.next().unwrap().unwrap().path();
    assert!(entries.next().is_none());
    assert_eq!(
        image_io::decode(&fs::read(saved_path).unwrap()).unwrap(),
        image()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn final_path_stays_absent_until_temp_file_is_complete_and_synced() {
    let dir = temp_dir();
    let temp_path = dir.join(".partial.tmp");
    let final_path = dir.join("complete.png");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .unwrap();
    let png = image_io::encode(&image(), image::ImageFormat::Png).unwrap();
    write_and_publish(&temp_path, &final_path, file, |file| {
        file.write_all(&png)?;
        assert!(!final_path.exists());
        assert_eq!(fs::read(&temp_path)?, png);
        file.sync_all()
    })
    .unwrap();
    assert!(final_path.exists());
    assert!(!temp_path.exists());
    assert_eq!(
        image_io::decode(&fs::read(&final_path).unwrap()).unwrap(),
        image()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_partial_write_removes_temp_without_publishing_final_path() {
    let dir = temp_dir();
    let temp_path = dir.join(".partial.tmp");
    let final_path = dir.join("partial.png");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .unwrap();
    let error = write_and_publish(&temp_path, &final_path, file, |file| {
        file.write_all(b"partial")?;
        Err(io::Error::other("injected write failure"))
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(!temp_path.exists());
    assert!(!final_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn publish_collision_preserves_existing_file_and_cleans_temp() {
    let dir = temp_dir();
    let temp_path = dir.join(".collision.tmp");
    let final_path = dir.join("capture.png");
    fs::write(&final_path, b"existing capture").unwrap();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .unwrap();
    let error = write_and_publish(&temp_path, &final_path, file, |file| {
        file.write_all(b"new capture")?;
        file.sync_all()
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&final_path).unwrap(), b"existing capture");
    assert!(!temp_path.exists());
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
