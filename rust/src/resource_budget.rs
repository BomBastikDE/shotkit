//! Portable accounting for explicitly registered, process-owned allocations.
//!
//! A budget counts the byte amounts supplied by callers, from admission until
//! the corresponding lease is released. Callers must include retained buffers
//! and temporary buffers that coexist, reserve before allocating, and keep the
//! lease alive at least as long as its allocation. Capacity rather than length
//! should be counted where a buffer retains spare capacity. Moving a lease
//! transfers ownership; putting it beside an immutable allocation in the same
//! `Arc` counts that allocation once regardless of the number of `Arc` clones.
//!
//! This is neither an RSS measurement nor an allocation/OOM guarantee. It does
//! not count unregistered buffers, allocator overhead, stacks, external codecs,
//! OS/GPU allocations, or allocations made by other processes. No process-global
//! budget is installed here: callers must share the same `ResourceBudget` to
//! enforce a common ceiling. A failed admission never changes existing leases.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

/// Admission policy only; `Reserved` does not guarantee physical memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionClass {
    /// Lower-priority work, such as a new import, preserves configured headroom.
    Normal,
    /// Work allowed to use headroom, such as an existing document operation.
    Reserved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetError {
    InvalidReserve {
        ceiling_bytes: usize,
        reserve_bytes: usize,
    },
    ArithmeticOverflow,
    LimitExceeded {
        requested_bytes: usize,
        used_bytes: usize,
        limit_bytes: usize,
    },
    InsufficientReservation {
        available_bytes: usize,
        requested_bytes: usize,
    },
    DifferentBudget,
    TransferExceedsAdmissionLimit {
        transfer_bytes: usize,
        used_bytes: usize,
        limit_bytes: usize,
    },
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReserve { ceiling_bytes, reserve_bytes } => write!(f, "reserve {reserve_bytes} exceeds budget ceiling {ceiling_bytes}"),
            Self::ArithmeticOverflow => f.write_str("resource byte accounting overflow"),
            Self::LimitExceeded { requested_bytes, used_bytes, limit_bytes } => write!(f, "cannot reserve {requested_bytes} additional bytes: {used_bytes} used, admission limit {limit_bytes}"),
            Self::InsufficientReservation { available_bytes, requested_bytes } => write!(f, "cannot transfer {requested_bytes} bytes from a {available_bytes}-byte lease"),
            Self::DifferentBudget => f.write_str("leases belong to different resource budgets"),
            Self::TransferExceedsAdmissionLimit { transfer_bytes, used_bytes, limit_bytes } => write!(f, "cannot transfer {transfer_bytes} bytes to this admission class: {used_bytes} used, admission limit {limit_bytes}"),
        }
    }
}

impl std::error::Error for BudgetError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub ceiling_bytes: usize,
    pub normal_limit_bytes: usize,
    pub used_bytes: usize,
}

#[derive(Debug)]
struct Accounting {
    used_bytes: usize,
}

#[derive(Debug)]
struct SharedBudget {
    ceiling_bytes: usize,
    normal_limit_bytes: usize,
    accounting: Mutex<Accounting>,
}

impl SharedBudget {
    fn lock(&self) -> MutexGuard<'_, Accounting> {
        // No caller callback or fallible operation runs under this lock. Every
        // mutation below follows all validation, preserving the invariant even
        // when recovering a lock poisoned by an unrelated unwind.
        self.accounting
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn limit(&self, class: AdmissionClass) -> usize {
        match class {
            AdmissionClass::Normal => self.normal_limit_bytes,
            AdmissionClass::Reserved => self.ceiling_bytes,
        }
    }

    fn admit(
        &self,
        used: usize,
        additional: usize,
        class: AdmissionClass,
    ) -> Result<usize, BudgetError> {
        let total = used
            .checked_add(additional)
            .ok_or(BudgetError::ArithmeticOverflow)?;
        let limit = self.limit(class);
        // Zero-byte operations remain valid even when reserved work has taken
        // the shared total above the normal admission limit.
        if additional != 0 && total > limit {
            return Err(BudgetError::LimitExceeded {
                requested_bytes: additional,
                used_bytes: used,
                limit_bytes: limit,
            });
        }
        Ok(total)
    }
}

