use crate::{
    AdaptiveRowIndex, CancellationGeneration, CancellationToken, Checkpoint, CheckpointIndexError,
    CsvBoundaryScanner, CsvDiagnostic, DocumentFormat, EmitControl, LineBoundaryScanner,
    PositionedRead, RecordSpan, ScanError, ScanProgress, ScanStatus, SourceChange,
    SourceFingerprint,
};
use core::fmt;
use std::io;
use std::num::NonZeroUsize;

/// Allocation and sampling limits for an incremental scan session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanSessionConfig {
    index_capacity: usize,
    initial_row_stride: u64,
    read_buffer_bytes: usize,
    cancellation_check_bytes: usize,
}

impl ScanSessionConfig {
    /// Creates a configuration. Values are validated when a session starts.
    #[must_use]
    pub const fn new(
        index_capacity: usize,
        initial_row_stride: u64,
        read_buffer_bytes: usize,
        cancellation_check_bytes: usize,
    ) -> Self {
        Self {
            index_capacity,
            initial_row_stride,
            read_buffer_bytes,
            cancellation_check_bytes,
        }
    }

    #[must_use]
    pub const fn index_capacity(self) -> usize {
        self.index_capacity
    }

    #[must_use]
    pub const fn initial_row_stride(self) -> u64 {
        self.initial_row_stride
    }

    #[must_use]
    pub const fn read_buffer_bytes(self) -> usize {
        self.read_buffer_bytes
    }

    #[must_use]
    pub const fn cancellation_check_bytes(self) -> usize {
        self.cancellation_check_bytes
    }
}

impl Default for ScanSessionConfig {
    fn default() -> Self {
        Self::new(4_096, 1_024, 64 * 1_024, 16 * 1_024)
    }
}

/// Why one call to [`ScanSession::step`] returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanStepStatus {
    /// More source bytes remain in the captured snapshot.
    Progress,
    /// The captured snapshot and end-of-input state are fully processed.
    Complete,
    /// The supplied cancellation generation became stale.
    Cancelled,
}

/// Work completed by one bounded session step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanStep {
    processed_bytes: usize,
    discovered_records: u64,
    status: ScanStepStatus,
}

impl ScanStep {
    /// Returns the number of bytes incorporated into scanner state.
    #[must_use]
    pub const fn processed_bytes(self) -> usize {
        self.processed_bytes
    }

    /// Returns the number of records first discovered by this step.
    #[must_use]
    pub const fn discovered_records(self) -> u64 {
        self.discovered_records
    }

    /// Returns why the step yielded control.
    #[must_use]
    pub const fn status(self) -> ScanStepStatus {
        self.status
    }
}

/// A snapshot of current, usable scan progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanSessionProgress {
    scanned_bytes: u64,
    source_bytes: u64,
    record_count: u64,
    complete: bool,
}

impl ScanSessionProgress {
    #[must_use]
    pub const fn scanned_bytes(self) -> u64 {
        self.scanned_bytes
    }

    #[must_use]
    pub const fn source_bytes(self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub const fn remaining_bytes(self) -> u64 {
        self.source_bytes.saturating_sub(self.scanned_bytes)
    }

    #[must_use]
    pub const fn record_count(self) -> u64 {
        self.record_count
    }

    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.complete
    }
}

enum SessionScanner {
    Delimited(CsvBoundaryScanner),
    Lines(LineBoundaryScanner),
}

impl SessionScanner {
    fn new(format: DocumentFormat) -> Self {
        match format.delimited_dialect() {
            Some(dialect) => Self::Delimited(CsvBoundaryScanner::new(dialect)),
            None => Self::Lines(LineBoundaryScanner::new()),
        }
    }

    fn offset(&self) -> u64 {
        match self {
            Self::Delimited(scanner) => scanner.offset(),
            Self::Lines(scanner) => scanner.offset(),
        }
    }

