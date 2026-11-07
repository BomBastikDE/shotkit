//! One fixed, opt-in recovery slot for an admission's rejected owned import.
//!
//! A recovery guard holds the full value outside the mutex during save I/O.
//! Early return and unwinding restore it; only `commit_saved` or explicit
//! `discard` resolve it. The slot remains busy throughout recovery, including
//! nested and parallel attempts. Transitions move values without allocation.
//!
//! Reserve with `try_reserve` BEFORE decoding when a later rejection must have
//! a guaranteed destination. Its image-free RAII reservation blocks all other
//! entrants until dropped or consumed by infallible `SlotReservation::store`.
//! No decoder or callback runs under the slot mutex.
//!
//! Callers must handle `try_store`'s returned value on Busy: this slot cannot
//! retain two rejected imports. It neither validates nor changes the original
//! admission error, image or lease. Read-only Image access can still be cloned
//! by the caller; such new allocations require their own budget. Dropping the
//! slot ends its ownership. Forgetting a guard leaks its value and keeps the
//! slot busy; panic=abort/process termination cannot run recovery destructors.

use crate::import_admission::{AdmissionError, RejectedImport};
use crate::stitch::Image;
use std::sync::{Mutex, MutexGuard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectedImportState {
    Empty,
    Reserved,
    Recoverable,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Empty,
    Busy,
}

enum Slot {
    Empty,
    Reserved,
    Recoverable(RejectedImport),
    Recovering,
}

/// One slot per instance. Share this instance to coordinate concurrent callers.
pub struct RejectedImportSlot {
    slot: Mutex<Slot>,
}

impl Default for RejectedImportSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl RejectedImportSlot {
    pub const fn new() -> Self {
        Self {
            slot: Mutex::new(Slot::Empty),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn state(&self) -> RejectedImportState {
        match &*self.lock() {
            Slot::Empty => RejectedImportState::Empty,
            Slot::Reserved => RejectedImportState::Reserved,
            Slot::Recoverable(_) => RejectedImportState::Recoverable,
            Slot::Recovering => RejectedImportState::Recovering,
        }
    }

    /// Claim the sole recovery destination before decoding. No image or lease
    /// is retained yet; dropping the ticket cancels this empty reservation.
    pub fn try_reserve(&self) -> Result<SlotReservation<'_>, RecoveryError> {
        let mut slot = self.lock();
        if !matches!(*slot, Slot::Empty) {
            return Err(RecoveryError::Busy);
        }
        *slot = Slot::Reserved;
        Ok(SlotReservation {
            owner: self,
            active: true,
        })
    }

    /// Busy returns the exact original value, including its admission error.
    #[allow(clippy::result_large_err)] // Returning ownership is the contract.
    pub fn try_store(&self, rejected: RejectedImport) -> Result<(), RejectedImport> {
        let mut slot = self.lock();
        if !matches!(*slot, Slot::Empty) {
            return Err(rejected);
        }
        *slot = Slot::Recoverable(rejected);
        Ok(())
    }

    pub fn try_recover(&self) -> Result<RecoveryGuard<'_>, RecoveryError> {
        let mut slot = self.lock();
        match &*slot {
            Slot::Empty => return Err(RecoveryError::Empty),
            Slot::Reserved | Slot::Recovering => return Err(RecoveryError::Busy),
            Slot::Recoverable(_) => {}
        }
        let Slot::Recoverable(rejected) = std::mem::replace(&mut *slot, Slot::Recovering) else {
            unreachable!("slot was checked under the same lock")
        };
        Ok(RecoveryGuard {
            owner: self,
            rejected: Some(rejected),
        })
    }

    /// Explicitly discard a pending value. Active recovery cannot be discarded.
    pub fn discard(&self) -> Result<(), RecoveryError> {
        let rejected = {
            let mut slot = self.lock();
            match &*slot {
                Slot::Empty => return Err(RecoveryError::Empty),
                Slot::Reserved | Slot::Recovering => return Err(RecoveryError::Busy),
                Slot::Recoverable(_) => {}
            }
            std::mem::replace(&mut *slot, Slot::Empty)
        };
        // RejectedImport declares Image before BudgetLease: pixels drop first.
        // Destruction happens outside the slot mutex.
        drop(rejected);
        Ok(())
    }
}

/// Exclusive, image-free destination for an import rejected after decode.
/// All fields are private: safe callers cannot forge or duplicate this ticket.
#[must_use = "hold the reservation across decode, then store a rejection or drop it"]
pub struct SlotReservation<'a> {
    owner: &'a RejectedImportSlot,
    active: bool,
}

impl SlotReservation<'_> {
    /// Atomically consume the exclusive reservation. This cannot be Busy and
    /// needs no allocation: only this ticket can replace its Reserved state.
    /// The complete original rejected value becomes recoverable unchanged.
    pub fn store(mut self, rejected: RejectedImport) {
        let mut slot = self.owner.lock();
        *slot = Slot::Recoverable(rejected);
        self.active = false;
    }
}

impl Drop for SlotReservation<'_> {
    fn drop(&mut self) {
        if self.active {
            *self.owner.lock() = Slot::Empty;
        }
    }
}

