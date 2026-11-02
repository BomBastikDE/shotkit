//! Generation-checked delayed capture lifecycle, independent of Win32 timers.
//! A wakeup may remain in an OS message queue after its request has completed.
//! It must never start a later request or start the same selection twice.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(usize);
impl Ticket {
    pub fn from_wire(value: usize) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }
    pub fn to_wire(self) -> usize {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeginError {
    Busy,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Running,
}
#[derive(Debug)]
pub struct DelayedCaptureState {
    last_generation: usize,
    current: Option<(Ticket, Phase)>,
}
impl Default for DelayedCaptureState {
    fn default() -> Self {
        Self::new()
    }
}
impl DelayedCaptureState {
    pub const fn new() -> Self {
        Self {
            last_generation: 0,
            current: None,
        }
    }
    pub fn begin(&mut self) -> Result<Ticket, BeginError> {
        if self.current.is_some() {
            return Err(BeginError::Busy);
        }
        // Tokens cross WPARAM unchanged on both 32- and 64-bit Windows.
        // Exhaustion disables new requests instead of reusing an old token.
        let generation = self
            .last_generation
            .checked_add(1)
            .ok_or(BeginError::Exhausted)?;
        let ticket = Ticket(generation);
        self.last_generation = generation;
        self.current = Some((ticket, Phase::Waiting));
        Ok(ticket)
    }
    pub fn is_pending(&self) -> bool {
        self.current.is_some()
    }
    pub fn is_waiting(&self, ticket: Ticket) -> bool {
        self.current == Some((ticket, Phase::Waiting))
    }
    pub fn try_start(&mut self, ticket: Ticket) -> bool {
        if !self.is_waiting(ticket) {
            return false;
        }
        self.current = Some((ticket, Phase::Running));
        true
    }
    pub fn finish(&mut self, ticket: Ticket) -> bool {
        if self.current != Some((ticket, Phase::Running)) {
            return false;
        }
        self.current = None;
        true
    }
    pub fn cancel_waiting(&mut self, ticket: Ticket) -> bool {
        if !self.is_waiting(ticket) {
            return false;
        }
        self.current = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_request_starts_once_even_with_duplicate_queued_wakeups() {
        let mut state = DelayedCaptureState::new();
        let ticket = state.begin().unwrap();
        assert!(state.is_waiting(ticket)); // worker checked before posting
        assert!(state.try_start(ticket)); // first queued message
        assert!(!state.try_start(ticket)); // duplicate during selector/modal loop
        assert!(state.is_pending());
        assert!(state.finish(ticket));
        assert!(!state.try_start(ticket)); // duplicate delivered after completion
        assert!(!state.is_pending());
    }

    #[test]
    fn stale_retry_cannot_start_a_new_request_before_its_delay() {
        let mut state = DelayedCaptureState::new();
        let old = state.begin().unwrap();
        assert!(state.is_waiting(old)); // old worker may now race with cancellation
        assert!(state.cancel_waiting(old));
        let new = state.begin().unwrap();
        assert_ne!(old, new);
        assert!(!state.is_waiting(old)); // pre-post check rejects stale worker
        assert!(!state.try_start(old)); // dispatch also rejects already queued message
        assert!(state.is_waiting(new));
        assert!(state.try_start(new));
    }

    #[test]
    fn stale_completion_and_send_failure_preserve_new_request() {
        let mut state = DelayedCaptureState::new();
        let old = state.begin().unwrap();
        assert!(state.try_start(old));
        assert!(state.finish(old));
        let new = state.begin().unwrap();
        assert!(!state.finish(old));
        assert!(!state.cancel_waiting(old));
        assert!(state.is_waiting(new));
    }

    #[test]
    fn send_failure_after_another_wakeup_started_does_not_unlock_running_capture() {
        let mut state = DelayedCaptureState::new();
        let ticket = state.begin().unwrap();
        assert!(state.try_start(ticket));
        assert!(!state.cancel_waiting(ticket));
        assert_eq!(state.begin(), Err(BeginError::Busy));
        assert!(state.finish(ticket));
        assert!(!state.is_pending());
    }

    #[test]
    fn busy_and_aborted_selection_keep_generation_identity() {
        let mut state = DelayedCaptureState::new();
        let ticket = state.begin().unwrap();
        assert_eq!(state.begin(), Err(BeginError::Busy));
        assert!(state.try_start(ticket));
        assert_eq!(state.begin(), Err(BeginError::Busy));
        // Finishing is shared by accepted, cancelled and failed selections.
        assert!(state.finish(ticket));
        assert_ne!(state.begin().unwrap(), ticket);
    }

    #[test]
    fn token_exhaustion_never_wraps_or_reuses_zero() {
        let mut state = DelayedCaptureState {
            last_generation: usize::MAX - 1,
            current: None,
        };
        let last = state.begin().unwrap();
        assert_eq!(last.to_wire(), usize::MAX);
        assert!(state.cancel_waiting(last));
        assert_eq!(state.begin(), Err(BeginError::Exhausted));
        assert_eq!(state.begin(), Err(BeginError::Exhausted));
        assert!(!state.is_pending());
        assert!(!state.try_start(last));
        assert_eq!(Ticket::from_wire(0), None);
        assert_eq!(Ticket::from_wire(usize::MAX), Some(last));
    }
}