    fn feed<F>(
        &mut self,
        bytes: &[u8],
        token: &CancellationToken,
        generation: CancellationGeneration,
        check_every: NonZeroUsize,
        emit: F,
    ) -> Result<ScanProgress, ScanError>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        match self {
            Self::Delimited(scanner) => {
                scanner.feed_cancellable(bytes, token, generation, check_every, emit)
            }
            Self::Lines(scanner) => {
                scanner.feed_cancellable(bytes, token, generation, check_every, emit)
            }
        }
    }

    fn finish<F>(self, emit: F) -> Option<CsvDiagnostic>
    where
        F: FnMut(RecordSpan) -> EmitControl,
    {
        match self {
            Self::Delimited(scanner) => scanner.finish(emit).diagnostic(),
            Self::Lines(scanner) => {
                scanner.finish(emit);
                None
            }
        }
    }
}

/// A reusable, bounded-memory scan of one captured source snapshot.
///
/// Completed record boundaries become available in [`Self::index`] after
/// every step; callers do not need to wait for the whole source to finish.
pub struct ScanSession {
    format: DocumentFormat,
    snapshot: SourceFingerprint,
    scanner: Option<SessionScanner>,
    index: AdaptiveRowIndex,
    record_count: u64,
    diagnostic: Option<CsvDiagnostic>,
    read_buffer: Vec<u8>,
    cancellation_check_bytes: NonZeroUsize,
    source_failure: Option<ScanSourceFailure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanSourceFailure {
    Changed {
        change: SourceChange,
        actual_size: u64,
    },
    EvidenceUnavailable,
}

impl ScanSession {
    /// Starts a session without scanning source content.
    ///
    /// # Errors
    /// Returns [`ScanSessionError`] for I/O or invalid configuration.
    pub fn new<R: PositionedRead + ?Sized>(
        source: &R,
        format: DocumentFormat,
        config: ScanSessionConfig,
    ) -> Result<Self, ScanSessionError> {
        let read_buffer_bytes =
            NonZeroUsize::new(config.read_buffer_bytes).ok_or(ScanSessionError::ZeroReadBuffer)?;
        let cancellation_check_bytes = NonZeroUsize::new(config.cancellation_check_bytes)
            .ok_or(ScanSessionError::ZeroCancellationInterval)?;
        let index = AdaptiveRowIndex::new(config.index_capacity, config.initial_row_stride)?;
        let snapshot = SourceFingerprint::capture(source)?;
        Ok(Self {
            format,
            snapshot,
            scanner: Some(SessionScanner::new(format)),
            index,
            record_count: 0,
            diagnostic: None,
            read_buffer: {
                let mut buffer = Vec::new();
                buffer
                    .try_reserve_exact(read_buffer_bytes.get())
                    .map_err(|_| ScanSessionError::ReadBufferUnavailable)?;
                buffer.resize(read_buffer_bytes.get(), 0);
                buffer
            },
            cancellation_check_bytes,
            source_failure: None,
        })
    }

    /// Processes no more than `byte_quantum` source bytes.
    ///
    /// A cancelled step is resumable with a fresh cancellation generation. The
    /// read buffer is retained and reused across calls.
    ///
    /// # Errors
    /// Returns [`ScanSessionError`] for I/O, source changes, scan failures, or
    /// invalid arguments.
    pub fn step<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
        byte_quantum: usize,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<ScanStep, ScanSessionError> {
        if byte_quantum == 0 {
            return Err(ScanSessionError::ZeroByteQuantum);
        }
        if token.is_cancelled(generation) {
            return Ok(ScanStep {
                processed_bytes: 0,
                discovered_records: 0,
                status: ScanStepStatus::Cancelled,
            });
        }
        self.ensure_source_unchanged(source)?;
        if self.scanner.is_none() {
            return Ok(ScanStep {
                processed_bytes: 0,
                discovered_records: 0,
                status: ScanStepStatus::Complete,
            });
        }

        let before_records = self.record_count;
        let offset = self.scanner_offset();
        if offset == self.snapshot.size() {
            self.finalize()?;
            return Ok(ScanStep {
                processed_bytes: 0,
                discovered_records: self.record_count - before_records,
                status: ScanStepStatus::Complete,
            });
        }

        let remaining = self.snapshot.size() - offset;
        let request = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(byte_quantum)
            .min(self.read_buffer.len());
        let count = source.read_at(offset, &mut self.read_buffer[..request])?;
        let count = crate::source::validate_read_count(count, request)?;
        if count == 0 {
            return Err(ScanSessionError::SourceShrank {
                expected_size: self.snapshot.size(),
                scanned_to: offset,
            });
        }
        // Validate again before scanner/index state consumes the buffer. If a
        // writer changed the source during read_at, this step is discarded and
        // the session is terminally poisoned rather than mixing generations.
        self.ensure_source_unchanged(source)?;

        let (scanner, index, record_count) = (
            self.scanner
                .as_mut()
                .ok_or(ScanSessionError::InvalidState)?,
            &mut self.index,
            &mut self.record_count,
        );
        let mut observer_error = None;
        let progress = scanner.feed(
            &self.read_buffer[..count],
            token,
            generation,
            self.cancellation_check_bytes,
            |span| observe_record(index, record_count, span, &mut observer_error),
        )?;
        if let Some(error) = observer_error {
            return Err(error);
        }

        let status = match progress.status() {
            ScanStatus::Cancelled => ScanStepStatus::Cancelled,
            ScanStatus::Stopped => return Err(ScanSessionError::InvalidState),
            ScanStatus::Complete if self.scanner_offset() == self.snapshot.size() => {
                self.ensure_source_unchanged(source)?;
                self.finalize()?;
                ScanStepStatus::Complete
            }
            ScanStatus::Complete => ScanStepStatus::Progress,
        };
        Ok(ScanStep {
            processed_bytes: progress.consumed(),
            discovered_records: self.record_count - before_records,
            status,
        })
    }

