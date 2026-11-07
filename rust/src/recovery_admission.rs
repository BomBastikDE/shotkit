//! Serializes capture sources and recovery so retained images cannot be
//! multiplied or silently replaced while recovery dialogs pump messages.
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    PendingRecovery,
}

/// Exclusive ownership of the shared source/recovery gate.
pub struct Permit<'a> {
    busy: &'a AtomicBool,
    release: bool,
}

impl Permit<'_> {
    /// Transfer release responsibility to the asynchronous completion path.
    pub fn disarm(&mut self) {
        self.release = false;
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if self.release {
            self.busy.store(false, Ordering::SeqCst);
        }
    }
}

/// Reserve before reading a source. Check recovery only after reserving, so
/// another producer or resolver cannot begin between the two operations.
pub fn acquire_source(
    busy: &AtomicBool,
    has_recovery: impl FnOnce() -> bool,
) -> Result<Permit<'_>, AdmissionError> {
    let permit = acquire_resolution(busy)?;
    if has_recovery() {
        return Err(AdmissionError::PendingRecovery);
    }
    Ok(permit)
}

/// Reserve while resolving retained data, without requiring that it be empty.
pub fn acquire_resolution(busy: &AtomicBool) -> Result<Permit<'_>, AdmissionError> {
    busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| AdmissionError::Busy)?;
    Ok(Permit {
        busy,
        release: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn pending_image_blocks_new_source_until_explicit_resolution() {
        let busy = AtomicBool::new(false);
        let retained = Mutex::new(vec![vec![1, 2, 3, 4]]);
        assert!(matches!(
            acquire_source(&busy, || !retained.lock().unwrap().is_empty()),
            Err(AdmissionError::PendingRecovery)
        ));
        assert_eq!(*retained.lock().unwrap(), vec![vec![1, 2, 3, 4]]);
        let resolution = acquire_resolution(&busy).unwrap();
        assert!(matches!(
            acquire_source(&busy, || false),
            Err(AdmissionError::Busy)
        ));
        // Cancellation retains the exact original, and a later successful save
        // explicitly removes it while holding the same exclusive gate.
        drop(resolution);
        assert_eq!(*retained.lock().unwrap(), vec![vec![1, 2, 3, 4]]);
        let resolution = acquire_resolution(&busy).unwrap();
        assert_eq!(retained.lock().unwrap().pop().unwrap(), vec![1, 2, 3, 4]);
        drop(resolution);
        assert!(acquire_source(&busy, || !retained.lock().unwrap().is_empty()).is_ok());
    }

    #[test]
    fn source_and_recovery_are_mutually_exclusive_across_threads() {
        let busy = AtomicBool::new(false);
        let source = acquire_source(&busy, || false).unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert!(matches!(
                        acquire_resolution(&busy),
                        Err(AdmissionError::Busy)
                    ))
                })
                .join()
                .unwrap();
        });
        drop(source);
        assert!(acquire_resolution(&busy).is_ok());
    }

    #[test]
    fn failure_or_panic_during_check_releases_gate() {
        let busy = AtomicBool::new(false);
        let result = std::panic::catch_unwind(|| {
            let _ = acquire_source(&busy, || panic!("fixture"));
        });
        assert!(result.is_err());
        assert!(acquire_resolution(&busy).is_ok());
    }

    #[test]
    fn asynchronous_handoff_keeps_gate_until_completion_releases_it() {
        let busy = AtomicBool::new(false);
        let mut source = acquire_source(&busy, || false).unwrap();
        source.disarm();
        drop(source);
        assert!(matches!(
            acquire_resolution(&busy),
            Err(AdmissionError::Busy)
        ));
        busy.store(false, Ordering::SeqCst); // actual completion releases it
        assert!(acquire_resolution(&busy).is_ok());
    }
}
