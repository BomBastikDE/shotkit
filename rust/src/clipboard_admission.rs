//! Budgeted clipboard handoff. Native reading and all Windows callbacks run
//! outside admission, budget and recovery mutexes. Claim both exclusive slots
//! before reading: every successfully decoded image has a recovery destination.
use crate::{
    image_io::BudgetedImage,
    import_admission::{ImportAdmission, Initialization},
    rejected_import::RejectedImportSlot,
    resource_budget::ResourceBudget,
};

/// `read` may try lower-priority formats only until it owns a decoded image.
/// Successful pixels with close/accounting failures travel in BudgetedImage,
/// never through Err. Complete the returned guard only after the full editor
/// handoff succeeds; its drop retains exact pixels and their lease for recovery.
pub fn acquire<'a>(
    admission: &'a ImportAdmission,
    rejected: &RejectedImportSlot,
    read: impl FnOnce(&ResourceBudget) -> Result<Option<BudgetedImage>, String>,
) -> Result<Option<Initialization<'a>>, String> {
    let ticket = admission.try_reserve_decode().map_err(|_| {
        "Ein Bildimport ist noch aktiv oder wartet auf Wiederherstellung.".to_owned()
    })?;
    let fallback = rejected
        .try_reserve()
        .map_err(|_| "Ein abgelehnter Bildimport wartet noch auf Wiederherstellung.".to_owned())?;
    let Some(decoded) = read(admission.budget())? else {
        return Ok(None);
    };
    let BudgetedImage {
        image,
        lease,
        post_decode_error,
    } = decoded;
    match ticket.try_begin(image, lease) {
        Ok(initialization) => {
            drop(fallback);
            if let Some(error) = post_decode_error {
                // Successful pixels are already owned by admission. Drop only
                // the guard, which leaves the same tracked allocation pending.
                drop(initialization);
                Err(format!(
                    "{error}. Das Zwischenablagebild bleibt zur Wiederherstellung im Speicher."
                ))
            } else {
                Ok(Some(initialization))
            }
        }
        Err(value) => {
            // Includes insufficient final capacity coverage. Retain the exact
            // image AND unchanged lease; never silently drop or decode again.
            fallback.store(value);
            let details = post_decode_error.unwrap_or_else(|| {
                "Das Zwischenablagebild konnte nicht übernommen werden".to_owned()
            });
            Err(format!(
                "{details}. Bild und Speicherreservierung bleiben zur Wiederherstellung erhalten."
            ))
        }
    }
}

