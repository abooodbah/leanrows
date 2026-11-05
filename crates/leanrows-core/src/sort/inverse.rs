//! Builds the source-row-to-rank map with sequential writes.
//!
//! Ranks arrive in sorted order, so their source rows are spread over the
//! whole map. Writing each rank in place costs a seek and an 8-byte write per
//! row. Instead each (row, rank) pair is appended to the bucket that holds
//! its block of rows, and each bucket is then placed in memory and written to
//! the map in large pieces. The buckets share one scratch file, `pairs.bin`,
//! whose regions are exact because every row occurs once.
use super::storage::{DiskFile, Owner, Reservation, read_exact_at, write_all_at};
use super::{SortError, SortResources};
use std::sync::Arc;

/// Rows per bucket. One bucket's ranks are in memory while it is placed.
pub(super) const BUCKET_ROWS: u64 = 512 * 1024;
/// Write buffer for each bucket while pairs are collected.
pub(super) const BUCKET_BUFFER_BYTES: usize = 16 * 1024;
/// Bytes read or written at a time while a bucket is placed.
const CHUNK_BYTES: usize = 64 * 1024;
const PAIR_BYTES: usize = 16;
const MAP_HEADER_BYTES: u64 = 32;

pub(super) struct InverseBuilder {
    records: u64,
    bucket_rows: u64,
    buckets: usize,
    buffer_bytes: usize,
    pairs: DiskFile,
    buffer: Vec<u8>,
    filled: Vec<usize>,
    written: Vec<u64>,
    ranks: Vec<u64>,
    chunk: Vec<u8>,
    bucket: usize,
    read: u64,
    _memory: Reservation,
}

impl InverseBuilder {
    pub fn new(
        owner: &Arc<Owner>,
        resources: &Arc<SortResources>,
        records: u64,
        bucket_rows: u64,
        buffer_bytes: usize,
    ) -> Result<Self, SortError> {
        if bucket_rows == 0 || buffer_bytes < PAIR_BYTES || !buffer_bytes.is_multiple_of(PAIR_BYTES)
        {
            return Err(SortError::InvalidConfig);
        }
        let buckets =
            usize::try_from(records.div_ceil(bucket_rows)).map_err(|_| SortError::Allocation)?;
        let held_rows =
            usize::try_from(bucket_rows.min(records)).map_err(|_| SortError::Allocation)?;
        let bytes = buckets
            .checked_mul(buffer_bytes + 2 * std::mem::size_of::<u64>())
            .and_then(|n| n.checked_add(held_rows.checked_mul(std::mem::size_of::<u64>())?))
            .and_then(|n| n.checked_add(CHUNK_BYTES))
            .ok_or(SortError::Allocation)?;
        let memory = resources.reserve(bytes as u64, true)?;
        let mut pairs = DiskFile::create(owner, resources, "pairs.bin")?;
        pairs.allocate(
            records
                .checked_mul(PAIR_BYTES as u64)
                .ok_or(SortError::InvalidScratch)?,
        )?;
        Ok(Self {
            records,
            bucket_rows,
            buckets,
            buffer_bytes,
            pairs,
            buffer: zeroed(
                buckets
                    .checked_mul(buffer_bytes)
                    .ok_or(SortError::Allocation)?,
            )?,
            filled: zeroed(buckets)?,
            written: zeroed(buckets)?,
            ranks: zeroed(held_rows)?,
            chunk: zeroed(CHUNK_BYTES)?,
            bucket: 0,
            read: 0,
            _memory: memory,
        })
    }

    fn bucket_start(&self, bucket: usize) -> u64 {
        bucket as u64 * self.bucket_rows
    }

    fn bucket_len(&self, bucket: usize) -> u64 {
        self.bucket_rows
            .min(self.records - self.bucket_start(bucket))
    }

    /// Records that source row `row` is shown at `rank`.
    pub fn push(&mut self, row: u64, rank: u64) -> Result<(), SortError> {
        if row >= self.records || rank >= self.records {
            return Err(SortError::InvalidScratch);
        }
        let bucket =
            usize::try_from(row / self.bucket_rows).map_err(|_| SortError::InvalidScratch)?;
        let at = bucket * self.buffer_bytes + self.filled[bucket];
        self.buffer[at..at + 8].copy_from_slice(&row.to_le_bytes());
        self.buffer[at + 8..at + 16].copy_from_slice(&rank.to_le_bytes());
        self.filled[bucket] += PAIR_BYTES;
        if self.filled[bucket] == self.buffer_bytes {
            self.flush(bucket)?;
        }
        Ok(())
    }