    /// Returns the source format used by this session.
    #[must_use]
    pub const fn format(&self) -> DocumentFormat {
        self.format
    }

    /// Returns the source size captured when the session started.
    #[must_use]
    pub const fn source_fingerprint(&self) -> SourceFingerprint {
        self.snapshot
    }

    /// Returns current scan progress without requiring end-of-file.
    #[must_use]
    pub fn progress(&self) -> ScanSessionProgress {
        ScanSessionProgress {
            scanned_bytes: self.scanner_offset(),
            source_bytes: self.snapshot.size(),
            record_count: self.record_count,
            complete: self.scanner.is_none(),
        }
    }

    /// Returns the number of complete logical records discovered so far.
    #[must_use]
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Returns the EOF diagnostic once a delimited scan has finalized.
    #[must_use]
    pub const fn diagnostic(&self) -> Option<CsvDiagnostic> {
        self.diagnostic
    }

    /// Returns the usable, adaptively bounded row index built so far.
    #[must_use]
    pub const fn index(&self) -> &AdaptiveRowIndex {
        &self.index
    }

    /// Reports whether EOF state has been finalized.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.scanner.is_none()
    }

    fn scanner_offset(&self) -> u64 {
        self.scanner
            .as_ref()
            .map_or(self.snapshot.size(), SessionScanner::offset)
    }

    fn ensure_source_unchanged<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
    ) -> Result<(), ScanSessionError> {
        if let Some(failure) = self.source_failure {
            return Err(self.source_failure_error(failure));
        }
        let current = match SourceFingerprint::capture(source) {
            Ok(current) => current,
            Err(error) => {
                self.source_failure = Some(ScanSourceFailure::EvidenceUnavailable);
                return Err(ScanSessionError::Io(error));
            }
        };
        let change = self.snapshot.compare(current);
        if !change.is_changed() {
            return Ok(());
        }
        let failure = ScanSourceFailure::Changed {
            change,
            actual_size: current.size(),
        };
        self.source_failure = Some(failure);
        Err(self.source_failure_error(failure))
    }

    fn source_failure_error(&self, failure: ScanSourceFailure) -> ScanSessionError {
        match failure {
            ScanSourceFailure::Changed {
                change: SourceChange::Shrank,
                ..
            } => ScanSessionError::SourceShrank {
                expected_size: self.snapshot.size(),
                scanned_to: self.scanner_offset(),
            },
            ScanSourceFailure::Changed {
                change: SourceChange::Grew,
                actual_size,
            } => ScanSessionError::SourceGrew {
                expected_size: self.snapshot.size(),
                actual_size,
            },
            ScanSourceFailure::Changed {
                change,
                actual_size,
            } => ScanSessionError::SourceChanged {
                change,
                expected_size: self.snapshot.size(),
                actual_size,
                scanned_to: self.scanner_offset(),
            },
            ScanSourceFailure::EvidenceUnavailable => ScanSessionError::SourceEvidenceUnavailable {
                scanned_to: self.scanner_offset(),
            },
        }
    }

    fn finalize(&mut self) -> Result<(), ScanSessionError> {
        let scanner = self.scanner.take().ok_or(ScanSessionError::InvalidState)?;
        let mut observer_error = None;
        let diagnostic = scanner.finish(|span| {
            observe_record(
                &mut self.index,
                &mut self.record_count,
                span,
                &mut observer_error,
            )
        });
        if let Some(error) = observer_error {
            return Err(error);
        }
        self.diagnostic = diagnostic;
        Ok(())
    }
}

