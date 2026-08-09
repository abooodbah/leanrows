//! A bounded, capacity-one mailbox for coalescing superseded work.

use std::sync::{Condvar, Mutex, MutexGuard};

/// Whether a send occupied an empty slot or replaced stale work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendOutcome {
    /// The mailbox was empty.
    Accepted,
    /// Pending work was atomically replaced by newer work.
    Replaced,
}

/// Returned when a sender targets a closed mailbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxClosed;

#[derive(Debug)]
struct State<T> {
    pending: Option<T>,
    closed: bool,
}

/// A fixed-capacity mailbox that retains only the newest pending value.
///
/// It cannot grow with event rate: capacity is exactly one. Replacing pending
/// viewport work is intentional because only the latest viewport remains useful.
#[derive(Debug)]
pub struct CoalescingMailbox<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

impl<T> Default for CoalescingMailbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CoalescingMailbox<T> {
    /// Creates an open, empty mailbox.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                pending: None,
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    /// Stores the latest value, replacing any pending stale value.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxClosed`] after the receiver closes the mailbox.
    pub fn send(&self, value: T) -> Result<SendOutcome, MailboxClosed> {
        let mut state = self.lock_state();
        if state.closed {
            return Err(MailboxClosed);
        }
        let outcome = if state.pending.replace(value).is_some() {
            SendOutcome::Replaced
        } else {
            SendOutcome::Accepted
        };
        self.ready.notify_one();
        Ok(outcome)
    }

    /// Takes the current value without waiting.
    pub fn try_take(&self) -> Option<T> {
        self.lock_state().pending.take()
    }

    /// Waits for one value, or returns `None` after close and drain.
    pub fn wait_take(&self) -> Option<T> {
        let mut state = self.lock_state();
        loop {
            if let Some(value) = state.pending.take() {
                return Some(value);
            }
            if state.closed {
                return None;
            }
            state = match self.ready.wait(state) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }

    /// Prevents future sends and wakes a waiting receiver.
    pub fn close(&self) {
        let mut state = self.lock_state();
        state.closed = true;
        self.ready.notify_all();
    }

    fn lock_state(&self) -> MutexGuard<'_, State<T>> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CoalescingMailbox, MailboxClosed, SendOutcome};

    #[test]
    fn newer_work_replaces_stale_work() {
        let mailbox = CoalescingMailbox::new();
        assert_eq!(mailbox.send(10), Ok(SendOutcome::Accepted));
        assert_eq!(mailbox.send(11), Ok(SendOutcome::Replaced));
        assert_eq!(mailbox.try_take(), Some(11));
        assert_eq!(mailbox.try_take(), None);
    }

    #[test]
    fn close_drains_then_rejects() {
        let mailbox = CoalescingMailbox::new();
        assert_eq!(mailbox.send(5), Ok(SendOutcome::Accepted));
        mailbox.close();
        assert_eq!(mailbox.wait_take(), Some(5));
        assert_eq!(mailbox.wait_take(), None);
        assert_eq!(mailbox.send(6), Err(MailboxClosed));
    }
}
