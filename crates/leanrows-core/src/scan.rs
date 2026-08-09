use crate::{CancellationGeneration, CancellationToken, RecordSpan};
use core::fmt;
use std::num::NonZeroUsize;

/// Delimiter and quote bytes for a CSV-like record stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CsvDialect {
    delimiter: u8,
    quote: u8,
}

impl CsvDialect {
    /// Creates a dialect whose delimiter and quote are distinct non-newline bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CsvDialectError`] for ambiguous or newline-valued syntax bytes.
    pub const fn new(delimiter: u8, quote: u8) -> Result<Self, CsvDialectError> {
        if delimiter == quote {
            return Err(CsvDialectError::SameDelimiterAndQuote);
        }
        if delimiter == b'\r' || delimiter == b'\n' {
            return Err(CsvDialectError::NewlineDelimiter);
        }
        if quote == b'\r' || quote == b'\n' {
            return Err(CsvDialectError::NewlineQuote);
        }
        Ok(Self { delimiter, quote })
    }

    /// Standard comma-separated values.
    #[must_use]
    pub const fn csv() -> Self {
        Self {
            delimiter: b',',
            quote: b'"',
        }
    }

    /// Tab-separated values with CSV-style quoting.
    #[must_use]
    pub const fn tsv() -> Self {
        Self {
            delimiter: b'\t',
            quote: b'"',
        }
    }

    /// Returns the field delimiter byte.
    #[must_use]
    pub const fn delimiter(self) -> u8 {
        self.delimiter
    }

    /// Returns the quote byte.
    #[must_use]
    pub const fn quote(self) -> u8 {
        self.quote
    }
}

impl Default for CsvDialect {
    fn default() -> Self {
        Self::csv()
    }
}

/// Why a CSV dialect could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsvDialectError {
    /// The same byte cannot identify both a delimiter and a quote.
    SameDelimiterAndQuote,
    /// Record separators cannot also delimit fields.
    NewlineDelimiter,
    /// Newline bytes cannot quote fields.
    NewlineQuote,
}

impl fmt::Display for CsvDialectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SameDelimiterAndQuote => formatter.write_str("delimiter and quote must differ"),
            Self::NewlineDelimiter => formatter.write_str("delimiter cannot be CR or LF"),
            Self::NewlineQuote => formatter.write_str("quote cannot be CR or LF"),
        }
    }
}

impl std::error::Error for CsvDialectError {}

/// Controls whether a scanner should continue after emitting a record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitControl {
    /// Continue scanning.
    Continue,
    /// Stop at the current record boundary.
    Stop,
}

/// Why a scanner returned to its caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanStatus {
    /// Every supplied byte was consumed.
    Complete,
    /// The record callback requested a stop.
    Stopped,
    /// The operation's cancellation generation became stale.
    Cancelled,
}

/// Progress made by one incremental scanner call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanProgress {
    consumed: usize,
    status: ScanStatus,
}

impl ScanProgress {
    /// Returns how many bytes from the supplied chunk became scanner state.
    #[must_use]
    pub const fn consumed(self) -> usize {
        self.consumed
    }

    /// Returns why control came back to the caller.
    #[must_use]
    pub const fn status(self) -> ScanStatus {
        self.status
    }
}

/// A scanner failure independent of file contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanError {
    /// The absolute byte position exceeded `u64::MAX`.
    OffsetOverflow,
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("scanner byte offset overflowed u64")
    }
}

impl std::error::Error for ScanError {}

/// A content diagnostic found when a CSV stream ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CsvDiagnostic {
    kind: CsvDiagnosticKind,
    offset: u64,
    record_start: u64,
}

impl CsvDiagnostic {
    /// Returns the diagnostic category.
    #[must_use]
    pub const fn kind(self) -> CsvDiagnosticKind {
        self.kind
    }

