//! Bounded literal search for the v0.1 query surface.
//!
//! This module intentionally implements only raw-byte `contains` matching.
//! Logical records are discovered by the same CSV and line scanners used by
//! viewport reconstruction. Matches are therefore allowed across physical
//! newlines inside quoted CSV fields, but never across logical-record edges.

use crate::{
    CancellationGeneration, CancellationToken, CsvBoundaryScanner, CsvDiagnostic, EmitControl,
    LineBoundaryScanner, PositionedRead, RecordSpan, ScanError, ScanProgress, ScanStatus,
    SourceChange, SourceFingerprint, ViewportFormat,
};
use core::fmt;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Bytes occupied by the versioned query-scratch header.
pub const QUERY_SCRATCH_HEADER_BYTES: u64 = 48;
/// Bytes occupied by one exact-length hit entry in query scratch storage.
pub const QUERY_HIT_ENTRY_BYTES: u64 = 24;

const SCRATCH_MAGIC: &[u8; 8] = b"LRQHIT01";
const SCRATCH_VERSION: u16 = 1;
const SCRATCH_HEADER_LEN: usize = 48;
const HIT_ENTRY_LEN: usize = 24;
const SCRATCH_NAME_PREFIX: &str = "leanrows-query-v1-";
const SCRATCH_NAME_SUFFIX: &str = ".hits";
const CREATE_ATTEMPTS: usize = 64;
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Comparison behavior for a literal byte query.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LiteralMatchMode {
    /// Compare every byte exactly.
    Exact,
    /// Fold only ASCII letters before comparison; non-ASCII bytes stay raw.
    AsciiCaseInsensitive,
}

/// Fixed resource limits and record rules for one query session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuerySessionConfig {
    format: ViewportFormat,
    read_buffer_bytes: NonZeroUsize,
    cancellation_check_bytes: NonZeroUsize,
    page_capacity: NonZeroUsize,
    scratch_quota_bytes: u64,
}

impl QuerySessionConfig {
    /// Creates a bounded query plan.
    ///
    /// The quota includes the versioned scratch header as well as every hit.
    ///
    /// # Errors
    /// Returns [`QueryError`] when a count is zero or the quota cannot hold the
    /// scratch header.
    pub fn new(
        format: ViewportFormat,
        read_buffer_bytes: usize,
        cancellation_check_bytes: usize,
        page_capacity: usize,
        scratch_quota_bytes: u64,
    ) -> Result<Self, QueryError> {
        let Some(read_buffer_bytes) = NonZeroUsize::new(read_buffer_bytes) else {
            return Err(QueryError::ZeroReadBuffer);
        };
        let Some(cancellation_check_bytes) = NonZeroUsize::new(cancellation_check_bytes) else {
            return Err(QueryError::ZeroCancellationInterval);
        };
        let Some(page_capacity) = NonZeroUsize::new(page_capacity) else {
            return Err(QueryError::ZeroPageCapacity);
        };
        if scratch_quota_bytes < QUERY_SCRATCH_HEADER_BYTES {
            return Err(QueryError::ScratchQuotaTooSmall {
                quota_bytes: scratch_quota_bytes,
                minimum_bytes: QUERY_SCRATCH_HEADER_BYTES,
            });
        }
        u64::try_from(page_capacity.get()).map_err(|_| QueryError::AddressSpace)?;
        Ok(Self {
            format,
            read_buffer_bytes,
            cancellation_check_bytes,
            page_capacity,
            scratch_quota_bytes,
        })
    }

    #[must_use]
    pub const fn format(self) -> ViewportFormat {
        self.format
    }

    #[must_use]
    pub const fn read_buffer_bytes(self) -> NonZeroUsize {
        self.read_buffer_bytes
    }

    #[must_use]
    pub const fn cancellation_check_bytes(self) -> NonZeroUsize {
        self.cancellation_check_bytes
    }

    #[must_use]
    pub const fn page_capacity(self) -> NonZeroUsize {
        self.page_capacity
    }

    #[must_use]
    pub const fn scratch_quota_bytes(self) -> u64 {
        self.scratch_quota_bytes
    }
}

/// One raw literal match, addressed against the original source snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct QueryHit {
    row: u64,
    span: RecordSpan,
}

impl QueryHit {
    #[must_use]
    pub const fn row(self) -> u64 {
        self.row
    }

    /// Returns the absolute, end-exclusive span of the matched bytes.
    #[must_use]
    pub const fn span(self) -> RecordSpan {
        self.span
    }
}

/// Details recorded when the next exact hit entry would exceed the disk quota.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryQuotaExceeded {
    quota_bytes: u64,
    used_bytes: u64,
    required_bytes: u64,
    stored_hits: u64,
    rejected_hit: QueryHit,
}

impl QueryQuotaExceeded {
    #[must_use]
    pub const fn quota_bytes(self) -> u64 {
        self.quota_bytes
    }

    #[must_use]
    pub const fn used_bytes(self) -> u64 {
        self.used_bytes
    }

    #[must_use]
    pub const fn required_bytes(self) -> u64 {
        self.required_bytes
    }

    #[must_use]
    pub const fn stored_hits(self) -> u64 {
        self.stored_hits
    }

    #[must_use]
    pub const fn rejected_hit(self) -> QueryHit {
        self.rejected_hit
    }
}

