use leanrows_core::{
    AdaptiveByteIndex, AdaptiveRowIndex, CancellationToken, Checkpoint, CheckpointLookup,
    DocumentFormat, EngineLimits, ManagedMemoryBudget, PositionedRead, ScanSession,
    ScanSessionConfig, ViewportFormat, read_viewport,
};
use std::cell::Cell;
use std::io;

#[derive(Clone, Debug)]
struct MemorySource(Vec<u8>);

impl PositionedRead for MemorySource {
    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.0.len()).map_err(io::Error::other)
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        if start >= self.0.len() {
            return Ok(0);
        }
        let count = destination.len().min(self.0.len() - start);
        destination[..count].copy_from_slice(&self.0[start..start + count]);
        Ok(count)
    }
}

#[derive(Clone, Copy, Debug)]
struct FixedIndex(Checkpoint);

impl CheckpointLookup for FixedIndex {
    fn nearest_checkpoint(&self, _row: u64) -> Checkpoint {
        self.0
    }
}

#[derive(Clone, Copy)]
enum IndexRef<'a> {
    Rows(&'a AdaptiveRowIndex),
    Bytes(&'a AdaptiveByteIndex),
}

impl CheckpointLookup for IndexRef<'_> {
    fn nearest_checkpoint(&self, row: u64) -> Checkpoint {
        match self {
            Self::Rows(index) => index.nearest_checkpoint(row),
            Self::Bytes(index) => index.nearest_checkpoint(row),
        }
    }
}

fn limits(
    read_bytes: usize,
    records: usize,
    preview_bytes: usize,
    payload_bytes: usize,
) -> Result<EngineLimits, Box<dyn std::error::Error>> {
    let scratch_bytes = 128 * 1024;
    let committed = read_bytes
        .checked_add(payload_bytes)
        .and_then(|value| value.checked_add(scratch_bytes))
        .ok_or("test budget overflow")?;
    let budget = ManagedMemoryBudget::new(committed, read_bytes, payload_bytes, 0, scratch_bytes)?;
    Ok(EngineLimits::new(
        read_bytes,
        records,
        preview_bytes,
        payload_bytes,
        1,
        budget,
    )?)
}

