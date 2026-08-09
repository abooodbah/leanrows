use crate::{
    CancellationGeneration, CancellationToken, CheckpointLookup, CsvBoundaryScanner, CsvDiagnostic,
    CsvDialect, EmitControl, EngineLimits, LineBoundaryScanner, PositionedRead, RecordSpan,
    ScanError, ScanProgress, ScanStatus, SourceChange, SourceFingerprint,
};
use core::fmt;
use std::io;

/// Boundary rules used to reconstruct rows from a checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportFormat {
    Delimited(CsvDialect),
    Lines,
}

/// One bounded prefix and the full byte span of a logical row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordPreview {
    row: u64,
    span: RecordSpan,
    bytes: Vec<u8>,
    truncated: bool,
}

impl RecordPreview {
    #[must_use]
    pub const fn row(&self) -> u64 {
        self.row
    }
    #[must_use]
    pub const fn span(&self) -> RecordSpan {
        self.span
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

pub(crate) fn required_metadata_capacity_bytes(record_count: usize) -> Option<usize> {
    let bytes_per_record = std::mem::size_of::<(u64, RecordSpan)>()
        .checked_add(std::mem::size_of::<RecordPreview>())?;
    record_count.checked_mul(bytes_per_record)
}

/// A bounded set of records reconstructed from a sparse checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Viewport {
    records: Vec<RecordPreview>,
    reached_eof: bool,
    csv_diagnostic: Option<CsvDiagnostic>,
}

impl Viewport {
    #[must_use]
    pub fn records(&self) -> &[RecordPreview] {
        &self.records
    }
    #[must_use]
    pub const fn reached_eof(&self) -> bool {
        self.reached_eof
    }
    #[must_use]
    pub const fn csv_diagnostic(&self) -> Option<CsvDiagnostic> {
        self.csv_diagnostic
    }
}

enum BoundaryScanner {
    Delimited(CsvBoundaryScanner),
    Lines(LineBoundaryScanner),
}

impl BoundaryScanner {
    fn new(format: ViewportFormat, offset: u64) -> Self {
        match format {
            ViewportFormat::Delimited(dialect) => {
                Self::Delimited(CsvBoundaryScanner::at_offset(dialect, offset))
            }
            ViewportFormat::Lines => Self::Lines(LineBoundaryScanner::at_offset(offset)),
        }
    }