    /// Returns the end-of-input offset at which the issue was confirmed.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Returns the byte offset at which the malformed record starts.
    #[must_use]
    pub const fn record_start(self) -> u64 {
        self.record_start
    }
}

/// Kinds of end-of-input CSV diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsvDiagnosticKind {
    /// Input ended while a quoted field was still open.
    UnterminatedQuotedField,
}

/// Final state returned after consuming a CSV scanner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CsvFinish {
    emitted: usize,
    stopped: bool,
    diagnostic: Option<CsvDiagnostic>,
}

impl CsvFinish {
    /// Returns the number of records emitted during finalization.
    #[must_use]
    pub const fn emitted(self) -> usize {
        self.emitted
    }

    /// Reports whether the callback stopped finalization.
    #[must_use]
    pub const fn stopped(self) -> bool {
        self.stopped
    }

    /// Returns an unterminated-quote diagnostic, when present.
    #[must_use]
    pub const fn diagnostic(self) -> Option<CsvDiagnostic> {
        self.diagnostic
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CsvState {
    FieldStart,
    Unquoted,
    Quoted,
    QuoteClosed,
}

/// Incrementally discovers CSV/TSV record boundaries without retaining fields.
#[derive(Clone, Debug)]
pub struct CsvBoundaryScanner {
    dialect: CsvDialect,
    state: CsvState,
    offset: u64,
    record_start: u64,
    pending_cr_end: Option<u64>,
}

impl CsvBoundaryScanner {
    /// Creates a scanner positioned at byte zero and a record boundary.
    #[must_use]
    pub const fn new(dialect: CsvDialect) -> Self {
        Self::at_offset(dialect, 0)
    }

    /// Creates a scanner at a known record-start checkpoint.
    #[must_use]
    pub const fn at_offset(dialect: CsvDialect, offset: u64) -> Self {
        Self {
            dialect,
            state: CsvState::FieldStart,
            offset,
            record_start: offset,
            pending_cr_end: None,
        }
    }

    /// Returns the absolute position immediately after all consumed bytes.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Scans a chunk and calls `emit` once per complete logical record.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::OffsetOverflow`] if the absolute position cannot be
    /// represented by `u64`.
    pub fn feed<F>(&mut self, chunk: &[u8], emit: F) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        self.feed_inner(chunk, None, emit)
    }

    /// Scans a chunk while periodically observing a cancellation generation.
    ///
    /// `check_every` bounds cancellation latency in processed bytes. A cancelled
    /// call leaves the first unconsumed byte untouched so callers may restart
    /// from the returned count.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::OffsetOverflow`] if the absolute position cannot be
    /// represented by `u64`.
    pub fn feed_cancellable<F>(
        &mut self,
        chunk: &[u8],
        token: &CancellationToken,
        generation: CancellationGeneration,
        check_every: NonZeroUsize,
        emit: F,
    ) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        self.feed_inner(
            chunk,
            Some(CancelCheck {
                token,
                generation,
                every: check_every.get(),
            }),
            emit,
        )
    }