#[test]
fn repeated_compaction_preserves_exact_row_reconstruction() -> Result<(), Box<dyn std::error::Error>>
{
    const ROW_COUNT: usize = 4_096;
    let mut bytes = Vec::new();
    let mut checkpoints = Vec::with_capacity(ROW_COUNT);
    let mut expected = Vec::with_capacity(ROW_COUNT);

    for row in 0..ROW_COUNT {
        let start = u64::try_from(bytes.len())?;
        checkpoints.push(Checkpoint::new(u64::try_from(row)?, start));
        let text = if row % 11 == 0 {
            format!("\"row {row}\ncontinued\",{}", row * 17)
        } else {
            format!("row,{row},{}", row * 17)
        };
        bytes.extend_from_slice(text.as_bytes());
        let end = u64::try_from(bytes.len())?;
        expected.push((start, end));
        if row % 3 == 0 {
            bytes.extend_from_slice(b"\r\n");
        } else {
            bytes.push(b'\n');
        }
    }

    let source = MemorySource(bytes);
    let token = CancellationToken::new();
    for capacity in [2, 3, 4, 5, 7] {
        let mut rows = AdaptiveRowIndex::new(capacity, 1)?;
        let mut byte_sampled = AdaptiveByteIndex::new(capacity, 1)?;
        for checkpoint in &checkpoints {
            rows.observe(*checkpoint)?;
            byte_sampled.observe(*checkpoint)?;
            assert!(rows.len() <= capacity);
            assert!(byte_sampled.len() <= capacity);
        }

        for entries in [rows.checkpoints(), byte_sampled.checkpoints()] {
            assert!(entries.windows(2).all(|pair| {
                pair[0].row() < pair[1].row() && pair[0].offset() < pair[1].offset()
            }));
            for checkpoint in entries {
                let row = usize::try_from(checkpoint.row())?;
                assert_eq!(checkpoint.offset(), expected[row].0);
            }
        }

        let mut state = 0xd1b5_4a32_d192_ed03_u64;
        for _ in 0..64 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let target = usize::try_from(state % u64::try_from(ROW_COUNT)?)?;
            for index in [IndexRef::Rows(&rows), IndexRef::Bytes(&byte_sampled)] {
                let checkpoint = index.nearest_checkpoint(u64::try_from(target)?);
                assert!(checkpoint.row() <= u64::try_from(target)?);
                let viewport = read_viewport(
                    &source,
                    ViewportFormat::Delimited(leanrows_core::CsvDialect::csv()),
                    &index,
                    u64::try_from(target)?,
                    limits(31, 3, 256, 768)?,
                    &token,
                    token.snapshot(),
                )?;
                let count = 3.min(ROW_COUNT - target);
                assert_eq!(viewport.records().len(), count);
                for (relative, record) in viewport.records().iter().enumerate() {
                    let row = target + relative;
                    let (start, end) = expected[row];
                    assert_eq!(record.row(), u64::try_from(row)?);
                    assert_eq!((record.span().start(), record.span().end()), (start, end));
                    let start = usize::try_from(start)?;
                    let end = usize::try_from(end)?;
                    assert_eq!(record.bytes(), &source.0[start..end]);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn checkpoint_coordinates_must_advance_together() -> Result<(), Box<dyn std::error::Error>> {
    let mut index = AdaptiveRowIndex::new(4, 1)?;
    index.observe(Checkpoint::new(1, 10))?;
    assert!(index.observe(Checkpoint::new(1, 11)).is_err());
    assert!(index.observe(Checkpoint::new(2, 10)).is_err());
    assert!(!index.observe(Checkpoint::new(1, 10))?.stored());
    Ok(())
}

#[test]
fn adversarial_capacity_values_fail_without_proportional_allocation()
-> Result<(), Box<dyn std::error::Error>> {
    assert!(AdaptiveRowIndex::new(1, 1).is_err());
    assert!(AdaptiveRowIndex::new(usize::MAX, 1).is_err());

    let budget = ManagedMemoryBudget::new(2, 1, 0, 0, 1)?;
    assert!(EngineLimits::new(1, usize::MAX, 0, 0, 1, budget).is_err());
    assert!(EngineLimits::new(1, 0, 0, 0, 1, budget).is_err());

    let empty = MemorySource(Vec::new());
    assert!(
        ScanSession::new(
            &empty,
            DocumentFormat::Log,
            ScanSessionConfig::new(2, 1, usize::MAX, 1),
        )
        .is_err()
    );

    let huge_read = usize::MAX - 128;
    let huge_budget = ManagedMemoryBudget::new(usize::MAX, huge_read, 0, 0, 128)?;
    let huge_limits = EngineLimits::new(huge_read, 1, 0, 0, 1, huge_budget)?;
    let token = CancellationToken::new();
    assert!(
        read_viewport(
            &empty,
            ViewportFormat::Lines,
            &FixedIndex(Checkpoint::new(0, 0)),
            0,
            huge_limits,
            &token,
            token.snapshot(),
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn zero_byte_previews_and_one_record_caps_remain_well_defined()
-> Result<(), Box<dyn std::error::Error>> {
    let source = MemorySource(b"\nx\n".to_vec());
    let index = AdaptiveRowIndex::new(2, 1)?;
    let token = CancellationToken::new();

    let empty = read_viewport(
        &source,
        ViewportFormat::Lines,
        &index,
        0,
        limits(1, 1, 0, 0)?,
        &token,
        token.snapshot(),
    )?;
    assert_eq!(empty.records().len(), 1);
    assert!(empty.records()[0].bytes().is_empty());
    assert!(!empty.records()[0].is_truncated());

    let nonempty = read_viewport(
        &source,
        ViewportFormat::Lines,
        &index,
        1,
        limits(1, 1, 0, 0)?,
        &token,
        token.snapshot(),
    )?;
    assert_eq!(nonempty.records().len(), 1);
    assert!(nonempty.records()[0].bytes().is_empty());
    assert!(nonempty.records()[0].is_truncated());
    Ok(())
}

struct OversizedCountSource;

impl PositionedRead for OversizedCountSource {
    fn size(&self) -> io::Result<u64> {
        Ok(1)
    }

    fn read_at(&self, _offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        Ok(destination.len().saturating_add(1))
    }
}

struct OversizedMaterializeSource {
    calls: Cell<usize>,
}

impl PositionedRead for OversizedMaterializeSource {
    fn size(&self) -> io::Result<u64> {
        Ok(2)
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        let call = self.calls.get();
        self.calls.set(call.saturating_add(1));
        if call == 0 {
            let input = b"x\n";
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            let count = destination.len().min(input.len().saturating_sub(start));
            destination[..count].copy_from_slice(&input[start..start + count]);
            Ok(count)
        } else {
            Ok(destination.len().saturating_add(1))
        }
    }
}

#[test]
fn invalid_positioned_read_counts_are_errors_instead_of_panics()
-> Result<(), Box<dyn std::error::Error>> {
    let token = CancellationToken::new();
    let generation = token.snapshot();
    let mut session = ScanSession::new(
        &OversizedCountSource,
        DocumentFormat::Log,
        ScanSessionConfig::new(2, 1, 1, 1),
    )?;
    assert!(
        session
            .step(&OversizedCountSource, 1, &token, generation)
            .is_err()
    );

    let index = FixedIndex(Checkpoint::new(0, 0));
    assert!(
        read_viewport(
            &OversizedCountSource,
            ViewportFormat::Lines,
            &index,
            0,
            limits(1, 1, 1, 1)?,
            &token,
            generation,
        )
        .is_err()
    );

    let materialize = OversizedMaterializeSource {
        calls: Cell::new(0),
    };
    assert!(
        read_viewport(
            &materialize,
            ViewportFormat::Lines,
            &index,
            0,
            limits(2, 1, 1, 1)?,
            &token,
            generation,
        )
        .is_err()
    );
    Ok(())
}

struct TailSource {
    start: u64,
    bytes: Vec<u8>,
}

impl PositionedRead for TailSource {
    fn size(&self) -> io::Result<u64> {
        Ok(u64::MAX)
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        let relative = offset
            .checked_sub(self.start)
            .ok_or_else(|| io::Error::other("read before synthetic tail"))?;
        let start = usize::try_from(relative).map_err(io::Error::other)?;
        if start >= self.bytes.len() {
            return Ok(0);
        }
        let count = destination.len().min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[test]
fn viewport_arithmetic_is_checked_near_u64_max() -> Result<(), Box<dyn std::error::Error>> {
    let start = u64::MAX - 3;
    let source = TailSource {
        start,
        bytes: vec![0xff, b'\r', b'\n'],
    };
    let row = u64::MAX - 1;
    let index = FixedIndex(Checkpoint::new(row, start));
    let token = CancellationToken::new();
    let viewport = read_viewport(
        &source,
        ViewportFormat::Lines,
        &index,
        row,
        limits(1, 1, 1, 1)?,
        &token,
        token.snapshot(),
    )?;
    assert!(viewport.reached_eof());
    assert_eq!(viewport.records().len(), 1);
    assert_eq!(viewport.records()[0].row(), row);
    assert_eq!(viewport.records()[0].span().start(), start);
    assert_eq!(viewport.records()[0].span().end(), start + 1);
    assert_eq!(viewport.records()[0].bytes(), &[0xff]);
    Ok(())
}
