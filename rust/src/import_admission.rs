//! Portable, opt-in import ownership; this module installs no application hooks.
//!
//! Reserve `try_reserve_decode()` BEFORE calling
//! `image_io::decode_file_with_budget` using `budget()`, then pass the owned
//! Image and lease to the ticket's `try_begin`. Decode error drops the ticket;
//! admission rejection returns both decoded values intact. Existing already-
//! decoded callers can still use the owner's `try_begin` convenience API.
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
    /// An exclusive ticket is decoding; no image has been admitted yet.
    Decoding,
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

enum Pending {
    Decoding,
    Image {
        image: Arc<TrackedImage>,
        state: PendingState,
    },
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
            .map_or(PendingState::Empty, |pending| match pending {
                Pending::Decoding => PendingState::Decoding,
                Pending::Image { state, .. } => *state,
            })
    }

    /// Claim the exclusive slot BEFORE decoding. A decoder can use `budget()`
    /// outside the mutex, then transfer its Image and lease with the ticket's
    /// `try_begin`. Dropping the ticket (including decode error/unwind) clears
    /// only its image-free reservation. A forgotten ticket remains Busy.
    pub fn try_reserve_decode(&self) -> Result<DecodeReservation<'_>, AdmissionError> {
        let mut pending = self.lock();
        if pending.is_some() {
            return Err(AdmissionError::Busy);
        }
        *pending = Some(Pending::Decoding);
        Ok(DecodeReservation {
            owner: self,
            active: true,
        })
    }

    /// Admit already-decoded owned pixels. Existing callers can keep this API;
    /// new decode paths should claim `try_reserve_decode` before doing work.
    /// Any rejection returns Image and lease unchanged to the caller.
    pub fn try_begin(
        &self,
        image: Image,
        lease: BudgetLease,
    ) -> Result<Initialization<'_>, RejectedImport> {
        match self.try_reserve_decode() {
            Ok(ticket) => ticket.try_begin(image, lease),
            Err(error) => Err(RejectedImport {
                error,
                image,
                lease,
            }),
        }
    }

    fn validate(&self, image: &Image, lease: &mut BudgetLease) -> Option<AdmissionError> {
        if image.validate().is_err() {
            return Some(AdmissionError::InvalidImage);
        }
        if lease.bytes() < image.pixels.capacity() {
            return Some(AdmissionError::InsufficientLease);
        }
        // A zero transfer checks budget identity without changing amounts.
        let mut probe = self
            .budget
            .try_acquire(0, AdmissionClass::Normal)
            .expect("zero-byte reservation cannot fail");
        lease
            .transfer_to(&mut probe, 0)
            .err()
            .map(|_| AdmissionError::DifferentBudget)
    }

    pub fn try_recover(&self) -> Result<Initialization<'_>, AdmissionError> {
        let mut slot = self.lock();
        match slot.as_mut() {
            Some(Pending::Image { image, state }) if *state == PendingState::Recoverable => {
                *state = PendingState::Initializing;
                Ok(Initialization {
                    owner: self,
                    image: Arc::clone(image),
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
                Some(Pending::Image {
                    state: PendingState::Recoverable,
                    ..
                }) => {}
                Some(_) => return Err(AdmissionError::Busy),
                None => return Err(AdmissionError::NoRecoverableImport),
            }
            slot.take()
        };
        drop(discarded);
        Ok(())
    }
}

/// Exclusive image-free decode reservation. Hold across decoding, outside the
/// mutex. On validation rejection `try_begin` returns the original Image and
/// lease, and releases this ticket's slot. The caller then explicitly chooses
/// retry, retention or disposal; no decoded result is silently consumed.
#[must_use = "hold the ticket across decoding, then admit or drop it"]
pub struct DecodeReservation<'a> {
    owner: &'a ImportAdmission,
    active: bool,
}

impl<'a> DecodeReservation<'a> {
    /// Validate before replacing the Decoding slot. Success transitions directly
    /// to Initializing; dropping its guard subsequently retains it Recoverable.
    /// Arc allocation precedes the transition. No callback or decoder runs under
    /// the mutex, and completion/discard need no container allocation.
    pub fn try_begin(
        mut self,
        image: Image,
        mut lease: BudgetLease,
    ) -> Result<Initialization<'a>, RejectedImport> {
        if let Some(error) = self.owner.validate(&image, &mut lease) {
            return Err(RejectedImport {
                error,
                image,
                lease,
            });
        }
        let mut pending = self.owner.lock();
        if !matches!(pending.as_ref(), Some(Pending::Decoding)) {
            return Err(RejectedImport {
                error: AdmissionError::Busy,
                image,
                lease,
            });
        }
        let image = Arc::new(TrackedImage { image, lease });
        *pending = Some(Pending::Image {
            image: Arc::clone(&image),
            state: PendingState::Initializing,
        });
        self.active = false;
        Ok(Initialization {
            owner: self.owner,
            image,
            active: true,
        })
    }
}

