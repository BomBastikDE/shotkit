//! Admission for retained ordinary screenshot pixels only. The caller shares
//! the import ResourceBudget and supplies a native fill callback. Scroll,
//! clipboard, editing, GDI allocations and total process RSS are outside scope.
//! No callback runs under a budget, admission or recovery mutex.
use crate::{
    capture_buffer::{bgra_len, try_bgra_buffer},
    import_admission::{ImportAdmission, Initialization},
    rejected_import::RejectedImportSlot,
    resource_budget::AdmissionClass,
    stitch::Image,
};

/// Claim both fallback destinations and pixel bytes BEFORE allocation/native
/// acquisition. Account for actual Vec capacity before invoking the callback.
/// A successful fill is retained even if later admission/handoff fails. The
/// returned initialization guard must complete only after every output/handoff
/// has succeeded; early return or unwind retains its Image and lease together.
pub fn acquire<'a>(
    admission: &'a ImportAdmission,
    rejected: &RejectedImportSlot,
    width: usize,
    height: usize,
    fill: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<Initialization<'a>, String> {
    acquire_with_buffer(
        admission,
        rejected,
        width,
        height,
        || try_bgra_buffer(width, height).map_err(str::to_owned),
        fill,
    )
}

fn acquire_with_buffer<'a>(
    admission: &'a ImportAdmission,
    rejected: &RejectedImportSlot,
    width: usize,
    height: usize,
    allocate: impl FnOnce() -> Result<Vec<u8>, String>,
    fill: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<Initialization<'a>, String> {
    let bytes = bgra_len(width, height)?;
    let ticket = admission.try_reserve_decode().map_err(|_| {
        "Eine Aufnahme ist noch aktiv oder wartet auf Wiederherstellung.".to_owned()
    })?;
    let fallback = rejected
        .try_reserve()
        .map_err(|_| "Eine Aufnahme wartet noch auf Wiederherstellung.".to_owned())?;
    let mut lease = admission
        .budget()
        .try_acquire(bytes, AdmissionClass::Normal)
        .map_err(|error| format!("Kein freies Bildbudget für eine weitere Aufnahme: {error}"))?;
    let mut pixels = allocate()?;
    if pixels.len() != bytes {
        return Err("Aufnahmepuffer passt nicht zu den Bildschirmgrenzen.".into());
    }
    // try_reserve_exact may supply spare capacity. Reject before screen
    // acquisition if the allocator's actual capacity needs unavailable bytes.
    lease.try_resize(pixels.capacity()).map_err(|error| {
        format!("Der Aufnahmepuffer überschreitet das freie Bildbudget: {error}")
    })?;
    fill(&mut pixels)?;
    // Geometry and initialized length are fixed before fill, which can mutate
    // bytes but cannot resize the slice. Do not insert fallible transformations
    // between successful acquisition and admission.
    let image = Image {
        width,
        height,
        pixels,
    };
    match ticket.try_begin(image, lease) {
        Ok(initialization) => Ok(initialization),
        Err(value) => {
            fallback.store(value);
            Err("Die Aufnahme konnte nicht übernommen werden. Bild und Speicherreservierung bleiben zur Wiederherstellung erhalten.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        image_handle::ImageHandle,
        import_admission::{AdmissionError, PendingState},
        resource_budget::ResourceBudget,
    };
    use std::sync::Arc;

    #[test]
    fn retained_capture_blocks_next_acquisition_before_callback_and_releases_on_last_owner() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let first = acquire(&admission, &rejected, 2, 1, |pixels| {
            assert_eq!(budget.snapshot().used_bytes, pixels.len());
            pixels.fill(17);
            // These calls would deadlock if a mutex crossed the callback.
            assert!(matches!(
                admission.try_reserve_decode(),
                Err(AdmissionError::Busy)
            ));
            assert!(acquire(&admission, &rejected, 1, 1, |_| panic!("nested fill")).is_err());
            Ok(())
        })
        .unwrap();
        let handle = ImageHandle::from_imported(first.complete());
        let clone = handle.clone();
        assert!(acquire(&admission, &rejected, 1, 1, |_| panic!(
            "budget rejection filled pixels"
        ))
        .is_err());
        assert_eq!(admission.pending_state(), PendingState::Empty);
        assert_eq!(handle.pixels, vec![17; 8]);
        drop(handle);
        assert_eq!(budget.snapshot().used_bytes, 8);
        drop(clone);
        assert_eq!(budget.snapshot().used_bytes, 0);
        drop(
            acquire(&admission, &rejected, 1, 1, |_| Ok(()))
                .unwrap()
                .complete(),
        );
    }

    #[test]
    fn failed_handoff_unwind_and_recovery_cancellation_preserve_same_allocation() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let mut escaped = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = acquire(&admission, &rejected, 1, 1, |pixels| {
                pixels.copy_from_slice(&[1, 2, 3, 255]);
                Ok(())
            })
            .unwrap();
            escaped = Some(ImageHandle::from_imported(Arc::clone(guard.image())));
            panic!("handoff failed");
        }));
        assert!(result.is_err());
        assert_eq!(admission.pending_state(), PendingState::Recoverable);
        let recovery = admission.try_recover().unwrap();
        let original = escaped.as_ref().unwrap();
        assert!(std::ptr::eq(recovery.image().image(), original.image()));
        drop(recovery); // cancelled save
        assert_eq!(budget.snapshot().used_bytes, 4);
        admission.discard_recoverable().unwrap(); // explicit resolution
        assert_eq!(budget.snapshot().used_bytes, 4);
        drop(escaped);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn failed_native_fill_and_invalid_geometry_release_empty_reservations() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        assert!(acquire(&admission, &rejected, 0, 1, |_| panic!("invalid size")).is_err());
        assert!(acquire(
            &admission,
            &rejected,
            1,
            1,
            |_| Err("native failure".into())
        )
        .is_err());
        assert_eq!(admission.pending_state(), PendingState::Empty);
        assert_eq!(budget.snapshot().used_bytes, 0);
        drop(
            acquire(&admission, &rejected, 1, 1, |_| Ok(()))
                .unwrap()
                .complete(),
        );
    }

    #[test]
    fn import_and_capture_share_the_same_admission_ceiling() {
        let budget = ResourceBudget::new(12, 4).unwrap();
        let imports = ImportAdmission::new(budget.clone());
        let captures = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let imported = imports
            .try_begin(
                Image::new(1, 1, vec![9; 4]).unwrap(),
                budget.try_acquire(4, AdmissionClass::Normal).unwrap(),
            )
            .unwrap()
            .complete();
        let capture = acquire(&captures, &rejected, 1, 1, |_| Ok(()))
            .unwrap()
            .complete();
        assert!(acquire(&captures, &rejected, 1, 1, |_| panic!("normal headroom")).is_err());
        assert_eq!(imported.image().pixels, [9; 4]);
        assert_eq!(budget.snapshot().used_bytes, 8);
        drop(capture);
        drop(imported);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
    #[test]
    fn capture_lease_follows_document_undo_and_redo_without_counting_shared_clones_twice() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        let guard = acquire(&admission, &rejected, 2, 1, |pixels| {
            pixels.fill(31);
            Ok(())
        })
        .unwrap();
        let source = ImageHandle::from_imported(guard.complete());
        let identity = source.identity();
        let mut document = crate::document::Document::from_image_handle(source).unwrap();
        document.begin_edit().unwrap();
        document
            .replace_image(Image::new(1, 1, vec![77; 4]).unwrap())
            .unwrap();
        assert!(document.commit_edit().unwrap());
        assert_eq!(document.undo_image_arc().unwrap().identity(), identity);
        assert_eq!(budget.snapshot().used_bytes, 8);
        assert!(document.undo());
        assert_eq!(document.image_arc().identity(), identity);
        assert!(document.redo());
        assert_eq!(document.undo_image_arc().unwrap().identity(), identity);
        drop(admission);
        assert_eq!(budget.snapshot().used_bytes, 8);
        drop(document);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn allocator_spare_capacity_must_be_admitted_before_native_fill() {
        let budget = ResourceBudget::new(4, 0).unwrap();
        let admission = ImportAdmission::new(budget.clone());
        let rejected = RejectedImportSlot::new();
        assert!(acquire_with_buffer(
            &admission,
            &rejected,
            1,
            1,
            || {
                assert_eq!(budget.snapshot().used_bytes, 4); // before allocation
                let mut pixels = Vec::with_capacity(8);
                pixels.resize(4, 0);
                Ok(pixels)
            },
            |_| panic!("capacity rejection must precede screen acquisition")
        )
        .is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
        assert_eq!(admission.pending_state(), PendingState::Empty);
        drop(
            acquire(&admission, &rejected, 1, 1, |_| Ok(()))
                .unwrap()
                .complete(),
        );
    }
}