/// Observable state after a cooperative query step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryStepStatus {
    /// More source or record bytes remain to be processed.
    Running,
    /// The supplied cancellation generation became stale; a fresh generation
    /// may resume the same session.
    Cancelled,
    /// The source snapshot was searched completely.
    Complete,
    /// The next hit could not fit; this terminal result retains the source and
    /// every hit stored before the exact quota boundary.
    QuotaExceeded(QueryQuotaExceeded),
}

/// Monotonic progress for a query session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryProgress {
    source_bytes: u64,
    scanned_bytes: u64,
    discovered_records: u64,
    examined_records: u64,
    stored_hits: u64,
    scratch_bytes: u64,
    active_row: Option<u64>,
    csv_diagnostic: Option<CsvDiagnostic>,
}

impl QueryProgress {
    #[must_use]
    pub const fn source_bytes(self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub const fn scanned_bytes(self) -> u64 {
        self.scanned_bytes
    }

    #[must_use]
    pub const fn discovered_records(self) -> u64 {
        self.discovered_records
    }

    #[must_use]
    pub const fn examined_records(self) -> u64 {
        self.examined_records
    }

    #[must_use]
    pub const fn stored_hits(self) -> u64 {
        self.stored_hits
    }

    #[must_use]
    pub const fn scratch_bytes(self) -> u64 {
        self.scratch_bytes
    }

    #[must_use]
    pub const fn active_row(self) -> Option<u64> {
        self.active_row
    }

    #[must_use]
    pub const fn csv_diagnostic(self) -> Option<CsvDiagnostic> {
        self.csv_diagnostic
    }
}

/// A borrowed view of the session's one fixed-capacity in-memory page.
#[derive(Clone, Copy, Debug)]
pub struct QueryHitPage<'a> {
    first_hit: u64,
    hits: &'a [QueryHit],
}

impl<'a> QueryHitPage<'a> {
    #[must_use]
    pub const fn first_hit(self) -> u64 {
        self.first_hit
    }

    #[must_use]
    pub const fn hits(&self) -> &'a [QueryHit] {
        self.hits
    }
}

/// Outcome of a startup-only stale-query recovery pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueryRecoveryReport {
    removed_files: u64,
    retained_candidates: u64,
}

impl QueryRecoveryReport {
    #[must_use]
    pub const fn removed_files(self) -> u64 {
        self.removed_files
    }

    #[must_use]
    pub const fn retained_candidates(self) -> u64 {
        self.retained_candidates
    }
}

/// Query construction, I/O, or integrity failure.
#[derive(Debug)]
pub enum QueryError {
    Io(io::Error),
    Scan(ScanError),
    EmptyNeedle,
    ZeroReadBuffer,
    ZeroCancellationInterval,
    ZeroPageCapacity,
    ZeroWorkBudget,
    ScratchQuotaTooSmall {
        quota_bytes: u64,
        minimum_bytes: u64,
    },
    UnsafeScratchDirectory,
    ScratchNameExhausted,
    ScratchCorrupt,
    AllocationUnavailable,
    SourceShrank,
    SourceGrew {
        expected_size: u64,
        actual_size: u64,
    },
    SourceChanged {
        change: SourceChange,
        expected_size: u64,
        actual_size: u64,
    },
    SourceEvidenceUnavailable,
    RowOverflow,
    AddressSpace,
}

impl From<io::Error> for QueryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ScanError> for QueryError {
    fn from(error: ScanError) -> Self {
        Self::Scan(error)
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNeedle => formatter.write_str("literal query cannot be empty"),
            Self::ZeroReadBuffer => formatter.write_str("query read buffer cannot be zero"),
            Self::ZeroCancellationInterval => {
                formatter.write_str("query cancellation interval cannot be zero")
            }
            Self::ZeroPageCapacity => formatter.write_str("query page capacity cannot be zero"),
            Self::ZeroWorkBudget => formatter.write_str("query work budget cannot be zero"),
            Self::ScratchQuotaTooSmall { .. } => {
                formatter.write_str("query scratch quota cannot hold its header")
            }
            Self::UnsafeScratchDirectory => {
                formatter.write_str("query scratch directory is not a safe regular directory")
            }
            Self::ScratchNameExhausted => {
                formatter.write_str("could not reserve a unique query scratch artifact")
            }
            Self::ScratchCorrupt => formatter.write_str("query scratch artifact is corrupt"),
            Self::AllocationUnavailable => {
                formatter.write_str("query bounded allocation is unavailable")
            }
            Self::SourceShrank => formatter.write_str("query source shrank during the snapshot"),
            Self::SourceGrew {
                expected_size,
                actual_size,
            } => write!(
                formatter,
                "query source grew from snapshot size {expected_size} to {actual_size}"
            ),
            Self::SourceChanged {
                change,
                expected_size,
                actual_size,
            } => write!(
                formatter,
                "query source changed ({change:?}) from size {expected_size} to {actual_size}"
            ),
            Self::SourceEvidenceUnavailable => {
                formatter.write_str("query source evidence became unavailable")
            }
            Self::RowOverflow => formatter.write_str("query row address overflowed u64"),
            Self::AddressSpace => formatter.write_str("query byte address is not representable"),
            Self::Io(_) => formatter.write_str("query I/O failed"),
            Self::Scan(_) => formatter.write_str("query record scan failed"),
        }
    }
}

impl std::error::Error for QueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Scan(error) => Some(error),
            _ => None,
        }
    }
}