    fn flush(&mut self, bucket: usize) -> Result<(), SortError> {
        let count = self.filled[bucket];
        if count == 0 {
            return Ok(());
        }
        // A bucket receives exactly one pair per row in its block; more
        // would mean a row was ranked twice.
        if self.written[bucket] + count as u64 > self.bucket_len(bucket) * PAIR_BYTES as u64 {
            return Err(SortError::InvalidScratch);
        }
        let offset = self.bucket_start(bucket) * PAIR_BYTES as u64 + self.written[bucket];
        let start = bucket * self.buffer_bytes;
        write_all_at(&self.pairs, offset, &self.buffer[start..start + count])?;
        self.written[bucket] += count as u64;
        self.filled[bucket] = 0;
        Ok(())
    }

    /// Writes what is left in the bucket buffers once every rank is pushed.
    pub fn finish_collecting(&mut self) -> Result<(), SortError> {
        for bucket in 0..self.buckets {
            self.flush(bucket)?;
            if self.written[bucket] != self.bucket_len(bucket) * PAIR_BYTES as u64 {
                return Err(SortError::InvalidScratch);
            }
        }
        Ok(())
    }

    /// Places up to one chunk of pairs, and writes a bucket to the map once
    /// all of its pairs are placed. Returns true when the map is complete.
    pub fn place(&mut self, inverse: &DiskFile, budget: &mut usize) -> Result<bool, SortError> {
        if self.bucket == self.buckets {
            return Ok(true);
        }
        let base = self.bucket_start(self.bucket);
        let len = self.bucket_len(self.bucket);
        let total = len * PAIR_BYTES as u64;
        let count = usize::try_from((total - self.read).min(CHUNK_BYTES as u64))
            .map_err(|_| SortError::InvalidScratch)?;
        let held = usize::try_from(len).map_err(|_| SortError::InvalidScratch)?;
        if self.read == 0 {
            // No rank equals u64::MAX, so a slot still holding it is empty.
            self.ranks[..held].fill(u64::MAX);
        }
        read_exact_at(
            &self.pairs.file,
            base * PAIR_BYTES as u64 + self.read,
            &mut self.chunk[..count],
        )?;
        for pair in self.chunk[..count].chunks_exact(PAIR_BYTES) {
            let row = u64::from_le_bytes(
                pair[..8]
                    .try_into()
                    .map_err(|_| SortError::InvalidScratch)?,
            );
            let rank = u64::from_le_bytes(
                pair[8..]
                    .try_into()
                    .map_err(|_| SortError::InvalidScratch)?,
            );
            let slot = row
                .checked_sub(base)
                .filter(|slot| *slot < len)
                .ok_or(SortError::InvalidScratch)?;
            let slot =
                &mut self.ranks[usize::try_from(slot).map_err(|_| SortError::InvalidScratch)?];
            // Each bucket holds one pair per row, so a row seen twice means
            // another row of the bucket is missing.
            if *slot != u64::MAX {
                return Err(SortError::InvalidScratch);
            }
            *slot = rank;
        }
        self.read += count as u64;
        *budget = budget.saturating_sub(count);
        if self.read == total {
            let rows = held;
            let per_chunk = CHUNK_BYTES / std::mem::size_of::<u64>();
            let mut done = 0;
            while done < rows {
                let take = (rows - done).min(per_chunk);
                for (index, rank) in self.ranks[done..done + take].iter().enumerate() {
                    self.chunk[index * 8..index * 8 + 8].copy_from_slice(&rank.to_le_bytes());
                }
                write_all_at(
                    inverse,
                    MAP_HEADER_BYTES + (base + done as u64) * 8,
                    &self.chunk[..take * 8],
                )?;
                done += take;
            }
            self.bucket += 1;
            self.read = 0;
        }
        Ok(self.bucket == self.buckets)
    }
}

