use core::fmt;
use std::num::NonZeroUsize;

/// Accounts for engine-owned allocations covered by a memory promise.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedMemoryBudget {
    limit: NonZeroUsize,
    read_buffers: usize,
    viewport_payload: usize,
    checkpoints: usize,
    scratch: usize,
}

impl ManagedMemoryBudget {
    /// Creates a plan whose managed commitments do not exceed `limit_bytes`.
    ///
    /// # Errors
    /// Returns [`BudgetError`] for zero, overflow, or an exceeded ceiling.
    pub fn new(
        limit_bytes: usize,
        read_buffers_bytes: usize,
        viewport_payload_bytes: usize,
        checkpoint_bytes: usize,
        scratch_bytes: usize,
    ) -> Result<Self, BudgetError> {
        let Some(limit_bytes) = NonZeroUsize::new(limit_bytes) else {
            return Err(BudgetError::ZeroLimit);
        };
        let committed = read_buffers_bytes
            .checked_add(viewport_payload_bytes)
            .and_then(|value| value.checked_add(checkpoint_bytes))
            .and_then(|value| value.checked_add(scratch_bytes))
            .ok_or(BudgetError::ArithmeticOverflow)?;
        if committed > limit_bytes.get() {
            return Err(BudgetError::Exceeded {
                limit: limit_bytes.get(),
                committed,
            });
        }
        Ok(Self {
            limit: limit_bytes,
            read_buffers: read_buffers_bytes,
            viewport_payload: viewport_payload_bytes,
            checkpoints: checkpoint_bytes,
            scratch: scratch_bytes,
        })
    }

    #[must_use]
    pub const fn limit_bytes(self) -> usize {
        self.limit.get()
    }
    #[must_use]
    pub const fn committed_bytes(self) -> usize {
        self.read_buffers + self.viewport_payload + self.checkpoints + self.scratch
    }
    #[must_use]
    pub const fn remaining_bytes(self) -> usize {
        self.limit.get() - self.committed_bytes()
    }
    #[must_use]
    pub const fn read_buffers_bytes(self) -> usize {
        self.read_buffers
    }
    #[must_use]
    pub const fn viewport_payload_bytes(self) -> usize {
        self.viewport_payload
    }
    #[must_use]
    pub const fn checkpoint_bytes(self) -> usize {
        self.checkpoints
    }
    #[must_use]
    pub const fn scratch_bytes(self) -> usize {
        self.scratch
    }
}

/// Limits applied to one viewport reconstruction operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineLimits {
    read_buffer_bytes: NonZeroUsize,
    max_viewport_records: NonZeroUsize,
    max_record_preview_bytes: usize,
    max_viewport_payload_bytes: usize,
    cancellation_check_bytes: NonZeroUsize,
}

impl EngineLimits {
    /// Creates viewport limits and verifies their managed allocations fit.
    ///
    /// # Errors
    /// Returns [`BudgetError`] for zero values or a budget mismatch.
    pub fn new(
        read_buffer_bytes: usize,
        max_viewport_records: usize,
        max_record_preview_bytes: usize,
        max_viewport_payload_bytes: usize,
        cancellation_check_bytes: usize,
        budget: ManagedMemoryBudget,
    ) -> Result<Self, BudgetError> {
        let Some(read_buffer_bytes) = NonZeroUsize::new(read_buffer_bytes) else {
            return Err(BudgetError::ZeroReadBuffer);
        };
        let Some(max_viewport_records) = NonZeroUsize::new(max_viewport_records) else {
            return Err(BudgetError::ZeroViewportRecords);
        };
        let required_metadata =
            crate::viewport::required_metadata_capacity_bytes(max_viewport_records.get())
                .ok_or(BudgetError::ArithmeticOverflow)?;
        if required_metadata > budget.scratch_bytes() {
            return Err(BudgetError::ViewportMetadataExceedsScratch {
                required: required_metadata,
                available: budget.scratch_bytes(),
            });
        }
        let Some(cancellation_check_bytes) = NonZeroUsize::new(cancellation_check_bytes) else {
            return Err(BudgetError::ZeroCancellationInterval);
        };
        if max_record_preview_bytes > max_viewport_payload_bytes {
            return Err(BudgetError::PreviewExceedsViewport);
        }
        if read_buffer_bytes.get() > budget.read_buffers_bytes()
            || max_viewport_payload_bytes > budget.viewport_payload_bytes()
        {
            return Err(BudgetError::PlanMismatch);
        }
        Ok(Self {
            read_buffer_bytes,
            max_viewport_records,
            max_record_preview_bytes,
            max_viewport_payload_bytes,
            cancellation_check_bytes,
        })
    }

    #[must_use]
    pub const fn read_buffer_bytes(self) -> NonZeroUsize {
        self.read_buffer_bytes
    }
    #[must_use]
    pub const fn max_viewport_records(self) -> NonZeroUsize {
        self.max_viewport_records
    }
    #[must_use]
    pub const fn max_record_preview_bytes(self) -> usize {
        self.max_record_preview_bytes
    }
    #[must_use]
    pub const fn max_viewport_payload_bytes(self) -> usize {
        self.max_viewport_payload_bytes
    }
    #[must_use]
    pub const fn cancellation_check_bytes(self) -> NonZeroUsize {
        self.cancellation_check_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetError {
    // Variants remain machine-readable for callers.
    ZeroLimit,
    ZeroReadBuffer,
    ZeroViewportRecords,
    ZeroCancellationInterval,
    PreviewExceedsViewport,
    PlanMismatch,
    ArithmeticOverflow,
    ViewportMetadataExceedsScratch { required: usize, available: usize },
    Exceeded { limit: usize, committed: usize },
}

impl fmt::Display for BudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid managed memory budget")
    }
}

impl std::error::Error for BudgetError {}

#[cfg(test)]
mod tests {
    use super::{EngineLimits, ManagedMemoryBudget};

    #[test]
    fn plan_accounts_for_all_managed_buckets() -> Result<(), Box<dyn std::error::Error>> {
        let budget = ManagedMemoryBudget::new(1_984, 256, 512, 128, 1_024)?;
        assert_eq!(budget.committed_bytes(), 1_920);
        assert_eq!(budget.remaining_bytes(), 64);
        let limits = EngineLimits::new(128, 10, 32, 320, 64, budget)?;
        assert_eq!(limits.max_viewport_records().get(), 10);
        Ok(())
    }
}