impl QueryError {
    /// Reports whether this error terminally invalidates query snapshot
    /// continuity.
    #[must_use]
    pub const fn is_source_integrity_failure(&self) -> bool {
        matches!(
            self,
            Self::SourceShrank
                | Self::SourceGrew { .. }
                | Self::SourceChanged { .. }
                | Self::SourceEvidenceUnavailable
        )
    }
}

#[derive(Clone, Debug)]
struct CompiledLiteral {
    bytes: Vec<u8>,
    failure: Vec<usize>,
    mode: LiteralMatchMode,
    length_u64: u64,
}

impl CompiledLiteral {
    fn new(needle: &[u8], mode: LiteralMatchMode) -> Result<Self, QueryError> {
        if needle.is_empty() {
            return Err(QueryError::EmptyNeedle);
        }
        let length_u64 = u64::try_from(needle.len()).map_err(|_| QueryError::AddressSpace)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(needle.len())
            .map_err(|_| QueryError::AllocationUnavailable)?;
        bytes.extend_from_slice(needle);
        if mode == LiteralMatchMode::AsciiCaseInsensitive {
            for byte in &mut bytes {
                *byte = byte.to_ascii_lowercase();
            }
        }

        let mut failure = Vec::new();
        failure
            .try_reserve_exact(bytes.len())
            .map_err(|_| QueryError::AllocationUnavailable)?;
        failure.resize(bytes.len(), 0);
        for index in 1..bytes.len() {
            let mut candidate = failure[index - 1];
            while candidate > 0 && bytes[index] != bytes[candidate] {
                candidate = failure[candidate - 1];
            }
            if bytes[index] == bytes[candidate] {
                candidate += 1;
            }
            failure[index] = candidate;
        }
        Ok(Self {
            bytes,
            failure,
            mode,
            length_u64,
        })
    }

    fn advance(&self, mut state: usize, byte: u8) -> (usize, bool) {
        let candidate = if self.mode == LiteralMatchMode::AsciiCaseInsensitive {
            byte.to_ascii_lowercase()
        } else {
            byte
        };
        while state > 0 && self.bytes[state] != candidate {
            state = self.failure[state - 1];
        }
        if self.bytes[state] == candidate {
            state += 1;
        }
        if state == self.bytes.len() {
            (self.failure[state - 1], true)
        } else {
            (state, false)
        }
    }
}

#[derive(Debug)]
enum QueryBoundaryScanner {
    Csv(CsvBoundaryScanner),
    Lines(LineBoundaryScanner),
}

impl QueryBoundaryScanner {
    const fn new(format: ViewportFormat) -> Self {
        match format {
            ViewportFormat::Delimited(dialect) => Self::Csv(CsvBoundaryScanner::new(dialect)),
            ViewportFormat::Lines => Self::Lines(LineBoundaryScanner::new()),
        }
    }

    const fn offset(&self) -> u64 {
        match self {
            Self::Csv(scanner) => scanner.offset(),
            Self::Lines(scanner) => scanner.offset(),
        }
    }

    fn feed_one(
        &mut self,
        bytes: &[u8],
        token: &CancellationToken,
        generation: CancellationGeneration,
        check_every: NonZeroUsize,
        emitted: &mut Option<RecordSpan>,
    ) -> Result<ScanProgress, ScanError> {
        let mut accept = |span| {
            *emitted = Some(span);
            EmitControl::Stop
        };
        match self {
            Self::Csv(scanner) => {
                scanner.feed_cancellable(bytes, token, generation, check_every, &mut accept)
            }
            Self::Lines(scanner) => {
                scanner.feed_cancellable(bytes, token, generation, check_every, &mut accept)
            }
        }
    }