    fn offset(&self) -> u64 {
        match self {
            Self::Delimited(scanner) => scanner.offset(),
            Self::Lines(scanner) => scanner.offset(),
        }
    }
}

struct SpanCollector {
    next_row: u64,
    first_row: u64,
    limit: usize,
    spans: Vec<(u64, RecordSpan)>,
    row_overflow: bool,
}

impl SpanCollector {
    fn accept(&mut self, span: RecordSpan) -> EmitControl {
        let row = self.next_row;
        let Some(next) = row.checked_add(1) else {
            self.row_overflow = true;
            return EmitControl::Stop;
        };
        self.next_row = next;
        if row >= self.first_row {
            self.spans.push((row, span));
        }
        if self.spans.len() == self.limit {
            EmitControl::Stop
        } else {
            EmitControl::Continue
        }
    }
}

/// Reconstructs a viewport from the nearest sparse checkpoint.
///
/// # Errors
/// Returns [`ViewportError`] for I/O, cancellation, scan, or checkpoint failure.
pub fn read_viewport<R, I>(
    source: &R,
    format: ViewportFormat,
    index: &I,
    first_row: u64,
    limits: EngineLimits,
    token: &CancellationToken,
    generation: CancellationGeneration,
) -> Result<Viewport, ViewportError>
where
    R: PositionedRead,
    I: CheckpointLookup,
{
    if token.is_cancelled(generation) {
        return Err(ViewportError::Cancelled);
    }
    let source_snapshot = SourceFingerprint::capture(source)?;
    let source_size = source_snapshot.size();
    let checkpoint = index.nearest_checkpoint(first_row);
    if checkpoint.row() > first_row || checkpoint.offset() > source_size {
        return Err(ViewportError::InvalidCheckpoint);
    }
    let mut scanner = BoundaryScanner::new(format, checkpoint.offset());
    let mut spans = Vec::new();
    spans
        .try_reserve_exact(limits.max_viewport_records().get())
        .map_err(|_| ViewportError::AllocationUnavailable)?;
    let mut collector = SpanCollector {
        next_row: checkpoint.row(),
        first_row,
        limit: limits.max_viewport_records().get(),
        spans,
        row_overflow: false,
    };
    let mut read_buffer = zeroed_buffer(limits.read_buffer_bytes().get())?;
    let mut csv_diagnostic = None;

    while scanner.offset() < source_size && collector.spans.len() < collector.limit {
        if token.is_cancelled(generation) {
            return Err(ViewportError::Cancelled);
        }
        let remaining = source_size - scanner.offset();
        let request = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(read_buffer.len());
        ensure_source_unchanged(source, source_snapshot)?;
        let count = source.read_at(scanner.offset(), &mut read_buffer[..request])?;
        let count = crate::source::validate_read_count(count, request)?;
        if count == 0 {
            return Err(ViewportError::SourceShrank);
        }
        ensure_source_unchanged(source, source_snapshot)?;
        let progress = feed_scanner(
            &mut scanner,
            &read_buffer[..count],
            token,
            generation,
            limits,
            &mut collector,
        )?;
        if collector.row_overflow {
            return Err(ViewportError::RowOverflow);
        }
        match progress.status() {
            ScanStatus::Cancelled => return Err(ViewportError::Cancelled),
            ScanStatus::Stopped => break,
            ScanStatus::Complete => {}
        }
    }

    let scanned_to_eof = scanner.offset() == source_size;
    if scanned_to_eof && collector.spans.len() < collector.limit {
        match scanner {
            BoundaryScanner::Delimited(scanner) => {
                let finish = scanner.finish(|span| collector.accept(span));
                csv_diagnostic = finish.diagnostic();
            }
            BoundaryScanner::Lines(scanner) => {
                scanner.finish(|span| collector.accept(span));
            }
        }
        if collector.row_overflow {
            return Err(ViewportError::RowOverflow);
        }
    }

    let records = materialize_previews(
        source,
        source_snapshot,
        collector.spans,
        limits,
        token,
        generation,
    )?;
    ensure_source_unchanged(source, source_snapshot)?;
    Ok(Viewport {
        records,
        reached_eof: scanned_to_eof,
        csv_diagnostic,
    })
}

fn feed_scanner(
    scanner: &mut BoundaryScanner,
    bytes: &[u8],
    token: &CancellationToken,
    generation: CancellationGeneration,
    limits: EngineLimits,
    collector: &mut SpanCollector,
) -> Result<ScanProgress, ScanError> {
    match scanner {
        BoundaryScanner::Delimited(scanner) => scanner.feed_cancellable(
            bytes,
            token,
            generation,
            limits.cancellation_check_bytes(),
            |span| collector.accept(span),
        ),
        BoundaryScanner::Lines(scanner) => scanner.feed_cancellable(
            bytes,
            token,
            generation,
            limits.cancellation_check_bytes(),
            |span| collector.accept(span),
        ),
    }
}

fn materialize_previews<R: PositionedRead>(
    source: &R,
    source_snapshot: SourceFingerprint,
    spans: Vec<(u64, RecordSpan)>,
    limits: EngineLimits,
    token: &CancellationToken,
    generation: CancellationGeneration,
) -> Result<Vec<RecordPreview>, ViewportError> {
    let mut payload_remaining = limits.max_viewport_payload_bytes();
    let mut records = Vec::new();
    records
        .try_reserve_exact(spans.len())
        .map_err(|_| ViewportError::AllocationUnavailable)?;
    for (row, span) in spans {
        if token.is_cancelled(generation) {
            return Err(ViewportError::Cancelled);
        }
        let record_size = usize::try_from(span.len()).unwrap_or(usize::MAX);
        let copy_size = record_size
            .min(limits.max_record_preview_bytes())
            .min(payload_remaining);
        let mut bytes = zeroed_buffer(copy_size)?;
        let mut filled = 0;
        while filled < copy_size {
            if token.is_cancelled(generation) {
                return Err(ViewportError::Cancelled);
            }
            let relative = u64::try_from(filled).map_err(|_| ViewportError::AddressSpace)?;
            let remaining = copy_size - filled;
            ensure_source_unchanged(source, source_snapshot)?;
            let count = source.read_at(span.start() + relative, &mut bytes[filled..])?;
            let count = crate::source::validate_read_count(count, remaining)?;
            if count == 0 {
                return Err(ViewportError::SourceShrank);
            }
            ensure_source_unchanged(source, source_snapshot)?;
            filled += count;
        }
        payload_remaining -= copy_size;
        records.push(RecordPreview {
            row,
            span,
            bytes,
            truncated: u64::try_from(copy_size).map_or(true, |size| size < span.len()),
        });
    }
    Ok(records)
}

fn ensure_source_unchanged<R: PositionedRead>(
    source: &R,
    baseline: SourceFingerprint,
) -> Result<(), ViewportError> {
    let current = SourceFingerprint::capture(source)?;
    let change = baseline.compare(current);
    if change.is_changed() {
        Err(ViewportError::SourceChanged {
            change,
            expected_size: baseline.size(),
            actual_size: current.size(),
        })
    } else {
        Ok(())
    }
}

fn zeroed_buffer(length: usize) -> Result<Vec<u8>, ViewportError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| ViewportError::AllocationUnavailable)?;
    buffer.resize(length, 0);
    Ok(buffer)
}