/// Clones share one accounting pool; constructing another budget creates a pool.
#[derive(Clone, Debug)]
pub struct ResourceBudget {
    shared: Arc<SharedBudget>,
}

impl ResourceBudget {
    /// `normal_reserve_bytes = 0` disables the headroom distinction.
    pub fn new(ceiling_bytes: usize, normal_reserve_bytes: usize) -> Result<Self, BudgetError> {
        let normal_limit_bytes =
            ceiling_bytes
                .checked_sub(normal_reserve_bytes)
                .ok_or(BudgetError::InvalidReserve {
                    ceiling_bytes,
                    reserve_bytes: normal_reserve_bytes,
                })?;
        Ok(Self {
            shared: Arc::new(SharedBudget {
                ceiling_bytes,
                normal_limit_bytes,
                accounting: Mutex::new(Accounting { used_bytes: 0 }),
            }),
        })
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            ceiling_bytes: self.shared.ceiling_bytes,
            normal_limit_bytes: self.shared.normal_limit_bytes,
            used_bytes: self.shared.lock().used_bytes,
        }
    }

    pub fn try_acquire(
        &self,
        bytes: usize,
        class: AdmissionClass,
    ) -> Result<BudgetLease, BudgetError> {
        let mut accounting = self.shared.lock();
        let total = self.shared.admit(accounting.used_bytes, bytes, class)?;
        accounting.used_bytes = total;
        Ok(BudgetLease {
            shared: Arc::clone(&self.shared),
            bytes,
            class,
        })
    }
}

/// Exclusive ownership of registered bytes, released on drop or cancellation.
///
/// Intentionally not `Clone`: use `Arc<BudgetLease>` (or put a lease and its
/// allocation in one `Arc`-owned struct) to share immutable ownership. Growing
/// must succeed before allocating; shrinking/releasing must follow disposal of
/// the corresponding bytes. `try_resize` preserves this lease's admission class.
#[derive(Debug)]
pub struct BudgetLease {
    shared: Arc<SharedBudget>,
    bytes: usize,
    class: AdmissionClass,
}

impl BudgetLease {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn class(&self) -> AdmissionClass {
        self.class
    }

    /// Resize atomically; a rejected growth leaves this lease unchanged.
    pub fn try_resize(&mut self, new_bytes: usize) -> Result<(), BudgetError> {
        let mut accounting = self.shared.lock();
        let total = if new_bytes > self.bytes {
            self.shared
                .admit(accounting.used_bytes, new_bytes - self.bytes, self.class)?
        } else {
            accounting.used_bytes - (self.bytes - new_bytes)
        };
        accounting.used_bytes = total;
        self.bytes = new_bytes;
        Ok(())
    }

    /// Divide ownership without changing the shared total or admission class.
    pub fn split_off(&mut self, bytes: usize) -> Result<Self, BudgetError> {
        let remainder =
            self.bytes
                .checked_sub(bytes)
                .ok_or(BudgetError::InsufficientReservation {
                    available_bytes: self.bytes,
                    requested_bytes: bytes,
                })?;
        let split = Self {
            shared: Arc::clone(&self.shared),
            bytes,
            class: self.class,
        };
        self.bytes = remainder;
        Ok(split)
    }

    /// Move bytes to another lease in the same pool without releasing them.
    ///
    /// The receiving lease keeps its original admission class. Transfers to a
    /// normal lease are rejected when the shared total exceeds its normal limit.
    /// All validation happens before either lease changes; different pools are
    /// rejected even for a zero-byte transfer.
    pub fn transfer_to(&mut self, target: &mut Self, bytes: usize) -> Result<(), BudgetError> {
        if !Arc::ptr_eq(&self.shared, &target.shared) {
            return Err(BudgetError::DifferentBudget);
        }
        let remainder =
            self.bytes
                .checked_sub(bytes)
                .ok_or(BudgetError::InsufficientReservation {
                    available_bytes: self.bytes,
                    requested_bytes: bytes,
                })?;
        let target_bytes = target
            .bytes
            .checked_add(bytes)
            .ok_or(BudgetError::ArithmeticOverflow)?;
        let accounting = self.shared.lock();
        let limit = self.shared.limit(target.class);
        if bytes != 0 && accounting.used_bytes > limit {
            return Err(BudgetError::TransferExceedsAdmissionLimit {
                transfer_bytes: bytes,
                used_bytes: accounting.used_bytes,
                limit_bytes: limit,
            });
        }
        self.bytes = remainder;
        target.bytes = target_bytes;
        Ok(())
    }