    fn feed_inner<F>(
        &mut self,
        chunk: &[u8],
        cancel: Option<CancelCheck<'_>>,
        mut emit: F,
    ) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        let mut consumed = 0;
        while consumed < chunk.len() {
            if cancel.is_some_and(|check| {
                consumed % check.every == 0 && check.token.is_cancelled(check.generation)
            }) {
                return Ok(ScanProgress {
                    consumed,
                    status: ScanStatus::Cancelled,
                });
            }

            let byte = chunk[consumed];
            if let Some(record_end) = self.pending_cr_end {
                if byte == b'\n' {
                    self.pending_cr_end = None;
                    self.advance_offset()?;
                    consumed += 1;
                    if self.complete_record(record_end, self.offset, &mut emit) {
                        return Ok(ScanProgress {
                            consumed,
                            status: ScanStatus::Stopped,
                        });
                    }
                    continue;
                }

                self.pending_cr_end = None;
                if self.complete_record(record_end, self.offset, &mut emit) {
                    return Ok(ScanProgress {
                        consumed,
                        status: ScanStatus::Stopped,
                    });
                }
            }

            if self.state != CsvState::Quoted && byte == b'\r' {
                self.pending_cr_end = Some(self.offset);
                self.advance_offset()?;
                consumed += 1;
                continue;
            }
            if self.state != CsvState::Quoted && byte == b'\n' {
                let record_end = self.offset;
                self.advance_offset()?;
                consumed += 1;
                if self.complete_record(record_end, self.offset, &mut emit) {
                    return Ok(ScanProgress {
                        consumed,
                        status: ScanStatus::Stopped,
                    });
                }
                continue;
            }

            self.state = match (self.state, byte) {
                (CsvState::FieldStart, value) if value == self.dialect.quote => CsvState::Quoted,
                (CsvState::FieldStart, value) if value == self.dialect.delimiter => {
                    CsvState::FieldStart
                }
                (CsvState::Unquoted, value) if value == self.dialect.delimiter => {
                    CsvState::FieldStart
                }
                (CsvState::Quoted, value) if value == self.dialect.quote => CsvState::QuoteClosed,
                (CsvState::Quoted, _) => CsvState::Quoted,
                (CsvState::QuoteClosed, value) if value == self.dialect.quote => CsvState::Quoted,
                (CsvState::QuoteClosed, value) if value == self.dialect.delimiter => {
                    CsvState::FieldStart
                }
                (CsvState::FieldStart | CsvState::Unquoted | CsvState::QuoteClosed, _) => {
                    CsvState::Unquoted
                }
            };
            self.advance_offset()?;
            consumed += 1;
        }

        Ok(ScanProgress {
            consumed,
            status: ScanStatus::Complete,
        })
    }

    /// Consumes the scanner, emits a final unterminated record, and diagnoses an
    /// open quoted field without attempting content repair.
    pub fn finish<F>(mut self, mut emit: F) -> CsvFinish
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        let diagnostic = (self.state == CsvState::Quoted).then_some(CsvDiagnostic {
            kind: CsvDiagnosticKind::UnterminatedQuotedField,
            offset: self.offset,
            record_start: self.record_start,
        });
        let mut emitted = 0;

        if let Some(record_end) = self.pending_cr_end.take() {
            emitted += 1;
            let stopped = self.complete_record(record_end, self.offset, &mut emit);
            return CsvFinish {
                emitted,
                stopped,
                diagnostic,
            };
        }

        if self.record_start < self.offset {
            emitted += 1;
            let span = RecordSpan::new(self.record_start, self.offset)
                .unwrap_or_else(|_| unreachable!("scanner offsets are monotonic"));
            let stopped = emit(span) == EmitControl::Stop;
            return CsvFinish {
                emitted,
                stopped,
                diagnostic,
            };
        }

        CsvFinish {
            emitted,
            stopped: false,
            diagnostic,
        }
    }

    fn advance_offset(&mut self) -> Result<(), ScanError> {
        self.offset = self
            .offset
            .checked_add(1)
            .ok_or(ScanError::OffsetOverflow)?;
        Ok(())
    }

    fn complete_record<F>(&mut self, end: u64, next_start: u64, emit: &mut F) -> bool
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        let span = RecordSpan::new(self.record_start, end)
            .unwrap_or_else(|_| unreachable!("scanner offsets are monotonic"));
        self.record_start = next_start;
        self.state = CsvState::FieldStart;
        emit(span) == EmitControl::Stop
    }
}

#[derive(Clone, Copy)]
struct CancelCheck<'a> {
    token: &'a CancellationToken,
    generation: CancellationGeneration,
    every: usize,
}

/// Final state returned after consuming a line scanner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineFinish {
    emitted: usize,
    stopped: bool,
}