#[derive(Debug)]
pub enum ViewportError {
    Io(io::Error),
    Scan(ScanError),
    Cancelled,
    InvalidCheckpoint,
    SourceShrank,
    SourceChanged {
        change: SourceChange,
        expected_size: u64,
        actual_size: u64,
    },
    RowOverflow,
    AddressSpace,
    AllocationUnavailable,
}

impl From<io::Error> for ViewportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ScanError> for ViewportError {
    fn from(error: ScanError) -> Self {
        Self::Scan(error)
    }
}

impl fmt::Display for ViewportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceChanged {
                change,
                expected_size,
                actual_size,
            } => write!(
                formatter,
                "viewport source changed ({change:?}) from size {expected_size} to {actual_size}"
            ),
            Self::SourceShrank => {
                formatter.write_str("viewport source returned EOF before its snapshot boundary")
            }
            _ => formatter.write_str("viewport reconstruction failed"),
        }
    }
}

impl std::error::Error for ViewportError {}

#[cfg(test)]
mod tests {
    use super::{ViewportFormat, read_viewport};
    use crate::{
        AdaptiveRowIndex, CancellationToken, Checkpoint, CsvBoundaryScanner, CsvDialect,
        EmitControl, EngineLimits, ManagedMemoryBudget, PositionedRead, RecordSpan,
    };
    use std::cell::Cell;
    use std::io;

    struct MemorySource {
        bytes: Vec<u8>,
        largest_request: Cell<usize>,
    }