/// Try native representations in the supplied order. A successful decode ends
/// fallback even if close or accounting failed; only image-free failures may
/// advance. `close` runs after all source borrows/native locks have ended.
pub fn read_formats<F: Copy>(
    formats: &[(F, &str)],
    mut available: impl FnMut(F) -> bool,
    mut decode: impl FnMut(F) -> Result<BudgetedImage, String>,
    close: impl FnOnce() -> Result<(), String>,
) -> Result<Option<BudgetedImage>, String> {
    let mut failures = Vec::new();
    for &(format, name) in formats {
        if !available(format) {
            continue;
        }
        match decode(format) {
            Ok(mut image) => {
                if let Err(error) = close() {
                    image.retain_error(error);
                }
                return Ok(Some(image));
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    close()?;
    if failures.is_empty() {
        Ok(None)
    } else {
        Err(format!(
            "Clipboard image formats were present but unreadable: {}",
            failures.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        image_handle::ImageHandle,
        import_admission::{AdmissionError, PendingState},
        rejected_import::RejectedImportState,
        resource_budget::AdmissionClass,
        stitch::Image,
    };
    use std::{cell::Cell, sync::Arc};

    const FORMATS: [(u32, &str); 4] = [(1, "PNG"), (2, "DIBV5"), (3, "DIB"), (4, "BITMAP")];

    fn decoded(budget: &ResourceBudget, value: u8) -> BudgetedImage {
        // Reserve before allocation, just as the native reader must.
        let lease = budget.try_acquire(4, AdmissionClass::Normal).unwrap();
        BudgetedImage::from_decoded(Image::new(1, 1, vec![value, 2, 3, 0]).unwrap(), lease)
    }

    #[test]
    fn format_fallback_advances_only_before_success_and_preserves_close_failure() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let closes = Cell::new(0);
        let mut attempts = Vec::new();
        let mut expected_ptr = std::ptr::null();
        let image = read_formats(
            &FORMATS,
            |_| true,
            |format| {
                attempts.push(format);
                match format {
                    1 => Err("malformed PNG".into()),
                    2 => {
                        let image = decoded(&budget, 17);
                        expected_ptr = image.image.pixels.as_ptr();
                        Ok(image)
                    }
                    _ => panic!("fallback after owned success"),
                }
            },
            || {
                closes.set(closes.get() + 1);
                Err("CloseClipboard failed".into())
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(attempts, [1, 2]);
        assert_eq!(closes.get(), 1);
        assert_eq!(image.image.pixels.as_ptr(), expected_ptr);
        assert_eq!(image.image.pixels, [17, 2, 3, 0]);
        assert!(image
            .post_decode_error
            .as_deref()
            .unwrap()
            .contains("CloseClipboard"));
        assert_eq!(image.lease.bytes(), image.image.pixels.capacity());
        drop(image);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn unavailable_and_failed_formats_close_once_without_pixels() {
        let closes = Cell::new(0);
        assert!(read_formats(
            &FORMATS,
            |_| false,
            |_| panic!("unavailable format decoded"),
            || {
                closes.set(closes.get() + 1);
                Ok(())
            },
        )
        .unwrap()
        .is_none());
        let mut attempts = Vec::new();
        let error = read_formats(
            &FORMATS,
            |_| true,
            |format| {
                attempts.push(format);
                Err("unreadable".into())
            },
            || {
                closes.set(closes.get() + 1);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(attempts, [1, 2, 3, 4]);
        assert!(error.contains("PNG: unreadable"));
        assert!(error.contains("BITMAP: unreadable"));
        assert_eq!(closes.get(), 2);
    }

    #[test]
    fn png_budget_refusal_can_fall_back_to_budgeted_dib_before_owning_pixels() {
        let original = Image::new(1, 1, vec![17, 31, 53, 128]).unwrap();
        let png = crate::image_io::encode(&original, image::ImageFormat::Png).unwrap();
        let dib = crate::output_windows::encode_dibv5(&original).unwrap();
        let budget = ResourceBudget::new(32, 0).unwrap();
        let mut attempts = Vec::new();
        let decoded = read_formats(
            &FORMATS,
            |format| format <= 2,
            |format| {
                attempts.push(format);
                if format == 1 {
                    crate::image_io::decode_with_budget(&png, &budget)
                } else {
                    crate::output_windows::decode_dib_with_budget(&dib, true, &budget)
                }
            },
            || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(attempts, [1, 2]);
        assert_eq!(decoded.image, original);
        assert!(decoded.post_decode_error.is_none());
        assert_eq!(
            budget.snapshot().used_bytes,
            decoded.image.pixels.capacity()
        );
        drop(decoded);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn decode_and_close_callbacks_hold_no_admission_or_recovery_mutex() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let initialization = acquire(&admission, &rejected, |budget| {
            read_formats(
                &FORMATS,
                |_| true,
                |_| {
                    assert!(matches!(
                        admission.try_reserve_decode(),
                        Err(AdmissionError::Busy)
                    ));
                    assert!(rejected.try_reserve().is_err());
                    assert!(acquire(&admission, &rejected, |_| panic!("nested read")).is_err());
                    Ok(decoded(budget, 11))
                },
                || {
                    // These reentrant queries would deadlock under a mutex.
                    assert_eq!(admission.pending_state(), PendingState::Decoding);
                    assert_eq!(rejected.state(), RejectedImportState::Reserved);
                    assert_eq!(budget.snapshot().used_bytes, 4);
                    assert!(matches!(admission.try_recover(), Err(AdmissionError::Busy)));
                    Ok(())
                },
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(initialization.image().image().pixels, [11, 2, 3, 0]);
        drop(initialization.complete());
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn close_failure_retains_exact_image_then_cancel_failed_save_and_unwind_keep_it() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let mut expected_ptr = std::ptr::null();
        let result = acquire(&admission, &rejected, |budget| {
            read_formats(
                &FORMATS,
                |_| true,
                |_| {
                    let image = decoded(budget, 23);
                    expected_ptr = image.image.pixels.as_ptr();
                    Ok(image)
                },
                || Err("CloseClipboard failed".into()),
            )
        });
        assert!(matches!(result, Err(ref error) if error.contains("CloseClipboard")));
        assert_eq!(admission.pending_state(), PendingState::Recoverable);
        assert_eq!(rejected.state(), RejectedImportState::Empty);
        assert!(acquire(&admission, &rejected, |_| panic!("new image over recovery")).is_err());
        let recovery = admission.try_recover().unwrap();
        assert_eq!(recovery.image().image().pixels.as_ptr(), expected_ptr);
        assert_eq!(recovery.image().image().pixels, [23, 2, 3, 0]);
        drop(recovery); // user cancels
        fn failed_save(admission: &ImportAdmission) -> Result<(), &'static str> {
            let _recovery = admission.try_recover().unwrap();
            Err("injected save failure")
        }
        assert!(failed_save(&admission).is_err());
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _recovery = admission.try_recover().unwrap();
            panic!("save interrupted");
        }));
        assert!(unwind.is_err());
        let recovery = admission.try_recover().unwrap();
        assert_eq!(recovery.image().image().pixels.as_ptr(), expected_ptr);
        assert_eq!(budget.snapshot().used_bytes, 4);
        drop(recovery.complete()); // explicit successful resolution
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn refused_capacity_growth_retains_exact_pixels_and_original_lease_without_fallback() {
        let budget = ResourceBudget::new(4, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let mut expected_ptr = std::ptr::null();
        let mut expected_capacity = 0;
        let result = acquire(&admission, &rejected, |budget| {
            read_formats(
                &FORMATS,
                |_| true,
                |format| {
                    assert_eq!(format, 1, "accounting failure cannot cause fallback");
                    let lease = budget.try_acquire(4, AdmissionClass::Normal).unwrap();
                    // Inject decoder capacity beyond its admitted prediction.
                    let mut pixels = Vec::with_capacity(8);
                    pixels.extend_from_slice(&[31, 2, 3, 0]);
                    expected_ptr = pixels.as_ptr();
                    expected_capacity = pixels.capacity();
                    Ok(BudgetedImage::from_decoded(
                        Image::new(1, 1, pixels).unwrap(),
                        lease,
                    ))
                },
                || Err("CloseClipboard failed".into()),
            )
        });
        assert!(matches!(result, Err(ref error)
            if error.contains("verbucht") && error.contains("CloseClipboard")));
        assert_eq!(admission.pending_state(), PendingState::Empty);
        assert_eq!(rejected.state(), RejectedImportState::Recoverable);
        let recovery = rejected.try_recover().unwrap();
        assert_eq!(recovery.error(), AdmissionError::InsufficientLease);
        assert_eq!(recovery.image().pixels.as_ptr(), expected_ptr);
        assert_eq!(recovery.image().pixels.capacity(), expected_capacity);
        assert_eq!(recovery.image().pixels, [31, 2, 3, 0]);
        assert_eq!(recovery.reserved_bytes(), 4);
        drop(recovery); // failed/cancelled recovery does not lose underreserved data
        assert_eq!(budget.snapshot().used_bytes, 4);
        rejected.try_recover().unwrap().commit_saved();
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn no_image_decode_failure_and_preexisting_rejected_slot_release_only_empty_reservations() {
        let budget = ResourceBudget::new(4, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        assert!(acquire(&admission, &rejected, |_| Ok(None))
            .unwrap()
            .is_none());
        assert!(acquire(&admission, &rejected, |_| Err("decode failed".into())).is_err());
        assert_eq!(admission.pending_state(), PendingState::Empty);
        assert_eq!(budget.snapshot().used_bytes, 0);
        let reserved = rejected.try_reserve().unwrap();
        assert!(acquire(&admission, &rejected, |_| panic!(
            "read without recovery destination"
        ))
        .is_err());
        assert_eq!(admission.pending_state(), PendingState::Empty);
        drop(reserved);
    }

    #[test]
    fn failed_editor_handoff_retains_shared_handle_and_lease_until_last_owner() {
        let budget = ResourceBudget::new(4, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let initialization = acquire(&admission, &rejected, |budget| {
            Ok(Some(decoded(budget, 37)))
        })
        .unwrap()
        .unwrap();
        let handle = ImageHandle::from_imported(Arc::clone(initialization.image()));
        let ptr = handle.pixels.as_ptr();
        drop(initialization); // editor setup failed after taking its handle
        let recovery = admission.try_recover().unwrap();
        assert_eq!(recovery.image().image().pixels.as_ptr(), ptr);
        drop(recovery.complete()); // explicitly saved, editor still owns handle
        assert_eq!(budget.snapshot().used_bytes, 4);
        assert!(acquire(&admission, &rejected, |budget| {
            // No replacement: actual next allocation is refused before decode.
            let _lease = budget
                .try_acquire(4, AdmissionClass::Normal)
                .map_err(|error| error.to_string())?;
            panic!("exhausted budget admitted another buffer")
        })
        .is_err());
        assert_eq!(handle.pixels, [37, 2, 3, 0]);
        drop(handle);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
}