    /// Explicitly release this reservation; dropping it has the same effect.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for BudgetLease {
    fn drop(&mut self) {
        let mut accounting = self.shared.lock();
        // Private fields and exclusive lease mutations guarantee this relation.
        accounting.used_bytes -= self.bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn zero_budget_and_invalid_reserve() {
        let budget = ResourceBudget::new(0, 0).unwrap();
        let mut zero = budget.try_acquire(0, AdmissionClass::Normal).unwrap();
        zero.try_resize(0).unwrap();
        assert!(budget.try_acquire(1, AdmissionClass::Reserved).is_err());
        assert_eq!(budget.snapshot().used_bytes, 0);
        assert_eq!(
            ResourceBudget::new(3, 4).unwrap_err(),
            BudgetError::InvalidReserve {
                ceiling_bytes: 3,
                reserve_bytes: 4
            }
        );
    }

    #[test]
    fn exact_boundary_and_overflow_are_atomic() {
        let budget = ResourceBudget::new(usize::MAX, 0).unwrap();
        let mut lease = budget
            .try_acquire(usize::MAX - 1, AdmissionClass::Normal)
            .unwrap();
        lease.try_resize(usize::MAX).unwrap();
        assert_eq!(
            budget.try_acquire(1, AdmissionClass::Reserved).unwrap_err(),
            BudgetError::ArithmeticOverflow
        );
        assert_eq!(lease.bytes(), usize::MAX);
        assert_eq!(budget.snapshot().used_bytes, usize::MAX);
        lease.release();
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn headroom_and_original_resize_class() {
        let budget = ResourceBudget::new(100, 20).unwrap();
        let mut normal = budget.try_acquire(80, AdmissionClass::Normal).unwrap();
        assert!(budget.try_acquire(1, AdmissionClass::Normal).is_err());
        let mut reserved = budget.try_acquire(20, AdmissionClass::Reserved).unwrap();
        assert!(normal.try_resize(81).is_err());
        assert!(reserved.try_resize(21).is_err());
        assert_eq!(
            (
                normal.bytes(),
                reserved.bytes(),
                budget.snapshot().used_bytes
            ),
            (80, 20, 100)
        );
        assert!(budget.try_acquire(0, AdmissionClass::Normal).is_ok());
        normal.try_resize(40).unwrap();
        reserved.try_resize(30).unwrap();
        normal.try_resize(50).unwrap();
        assert_eq!(budget.snapshot().used_bytes, 80);
        assert!(normal.try_resize(51).is_err());
        reserved.try_resize(50).unwrap();
        assert_eq!(budget.snapshot().used_bytes, 100);
    }

    #[test]
    fn zero_reserve_allows_both_classes_to_use_ceiling() {
        for class in [AdmissionClass::Normal, AdmissionClass::Reserved] {
            let budget = ResourceBudget::new(10, 0).unwrap();
            let _lease = budget.try_acquire(10, class).unwrap();
            assert!(budget.try_acquire(1, class).is_err());
        }
    }

    #[test]
    fn shared_immutable_lease_is_counted_and_released_once() {
        let budget = ResourceBudget::new(10, 0).unwrap();
        let first = Arc::new(budget.try_acquire(10, AdmissionClass::Normal).unwrap());
        let second = Arc::clone(&first);
        let third = Arc::clone(&second);
        assert_eq!(budget.snapshot().used_bytes, 10);
        drop(first);
        drop(second);
        assert_eq!(budget.snapshot().used_bytes, 10);
        drop(third);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn cancellation_releases_new_work_preserving_existing_document() {
        let budget = ResourceBudget::new(100, 0).unwrap();
        let document = budget.try_acquire(60, AdmissionClass::Reserved).unwrap();
        let cancelled: Result<(), ()> = {
            let _temporary = budget.try_acquire(40, AdmissionClass::Normal).unwrap();
            assert!(budget.try_acquire(1, AdmissionClass::Normal).is_err());
            Err(())
        };
        assert!(cancelled.is_err());
        assert_eq!(document.bytes(), 60);
        assert_eq!(budget.snapshot().used_bytes, 60);
    }

    #[test]
    fn split_transfer_and_failures_preserve_accounting() {
        let budget = ResourceBudget::new(100, 0).unwrap();
        let mut source = budget.try_acquire(100, AdmissionClass::Normal).unwrap();
        assert!(source.split_off(101).is_err());
        let mut target = source.split_off(40).unwrap();
        assert_eq!(
            (source.bytes(), target.bytes(), budget.snapshot().used_bytes),
            (60, 40, 100)
        );
        source.transfer_to(&mut target, 10).unwrap();
        assert!(source.transfer_to(&mut target, 51).is_err());
        assert_eq!((source.bytes(), target.bytes()), (50, 50));
        let other = ResourceBudget::new(100, 0).unwrap();
        let mut foreign = other.try_acquire(1, AdmissionClass::Normal).unwrap();
        assert_eq!(
            source.transfer_to(&mut foreign, 1),
            Err(BudgetError::DifferentBudget)
        );
        assert_eq!(
            source.transfer_to(&mut foreign, 0),
            Err(BudgetError::DifferentBudget)
        );
        assert_eq!((source.bytes(), foreign.bytes()), (50, 1));
        drop(target);
        assert_eq!(budget.snapshot().used_bytes, 50);
        source.release();
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn transfer_cannot_bypass_normal_headroom() {
        let budget = ResourceBudget::new(100, 20).unwrap();
        let mut normal = budget.try_acquire(60, AdmissionClass::Normal).unwrap();
        let mut reserved = budget.try_acquire(40, AdmissionClass::Reserved).unwrap();
        assert_eq!(
            reserved.transfer_to(&mut normal, 1),
            Err(BudgetError::TransferExceedsAdmissionLimit {
                transfer_bytes: 1,
                used_bytes: 100,
                limit_bytes: 80,
            })
        );
        assert_eq!((normal.bytes(), reserved.bytes()), (60, 40));
        normal.transfer_to(&mut reserved, 10).unwrap();
        reserved.try_resize(20).unwrap();
        reserved.transfer_to(&mut normal, 10).unwrap();
        assert_eq!(
            (
                normal.bytes(),
                reserved.bytes(),
                budget.snapshot().used_bytes
            ),
            (60, 10, 70)
        );
    }

    #[test]
    fn concurrent_admission_never_exceeds_ceiling() {
        const WORKERS: usize = 16;
        let budget = ResourceBudget::new(40, 0).unwrap();
        let start = Arc::new(Barrier::new(WORKERS + 1));
        let admitted = Arc::new(Barrier::new(WORKERS + 1));
        let finish = Arc::new(Barrier::new(WORKERS + 1));
        let handles: Vec<_> = (0..WORKERS)
            .map(|_| {
                let (budget, start, admitted, finish) = (
                    budget.clone(),
                    Arc::clone(&start),
                    Arc::clone(&admitted),
                    Arc::clone(&finish),
                );
                thread::spawn(move || {
                    start.wait();
                    let lease = budget.try_acquire(10, AdmissionClass::Normal);
                    let success = lease.is_ok();
                    admitted.wait();
                    finish.wait();
                    drop(lease);
                    success
                })
            })
            .collect();
        start.wait();
        admitted.wait();
        assert_eq!(budget.snapshot().used_bytes, 40);
        finish.wait();
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 4);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn poisoned_mutex_still_releases_and_enforces_admission() {
        let budget = ResourceBudget::new(10, 0).unwrap();
        let lease = budget.try_acquire(6, AdmissionClass::Normal).unwrap();
        let shared = Arc::clone(&budget.shared);
        assert!(thread::spawn(move || {
            let _guard = shared.accounting.lock().unwrap();
            panic!("simulate an unwind while holding accounting lock");
        })
        .join()
        .is_err());
        assert_eq!(budget.snapshot().used_bytes, 6);
        assert!(budget.try_acquire(5, AdmissionClass::Normal).is_err());
        drop(lease);
        let replacement = budget.try_acquire(10, AdmissionClass::Reserved).unwrap();
        drop(replacement);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }
}
