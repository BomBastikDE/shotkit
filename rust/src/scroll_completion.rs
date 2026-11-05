//! Owned scroll results outlive a failed or stale Win32 wakeup.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowIdentity {
    pub handle: usize,
    pub generation: usize,
}

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

pub struct Completion<T> {
    pub owner: WindowIdentity,
    pub was_visible: bool,
    pub result: T,
}
struct Job<T> {
    ticket: Ticket,
    owner: WindowIdentity,
    was_visible: bool,
    result: Option<T>,
}

pub struct ScrollCompletion<T> {
    generation: usize,
    current: Option<Job<T>>,
}
impl<T> Default for ScrollCompletion<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> ScrollCompletion<T> {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            current: None,
        }
    }

    pub fn begin(&mut self, owner: WindowIdentity, was_visible: bool) -> Option<Ticket> {
        if self.current.is_some() || owner.handle == 0 || owner.generation == 0 {
            return None;
        }
        let generation = self.generation.checked_add(1)?;
        self.generation = generation;
        let ticket = Ticket(generation);
        self.current = Some(Job {
            ticket,
            owner,
            was_visible,
            result: None,
        });
        Some(ticket)
    }

    /// Only the matching worker may publish, and never replace a retained result.
    pub fn publish(&mut self, ticket: Ticket, result: T) -> bool {
        let Some(job) = self.current.as_mut() else {
            return false;
        };
        if job.ticket != ticket || job.result.is_some() {
            return false;
        }
        job.result = Some(result);
        true
    }

    pub fn ready_ticket(&self) -> Option<Ticket> {
        let job = self.current.as_ref()?;
        job.result.as_ref()?;
        Some(job.ticket)
    }

    /// Both the posted wakeup and UI polling use this single ownership transfer.
    pub fn take(&mut self, ticket: Ticket) -> Option<Completion<T>> {
        if self.ready_ticket() != Some(ticket) {
            return None;
        }
        let job = self.current.take()?;
        Some(Completion {
            owner: job.owner,
            was_visible: job.was_visible,
            result: job.result?,
        })
    }

    /// Used only before a worker exists, when the fallback timer cannot start.
    pub fn cancel_unstarted(&mut self, ticket: Ticket) -> bool {
        if !self
            .current
            .as_ref()
            .is_some_and(|job| job.ticket == ticket && job.result.is_none())
        {
            return false;
        }
        self.current = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(generation: usize) -> WindowIdentity {
        WindowIdentity {
            handle: 123,
            generation,
        }
    }

    #[test]
    fn failed_post_retains_result_until_poll_and_duplicate_wakeup_is_harmless() {
        let mut state = ScrollCompletion::new();
        let ticket = state.begin(owner(1), true).unwrap();
        assert!(state.take(ticket).is_none());
        assert!(state.publish(ticket, String::from("owned pixels")));
        // Posting may fail permanently; publication remains ready for the UI timer.
        assert_eq!(state.ready_ticket(), Some(ticket));
        assert!(state.begin(owner(2), false).is_none());
        let completion = state.take(ticket).unwrap();
        assert_eq!(completion.owner, owner(1));
        assert!(completion.was_visible);
        assert_eq!(completion.result, "owned pixels");
        assert!(state.take(ticket).is_none());
        assert_eq!(state.ready_ticket(), None);
    }

    #[test]
    fn old_worker_and_queued_wakeup_cannot_take_or_replace_new_result() {
        let mut state = ScrollCompletion::new();
        let old = state.begin(owner(1), true).unwrap();
        assert!(state.publish(old, Err::<(), _>("old")));
        state.take(old).unwrap();
        let new = state.begin(owner(2), false).unwrap();
        assert_ne!(old, new);
        assert!(!state.publish(old, Err("stale worker")));
        assert!(state.publish(new, Ok(())));
        assert!(!state.publish(new, Err("duplicate worker")));
        assert!(state.take(old).is_none());
        assert!(!state.cancel_unstarted(old));
        let completion = state.take(new).unwrap();
        assert_eq!(completion.owner, owner(2));
        assert!(!completion.was_visible);
        assert_eq!(completion.result, Ok(()));
    }

    #[test]
    fn failed_timer_setup_can_cancel_without_reusing_a_ticket() {
        let mut state = ScrollCompletion::<()>::new();
        let old = state.begin(owner(1), true).unwrap();
        assert!(state.cancel_unstarted(old));
        let new = state.begin(owner(1), true).unwrap();
        assert_ne!(old, new);
        assert!(state.publish(new, ()));
        assert!(!state.cancel_unstarted(new));
        assert!(state.take(new).is_some());
    }

    #[test]
    fn invalid_identity_and_token_exhaustion_never_reuse_a_generation() {
        let mut state = ScrollCompletion::<()>::new();
        assert!(state.begin(owner(0), true).is_none());
        assert!(state
            .begin(
                WindowIdentity {
                    handle: 0,
                    generation: 1
                },
                true
            )
            .is_none());
        state.generation = usize::MAX - 1;
        let last = state.begin(owner(1), false).unwrap();
        assert_eq!(last.to_wire(), usize::MAX);
        assert!(state.cancel_unstarted(last));
        assert!(state.begin(owner(2), false).is_none());
        assert_eq!(Ticket::from_wire(0), None);
    }
}
