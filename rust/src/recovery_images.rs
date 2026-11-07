//! Retains recovery values in the same last-in, first-out order as a `Vec`.
//!
//! The first value is stored inline, so retaining it does not allocate storage
//! for the recovery collection. Unexpected additional values are preserved in
//! an overflow `Vec` rather than replacing or dropping an existing value.
//! Growing that overflow can still allocate and run out of memory. This is not
//! a bound on image allocations or total process RSS.

pub struct RecoveryImages<T> {
    primary: Option<T>,
    overflow: Vec<T>,
}

impl<T> RecoveryImages<T> {
    pub const fn new() -> Self {
        Self {
            primary: None,
            overflow: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.primary.is_none() && self.overflow.is_empty()
    }

    pub fn push(&mut self, value: T) {
        if self.primary.is_none() {
            self.primary = Some(value);
        } else {
            self.overflow.push(value);
        }
    }

    pub fn last(&self) -> Option<&T> {
        self.overflow.last().or(self.primary.as_ref())
    }

    pub fn pop(&mut self) -> Option<T> {
        self.overflow.pop().or_else(|| self.primary.take())
    }
}

impl<T> Default for RecoveryImages<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_value_uses_inline_slot_without_allocating_overflow() {
        let mut retained = RecoveryImages::new();
        assert_eq!(retained.overflow.capacity(), 0);

        retained.push([1, 2, 3, 4]);

        assert!(!retained.is_empty());
        assert_eq!(retained.primary, Some([1, 2, 3, 4]));
        assert_eq!(retained.last(), Some(&[1, 2, 3, 4]));
        assert_eq!(retained.overflow.len(), 0);
        assert_eq!(retained.overflow.capacity(), 0);
        assert_eq!(retained.pop(), Some([1, 2, 3, 4]));
    }

    #[test]
    fn unexpected_additional_values_are_preserved_in_lifo_order() {
        let mut retained = RecoveryImages::new();
        retained.push([1, 2]);
        retained.push([3, 4]);
        retained.push([5, 6]);

        assert_eq!(retained.primary, Some([1, 2]));
        assert_eq!(retained.last(), Some(&[5, 6]));
        assert_eq!(retained.last(), Some(&[5, 6]));
        assert_eq!(retained.pop(), Some([5, 6]));
        assert_eq!(retained.last(), Some(&[3, 4]));
        assert_eq!(retained.pop(), Some([3, 4]));
        assert!(!retained.is_empty());
        assert_eq!(retained.last(), Some(&[1, 2]));
        assert_eq!(retained.pop(), Some([1, 2]));
        assert!(retained.is_empty());
    }

    #[test]
    fn slots_can_be_reused_after_partial_and_complete_resolution() {
        let mut retained = RecoveryImages::new();
        retained.push(1);
        retained.push(2);
        assert_eq!(retained.pop(), Some(2));
        retained.push(3);
        assert_eq!(retained.pop(), Some(3));
        assert_eq!(retained.pop(), Some(1));

        let overflow_capacity = retained.overflow.capacity();
        retained.push(4);
        assert_eq!(retained.primary, Some(4));
        assert!(retained.overflow.is_empty());
        assert_eq!(retained.overflow.capacity(), overflow_capacity);
        retained.push(5);
        assert_eq!(retained.pop(), Some(5));
        assert_eq!(retained.pop(), Some(4));
        assert!(retained.is_empty());
    }

    #[test]
    fn empty_access_is_repeatable_and_allows_later_retention() {
        let mut retained = RecoveryImages::<u8>::new();
        assert!(retained.is_empty());
        assert_eq!(retained.last(), None);
        assert_eq!(retained.pop(), None);
        assert_eq!(retained.pop(), None);
        assert_eq!(retained.last(), None);

        retained.push(7);
        assert_eq!(retained.pop(), Some(7));
        assert!(retained.is_empty());
        assert_eq!(retained.last(), None);
        assert_eq!(retained.pop(), None);
    }
}