    fn finish(self) -> (Option<RecordSpan>, Option<CsvDiagnostic>) {
        let mut emitted = None;
        let diagnostic = match self {
            Self::Csv(scanner) => scanner
                .finish(|span| {
                    emitted = Some(span);
                    EmitControl::Stop
                })
                .diagnostic(),
            Self::Lines(scanner) => {
                scanner.finish(|span| {
                    emitted = Some(span);
                    EmitControl::Stop
                });
                None
            }
        };
        (emitted, diagnostic)
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingRecord {
    row: u64,
    span: RecordSpan,
    cursor: u64,
    matcher_state: usize,
}

#[derive(Debug)]
struct HitPageBuffer {
    capacity: usize,
    first_hit: u64,
    hits: Vec<QueryHit>,
}

impl HitPageBuffer {
    fn new(capacity: NonZeroUsize) -> Result<Self, QueryError> {
        let mut hits = Vec::new();
        hits.try_reserve_exact(capacity.get())
            .map_err(|_| QueryError::AllocationUnavailable)?;
        Ok(Self {
            capacity: capacity.get(),
            first_hit: 0,
            hits,
        })
    }

    fn append(&mut self, index: u64, hit: QueryHit) -> Result<(), QueryError> {
        let length = u64::try_from(self.hits.len()).map_err(|_| QueryError::AddressSpace)?;
        let follows_page = self.first_hit.checked_add(length) == Some(index);
        if self.hits.is_empty() || !follows_page || self.hits.len() == self.capacity {
            self.hits.clear();
            self.first_hit = index;
        }
        self.hits.push(hit);
        Ok(())
    }

    fn view(&self) -> QueryHitPage<'_> {
        QueryHitPage {
            first_hit: self.first_hit,
            hits: &self.hits,
        }
    }
}

#[derive(Debug)]
struct ScratchHitStore {
    file: Option<File>,
    path: PathBuf,
    directory: PathBuf,
    owner: [u8; 16],
    quota_bytes: u64,
    used_bytes: u64,
    entries: u64,
}

enum ScratchAppend {
    Stored(u64),
    Quota { required_bytes: u64 },
}

impl ScratchHitStore {
    fn create(directory: &Path, quota_bytes: u64) -> Result<Self, QueryError> {
        let directory = canonical_scratch_directory(directory)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        for _ in 0..CREATE_ATTEMPTS {
            let owner = fresh_owner();
            let name = scratch_file_name(owner)?;
            let path = directory.join(name);
            match options.open(&path) {
                Ok(file) => {
                    let mut store = Self {
                        file: Some(file),
                        path,
                        directory,
                        owner,
                        quota_bytes,
                        used_bytes: QUERY_SCRATCH_HEADER_BYTES,
                        entries: 0,
                    };
                    store.write_header()?;
                    return Ok(store);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(QueryError::Io(error)),
            }
        }
        Err(QueryError::ScratchNameExhausted)
    }

    fn write_header(&mut self) -> Result<(), QueryError> {
        let header = scratch_header(self.owner);
        let file = self.file.as_mut().ok_or(QueryError::ScratchCorrupt)?;
        file.write_all(&header)?;
        Ok(())
    }

    fn append(&mut self, hit: QueryHit) -> Result<ScratchAppend, QueryError> {
        let required_bytes = self
            .used_bytes
            .checked_add(QUERY_HIT_ENTRY_BYTES)
            .ok_or(QueryError::AddressSpace)?;
        if required_bytes > self.quota_bytes {
            return Ok(ScratchAppend::Quota { required_bytes });
        }
        let index = self.entries;
        let bytes = encode_hit(hit);
        let file = self.file.as_mut().ok_or(QueryError::ScratchCorrupt)?;
        file.seek(SeekFrom::Start(self.used_bytes))?;
        file.write_all(&bytes)?;
        self.used_bytes = required_bytes;
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or(QueryError::AddressSpace)?;
        Ok(ScratchAppend::Stored(index))
    }

    fn read_hit(&mut self, index: u64) -> Result<Option<QueryHit>, QueryError> {
        if index >= self.entries {
            return Ok(None);
        }
        let offset = QUERY_HIT_ENTRY_BYTES
            .checked_mul(index)
            .and_then(|relative| QUERY_SCRATCH_HEADER_BYTES.checked_add(relative))
            .ok_or(QueryError::AddressSpace)?;
        let file = self.file.as_mut().ok_or(QueryError::ScratchCorrupt)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = [0_u8; HIT_ENTRY_LEN];
        file.read_exact(&mut bytes)?;
        Ok(Some(decode_hit(bytes)?))
    }
}

impl Drop for ScratchHitStore {
    fn drop(&mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
            drop(file);
        }
        if self.path.parent() != Some(self.directory.as_path()) {
            return;
        }
        if parse_owner_from_name(self.path.file_name()) != Some(self.owner) {
            return;
        }
        let Ok(directory_metadata) = fs::symlink_metadata(&self.directory) else {
            return;
        };
        if !directory_metadata.is_dir() || metadata_is_reparse(&directory_metadata) {
            return;
        }
        let Ok(canonical_directory) = fs::canonicalize(&self.directory) else {
            return;
        };
        if canonical_directory != self.directory {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if !metadata.is_file() || metadata_is_reparse(&metadata) {
            return;
        }
        let Ok(canonical_path) = fs::canonicalize(&self.path) else {
            return;
        };
        if canonical_path.parent() == Some(self.directory.as_path()) {
            let _ = fs::remove_file(canonical_path);
        }
    }
}

/// A sequential literal-query session whose memory and scratch growth are fixed.
///
/// The caller retains source access through [`Self::source`] even when the hit
/// quota is exhausted. Dropping the session closes and removes its unique
/// scratch artifact.
#[derive(Debug)]
pub struct QuerySession<R> {
    source: R,
    source_snapshot: SourceFingerprint,
    source_failure: Option<QuerySourceFailure>,
    scanner: Option<QueryBoundaryScanner>,
    literal: CompiledLiteral,
    config: QuerySessionConfig,
    buffer: Vec<u8>,
    pending: Option<PendingRecord>,
    scratch: ScratchHitStore,
    page: HitPageBuffer,
    next_row: u64,
    examined_records: u64,
    csv_diagnostic: Option<CsvDiagnostic>,
    status: QueryStepStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuerySourceFailure {
    Changed {
        change: SourceChange,
        actual_size: u64,
    },
    EvidenceUnavailable,
    ShortRead,
}

impl<R: PositionedRead> QuerySession<R> {
    /// Creates a session and its uniquely owned scratch hit file.
    ///
    /// `scratch_directory` must be a pre-existing, dedicated application
    /// directory. It is canonicalized before the unique file is created; no
    /// file is ever written alongside the query source by this API.
    ///
    /// # Errors
    /// Returns [`QueryError`] for invalid limits, an empty needle, fallible
    /// allocation failure, unsafe scratch paths, or source/scratch I/O.
    pub fn new(
        source: R,
        needle: &[u8],
        mode: LiteralMatchMode,
        config: QuerySessionConfig,
        scratch_directory: impl AsRef<Path>,
    ) -> Result<Self, QueryError> {
        let literal = CompiledLiteral::new(needle, mode)?;
        let source_snapshot = SourceFingerprint::capture(&source)?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(config.read_buffer_bytes.get())
            .map_err(|_| QueryError::AllocationUnavailable)?;
        buffer.resize(config.read_buffer_bytes.get(), 0);
        let page = HitPageBuffer::new(config.page_capacity)?;
        let scratch =
            ScratchHitStore::create(scratch_directory.as_ref(), config.scratch_quota_bytes)?;
        Ok(Self {
            source,
            source_snapshot,
            source_failure: None,
            scanner: Some(QueryBoundaryScanner::new(config.format)),
            literal,
            config,
            buffer,
            pending: None,
            scratch,
            page,
            next_row: 0,
            examined_records: 0,
            csv_diagnostic: None,
            status: QueryStepStatus::Running,
        })
    }