impl LineFinish {
    /// Returns the number of records emitted during finalization.
    #[must_use]
    pub const fn emitted(self) -> usize {
        self.emitted
    }

    /// Reports whether the callback stopped finalization.
    #[must_use]
    pub const fn stopped(self) -> bool {
        self.stopped
    }
}

/// Incrementally discovers CR, LF, and CRLF-delimited line boundaries.
#[derive(Clone, Debug)]
pub struct LineBoundaryScanner {
    offset: u64,
    record_start: u64,
    pending_cr_end: Option<u64>,
}

impl LineBoundaryScanner {
    /// Creates a scanner at byte zero.
    #[must_use]
    pub const fn new() -> Self {
        Self::at_offset(0)
    }

    /// Creates a scanner at a known line boundary.
    #[must_use]
    pub const fn at_offset(offset: u64) -> Self {
        Self {
            offset,
            record_start: offset,
            pending_cr_end: None,
        }
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Scans a line-oriented chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::OffsetOverflow`] if the absolute position cannot be
    /// represented by `u64`.
    pub fn feed<F>(&mut self, chunk: &[u8], emit: F) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        self.feed_inner(chunk, None, emit)
    }

    /// Scans a line-oriented chunk with cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::OffsetOverflow`] if the absolute position cannot be
    /// represented by `u64`.
    pub fn feed_cancellable<F>(
        &mut self,
        chunk: &[u8],
        token: &CancellationToken,
        generation: CancellationGeneration,
        check_every: NonZeroUsize,
        emit: F,
    ) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        self.feed_inner(
            chunk,
            Some(CancelCheck {
                token,
                generation,
                every: check_every.get(),
            }),
            emit,
        )
    }

    fn feed_inner<F>(
        &mut self,
        chunk: &[u8],
        cancel: Option<CancelCheck<'_>>,
        mut emit: F,
    ) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        let mut consumed = 0;
        while consumed < chunk.len() {
            if cancel.is_some_and(|check| {
                consumed % check.every == 0 && check.token.is_cancelled(check.generation)
            }) {
                return Ok(ScanProgress {
                    consumed,
                    status: ScanStatus::Cancelled,
                });
            }
            let byte = chunk[consumed];

            if let Some(record_end) = self.pending_cr_end {
                if byte == b'\n' {
                    self.pending_cr_end = None;
                    self.advance_offset()?;
                    consumed += 1;
                    if self.complete_record(record_end, self.offset, &mut emit) {
                        return Ok(ScanProgress {
                            consumed,
                            status: ScanStatus::Stopped,
                        });
                    }
                    continue;
                }
                self.pending_cr_end = None;
                if self.complete_record(record_end, self.offset, &mut emit) {
                    return Ok(ScanProgress {
                        consumed,
                        status: ScanStatus::Stopped,
                    });
                }
            }

            if byte == b'\r' {
                self.pending_cr_end = Some(self.offset);
                self.advance_offset()?;
                consumed += 1;
                continue;
            }
            if byte == b'\n' {
                let record_end = self.offset;
                self.advance_offset()?;
                consumed += 1;
                if self.complete_record(record_end, self.offset, &mut emit) {
                    return Ok(ScanProgress {
                        consumed,
                        status: ScanStatus::Stopped,
                    });
                }
                continue;
            }
            self.advance_offset()?;
            consumed += 1;
        }
        Ok(ScanProgress {
            consumed,
            status: ScanStatus::Complete,
        })
    }

    /// Consumes the scanner and emits a final unterminated line.
    pub fn finish<F>(mut self, mut emit: F) -> LineFinish
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        if let Some(record_end) = self.pending_cr_end.take() {
            let stopped = self.complete_record(record_end, self.offset, &mut emit);
            return LineFinish {
                emitted: 1,
                stopped,
            };
        }
        if self.record_start < self.offset {
            let span = RecordSpan::new(self.record_start, self.offset)
                .unwrap_or_else(|_| unreachable!("scanner offsets are monotonic"));
            return LineFinish {
                emitted: 1,
                stopped: emit(span) == EmitControl::Stop,
            };
        }
        LineFinish {
            emitted: 0,
            stopped: false,
        }
    }

    fn advance_offset(&mut self) -> Result<(), ScanError> {
        self.offset = self
            .offset
            .checked_add(1)
            .ok_or(ScanError::OffsetOverflow)?;
        Ok(())
    }

    fn complete_record<F>(&mut self, end: u64, next_start: u64, emit: &mut F) -> bool
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        let span = RecordSpan::new(self.record_start, end)
            .unwrap_or_else(|_| unreachable!("scanner offsets are monotonic"));
        self.record_start = next_start;
        emit(span) == EmitControl::Stop
    }
}

