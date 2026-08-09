use std::sync::atomic::{AtomicU64, Ordering};

/// A captured generation of a [`CancellationToken`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CancellationGeneration(u64);

/// Cooperative cancellation without allocation, locks, or background threads.
#[derive(Debug, Default)]
pub struct CancellationToken {
    generation: AtomicU64,
}

impl CancellationToken {
    /// Creates a token at generation zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
        }
    }

    /// Captures the generation that a new operation should observe.
    #[must_use]
    pub fn snapshot(&self) -> CancellationGeneration {
        CancellationGeneration(self.generation.load(Ordering::Acquire))
    }

    /// Cancels operations holding an older generation and returns the new one.
    pub fn cancel(&self) -> CancellationGeneration {
        let previous = self.generation.fetch_add(1, Ordering::AcqRel);
        CancellationGeneration(previous.wrapping_add(1))
    }

    /// Reports whether the supplied generation has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self, observed: CancellationGeneration) -> bool {
        self.generation.load(Ordering::Acquire) != observed.0
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;

    #[test]
    fn generations_cancel_only_existing_work() {
        let token = CancellationToken::new();
        let first = token.snapshot();
        assert!(!token.is_cancelled(first));
        let second = token.cancel();
        assert!(token.is_cancelled(first));
        assert!(!token.is_cancelled(second));
        assert_eq!(token.snapshot(), second);
    }
}