    /// Advances the query by no more than `max_work_bytes` of scanner and
    /// matcher work. Cancelled sessions resume by passing a fresh generation.
    ///
    /// # Errors
    /// Returns [`QueryError`] for zero work, source changes, address overflow,
    /// scan failure, scratch corruption, or I/O failure.
    pub fn advance(
        &mut self,
        max_work_bytes: usize,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<QueryStepStatus, QueryError> {
        let Some(mut remaining) = NonZeroUsize::new(max_work_bytes).map(NonZeroUsize::get) else {
            return Err(QueryError::ZeroWorkBudget);
        };
        if matches!(
            self.status,
            QueryStepStatus::Complete | QueryStepStatus::QuotaExceeded(_)
        ) {
            if token.is_cancelled(generation) {
                return Ok(self.status);
            }
            self.ensure_source_unchanged()?;
            return Ok(self.status);
        }
        self.status = QueryStepStatus::Running;

        loop {
            if token.is_cancelled(generation) {
                self.status = QueryStepStatus::Cancelled;
                return Ok(self.status);
            }
            self.ensure_source_unchanged()?;
            if self.pending.is_some() {
                let outcome = self.advance_pending(remaining, token, generation)?;
                remaining = remaining.saturating_sub(outcome.processed);
                match outcome.status {
                    PendingStatus::Running => {}
                    PendingStatus::Cancelled => {
                        self.status = QueryStepStatus::Cancelled;
                        return Ok(self.status);
                    }
                    PendingStatus::Complete => self.complete_pending()?,
                    PendingStatus::QuotaExceeded(details) => {
                        self.status = QueryStepStatus::QuotaExceeded(details);
                        return Ok(self.status);
                    }
                }
            } else if self
                .scanner_offset()
                .is_some_and(|offset| offset < self.source_snapshot.size())
            {
                let outcome = self.advance_scanner(remaining, token, generation)?;
                remaining = remaining.saturating_sub(outcome.consumed());
                match outcome.status() {
                    ScanStatus::Cancelled => {
                        self.status = QueryStepStatus::Cancelled;
                        return Ok(self.status);
                    }
                    ScanStatus::Complete | ScanStatus::Stopped => {}
                }
            } else if let Some(scanner) = self.scanner.take() {
                let (span, diagnostic) = scanner.finish();
                self.csv_diagnostic = diagnostic;
                if let Some(span) = span {
                    self.queue_record(span)?;
                } else {
                    self.status = QueryStepStatus::Complete;
                    return Ok(self.status);
                }
            } else {
                self.status = QueryStepStatus::Complete;
                return Ok(self.status);
            }

            if remaining == 0 {
                return Ok(self.status);
            }
        }
    }

    #[must_use]
    pub const fn status(&self) -> QueryStepStatus {
        self.status
    }

    #[must_use]
    pub fn progress(&self) -> QueryProgress {
        QueryProgress {
            source_bytes: self.source_snapshot.size(),
            scanned_bytes: self.scanner_offset().unwrap_or(self.source_snapshot.size()),
            discovered_records: self.next_row,
            examined_records: self.examined_records,
            stored_hits: self.scratch.entries,
            scratch_bytes: self.scratch.used_bytes,
            active_row: self.pending.map(|record| record.row),
            csv_diagnostic: self.csv_diagnostic,
        }
    }

    /// Returns the source without transferring or invalidating it.
    #[must_use]
    pub const fn source(&self) -> &R {
        &self.source
    }

    /// Returns the unique transient scratch path owned by this session.
    #[must_use]
    pub fn scratch_path(&self) -> &Path {
        &self.scratch.path
    }

    #[must_use]
    pub const fn hit_count(&self) -> u64 {
        self.scratch.entries
    }

    /// Loads up to the configured page capacity starting at `first_hit`.
    ///
    /// # Errors
    /// Returns [`QueryError`] if the scratch file cannot be read exactly.
    pub fn read_page(&mut self, first_hit: u64) -> Result<QueryHitPage<'_>, QueryError> {
        self.page.hits.clear();
        self.page.first_hit = first_hit;
        let available = self.scratch.entries.saturating_sub(first_hit);
        let capacity = u64::try_from(self.page.capacity).map_err(|_| QueryError::AddressSpace)?;
        let count = available.min(capacity);
        for relative in 0..count {
            let index = first_hit
                .checked_add(relative)
                .ok_or(QueryError::AddressSpace)?;
            let hit = self
                .scratch
                .read_hit(index)?
                .ok_or(QueryError::ScratchCorrupt)?;
            self.page.hits.push(hit);
        }
        Ok(self.page.view())
    }