fn observe_record(
    index: &mut AdaptiveRowIndex,
    record_count: &mut u64,
    span: RecordSpan,
    error: &mut Option<ScanSessionError>,
) -> EmitControl {
    let Some(next_count) = record_count.checked_add(1) else {
        *error = Some(ScanSessionError::RecordCountOverflow);
        return EmitControl::Stop;
    };
    if let Err(index_error) = index.observe(Checkpoint::new(*record_count, span.start())) {
        *error = Some(ScanSessionError::Index(index_error));
        return EmitControl::Stop;
    }
    *record_count = next_count;
    EmitControl::Continue
}

/// A failure that prevents an incremental session step from completing.
#[derive(Debug)]
pub enum ScanSessionError {
    Io(io::Error),
    Scan(ScanError),
    Index(CheckpointIndexError),
    ZeroReadBuffer,
    ReadBufferUnavailable,
    ZeroCancellationInterval,
    ZeroByteQuantum,
    SourceShrank {
        expected_size: u64,
        scanned_to: u64,
    },
    SourceGrew {
        expected_size: u64,
        actual_size: u64,
    },
    SourceChanged {
        change: SourceChange,
        expected_size: u64,
        actual_size: u64,
        scanned_to: u64,
    },
    SourceEvidenceUnavailable {
        scanned_to: u64,
    },
    RecordCountOverflow,
    InvalidState,
}

impl From<io::Error> for ScanSessionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ScanError> for ScanSessionError {
    fn from(error: ScanError) -> Self {
        Self::Scan(error)
    }
}

impl From<CheckpointIndexError> for ScanSessionError {
    fn from(error: CheckpointIndexError) -> Self {
        Self::Index(error)
    }
}

impl ScanSessionError {
    /// Reports whether this error permanently invalidates source snapshot
    /// continuity for the session.
    #[must_use]
    pub const fn is_source_integrity_failure(&self) -> bool {
        matches!(
            self,
            Self::SourceShrank { .. }
                | Self::SourceGrew { .. }
                | Self::SourceChanged { .. }
                | Self::SourceEvidenceUnavailable { .. }
        )
    }
}

impl fmt::Display for ScanSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "scan session I/O failed: {error}"),
            Self::Scan(error) => write!(formatter, "scan session boundary scan failed: {error}"),
            Self::Index(error) => write!(formatter, "scan session index failed: {error}"),
            Self::ZeroReadBuffer => formatter.write_str("scan read buffer must be non-zero"),
            Self::ReadBufferUnavailable => {
                formatter.write_str("scan read buffer capacity is unavailable")
            }
            Self::ZeroCancellationInterval => {
                formatter.write_str("cancellation interval must be non-zero")
            }
            Self::ZeroByteQuantum => formatter.write_str("scan byte quantum must be non-zero"),
            Self::SourceShrank {
                expected_size,
                scanned_to,
            } => write!(
                formatter,
                "source shrank below snapshot size {expected_size} after offset {scanned_to}"
            ),
            Self::SourceGrew {
                expected_size,
                actual_size,
            } => write!(
                formatter,
                "source grew from snapshot size {expected_size} to {actual_size}"
            ),
            Self::SourceChanged {
                change,
                expected_size,
                actual_size,
                scanned_to,
            } => write!(
                formatter,
                "source snapshot changed ({change:?}) from size {expected_size} to {actual_size} after offset {scanned_to}"
            ),
            Self::SourceEvidenceUnavailable { scanned_to } => write!(
                formatter,
                "source snapshot evidence became unavailable after offset {scanned_to}"
            ),
            Self::RecordCountOverflow => formatter.write_str("logical record count overflowed u64"),
            Self::InvalidState => formatter.write_str("scan session entered an invalid state"),
        }
    }
}