impl Drop for DecodeReservation<'_> {
    fn drop(&mut self) {
        if self.active {
            let mut pending = self.owner.lock();
            if matches!(pending.as_ref(), Some(Pending::Decoding)) {
                *pending = None;
            }
        }
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
        match pending {
            Pending::Image { image, state } => {
                debug_assert_eq!(state, PendingState::Initializing);
                self.active = false;
                image
            }
            Pending::Decoding => unreachable!("initialization guard owns image slot"),
        }
    }

    /// Report an ordinary initialization error; Drop provides the same fallback.
    pub fn mark_recoverable(self) {
        drop(self);
    }
}

impl Drop for Initialization<'_> {
    fn drop(&mut self) {
        if self.active {
            if let Some(Pending::Image { state, .. }) = self.owner.lock().as_mut() {
                *state = PendingState::Recoverable;
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
    fn decode_ticket_blocks_concurrent_and_nested_entry_until_drop() {
        let owner = ImportAdmission::new(ResourceBudget::new(16, 0).unwrap());
        let ticket = owner.try_reserve_decode().unwrap();
        assert_eq!(owner.pending_state(), PendingState::Decoding);
        assert!(matches!(owner.try_recover(), Err(AdmissionError::Busy)));
        assert_eq!(owner.discard_recoverable(), Err(AdmissionError::Busy));
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert!(matches!(
                        owner.try_reserve_decode(),
                        Err(AdmissionError::Busy)
                    ));
                    let (image, lease) = image_and_lease(owner.budget(), 2);
                    let rejected = owner.try_begin(image, lease).err().unwrap();
                    assert_eq!(rejected.error, AdmissionError::Busy);
                    assert_eq!(rejected.image.pixels, vec![2; 4]);
                })
                .join()
                .unwrap();
        });
        assert_eq!(owner.budget().snapshot().used_bytes, 0);
        drop(ticket); // e.g. a decoder returned Err before producing an image
        assert_eq!(owner.pending_state(), PendingState::Empty);
        drop(owner.try_reserve_decode().unwrap());
    }

    #[test]
    fn decode_unwind_releases_only_its_empty_reservation() {
        let owner = ImportAdmission::new(ResourceBudget::new(8, 0).unwrap());
        let existing = begin(&owner, 11).complete();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ticket = owner.try_reserve_decode().unwrap();
            let (_temporary, _lease) = image_and_lease(owner.budget(), 3);
            panic!("decode failed");
        }));
        assert!(result.is_err());
        assert_eq!(owner.pending_state(), PendingState::Empty);
        assert_eq!(owner.budget().snapshot().used_bytes, 4);
        assert_eq!(existing.image().pixels, vec![11; 4]);
    }

    #[test]
    fn ticket_success_transitions_to_initializing_then_retains_on_abort() {
        let owner = ImportAdmission::new(ResourceBudget::new(8, 0).unwrap());
        let ticket = owner.try_reserve_decode().unwrap();
        let (image, lease) = image_and_lease(owner.budget(), 5);
        let guard = ticket.try_begin(image, lease).unwrap();
        assert_eq!(owner.pending_state(), PendingState::Initializing);
        assert!(matches!(
            owner.try_reserve_decode(),
            Err(AdmissionError::Busy)
        ));
        drop(guard);
        assert_eq!(owner.pending_state(), PendingState::Recoverable);
        assert!(matches!(
            owner.try_reserve_decode(),
            Err(AdmissionError::Busy)
        ));
        let image = owner.try_recover().unwrap().complete();
        assert_eq!(image.image().pixels, vec![5; 4]);
        assert_eq!(owner.pending_state(), PendingState::Empty);
        drop(image);
        assert_eq!(owner.budget().snapshot().used_bytes, 0);
    }

    #[test]
    fn ticket_rejections_return_identical_decoded_allocation_and_lease() {
        let owner = ImportAdmission::new(ResourceBudget::new(16, 0).unwrap());
        let foreign = ResourceBudget::new(16, 0).unwrap();
        for expected in [
            AdmissionError::InvalidImage,
            AdmissionError::InsufficientLease,
            AdmissionError::DifferentBudget,
        ] {
            let ticket = owner.try_reserve_decode().unwrap();
            let mut pixels = Vec::with_capacity(8);
            pixels.extend_from_slice(&[1, 2, 3, 4]);
            let ptr = pixels.as_ptr();
            let image = Image {
                width: if expected == AdmissionError::InvalidImage {
                    2
                } else {
                    1
                },
                height: 1,
                pixels,
            };
            let bytes = if expected == AdmissionError::InsufficientLease {
                3
            } else {
                8
            };
            let pool = if expected == AdmissionError::DifferentBudget {
                &foreign
            } else {
                owner.budget()
            };
            let lease = pool.try_acquire(bytes, AdmissionClass::Reserved).unwrap();
            let rejected = ticket.try_begin(image, lease).err().unwrap();
            assert_eq!(rejected.error, expected);
            assert_eq!(rejected.image.pixels.as_ptr(), ptr);
            assert_eq!(rejected.image.pixels.capacity(), 8);
            assert_eq!(rejected.image.pixels, vec![1, 2, 3, 4]);
            assert_eq!(rejected.lease.bytes(), bytes);
            assert_eq!(rejected.lease.class(), AdmissionClass::Reserved);
            assert_eq!(pool.snapshot().used_bytes, bytes);
            assert_eq!(owner.pending_state(), PendingState::Empty);
            drop(rejected);
            assert_eq!(pool.snapshot().used_bytes, 0);
        }
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