fn zeroed<T: Clone + Default>(len: usize) -> Result<Vec<T>, SortError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| SortError::Allocation)?;
    values.resize(len, T::default());
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::{InverseBuilder, MAP_HEADER_BYTES};
    use crate::sort::storage::{DiskFile, Owner, map_header, read_exact_at};
    use crate::sort::{SortError, SortResources};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> Result<Scratch, SortError> {
        let path = std::env::temp_dir().join(format!(
            "leanrows-inverse-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(Scratch(path))
    }

    /// Builds the inverse map of `rows` (the source row shown at each rank)
    /// with small buckets and returns the rank stored for every source row.
    fn build(rows: &[u64], bucket_rows: u64, buffer_bytes: usize) -> Result<Vec<u64>, SortError> {
        let scratch = scratch()?;
        let resources = SortResources::new(16 * 1024 * 1024, 1024 * 1024);
        let records = rows.len() as u64;
        let ranks = {
            let owner = Owner::create(&scratch.0, &resources)?;
            let mut inverse = DiskFile::create(&owner, &resources, "inverse.bin")?;
            inverse.append(&map_header(records, 0, 8))?;
            inverse.allocate(records * 8)?;
            let mut builder =
                InverseBuilder::new(&owner, &resources, records, bucket_rows, buffer_bytes)?;
            for (rank, row) in rows.iter().enumerate() {
                builder.push(*row, rank as u64)?;
            }
            builder.finish_collecting()?;
            let mut budget = usize::MAX;
            while !builder.place(&inverse, &mut budget)? {}
            let mut bytes = vec![0; rows.len() * 8];
            read_exact_at(&inverse.file, MAP_HEADER_BYTES, &mut bytes)?;
            bytes
                .chunks_exact(8)
                .map(|b| b.try_into().map(u64::from_le_bytes))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| SortError::InvalidScratch)?
        };
        assert_eq!(resources.memory_bytes(), 0);
        assert_eq!(resources.disk_bytes(), 0);
        Ok(ranks)
    }

    #[test]
    fn every_row_gets_its_rank_across_bucket_edges() -> Result<(), SortError> {
        for count in [1_u64, 2, 3, 4, 10, 97] {
            let rows: Vec<u64> = (0..count).map(|rank| (rank * 31 + 7) % count).collect();
            for bucket_rows in [1, 2, 3, 5, 200] {
                for buffer_bytes in [16, 32, 48] {
                    let ranks = build(&rows, bucket_rows, buffer_bytes)?;
                    for (rank, row) in rows.iter().enumerate() {
                        assert_eq!(
                            ranks[usize::try_from(*row).unwrap_or_default()],
                            rank as u64,
                            "{count} rows, buckets of {bucket_rows}, {buffer_bytes}-byte buffers"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn placing_uses_the_work_budget() -> Result<(), SortError> {
        // A step stops once its budget is spent, so placing must charge the
        // bytes it reads; otherwise one step would place the whole map.
        let scratch = scratch()?;
        let resources = SortResources::new(16 * 1024 * 1024, 1024 * 1024);
        let owner = Owner::create(&scratch.0, &resources)?;
        let mut inverse = DiskFile::create(&owner, &resources, "inverse.bin")?;
        inverse.append(&map_header(100, 0, 8))?;
        inverse.allocate(800)?;
        let mut builder = InverseBuilder::new(&owner, &resources, 100, 50, 16)?;
        for rank in 0..100 {
            builder.push(99 - rank, rank)?;
        }
        builder.finish_collecting()?;
        let mut budget = 1_000;
        assert!(!builder.place(&inverse, &mut budget)?);
        assert_eq!(budget, 200, "50 pairs of 16 bytes were read");
        Ok(())
    }

    #[test]
    fn a_row_ranked_twice_or_out_of_range_is_refused() {
        // Row 1 twice and row 2 never: its bucket overflows or comes up short.
        for bucket_rows in [1, 2, 4] {
            assert!(matches!(
                build(&[0, 1, 1, 3], bucket_rows, 16),
                Err(SortError::InvalidScratch)
            ));
        }
        assert!(matches!(
            build(&[0, 4, 1, 2], 2, 16),
            Err(SortError::InvalidScratch)
        ));
    }

    #[test]
    fn buffers_count_against_the_memory_limit() -> Result<(), SortError> {
        let scratch = scratch()?;
        let resources = SortResources::new(1024 * 1024, 1024 * 1024 * 1024);
        let owner = Owner::create(&scratch.0, &resources)?;
        // 1M rows in buckets of 512 Ki hold 4 MiB of ranks: over a 1 MiB limit.
        let refused = InverseBuilder::new(&owner, &resources, 1 << 20, 512 * 1024, 16 * 1024);
        assert!(matches!(refused, Err(SortError::Allocation)));
        assert_eq!(resources.memory_bytes(), 0);
        Ok(())
    }
}