#[must_use = "dropping recovery restores the rejected import; commit only after saving"]
pub struct RecoveryGuard<'a> {
    owner: &'a RejectedImportSlot,
    rejected: Option<RejectedImport>,
}

impl RecoveryGuard<'_> {
    pub fn image(&self) -> &Image {
        &self.rejected.as_ref().expect("active recovery").image
    }

    pub fn error(&self) -> AdmissionError {
        self.rejected.as_ref().expect("active recovery").error
    }

    pub fn reserved_bytes(&self) -> usize {
        self.rejected
            .as_ref()
            .expect("active recovery")
            .lease
            .bytes()
    }

    /// Call only after the external save succeeded. No I/O is performed here.
    pub fn commit_saved(mut self) {
        self.resolve();
    }

    /// Explicit user/caller decision to release the rejected image unsaved.
    pub fn discard(mut self) {
        self.resolve();
    }

    fn resolve(&mut self) {
        let rejected = self.rejected.take();
        // Keep the slot busy until pixels and reservation have both gone.
        // Image precedes lease in RejectedImport's field drop order.
        drop(rejected);
        *self.owner.lock() = Slot::Empty;
    }
}

impl Drop for RecoveryGuard<'_> {
    fn drop(&mut self) {
        if let Some(rejected) = self.rejected.take() {
            // No other API can replace Recovering, so this cannot evict data.
            *self.owner.lock() = Slot::Recoverable(rejected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_budget::{AdmissionClass, ResourceBudget};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn rejected(budget: &ResourceBudget, byte: u8) -> RejectedImport {
        let image = Image::new(1, 1, vec![byte; 4]).unwrap();
        let lease = budget
            .try_acquire(image.pixels.capacity(), AdmissionClass::Normal)
            .unwrap();
        RejectedImport {
            image,
            lease,
            error: AdmissionError::DifferentBudget,
        }
    }

    #[test]
    fn reservation_excludes_nested_and_parallel_operations_until_cancelled() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let slot = RejectedImportSlot::new();
        let ticket = slot.try_reserve().ok().unwrap();
        assert_eq!(slot.state(), RejectedImportState::Reserved);
        assert!(matches!(slot.try_reserve(), Err(RecoveryError::Busy)));
        assert!(matches!(slot.try_recover(), Err(RecoveryError::Busy)));
        assert_eq!(slot.discard(), Err(RecoveryError::Busy));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(matches!(slot.try_reserve(), Err(RecoveryError::Busy)));
                assert!(matches!(slot.try_recover(), Err(RecoveryError::Busy)));
                assert_eq!(slot.discard(), Err(RecoveryError::Busy));
                let value = rejected(&budget, 17);
                let pointer = value.image.pixels.as_ptr();
                let value = slot.try_store(value).unwrap_err();
                assert_eq!(value.image.pixels.as_ptr(), pointer);
                assert_eq!(value.error, AdmissionError::DifferentBudget);
                assert_eq!(value.lease.bytes(), 4);
            });
        });
        assert_eq!(budget.snapshot().used_bytes, 0);
        drop(ticket);
        assert_eq!(slot.state(), RejectedImportState::Empty);
        drop(slot.try_reserve().ok().unwrap());
    }

    #[test]
    fn reservation_unwind_cancels_without_an_image() {
        let slot = RejectedImportSlot::new();
        assert!(catch_unwind(AssertUnwindSafe(|| {
            let _ticket = slot.try_reserve().ok().unwrap();
            panic!("decode failed before a rejected import existed");
        }))
        .is_err());
        assert_eq!(slot.state(), RejectedImportState::Empty);
        drop(slot.try_reserve().ok().unwrap());
        assert!(matches!(slot.try_recover(), Err(RecoveryError::Empty)));
    }

    #[test]
    fn reserved_store_preserves_pixels_error_and_full_original_lease() {
        let budget = ResourceBudget::new(32, 0).unwrap();
        let slot = RejectedImportSlot::new();
        let ticket = slot.try_reserve().ok().unwrap();
        // Simulate decode only after the recovery destination is reserved.
        let image = Image::new(1, 1, vec![23; 4]).unwrap();
        let pointer = image.pixels.as_ptr();
        let lease = budget.try_acquire(12, AdmissionClass::Normal).unwrap();
        assert!(catch_unwind(AssertUnwindSafe(move || {
            ticket.store(RejectedImport {
                error: AdmissionError::InsufficientLease,
                image,
                lease,
            });
            panic!("caller unwound after storing its rejection");
        }))
        .is_err());
        assert_eq!(slot.state(), RejectedImportState::Recoverable);
        assert!(matches!(slot.try_reserve(), Err(RecoveryError::Busy)));
        assert_eq!(budget.snapshot().used_bytes, 12);
        let guard = slot.try_recover().ok().unwrap();
        assert!(matches!(slot.try_reserve(), Err(RecoveryError::Busy)));
        assert_eq!(guard.image().pixels.as_ptr(), pointer);
        assert_eq!(guard.image().pixels, [23; 4]);
        assert_eq!(guard.error(), AdmissionError::InsufficientLease);
        assert_eq!(guard.reserved_bytes(), 12);
        drop(guard);
        let guard = slot.try_recover().ok().unwrap();
        assert_eq!(guard.image().pixels.as_ptr(), pointer);
        assert_eq!(guard.reserved_bytes(), 12);
        guard.commit_saved();
        assert_eq!(budget.snapshot().used_bytes, 0);
        drop(slot.try_reserve().ok().unwrap());
    }

    #[test]
    fn successful_save_releases_budget_and_allows_next_value() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let slot = RejectedImportSlot::new();
        let value = rejected(&budget, 7);
        let pointer = value.image.pixels.as_ptr();
        slot.try_store(value).unwrap();
        let guard = slot.try_recover().ok().unwrap();
        assert_eq!(guard.image().pixels.as_ptr(), pointer);
        assert_eq!(guard.image().pixels, [7; 4]);
        assert_eq!(guard.error(), AdmissionError::DifferentBudget);
        assert_eq!(guard.reserved_bytes(), 4);
        assert_eq!(budget.snapshot().used_bytes, 4);
        guard.commit_saved();
        assert_eq!(slot.state(), RejectedImportState::Empty);
        assert_eq!(budget.snapshot().used_bytes, 0);
        slot.try_store(rejected(&budget, 8)).unwrap();
        slot.discard().unwrap();
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn cancellation_and_unwind_restore_original_allocation_and_lease() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let slot = RejectedImportSlot::new();
        let value = rejected(&budget, 13);
        let pointer = value.image.pixels.as_ptr();
        slot.try_store(value).unwrap();
        fn cancelled(slot: &RejectedImportSlot) -> Result<(), &'static str> {
            let _guard = slot.try_recover().ok().unwrap();
            Err("save failed")?;
            Ok(())
        }
        assert_eq!(cancelled(&slot), Err("save failed"));
        assert_eq!(slot.state(), RejectedImportState::Recoverable);
        assert!(catch_unwind(AssertUnwindSafe(|| {
            let guard = slot.try_recover().ok().unwrap();
            assert_eq!(guard.image().pixels.as_ptr(), pointer);
            panic!("save unwound");
        }))
        .is_err());
        assert_eq!(slot.state(), RejectedImportState::Recoverable);
        assert_eq!(budget.snapshot().used_bytes, 4);
        let guard = slot.try_recover().ok().unwrap();
        assert_eq!(guard.image().pixels.as_ptr(), pointer);
        assert_eq!(guard.image().pixels, [13; 4]);
        assert_eq!(guard.error(), AdmissionError::DifferentBudget);
        guard.discard();
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn busy_returns_unchanged_value_for_pending_and_active_recovery() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let slot = RejectedImportSlot::new();
        slot.try_store(rejected(&budget, 1)).unwrap();
        let value = rejected(&budget, 2);
        let pointer = value.image.pixels.as_ptr();
        let value = slot.try_store(value).unwrap_err();
        assert_eq!(value.image.pixels.as_ptr(), pointer);
        let guard = slot.try_recover().ok().unwrap();
        let value = slot.try_store(value).unwrap_err();
        assert_eq!(value.image.pixels.as_ptr(), pointer);
        assert_eq!(value.image.pixels, [2; 4]);
        assert_eq!(value.error, AdmissionError::DifferentBudget);
        assert_eq!(value.lease.bytes(), 4);
        assert!(matches!(slot.try_recover(), Err(RecoveryError::Busy)));
        assert_eq!(slot.discard(), Err(RecoveryError::Busy));
        assert_eq!(slot.state(), RejectedImportState::Recovering);
        assert_eq!(budget.snapshot().used_bytes, 8);
        drop(guard);
        slot.discard().unwrap();
        assert_eq!(budget.snapshot().used_bytes, 4);
        drop(value);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn parallel_operations_see_busy_without_holding_io_lock() {
        let budget = ResourceBudget::new(16, 0).unwrap();
        let slot = RejectedImportSlot::new();
        slot.try_store(rejected(&budget, 5)).unwrap();
        let guard = slot.try_recover().ok().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert_eq!(slot.state(), RejectedImportState::Recovering);
                assert!(matches!(slot.try_recover(), Err(RecoveryError::Busy)));
                assert_eq!(slot.discard(), Err(RecoveryError::Busy));
                let value = rejected(&budget, 9);
                let pointer = value.image.pixels.as_ptr();
                let value = slot.try_store(value).unwrap_err();
                assert_eq!(value.image.pixels.as_ptr(), pointer);
                assert_eq!(value.image.pixels, [9; 4]);
            });
            assert_eq!(guard.image().pixels, [5; 4]);
        });
        assert_eq!(budget.snapshot().used_bytes, 4);
        drop(guard);
        slot.try_recover().ok().unwrap().commit_saved();
        assert_eq!(budget.snapshot().used_bytes, 0);
        assert_eq!(slot.discard(), Err(RecoveryError::Empty));
        assert!(matches!(slot.try_recover(), Err(RecoveryError::Empty)));
    }
}