impl std::error::Error for ScanSessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Scan(error) => Some(error),
            Self::Index(error) => Some(error),
            Self::ZeroReadBuffer
            | Self::ReadBufferUnavailable
            | Self::ZeroCancellationInterval
            | Self::ZeroByteQuantum
            | Self::SourceShrank { .. }
            | Self::SourceGrew { .. }
            | Self::SourceChanged { .. }
            | Self::SourceEvidenceUnavailable { .. }
            | Self::RecordCountOverflow
            | Self::InvalidState => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScanSession, ScanSessionConfig, ScanSessionError, ScanStepStatus};
    use crate::{
        CancellationToken, CsvDiagnosticKind, DocumentFormat, EngineLimits, ManagedMemoryBudget,
        PositionedRead, SourceChange, SourceFingerprint, SourceIdentity, SourceRevision,
        ViewportFormat, read_viewport,
    };
    use std::cell::{Cell, RefCell};
    use std::io;

    struct MutableSizeSource {
        bytes: Vec<u8>,
        visible_size: Cell<usize>,
    }

    impl MutableSizeSource {
        fn new(bytes: Vec<u8>) -> Self {
            let size = bytes.len();
            Self {
                bytes,
                visible_size: Cell::new(size),
            }
        }

        fn resize_visible(&self, size: usize) {
            self.visible_size.set(size);
        }
    }

    impl PositionedRead for MutableSizeSource {
        fn size(&self) -> io::Result<u64> {
            u64::try_from(self.visible_size.get()).map_err(io::Error::other)
        }

        fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            let visible = self.visible_size.get().min(self.bytes.len());
            if start >= visible {
                return Ok(0);
            }
            let count = destination.len().min(visible - start);
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            Ok(count)
        }
    }

    #[derive(Clone, Copy)]
    enum Mutation {
        Rewrite,
        Replace,
    }

    struct VersionedSource {
        bytes: RefCell<Vec<u8>>,
        identity: Cell<u128>,
        revision: Cell<u128>,
        mutate_after_read: Cell<Option<Mutation>>,
    }

    impl VersionedSource {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes: RefCell::new(bytes),
                identity: Cell::new(1),
                revision: Cell::new(1),
                mutate_after_read: Cell::new(None),
            }
        }

        fn mutate(&self, mutation: Mutation) {
            match mutation {
                Mutation::Rewrite => {
                    for byte in self.bytes.borrow_mut().iter_mut() {
                        *byte = b'Z';
                    }
                }
                Mutation::Replace => {
                    self.identity.set(self.identity.get().saturating_add(1));
                    for byte in self.bytes.borrow_mut().iter_mut() {
                        *byte = b'R';
                    }
                }
            }
            self.revision.set(self.revision.get().saturating_add(1));
        }

        fn mutate_on_next_read(&self, mutation: Mutation) {
            self.mutate_after_read.set(Some(mutation));
        }

        fn restore(&self, bytes: &[u8]) {
            self.bytes.replace(bytes.to_vec());
            self.identity.set(1);
            self.revision.set(1);
            self.mutate_after_read.set(None);
        }
    }

    impl PositionedRead for VersionedSource {
        fn size(&self) -> io::Result<u64> {
            u64::try_from(self.bytes.borrow().len()).map_err(io::Error::other)
        }

        fn fingerprint(&self) -> io::Result<SourceFingerprint> {
            Ok(SourceFingerprint::with_evidence(
                self.size()?,
                Some(SourceIdentity::new(31, self.identity.get())),
                Some(SourceRevision::new(self.revision.get(), 0)),
            ))
        }

        fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            let bytes = self.bytes.borrow();
            if start >= bytes.len() {
                return Ok(0);
            }
            let count = destination.len().min(bytes.len() - start);
            destination[..count].copy_from_slice(&bytes[start..start + count]);
            drop(bytes);
            if let Some(mutation) = self.mutate_after_read.take() {
                self.mutate(mutation);
            }
            Ok(count)
        }
    }

    fn config(read_buffer_bytes: usize) -> ScanSessionConfig {
        ScanSessionConfig::new(32, 1, read_buffer_bytes, 3)
    }

    fn run_to_completion(
        source: &MutableSizeSource,
        format: DocumentFormat,
        quantum: usize,
    ) -> Result<ScanSession, Box<dyn std::error::Error>> {
        let token = CancellationToken::new();
        let generation = token.snapshot();
        let mut session = ScanSession::new(source, format, config(7))?;
        for _ in 0..10_000 {
            let step = session.step(source, quantum, &token, generation)?;
            assert!(step.processed_bytes() <= quantum);
            if step.status() == ScanStepStatus::Complete {
                return Ok(session);
            }
        }
        Err("scan did not complete within the test guard".into())
    }

    #[test]
    fn one_byte_steps_preserve_multiline_and_escaped_quote_boundaries()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = b"a,b\r\n\"multi\nline\",\"escaped \"\"quote\"\"\"\nlast,row".to_vec();
        for quantum in 1..=input.len() {
            let source = MutableSizeSource::new(input.clone());
            let session = run_to_completion(&source, DocumentFormat::Csv, quantum)?;
            assert_eq!(session.record_count(), 3);
            assert_eq!(
                session.progress().scanned_bytes(),
                session.progress().source_bytes()
            );
            assert!(session.progress().is_complete());
            assert_eq!(session.diagnostic(), None);
            assert!(session.index().len() <= session.index().capacity_limit());
        }
        Ok(())
    }

    #[test]
    fn partial_index_supports_a_viewport_before_the_full_scan_finishes()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = b"head,one\n\"multi\nline\",\"x\"\"y\"\nthird,row\nfourth,row\n".to_vec();
        let source = MutableSizeSource::new(input);
        let token = CancellationToken::new();
        let generation = token.snapshot();
        let mut session = ScanSession::new(&source, DocumentFormat::Csv, config(5))?;
        while session.record_count() < 2 {
            let step = session.step(&source, 5, &token, generation)?;
            assert_ne!(step.status(), ScanStepStatus::Complete);
        }
        assert!(!session.is_complete());
        assert!(session.progress().remaining_bytes() > 0);

        let budget = ManagedMemoryBudget::new(4_096, 64, 512, 512, 512)?;
        let limits = EngineLimits::new(64, 1, 128, 128, 7, budget)?;
        let viewport = read_viewport(
            &source,
            ViewportFormat::from(DocumentFormat::Csv),
            session.index(),
            1,
            limits,
            &token,
            generation,
        )?;
        assert_eq!(viewport.records().len(), 1);
        assert_eq!(viewport.records()[0].row(), 1);
        assert_eq!(viewport.records()[0].bytes(), b"\"multi\nline\",\"x\"\"y\"");
        Ok(())
    }

    #[test]
    fn stale_generation_yields_without_consuming_and_a_fresh_one_resumes()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = MutableSizeSource::new(b"one\ntwo\n".to_vec());
        let token = CancellationToken::new();
        let stale = token.snapshot();
        token.cancel();
        let mut session = ScanSession::new(&source, DocumentFormat::Log, config(8))?;
        let cancelled = session.step(&source, 8, &token, stale)?;
        assert_eq!(cancelled.status(), ScanStepStatus::Cancelled);
        assert_eq!(cancelled.processed_bytes(), 0);
        assert_eq!(session.progress().scanned_bytes(), 0);

        let resumed = session.step(&source, 8, &token, token.snapshot())?;
        assert_eq!(resumed.status(), ScanStepStatus::Complete);
        assert!(resumed.processed_bytes() > 0);
        assert!(session.is_complete());
        Ok(())
    }

    #[test]
    fn malformed_csv_is_diagnosed_only_when_eof_is_finalized()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = MutableSizeSource::new(b"ok,row\n\"never\nclosed".to_vec());
        let session = run_to_completion(&source, DocumentFormat::Csv, 1)?;
        let diagnostic = session.diagnostic().ok_or("missing EOF diagnostic")?;
        assert_eq!(
            diagnostic.kind(),
            CsvDiagnosticKind::UnterminatedQuotedField
        );
        assert_eq!(session.record_count(), 2);
        Ok(())
    }

    #[test]
    fn size_changes_fail_closed_before_more_bytes_are_processed()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = MutableSizeSource::new(b"one\ntwo\nthree\n".to_vec());
        let token = CancellationToken::new();
        let generation = token.snapshot();
        let mut shrunk = ScanSession::new(&source, DocumentFormat::Log, config(4))?;
        shrunk.step(&source, 4, &token, generation)?;
        source.resize_visible(5);
        assert!(matches!(
            shrunk.step(&source, 4, &token, generation),
            Err(ScanSessionError::SourceShrank { .. })
        ));

        source.resize_visible(8);
        let mut grown = ScanSession::new(&source, DocumentFormat::Log, config(4))?;
        source.resize_visible(9);
        assert!(matches!(
            grown.step(&source, 4, &token, generation),
            Err(ScanSessionError::SourceGrew { .. })
        ));
        Ok(())
    }

    #[test]
    fn same_size_rewrite_and_replacement_poison_the_session()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = b"one\ntwo\nthree\n".to_vec();
        let cases = [
            (Mutation::Rewrite, SourceChange::RevisionChanged),
            (Mutation::Replace, SourceChange::IdentityChanged),
        ];
        for (mutation, expected) in cases {
            let source = VersionedSource::new(original.clone());
            let token = CancellationToken::new();
            let generation = token.snapshot();
            let mut session = ScanSession::new(&source, DocumentFormat::Log, config(4))?;
            session.step(&source, 4, &token, generation)?;
            let before = session.progress();
            source.mutate(mutation);
            assert!(matches!(
                session.step(&source, 4, &token, generation),
                Err(ScanSessionError::SourceChanged { change, .. }) if change == expected
            ));
            assert_eq!(session.progress(), before);

            source.restore(&original);
            assert!(matches!(
                session.step(&source, 4, &token, generation),
                Err(ScanSessionError::SourceChanged { change, .. }) if change == expected
            ));
            assert_eq!(session.progress(), before);
        }
        Ok(())
    }

    #[test]
    fn mid_read_rewrite_is_rejected_before_scanner_and_index_state_advance()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = b"one\ntwo\nthree\n".to_vec();
        let source = VersionedSource::new(original.clone());
        source.mutate_on_next_read(Mutation::Rewrite);
        let token = CancellationToken::new();
        let generation = token.snapshot();
        let mut session = ScanSession::new(&source, DocumentFormat::Log, config(8))?;
        assert!(matches!(
            session.step(&source, 8, &token, generation),
            Err(ScanSessionError::SourceChanged {
                change: SourceChange::RevisionChanged,
                ..
            })
        ));
        assert_eq!(session.progress().scanned_bytes(), 0);
        assert_eq!(session.record_count(), 0);
        assert_eq!(session.index().len(), 1);

        source.restore(&original);
        assert!(matches!(
            session.step(&source, 8, &token, generation),
            Err(ScanSessionError::SourceChanged {
                change: SourceChange::RevisionChanged,
                ..
            })
        ));
        assert_eq!(session.progress().scanned_bytes(), 0);
        Ok(())
    }

    #[test]
    fn stale_scan_generation_yields_before_changed_source_evidence_is_observed()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = VersionedSource::new(b"one\ntwo\n".to_vec());
        let token = CancellationToken::new();
        let stale = token.snapshot();
        token.cancel();
        let mut session = ScanSession::new(&source, DocumentFormat::Log, config(4))?;
        source.mutate(Mutation::Rewrite);
        let cancelled = session.step(&source, 4, &token, stale)?;
        assert_eq!(cancelled.status(), ScanStepStatus::Cancelled);
        assert_eq!(session.progress().scanned_bytes(), 0);
        assert!(matches!(
            session.step(&source, 4, &token, token.snapshot()),
            Err(ScanSessionError::SourceChanged {
                change: SourceChange::RevisionChanged,
                ..
            })
        ));
        assert_eq!(session.progress().scanned_bytes(), 0);
        Ok(())
    }
}