    /// Reads one hit, using the fixed page as the only in-memory hit cache.
    ///
    /// # Errors
    /// Returns [`QueryError`] if the scratch file cannot be read exactly.
    pub fn read_hit(&mut self, index: u64) -> Result<Option<QueryHit>, QueryError> {
        if index >= self.scratch.entries {
            return Ok(None);
        }
        if let Some(hit) = self.cached_hit(index) {
            return Ok(Some(hit));
        }
        let capacity = u64::try_from(self.page.capacity).map_err(|_| QueryError::AddressSpace)?;
        let first = (index / capacity) * capacity;
        self.read_page(first)?;
        Ok(self.cached_hit(index))
    }

    /// Returns the next valid hit index, or the first hit for `None`.
    #[must_use]
    pub const fn next_hit_index(&self, current: Option<u64>) -> Option<u64> {
        let candidate = match current {
            Some(index) => match index.checked_add(1) {
                Some(next) => next,
                None => return None,
            },
            None => 0,
        };
        if candidate < self.scratch.entries {
            Some(candidate)
        } else {
            None
        }
    }

    /// Returns the previous valid hit index, or the last hit for `None`.
    #[must_use]
    pub const fn previous_hit_index(&self, current: Option<u64>) -> Option<u64> {
        match current {
            Some(0) => None,
            Some(index) if index <= self.scratch.entries => Some(index - 1),
            Some(_) | None => self.scratch.entries.checked_sub(1),
        }
    }

    fn scanner_offset(&self) -> Option<u64> {
        self.scanner.as_ref().map(QueryBoundaryScanner::offset)
    }

    fn ensure_source_unchanged(&mut self) -> Result<(), QueryError> {
        if let Some(failure) = self.source_failure {
            return Err(self.source_failure_error(failure));
        }
        let current = match SourceFingerprint::capture(&self.source) {
            Ok(current) => current,
            Err(error) => {
                self.source_failure = Some(QuerySourceFailure::EvidenceUnavailable);
                return Err(QueryError::Io(error));
            }
        };
        let change = self.source_snapshot.compare(current);
        if !change.is_changed() {
            return Ok(());
        }
        let failure = QuerySourceFailure::Changed {
            change,
            actual_size: current.size(),
        };
        self.source_failure = Some(failure);
        Err(self.source_failure_error(failure))
    }

    fn source_failure_error(&self, failure: QuerySourceFailure) -> QueryError {
        match failure {
            QuerySourceFailure::Changed {
                change: SourceChange::Shrank,
                ..
            }
            | QuerySourceFailure::ShortRead => QueryError::SourceShrank,
            QuerySourceFailure::Changed {
                change: SourceChange::Grew,
                actual_size,
            } => QueryError::SourceGrew {
                expected_size: self.source_snapshot.size(),
                actual_size,
            },
            QuerySourceFailure::Changed {
                change,
                actual_size,
            } => QueryError::SourceChanged {
                change,
                expected_size: self.source_snapshot.size(),
                actual_size,
            },
            QuerySourceFailure::EvidenceUnavailable => QueryError::SourceEvidenceUnavailable,
        }
    }

    fn checked_read_at(&mut self, offset: u64, request: usize) -> Result<usize, QueryError> {
        self.ensure_source_unchanged()?;
        let count = self.source.read_at(offset, &mut self.buffer[..request])?;
        let count = crate::source::validate_read_count(count, request)?;
        if count == 0 {
            self.source_failure = Some(QuerySourceFailure::ShortRead);
            return Err(QueryError::SourceShrank);
        }
        // Do not scan or match the bytes until the source is proven unchanged
        // after the positioned read.
        self.ensure_source_unchanged()?;
        Ok(count)
    }

    fn queue_record(&mut self, span: RecordSpan) -> Result<(), QueryError> {
        let row = self.next_row;
        self.next_row = self
            .next_row
            .checked_add(1)
            .ok_or(QueryError::RowOverflow)?;
        self.pending = Some(PendingRecord {
            row,
            span,
            cursor: span.start(),
            matcher_state: 0,
        });
        Ok(())
    }

