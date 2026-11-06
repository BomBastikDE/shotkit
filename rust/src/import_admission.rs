//! Portable, opt-in import ownership; this module installs no application hooks.
//!
//! Decode with `image_io::decode_file_with_budget` using `budget()`, then pass
//! the owned Image and its lease to `try_begin`. Rejection returns both intact.
//! A private-field `TrackedImage` owns image AND lease, shared through Arc. Its
//! read-only API cannot extract the pixels or remove the lease: even unwrapping
//! an Arc moves both together. The last tracked owner releases the reservation
//! after dropping its pixels, without a Weak ledger, collection or owner-life
//! contract. Deep-cloning the exposed Image creates a separate allocation that
//! callers must budget independently, as with any other new allocation.
//!
//! The exclusive slot is Initializing while an editor is being created outside
//! the mutex. Nested/concurrent recovery and discard cannot take that slot.
//! Complete only after external handoff succeeds. Failure, early return and
//! unwinding leave the slot Recoverable; recover/discard are explicit actions.
//! Dropping the service itself ends its pending ownership, while any external
//! tracked handles retain their own reservation safely.
//!
//! Later app integration MUST explicitly adopt Arc<TrackedImage> (or another
//! ownership-preserving adapter) instead of Arc<Image>. This foundation does
//! not activate imports or retrofit existing document/editor image ownership.
//! It counts the supplied pixel lease, not Arc metadata, editor allocations,
//! decoder policy allowances or RSS. The only new allocation is Arc::new; Rust
//! has no stable fallible Arc constructor, so this is not an OOM guarantee.
//! Slot transitions need no dynamically growing container and do not allocate.

use crate::resource_budget::{AdmissionClass, BudgetLease, ResourceBudget};
use crate::stitch::Image;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingState {
    Empty,
    Initializing,
    Recoverable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    NoRecoverableImport,
    DifferentBudget,
    InsufficientLease,
    InvalidImage,
}

/// Ownership is returned intact on every rejected admission.
#[derive(Debug)]
pub struct RejectedImport {
    pub error: AdmissionError,
    pub image: Image,
    pub lease: BudgetLease,
}

/// Read-only tracked allocation. Fields stay private; there is deliberately no
/// Image extraction, mutable Image access, Clone or DerefMut implementation.
/// Arc::try_unwrap/get_mut remain safe: they cannot separate pixels and lease.
/// Field declaration order ensures pixels are dropped before the reservation.
#[derive(Debug)]
pub struct TrackedImage {
    image: Image,
    lease: BudgetLease,
}

impl TrackedImage {
    pub fn image(&self) -> &Image {
        &self.image
    }

    pub fn reserved_bytes(&self) -> usize {
        self.lease.bytes()
    }
}

struct Pending {
    image: Arc<TrackedImage>,
    state: PendingState,
}

/// Share this owner and its budget; a second owner has a second pending slot.
/// The single mutex protects only short state changes, never editor work.
pub struct ImportAdmission {
    budget: ResourceBudget,
    pending: Mutex<Option<Pending>>,
}

impl ImportAdmission {
    pub fn new(budget: ResourceBudget) -> Self {
        Self {
            budget,
            pending: Mutex::new(None),
        }
    }

    pub fn budget(&self) -> &ResourceBudget {
        &self.budget
    }

    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn pending_state(&self) -> PendingState {
        self.lock()
            .as_ref()
            .map_or(PendingState::Empty, |pending| pending.state)
    }

    /// Accept exclusively owned pixels and a sufficient lease from this pool.
    /// Validation finishes before the slot changes. Arc allocation follows
    /// validation and precedes the Initializing transition; there is no ledger
    /// or later container reservation that could fail during complete/discard.
    pub fn try_begin(
        &self,
        image: Image,
        mut lease: BudgetLease,
    ) -> Result<Initialization<'_>, RejectedImport> {
        let mut pending = self.lock();
        let error = if pending.is_some() {
            Some(AdmissionError::Busy)
        } else if image.validate().is_err() {
            Some(AdmissionError::InvalidImage)
        } else if lease.bytes() < image.pixels.capacity() {
            Some(AdmissionError::InsufficientLease)
        } else {
            // A zero transfer checks budget identity without changing amounts.
            let mut probe = self
                .budget
                .try_acquire(0, AdmissionClass::Normal)
                .expect("zero-byte reservation cannot fail");
            lease
                .transfer_to(&mut probe, 0)
                .err()
                .map(|_| AdmissionError::DifferentBudget)
        };
        if let Some(error) = error {
            return Err(RejectedImport {
                error,
                image,
                lease,
            });
        }
        let image = Arc::new(TrackedImage { image, lease });
        *pending = Some(Pending {
            image: Arc::clone(&image),
            state: PendingState::Initializing,
        });
        Ok(Initialization {
            owner: self,
            image,
            active: true,
        })
    }

    pub fn try_recover(&self) -> Result<Initialization<'_>, AdmissionError> {
        let mut slot = self.lock();
        match slot.as_mut() {
            Some(pending) if pending.state == PendingState::Recoverable => {
                pending.state = PendingState::Initializing;
                Ok(Initialization {
                    owner: self,
                    image: Arc::clone(&pending.image),
                    active: true,
                })
            }
            Some(_) => Err(AdmissionError::Busy),
            None => Err(AdmissionError::NoRecoverableImport),
        }
    }

    /// Explicitly resolve a Recoverable import. Handles already retained by an
    /// external editor continue to own their reservation until their last drop.
    pub fn discard_recoverable(&self) -> Result<(), AdmissionError> {
        let discarded = {
            let mut slot = self.lock();
            match slot.as_ref() {
                Some(pending) if pending.state == PendingState::Recoverable => {}
                Some(_) => return Err(AdmissionError::Busy),
                None => return Err(AdmissionError::NoRecoverableImport),
            }
            slot.take()
        };
        drop(discarded);
        Ok(())
    }
}