    impl MemorySource {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                largest_request: Cell::new(0),
            }
        }
    }

    impl PositionedRead for MemorySource {
        fn size(&self) -> io::Result<u64> {
            u64::try_from(self.bytes.len()).map_err(io::Error::other)
        }

        fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
            self.largest_request
                .set(self.largest_request.get().max(destination.len()));
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            if start >= self.bytes.len() {
                return Ok(0);
            }
            let count = destination.len().min(self.bytes.len() - start);
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            Ok(count)
        }
    }

    fn limits(
        read: usize,
        records: usize,
        per_record: usize,
        payload: usize,
    ) -> Result<EngineLimits, Box<dyn std::error::Error>> {
        let budget = ManagedMemoryBudget::new(16_384, read, payload, 4_096, 4_096)?;
        Ok(EngineLimits::new(
            read, records, per_record, payload, 31, budget,
        )?)
    }

    #[test]
    fn giant_record_uses_fixed_reads_and_a_bounded_prefix() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut input = vec![b'x'; 2 * 1024 * 1024];
        input.push(b'\n');
        let source = MemorySource::new(input);
        let index = AdaptiveRowIndex::new(8, 1)?;
        let token = CancellationToken::new();
        let viewport = read_viewport(
            &source,
            ViewportFormat::Lines,
            &index,
            0,
            limits(127, 1, 64, 64)?,
            &token,
            token.snapshot(),
        )?;
        assert_eq!(viewport.records().len(), 1);
        assert_eq!(viewport.records()[0].bytes().len(), 64);
        assert!(viewport.records()[0].is_truncated());
        assert!(source.largest_request.get() <= 127);
        Ok(())
    }

    fn all_csv_spans(input: &[u8]) -> Result<Vec<RecordSpan>, Box<dyn std::error::Error>> {
        let mut scanner = CsvBoundaryScanner::new(CsvDialect::csv());
        let mut spans = Vec::new();
        for chunk in input.chunks(11) {
            scanner.feed(chunk, |span| {
                spans.push(span);
                EmitControl::Continue
            })?;
        }
        scanner.finish(|span| {
            spans.push(span);
            EmitControl::Continue
        });
        Ok(spans)
    }

    #[test]
    fn deterministic_random_rows_reconstruct_from_adaptive_checkpoints()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut input = Vec::new();
        for row in 0..300_u64 {
            let text = if row % 7 == 0 {
                format!("\"row {row}\ncontinued\",{}\r\n", row * 3)
            } else {
                format!("row,{row},{}\n", row * 3)
            };
            input.extend_from_slice(text.as_bytes());
        }
        let spans = all_csv_spans(&input)?;
        assert_eq!(spans.len(), 300);
        let mut index = AdaptiveRowIndex::new(23, 1)?;
        for (row, span) in spans.iter().enumerate() {
            index.observe(Checkpoint::new(u64::try_from(row)?, span.start()))?;
        }
        let source = MemorySource::new(input);
        let token = CancellationToken::new();
        let mut state = 0x6a09_e667_f3bc_c909_u64;
        for _ in 0..100 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let target = usize::try_from(state % 300)?;
            let viewport = read_viewport(
                &source,
                ViewportFormat::Delimited(CsvDialect::csv()),
                &index,
                u64::try_from(target)?,
                limits(37, 3, 128, 384)?,
                &token,
                token.snapshot(),
            )?;
            let expected_count = 3_usize.min(spans.len() - target);
            assert_eq!(viewport.records().len(), expected_count);
            for (relative, record) in viewport.records().iter().enumerate() {
                let expected = spans[target + relative];
                assert_eq!(record.row(), u64::try_from(target + relative)?);
                assert_eq!(record.span(), expected);
                let start = usize::try_from(expected.start())?;
                let end = usize::try_from(expected.end())?;
                assert_eq!(record.bytes(), &source.bytes[start..end]);
                assert!(!record.is_truncated());
            }
        }
        Ok(())
    }

    #[test]
    fn cancelled_generation_stops_before_reading() -> Result<(), Box<dyn std::error::Error>> {
        let source = MemorySource::new(b"one\ntwo\n".to_vec());
        let index = AdaptiveRowIndex::new(4, 1)?;
        let token = CancellationToken::new();
        let stale = token.snapshot();
        token.cancel();
        let result = read_viewport(
            &source,
            ViewportFormat::Lines,
            &index,
            0,
            limits(8, 2, 8, 16)?,
            &token,
            stale,
        );
        assert!(result.is_err());
        assert_eq!(source.largest_request.get(), 0);
        Ok(())
    }
}