    fn advance_scanner(
        &mut self,
        work_bytes: usize,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<ScanProgress, QueryError> {
        let offset = self.scanner_offset().ok_or(QueryError::ScratchCorrupt)?;
        let remaining_source = self.source_snapshot.size() - offset;
        let request = usize::try_from(remaining_source)
            .unwrap_or(usize::MAX)
            .min(self.buffer.len())
            .min(work_bytes);
        let count = self.checked_read_at(offset, request)?;
        let mut emitted = None;
        let progress = self
            .scanner
            .as_mut()
            .ok_or(QueryError::ScratchCorrupt)?
            .feed_one(
                &self.buffer[..count],
                token,
                generation,
                self.config.cancellation_check_bytes,
                &mut emitted,
            )?;
        if let Some(span) = emitted {
            self.queue_record(span)?;
        }
        Ok(progress)
    }

    fn advance_pending(
        &mut self,
        work_bytes: usize,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<PendingAdvance, QueryError> {
        let pending = self.pending.ok_or(QueryError::ScratchCorrupt)?;
        if pending.cursor == pending.span.end() {
            return Ok(PendingAdvance::complete(0));
        }
        let remaining_record = pending.span.end() - pending.cursor;
        let request = usize::try_from(remaining_record)
            .unwrap_or(usize::MAX)
            .min(self.buffer.len())
            .min(work_bytes);
        let count = self.checked_read_at(pending.cursor, request)?;
        self.match_pending_bytes(count, token, generation)
    }

    fn match_pending_bytes(
        &mut self,
        count: usize,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<PendingAdvance, QueryError> {
        let pending = self.pending.ok_or(QueryError::ScratchCorrupt)?;
        let mut state = pending.matcher_state;
        let mut processed = 0;
        while processed < count {
            if processed % self.config.cancellation_check_bytes.get() == 0
                && token.is_cancelled(generation)
            {
                self.update_pending_cursor(pending.cursor, processed, state)?;
                return Ok(PendingAdvance::cancelled(processed));
            }
            let byte = self.buffer[processed];
            let (next_state, matched) = self.literal.advance(state, byte);
            state = next_state;
            processed += 1;
            if matched {
                let end = pending
                    .cursor
                    .checked_add(u64::try_from(processed).map_err(|_| QueryError::AddressSpace)?)
                    .ok_or(QueryError::AddressSpace)?;
                let start = end
                    .checked_sub(self.literal.length_u64)
                    .ok_or(QueryError::AddressSpace)?;
                let span = RecordSpan::new(start, end).map_err(|_| QueryError::AddressSpace)?;
                let hit = QueryHit {
                    row: pending.row,
                    span,
                };
                if let Some(details) = self.append_hit(hit)? {
                    self.update_pending_cursor(pending.cursor, processed, state)?;
                    return Ok(PendingAdvance::quota(processed, details));
                }
            }
        }
        self.update_pending_cursor(pending.cursor, processed, state)?;
        let complete = self
            .pending
            .is_some_and(|record| record.cursor == record.span.end());
        Ok(if complete {
            PendingAdvance::complete(processed)
        } else {
            PendingAdvance::running(processed)
        })
    }

    fn update_pending_cursor(
        &mut self,
        original_cursor: u64,
        processed: usize,
        matcher_state: usize,
    ) -> Result<(), QueryError> {
        let relative = u64::try_from(processed).map_err(|_| QueryError::AddressSpace)?;
        let cursor = original_cursor
            .checked_add(relative)
            .ok_or(QueryError::AddressSpace)?;
        let pending = self.pending.as_mut().ok_or(QueryError::ScratchCorrupt)?;
        pending.cursor = cursor;
        pending.matcher_state = matcher_state;
        Ok(())
    }

    fn append_hit(&mut self, hit: QueryHit) -> Result<Option<QueryQuotaExceeded>, QueryError> {
        match self.scratch.append(hit)? {
            ScratchAppend::Stored(index) => {
                self.page.append(index, hit)?;
                Ok(None)
            }
            ScratchAppend::Quota { required_bytes } => Ok(Some(QueryQuotaExceeded {
                quota_bytes: self.scratch.quota_bytes,
                used_bytes: self.scratch.used_bytes,
                required_bytes,
                stored_hits: self.scratch.entries,
                rejected_hit: hit,
            })),
        }
    }

    fn complete_pending(&mut self) -> Result<(), QueryError> {
        self.pending = None;
        self.examined_records = self
            .examined_records
            .checked_add(1)
            .ok_or(QueryError::RowOverflow)?;
        Ok(())
    }

    fn cached_hit(&self, index: u64) -> Option<QueryHit> {
        let relative = index.checked_sub(self.page.first_hit)?;
        let relative = usize::try_from(relative).ok()?;
        self.page.hits.get(relative).copied()
    }
}

#[derive(Clone, Copy)]
struct PendingAdvance {
    processed: usize,
    status: PendingStatus,
}

impl PendingAdvance {
    const fn running(processed: usize) -> Self {
        Self {
            processed,
            status: PendingStatus::Running,
        }
    }

    const fn cancelled(processed: usize) -> Self {
        Self {
            processed,
            status: PendingStatus::Cancelled,
        }
    }

    const fn complete(processed: usize) -> Self {
        Self {
            processed,
            status: PendingStatus::Complete,
        }
    }

    const fn quota(processed: usize, details: QueryQuotaExceeded) -> Self {
        Self {
            processed,
            status: PendingStatus::QuotaExceeded(details),
        }
    }
}

#[derive(Clone, Copy)]
enum PendingStatus {
    Running,
    Cancelled,
    Complete,
    QuotaExceeded(QueryQuotaExceeded),
}

/// Removes only structurally valid, directly contained `LeanRows` v1 query files.
///
/// Call this during startup, before any query sessions use the dedicated
/// application scratch directory. Unrecognized files, directories, links,
/// reparse points, malformed headers, and partial entries are retained.
///
/// # Errors
/// Returns [`QueryError`] if the directory is unsafe or enumeration/removal of
/// a valid owned candidate fails.
pub fn recover_stale_query_files(
    scratch_directory: impl AsRef<Path>,
) -> Result<QueryRecoveryReport, QueryError> {
    let directory = canonical_scratch_directory(scratch_directory.as_ref())?;
    let mut report = QueryRecoveryReport::default();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(owner) = parse_owner_from_name(Some(&entry.file_name())) else {
            continue;
        };
        let path = entry.path();
        if path.parent() != Some(directory.as_path()) || !is_recoverable_query_file(&path, owner)? {
            report.retained_candidates = report
                .retained_candidates
                .checked_add(1)
                .ok_or(QueryError::AddressSpace)?;
            continue;
        }
        fs::remove_file(path)?;
        report.removed_files = report
            .removed_files
            .checked_add(1)
            .ok_or(QueryError::AddressSpace)?;
    }
    Ok(report)
}

fn canonical_scratch_directory(directory: &Path) -> Result<PathBuf, QueryError> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(QueryError::UnsafeScratchDirectory);
    }
    let canonical = fs::canonicalize(directory)?;
    let canonical_metadata = fs::symlink_metadata(&canonical)?;
    if !canonical.is_absolute()
        || !canonical_metadata.is_dir()
        || metadata_is_reparse(&canonical_metadata)
    {
        return Err(QueryError::UnsafeScratchDirectory);
    }
    Ok(canonical)
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn fresh_owner() -> [u8; 16] {
    let elapsed = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(error) => error.duration().as_nanos(),
    };
    let sequence = u128::from(NEXT_OWNER.fetch_add(1, Ordering::Relaxed));
    let process = u128::from(std::process::id()) << 64;
    (elapsed.rotate_left(29) ^ process ^ sequence).to_le_bytes()
}

fn scratch_file_name(owner: [u8; 16]) -> Result<String, QueryError> {
    let encoded = encode_owner(owner)?;
    let total = SCRATCH_NAME_PREFIX
        .len()
        .checked_add(encoded.len())
        .and_then(|length| length.checked_add(SCRATCH_NAME_SUFFIX.len()))
        .ok_or(QueryError::AddressSpace)?;
    let mut name = String::new();
    name.try_reserve_exact(total)
        .map_err(|_| QueryError::AllocationUnavailable)?;
    name.push_str(SCRATCH_NAME_PREFIX);
    name.push_str(&encoded);
    name.push_str(SCRATCH_NAME_SUFFIX);
    Ok(name)
}

fn encode_owner(owner: [u8; 16]) -> Result<String, QueryError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::new();
    encoded
        .try_reserve_exact(32)
        .map_err(|_| QueryError::AllocationUnavailable)?;
    for byte in owner {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

fn parse_owner_from_name(name: Option<&OsStr>) -> Option<[u8; 16]> {
    let name = name?.to_str()?;
    let encoded = name
        .strip_prefix(SCRATCH_NAME_PREFIX)?
        .strip_suffix(SCRATCH_NAME_SUFFIX)?;
    if encoded.len() != 32
        || !encoded
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    let mut owner = [0_u8; 16];
    for (index, byte) in owner.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&encoded[start..start + 2], 16).ok()?;
    }
    Some(owner)
}

fn scratch_header(owner: [u8; 16]) -> [u8; SCRATCH_HEADER_LEN] {
    let mut header = [0_u8; SCRATCH_HEADER_LEN];
    header[0..8].copy_from_slice(SCRATCH_MAGIC);
    header[8..10].copy_from_slice(&SCRATCH_VERSION.to_le_bytes());
    header[10..12].copy_from_slice(&48_u16.to_le_bytes());
    header[12..14].copy_from_slice(&24_u16.to_le_bytes());
    header[16..32].copy_from_slice(&owner);
    header
}

fn is_recoverable_query_file(path: &Path, owner: [u8; 16]) -> Result<bool, QueryError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata_is_reparse(&metadata) {
        return Ok(false);
    }
    let canonical = fs::canonicalize(path)?;
    if canonical.parent() != path.parent() {
        return Ok(false);
    }
    let mut file = File::open(&canonical)?;
    let mut header = [0_u8; SCRATCH_HEADER_LEN];
    match file.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(error) => return Err(QueryError::Io(error)),
    }
    if header != scratch_header(owner) {
        return Ok(false);
    }
    let length = metadata.len();
    if length < QUERY_SCRATCH_HEADER_BYTES {
        return Ok(false);
    }
    Ok((length - QUERY_SCRATCH_HEADER_BYTES).is_multiple_of(QUERY_HIT_ENTRY_BYTES))
}

fn encode_hit(hit: QueryHit) -> [u8; HIT_ENTRY_LEN] {
    let mut bytes = [0_u8; HIT_ENTRY_LEN];
    bytes[0..8].copy_from_slice(&hit.row.to_le_bytes());
    bytes[8..16].copy_from_slice(&hit.span.start().to_le_bytes());
    bytes[16..24].copy_from_slice(&hit.span.end().to_le_bytes());
    bytes
}

fn decode_hit(bytes: [u8; HIT_ENTRY_LEN]) -> Result<QueryHit, QueryError> {
    let mut row = [0_u8; 8];
    let mut start = [0_u8; 8];
    let mut end = [0_u8; 8];
    row.copy_from_slice(&bytes[0..8]);
    start.copy_from_slice(&bytes[8..16]);
    end.copy_from_slice(&bytes[16..24]);
    let span = RecordSpan::new(u64::from_le_bytes(start), u64::from_le_bytes(end))
        .map_err(|_| QueryError::ScratchCorrupt)?;
    Ok(QueryHit {
        row: u64::from_le_bytes(row),
        span,
    })
}