/// RAII protects the pending import, not an editor's state. Dropping an active
/// guard makes it Recoverable and retains the tracked image in the owner.
#[must_use = "complete initialization or leave the import recoverable"]
pub struct Initialization<'a> {
    owner: &'a ImportAdmission,
    image: Arc<TrackedImage>,
    active: bool,
}

impl Initialization<'_> {
    pub fn image(&self) -> &Arc<TrackedImage> {
        &self.image
    }

    /// Call only after successful external handoff. Removing the exclusive slot
    /// and returning its tracked handle requires no allocation or callback.
    pub fn complete(mut self) -> Arc<TrackedImage> {
        let pending = self.owner.lock().take().expect("guard owns pending import");
        debug_assert_eq!(pending.state, PendingState::Initializing);
        self.active = false;
        pending.image
    }

    /// Report an ordinary initialization error; Drop provides the same fallback.
    pub fn mark_recoverable(self) {
        drop(self);
    }
}

impl Drop for Initialization<'_> {
    fn drop(&mut self) {
        if self.active {
            if let Some(pending) = self.owner.lock().as_mut() {
                pending.state = PendingState::Recoverable;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_and_lease(budget: &ResourceBudget, byte: u8) -> (Image, BudgetLease) {
        let image = Image::new(1, 1, vec![byte; 4]).unwrap();
        let lease = budget
            .try_acquire(image.pixels.capacity(), AdmissionClass::Normal)
            .unwrap();
        (image, lease)
    }

    fn begin(owner: &ImportAdmission, byte: u8) -> Initialization<'_> {
        let (image, lease) = image_and_lease(owner.budget(), byte);
        owner.try_begin(image, lease).unwrap()
    }

    #[test]
    fn nested_recovery_and_discard_cannot_take_initializing_slot() {
        let owner = ImportAdmission::new(ResourceBudget::new(16, 0).unwrap());
        let guard = begin(&owner, 7);
        assert_eq!(owner.pending_state(), PendingState::Initializing);
        assert!(matches!(owner.try_recover(), Err(AdmissionError::Busy)));
        assert_eq!(owner.discard_recoverable(), Err(AdmissionError::Busy));
        guard.mark_recoverable();
        assert_eq!(owner.pending_state(), PendingState::Recoverable);
        let image = owner.try_recover().unwrap().complete();
        assert_eq!(owner.pending_state(), PendingState::Empty);
        assert_eq!(image.image().pixels, vec![7; 4]);
    }

    #[test]
    fn last_arc_drop_releases_immediately_without_owner_or_collection() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let owner = ImportAdmission::new(budget.clone());
        let first = begin(&owner, 1).complete();
        let last = Arc::clone(&first);
        drop(owner);
        drop(first);
        assert_eq!(budget.snapshot().used_bytes, 4);
        assert_eq!(last.reserved_bytes(), 4);
        drop(last);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn unwrap_moves_pixels_and_lease_together_even_after_service_drop() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let owner = ImportAdmission::new(budget.clone());
        let tracked = Arc::try_unwrap(begin(&owner, 9).complete()).unwrap();
        drop(owner);
        assert_eq!(tracked.image().pixels, vec![9; 4]);
        assert_eq!(budget.snapshot().used_bytes, 4);
        // Rewrapping and exclusive access do not separate image and lease.
        let mut wrapped = Arc::new(tracked);
        assert_eq!(Arc::get_mut(&mut wrapped).unwrap().reserved_bytes(), 4);
        drop(wrapped);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn second_import_preserves_existing_content_and_shared_ceiling() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let owner = ImportAdmission::new(budget.clone());
        let existing = begin(&owner, 11).complete();
        let second = begin(&owner, 22);
        assert!(budget.try_acquire(1, AdmissionClass::Normal).is_err());
        drop(second);
        assert_eq!(existing.image().pixels, vec![11; 4]);
        owner.discard_recoverable().unwrap();
        assert_eq!(budget.snapshot().used_bytes, 4);
        assert_eq!(existing.image().pixels, vec![11; 4]);
        let next = begin(&owner, 33).complete();
        assert_eq!(next.image().pixels, vec![33; 4]);
    }

    #[test]
    fn busy_admission_returns_ownership_without_replacing_pending() {
        let owner = ImportAdmission::new(ResourceBudget::new(16, 0).unwrap());
        let guard = begin(&owner, 1);
        let (image, lease) = image_and_lease(owner.budget(), 2);
        let rejected = owner.try_begin(image, lease).err().unwrap();
        assert_eq!(rejected.error, AdmissionError::Busy);
        assert_eq!(rejected.image.pixels, vec![2; 4]);
        assert_eq!(owner.budget().snapshot().used_bytes, 8);
        drop(rejected);
        assert_eq!(guard.image().image().pixels, vec![1; 4]);
    }

    #[test]
    fn unwind_and_explicit_discard_preserve_retained_editor_handle() {
        let owner = ImportAdmission::new(ResourceBudget::new(8, 0).unwrap());
        let mut escaped = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = begin(&owner, 3);
            escaped = Some(Arc::clone(guard.image()));
            panic!("editor initialization aborted");
        }));
        assert!(result.is_err());
        assert_eq!(owner.pending_state(), PendingState::Recoverable);
        assert_eq!(owner.budget().snapshot().used_bytes, 4);
        owner.discard_recoverable().unwrap();
        assert_eq!(owner.pending_state(), PendingState::Empty);
        assert_eq!(owner.budget().snapshot().used_bytes, 4);
        drop(escaped);
        assert_eq!(owner.budget().snapshot().used_bytes, 0);
    }

    #[test]
    fn concurrent_recovery_is_busy_and_repeated_failure_keeps_original_image() {
        let owner = ImportAdmission::new(ResourceBudget::new(8, 0).unwrap());
        let guard = begin(&owner, 9);
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert!(matches!(owner.try_recover(), Err(AdmissionError::Busy)));
                    assert_eq!(owner.discard_recoverable(), Err(AdmissionError::Busy));
                })
                .join()
                .unwrap();
        });
        drop(guard);
        let guard = owner.try_recover().unwrap();
        assert_eq!(guard.image().image().pixels, vec![9; 4]);
        guard.mark_recoverable();
        assert_eq!(owner.budget().snapshot().used_bytes, 4);
        owner.discard_recoverable().unwrap();
        assert_eq!(owner.budget().snapshot().used_bytes, 0);
        assert!(matches!(
            owner.try_recover(),
            Err(AdmissionError::NoRecoverableImport)
        ));
    }

    #[test]
    fn invalid_underreserved_and_foreign_admissions_return_original_ownership() {
        let owner = ImportAdmission::new(ResourceBudget::new(8, 0).unwrap());
        let foreign = ResourceBudget::new(8, 0).unwrap();
        let (image, lease) = image_and_lease(&foreign, 1);
        let rejected = owner.try_begin(image, lease).err().unwrap();
        assert_eq!(rejected.error, AdmissionError::DifferentBudget);
        assert_eq!(foreign.snapshot().used_bytes, 4);
        drop(rejected);
        assert_eq!(foreign.snapshot().used_bytes, 0);
        let image = Image::new(1, 1, vec![5; 4]).unwrap();
        let lease = owner
            .budget()
            .try_acquire(3, AdmissionClass::Normal)
            .unwrap();
        let rejected = owner.try_begin(image, lease).err().unwrap();
        assert_eq!(rejected.error, AdmissionError::InsufficientLease);
        assert_eq!(rejected.lease.bytes(), 3);
        assert_eq!(rejected.image.pixels, vec![5; 4]);
        drop(rejected);
        let image = Image {
            width: 2,
            height: 1,
            pixels: vec![1; 4],
        };
        let lease = owner
            .budget()
            .try_acquire(4, AdmissionClass::Normal)
            .unwrap();
        let rejected = owner.try_begin(image, lease).err().unwrap();
        assert_eq!(rejected.error, AdmissionError::InvalidImage);
        assert_eq!(owner.pending_state(), PendingState::Empty);
        drop(rejected);
        assert_eq!(owner.budget().snapshot().used_bytes, 0);
    }
}
