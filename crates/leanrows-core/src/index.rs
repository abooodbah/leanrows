use crate::Checkpoint;
use core::fmt;

/// Looks up the latest known record boundary at or before a row.
pub trait CheckpointLookup {
    fn nearest_checkpoint(&self, row: u64) -> Checkpoint;
}

/// Result of observing a candidate checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndexUpdate {
    stored: bool,
    compacted: bool,
}

impl IndexUpdate {
    #[must_use]
    pub const fn stored(self) -> bool {
        self.stored
    }
    #[must_use]
    pub const fn compacted(self) -> bool {
        self.compacted
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cadence {
    Rows,
    Bytes,
}

#[derive(Clone, Debug)]
struct AdaptiveIndex {
    entries: Vec<Checkpoint>,
    capacity_limit: usize,
    stride: u64,
    cadence: Cadence,
    last_observed: Checkpoint,
}

impl AdaptiveIndex {
    fn new(
        capacity: usize,
        initial_stride: u64,
        cadence: Cadence,
    ) -> Result<Self, CheckpointIndexError> {
        if capacity < 2 {
            return Err(CheckpointIndexError::CapacityTooSmall);
        }
        if initial_stride == 0 {
            return Err(CheckpointIndexError::ZeroStride);
        }
        let origin = Checkpoint::new(0, 0);
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(capacity)
            .map_err(|_| CheckpointIndexError::CapacityUnavailable)?;
        entries.push(origin);
        Ok(Self {
            entries,
            capacity_limit: capacity,
            stride: initial_stride,
            cadence,
            last_observed: origin,
        })
    }

    fn observe(&mut self, candidate: Checkpoint) -> Result<IndexUpdate, CheckpointIndexError> {
        if candidate == self.last_observed {
            return Ok(IndexUpdate {
                stored: false,
                compacted: false,
            });
        }
        if candidate.row() <= self.last_observed.row()
            || candidate.offset() <= self.last_observed.offset()
        {
            return Err(CheckpointIndexError::NonMonotonic);
        }
        self.last_observed = candidate;
        let mut compacted = false;
        if self.entries.len() == self.capacity_limit {
            self.compact();
            compacted = true;
        }
        let previous = self.entries[self.entries.len() - 1];
        let delta = match self.cadence {
            Cadence::Rows => candidate.row().saturating_sub(previous.row()),
            Cadence::Bytes => candidate.offset().saturating_sub(previous.offset()),
        };
        let stored = delta >= self.stride;
        if stored {
            self.entries.push(candidate);
        }
        Ok(IndexUpdate { stored, compacted })
    }

    fn compact(&mut self) {
        let old_len = self.entries.len();
        let mut write = 1;
        let mut read = 2;
        while read < old_len {
            self.entries[write] = self.entries[read];
            write += 1;
            read += 2;
        }
        self.entries.truncate(write);
        self.stride = self.stride.saturating_mul(2);
    }

    fn nearest(&self, row: u64) -> Checkpoint {
        let after = self.entries.partition_point(|entry| entry.row() <= row);
        self.entries[after.saturating_sub(1)]
    }
}

/// Fixed-capacity checkpoints sampled by row distance.
#[derive(Clone, Debug)]
pub struct AdaptiveRowIndex {
    inner: AdaptiveIndex,
}

impl AdaptiveRowIndex {
    /// Creates an index containing the `(0, 0)` origin.
    ///
    /// # Errors
    /// Capacity must be at least two and stride must be non-zero.
    pub fn new(capacity: usize, initial_row_stride: u64) -> Result<Self, CheckpointIndexError> {
        Ok(Self {
            inner: AdaptiveIndex::new(capacity, initial_row_stride, Cadence::Rows)?,
        })
    }
    /// Observes a monotonically increasing record boundary.
    ///
    /// # Errors
    /// Returns an error for a regressing row or byte offset.
    pub fn observe(&mut self, checkpoint: Checkpoint) -> Result<IndexUpdate, CheckpointIndexError> {
        self.inner.observe(checkpoint)
    }
    #[must_use]
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.inner.entries
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }
    #[must_use]
    pub const fn capacity_limit(&self) -> usize {
        self.inner.capacity_limit
    }
    #[must_use]
    pub const fn stride(&self) -> u64 {
        self.inner.stride
    }
}

impl CheckpointLookup for AdaptiveRowIndex {
    fn nearest_checkpoint(&self, row: u64) -> Checkpoint {
        self.inner.nearest(row)
    }
}

/// Fixed-capacity checkpoints sampled by byte distance.
#[derive(Clone, Debug)]
pub struct AdaptiveByteIndex {
    inner: AdaptiveIndex,
}

impl AdaptiveByteIndex {
    /// Creates an index containing the `(0, 0)` origin.
    ///
    /// # Errors
    /// Capacity must be at least two and stride must be non-zero.
    pub fn new(capacity: usize, initial_byte_stride: u64) -> Result<Self, CheckpointIndexError> {
        Ok(Self {
            inner: AdaptiveIndex::new(capacity, initial_byte_stride, Cadence::Bytes)?,
        })
    }
    /// Observes a monotonically increasing record boundary.
    ///
    /// # Errors
    /// Returns an error for a regressing row or byte offset.
    pub fn observe(&mut self, checkpoint: Checkpoint) -> Result<IndexUpdate, CheckpointIndexError> {
        self.inner.observe(checkpoint)
    }
    #[must_use]
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.inner.entries
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }
    #[must_use]
    pub const fn capacity_limit(&self) -> usize {
        self.inner.capacity_limit
    }
    #[must_use]
    pub const fn stride(&self) -> u64 {
        self.inner.stride
    }
}