impl Default for LineBoundaryScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CsvBoundaryScanner, CsvDiagnosticKind, CsvDialect, EmitControl, LineBoundaryScanner,
        ScanError, ScanStatus,
    };
    use crate::RecordSpan;

    fn csv_spans(
        input: &[u8],
        chunk_size: usize,
    ) -> Result<(Vec<RecordSpan>, Option<CsvDiagnosticKind>), ScanError> {
        let mut scanner = CsvBoundaryScanner::new(CsvDialect::csv());
        let mut spans = Vec::new();
        for chunk in input.chunks(chunk_size) {
            let progress = scanner.feed(chunk, |span| {
                spans.push(span);
                EmitControl::Continue
            })?;
            assert_eq!(progress.status(), ScanStatus::Complete);
            assert_eq!(progress.consumed(), chunk.len());
        }
        let finish = scanner.finish(|span| {
            spans.push(span);
            EmitControl::Continue
        });
        Ok((spans, finish.diagnostic().map(super::CsvDiagnostic::kind)))
    }

    fn line_spans(input: &[u8], chunk_size: usize) -> Result<Vec<RecordSpan>, ScanError> {
        let mut scanner = LineBoundaryScanner::new();
        let mut spans = Vec::new();
        for chunk in input.chunks(chunk_size) {
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
    fn csv_boundaries_are_identical_for_every_chunk_size() -> Result<(), Box<dyn std::error::Error>>
    {
        let input = b"a,b\r\n\"multi\r\nline\",\"escaped \"\"quote\"\"\"\n\rbare,cr\rlast,";
        let baseline = csv_spans(input, input.len())?;
        for chunk_size in 1..=input.len() {
            assert_eq!(csv_spans(input, chunk_size)?, baseline);
        }
        let mut rows = Vec::new();
        for span in &baseline.0 {
            let start = usize::try_from(span.start())?;
            let end = usize::try_from(span.end())?;
            rows.push(&input[start..end]);
        }
        assert_eq!(
            rows,
            vec![
                &b"a,b"[..],
                &b"\"multi\r\nline\",\"escaped \"\"quote\"\"\""[..],
                &b""[..],
                &b"bare,cr"[..],
                &b"last,"[..]
            ]
        );
        Ok(())
    }

    #[test]
    fn unterminated_quote_is_reported_at_eof_for_every_chunk_size() -> Result<(), ScanError> {
        let input = b"ok\n\"never\nclosed";
        for chunk_size in 1..=input.len() {
            let (spans, diagnostic) = csv_spans(input, chunk_size)?;
            assert_eq!(spans.len(), 2);
            assert_eq!(diagnostic, Some(CsvDiagnosticKind::UnterminatedQuotedField));
        }
        Ok(())
    }

    #[test]
    fn line_boundaries_are_identical_for_every_chunk_size() -> Result<(), ScanError> {
        let input = b"one\r\ntwo\n\rthree\rfour";
        let baseline = line_spans(input, input.len())?;
        for chunk_size in 1..=input.len() {
            assert_eq!(line_spans(input, chunk_size)?, baseline);
        }
        Ok(())
    }
}
