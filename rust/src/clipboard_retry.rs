//! Bounded retry policy for opening the Windows clipboard.

use std::time::Duration;

const RETRY_DELAYS_MS: [u64; 4] = [10, 20, 40, 80];

/// Retry only the open operation, without dispatching window messages.
/// The caller must not include clipboard writes in `open`.
pub fn open_with_retry(open: impl FnMut() -> bool) -> bool {
    retry(open, std::thread::sleep)
}

fn retry(mut open: impl FnMut() -> bool, mut sleep: impl FnMut(Duration)) -> bool {
    for delay in RETRY_DELAYS_MS {
        if open() {
            return true;
        }
        sleep(Duration::from_millis(delay));
    }
    open()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn immediate_success_does_not_sleep_or_retry() {
        let calls = Cell::new(0);
        assert!(retry(
            || {
                calls.set(calls.get() + 1);
                true
            },
            |_| panic!("unexpected sleep")
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn temporary_contention_retries_only_until_success() {
        let calls = Cell::new(0);
        let mut sleeps = Vec::new();
        assert!(retry(
            || {
                calls.set(calls.get() + 1);
                calls.get() == 3
            },
            |delay| sleeps.push(delay)
        ));
        assert_eq!(calls.get(), 3);
        assert_eq!(
            sleeps,
            [Duration::from_millis(10), Duration::from_millis(20)]
        );
    }

    #[test]
    fn persistent_contention_has_finite_attempts_and_wait() {
        let calls = Cell::new(0);
        let mut sleeps = Vec::new();
        assert!(!retry(
            || {
                calls.set(calls.get() + 1);
                false
            },
            |delay| sleeps.push(delay)
        ));
        assert_eq!(calls.get(), 5);
        assert_eq!(sleeps.iter().sum::<Duration>(), Duration::from_millis(150));
    }
}