impl CheckpointLookup for AdaptiveByteIndex {
    fn nearest_checkpoint(&self, row: u64) -> Checkpoint {
        self.inner.nearest(row)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointIndexError {
    CapacityTooSmall,
    CapacityUnavailable,
    ZeroStride,
    NonMonotonic,
}

impl fmt::Display for CheckpointIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid adaptive checkpoint index")
    }
}

impl std::error::Error for CheckpointIndexError {}

#[cfg(test)]
mod tests {
    use super::{AdaptiveByteIndex, AdaptiveRowIndex, CheckpointIndexError, CheckpointLookup};
    use crate::Checkpoint;

    #[test]
    fn adaptive_indexes_stay_bounded_and_monotonic() -> Result<(), Box<dyn std::error::Error>> {
        let mut rows = AdaptiveRowIndex::new(17, 1)?;
        let mut bytes = AdaptiveByteIndex::new(13, 8)?;
        let mut offset = 0_u64;
        for row in 1..10_000_u64 {
            offset += 1 + (row.wrapping_mul(1_103_515_245) % 97);
            let checkpoint = Checkpoint::new(row, offset);
            rows.observe(checkpoint)?;
            bytes.observe(checkpoint)?;
            assert!(rows.len() <= rows.capacity_limit());
            assert!(bytes.len() <= bytes.capacity_limit());
        }
        for entries in [rows.checkpoints(), bytes.checkpoints()] {
            assert!(entries.windows(2).all(|pair| pair[0].row() < pair[1].row()));
            assert!(
                entries
                    .windows(2)
                    .all(|pair| pair[0].offset() < pair[1].offset())
            );
        }
        assert!(rows.nearest_checkpoint(5_000).row() <= 5_000);
        assert!(bytes.nearest_checkpoint(5_000).row() <= 5_000);
        Ok(())
    }

    #[test]
    fn either_coordinate_regression_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let mut rows = AdaptiveRowIndex::new(8, 1)?;
        rows.observe(Checkpoint::new(2, 20))?;
        assert_eq!(
            rows.observe(Checkpoint::new(1, 21)),
            Err(CheckpointIndexError::NonMonotonic)
        );
        assert_eq!(
            rows.observe(Checkpoint::new(3, 19)),
            Err(CheckpointIndexError::NonMonotonic)
        );
        Ok(())
    }
}
