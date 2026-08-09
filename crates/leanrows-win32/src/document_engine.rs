//! Bounded adapter between the core record engine and the native row cache.
//!
//! This module owns no threads or Win32 controls. A worker opens one document,
//! performs one cooperative scan step at a time, and transfers immutable
//! `UiRowSnapshot` values to the UI thread.

use core::fmt;
use std::cell::{Cell, RefCell};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use leanrows_core::{
    BudgetError, CancellationGeneration, CancellationToken, CsvDiagnostic, DocumentFormat,
    EngineLimits, FieldProjectionError, FieldProjectionLimits, FieldUtf8Status, FileSource,
    ManagedMemoryBudget, PositionedRead, RecordPreview, ScanSession, ScanSessionConfig,
    ScanSessionError, ScanStepStatus, SourceChange, SourceFingerprint, ViewportError,
    ViewportFormat, project_delimited_record, read_viewport,
};

const INDEX_CAPACITY: usize = 4_096;
const INITIAL_ROW_STRIDE: u64 = 1_024;
const READ_BUFFER_BYTES: usize = 64 * 1_024;
const READ_BUFFER_BYTES_U64: u64 = 64 * 1_024;
const CANCELLATION_CHECK_BYTES: usize = 16 * 1_024;
const MAX_SCAN_STEP_BYTES: usize = 64 * 1_024;
const MAX_VIEWPORT_ROWS: usize = 128;
const MAX_RECORD_PREVIEW_BYTES: usize = 4 * 1_024;
const MAX_VIEWPORT_PAYLOAD_BYTES: usize = MAX_VIEWPORT_ROWS * MAX_RECORD_PREVIEW_BYTES;
const VIEWPORT_SCRATCH_BYTES: usize = 128 * 1_024;
const CHECKPOINT_BUDGET_BYTES: usize =
    INDEX_CAPACITY * std::mem::size_of::<leanrows_core::Checkpoint>();
const CORE_OPERATION_BUDGET_BYTES: usize = 1_024 * 1_024;
const MAX_PROJECTED_FIELDS: usize = 64;
const MAX_DECODED_BYTES_PER_FIELD: usize = 1_024;
const MAX_DECODED_BYTES_PER_ROW: usize = MAX_RECORD_PREVIEW_BYTES;

/// Maximum retained preview code units, excluding the terminal NUL.
pub(crate) const MAX_PREVIEW_UTF16_UNITS: usize = 1_024;
const MAX_ROW_NUMBER_UTF16_UNITS: usize = 20;
const MAX_PATH_STORAGE_BYTES: usize = 64 * 1_024;
const STRONG_PATH_CHECK_BYTES: u64 = 4 * 1_024 * 1_024;
#[cfg(windows)]
const MAX_WINDOWS_PATH_UNITS: usize = 32_767;

/// A successfully opened document plus its immediately usable first viewport.
pub(crate) struct OpenedDocument {
    engine: DocumentEngine,
    first_snapshot: UiRowSnapshot,
}

/// A query-only clone of the retained document handle and its format.
///
/// The source wrapper revalidates both the cloned handle and the selected path
/// whenever the core query engine captures a fingerprint. This keeps an
/// atomic path replacement from silently searching a different snapshot than
/// the rows already displayed by the document worker.
pub(crate) struct DocumentQueryPlan {
    source: DocumentQuerySource,
    format: ViewportFormat,
}

impl DocumentQueryPlan {
    pub(crate) fn into_parts(self) -> (DocumentQuerySource, ViewportFormat) {
        (self.source, self.format)
    }
}

/// A positioned source tied to the exact document snapshot retained at open.
#[derive(Debug)]
pub(crate) struct DocumentQuerySource {
    source: FileSource,
    read_cache: RefCell<QueryReadCache>,
    path: PathBuf,
    source_snapshot: SourceFingerprint,
    #[cfg(windows)]
    windows_snapshot: windows_integrity::FileEvidence,
    integrity_failed: Cell<bool>,
}

#[derive(Debug)]
struct QueryReadCache {
    bytes: Vec<u8>,
    start: u64,
    valid: usize,
    loaded: bool,
}

impl QueryReadCache {
    fn new() -> Result<Self, DocumentEngineError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(READ_BUFFER_BYTES)
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        bytes.resize(READ_BUFFER_BYTES, 0);
        Ok(Self {
            bytes,
            start: 0,
            valid: 0,
            loaded: false,
        })
    }

    fn read_at(
        &mut self,
        source: &FileSource,
        offset: u64,
        destination: &mut [u8],
    ) -> io::Result<usize> {
        if destination.is_empty() {
            return Ok(0);
        }
        let valid_u64 = u64::try_from(self.valid).map_err(io::Error::other)?;
        let end = self
            .start
            .checked_add(valid_u64)
            .ok_or_else(|| io::Error::other("query read-cache address overflow"))?;
        if !self.loaded || offset < self.start || offset >= end {
            self.start = (offset / READ_BUFFER_BYTES_U64) * READ_BUFFER_BYTES_U64;
            self.valid = source.read_at(self.start, &mut self.bytes)?;
            if self.valid > self.bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "query source returned more bytes than the fixed cache buffer",
                ));
            }
            self.loaded = true;
        }
        let relative_u64 = offset
            .checked_sub(self.start)
            .ok_or_else(|| io::Error::other("query read-cache offset moved backwards"))?;
        let relative = usize::try_from(relative_u64).map_err(io::Error::other)?;
        if relative >= self.valid {
            return Ok(0);
        }
        let count = destination.len().min(self.valid - relative);
        destination[..count].copy_from_slice(&self.bytes[relative..relative + count]);
        Ok(count)
    }
}

impl DocumentQuerySource {
    fn verify_handle(&self) -> io::Result<SourceFingerprint> {
        if self.integrity_failed.get() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "query source snapshot integrity was previously lost",
            ));
        }

        // The Windows open deliberately excludes FILE_SHARE_WRITE. The
        // retained handle's bytes therefore cannot be changed in place, and
        // the worker performs the expensive path/file-ID validation after
        // every fixed query quantum. Returning the captured evidence here
        // avoids one metadata syscall for every short logical record.
        #[cfg(windows)]
        return Ok(self.source_snapshot);

        #[cfg(not(windows))]
        let handle_fingerprint = self.source.fingerprint()?;
        #[cfg(not(windows))]
        self.reject_change(
            self.source_snapshot.compare(handle_fingerprint),
            handle_fingerprint.size(),
        )?;
        #[cfg(not(windows))]
        Ok(handle_fingerprint)
    }

    fn verify_strong_source(&self) -> io::Result<SourceFingerprint> {
        let handle_fingerprint = self.source.fingerprint()?;
        self.reject_change(
            self.source_snapshot.compare(handle_fingerprint),
            handle_fingerprint.size(),
        )?;

        let entry_metadata = fs::symlink_metadata(&self.path)?;
        if !entry_metadata.is_file() || metadata_is_reparse(&entry_metadata) {
            self.integrity_failed.set(true);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "query source path is no longer the retained regular file",
            ));
        }

        #[cfg(windows)]
        {
            let handle_evidence = windows_integrity::capture(self.source.as_file())?;
            self.reject_change(
                self.windows_snapshot.compare(handle_evidence),
                handle_evidence.size(),
            )?;

            let path_file = open_source_file(&self.path)?;
            let path_metadata = path_file.metadata()?;
            if !path_metadata.is_file() || metadata_is_reparse(&path_metadata) {
                self.integrity_failed.set(true);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "query source path is no longer the retained regular file",
                ));
            }
            let path_evidence = windows_integrity::capture(&path_file)?;
            self.reject_change(
                self.windows_snapshot.compare(path_evidence),
                path_evidence.size(),
            )?;
        }

        let path_fingerprint = FileSource::fingerprint_path(&self.path)?;
        self.reject_change(
            self.source_snapshot.compare(path_fingerprint),
            path_fingerprint.size(),
        )?;
        Ok(handle_fingerprint)
    }

    fn reject_change(&self, change: SourceChange, actual_size: u64) -> io::Result<()> {
        if !change.is_changed() {
            return Ok(());
        }
        self.integrity_failed.set(true);
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "query source changed ({change:?}) from {} to {actual_size} bytes",
                self.source_snapshot.size()
            ),
        ))
    }
}

impl PositionedRead for DocumentQuerySource {
    fn size(&self) -> io::Result<u64> {
        self.verify_handle().map(SourceFingerprint::size)
    }

    fn fingerprint(&self) -> io::Result<SourceFingerprint> {
        self.verify_handle()
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        // QuerySession captures this wrapper's strong fingerprint immediately
        // before and after every positioned read. Keeping the read itself free
        // of duplicate path opens preserves the fixed-quantum throughput.
        self.read_cache
            .try_borrow_mut()
            .map_err(|_| io::Error::other("query read cache is already borrowed"))?
            .read_at(&self.source, offset, destination)
    }
}

impl OpenedDocument {
    /// Moves the state into a worker with the first viewport ready for delivery.
    pub(crate) fn into_parts(self) -> (DocumentEngine, UiRowSnapshot) {
        (self.engine, self.first_snapshot)
    }
}

/// One read-only document and its incrementally built core index.
pub(crate) struct DocumentEngine {
    format: DocumentFormat,
    path: PathBuf,
    source: FileSource,
    source_snapshot: SourceFingerprint,
    #[cfg(windows)]
    windows_snapshot: windows_integrity::FileEvidence,
    scan: ScanSession,
    viewport_limits: EngineLimits,
    projection_limits: FieldProjectionLimits,
    integrity_failed: Cell<bool>,
    bytes_since_strong_path_check: u64,
}

impl DocumentEngine {
    /// Opens a regular local file read-only and materializes a bounded first
    /// viewport before any full-file indexing loop begins.
    pub(crate) fn open(
        path: &Path,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<OpenedDocument, DocumentEngineError> {
        let format = infer_format(path)?;
        let path = validate_local_regular_path(path)?;
        let file = open_source_file(&path).map_err(|source| DocumentEngineError::Io {
            operation: "open file read-only",
            source,
        })?;
        let metadata = file.metadata().map_err(|source| DocumentEngineError::Io {
            operation: "inspect opened file",
            source,
        })?;
        if !metadata.is_file() {
            return Err(DocumentEngineError::NotRegularFile);
        }
        if metadata_is_reparse(&metadata) {
            return Err(DocumentEngineError::ReparsePointUnsupported);
        }
        #[cfg(windows)]
        let windows_snapshot =
            windows_integrity::capture(&file).map_err(|source| DocumentEngineError::Io {
                operation: "capture opened Windows file identity",
                source,
            })?;
        let source = FileSource::from_file(file).map_err(|source| DocumentEngineError::Io {
            operation: "capture opened file metadata",
            source,
        })?;
        let source_snapshot =
            SourceFingerprint::capture(&source).map_err(|source| DocumentEngineError::Io {
                operation: "capture opened file fingerprint",
                source,
            })?;
        let scan = ScanSession::new(
            &source,
            format,
            ScanSessionConfig::new(
                INDEX_CAPACITY,
                INITIAL_ROW_STRIDE,
                READ_BUFFER_BYTES,
                CANCELLATION_CHECK_BYTES,
            ),
        )?;
        let viewport_limits = viewport_limits()?;
        let projection_limits = FieldProjectionLimits::new(
            MAX_PROJECTED_FIELDS,
            MAX_DECODED_BYTES_PER_FIELD,
            MAX_DECODED_BYTES_PER_ROW,
        )?;
        let engine = Self {
            format,
            path,
            source,
            source_snapshot,
            #[cfg(windows)]
            windows_snapshot,
            scan,
            viewport_limits,
            projection_limits,
            integrity_failed: Cell::new(false),
            bytes_since_strong_path_check: 0,
        };
        let first_snapshot = engine.snapshot(0, token, generation)?;
        Ok(OpenedDocument {
            engine,
            first_snapshot,
        })
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn format(&self) -> DocumentFormat {
        self.format
    }

    /// Performs at most one fixed 64 KiB scan quantum.
    pub(crate) fn scan_step(
        &mut self,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<DocumentScanStep, DocumentEngineError> {
        if token.is_cancelled(generation) {
            return Ok(DocumentScanStep {
                processed_bytes: 0,
                discovered_rows: 0,
                status: DocumentScanStatus::Cancelled,
                progress: self.progress(),
            });
        }
        self.verify_source(false)?;
        let step = match self
            .scan
            .step(&self.source, MAX_SCAN_STEP_BYTES, token, generation)
        {
            Ok(step) => step,
            Err(error) => {
                if error.is_source_integrity_failure() {
                    self.integrity_failed.set(true);
                }
                return Err(DocumentEngineError::Scan(error));
            }
        };
        self.bytes_since_strong_path_check = self
            .bytes_since_strong_path_check
            .saturating_add(u64::try_from(step.processed_bytes()).unwrap_or(u64::MAX));
        let strong_path_check = self.bytes_since_strong_path_check >= STRONG_PATH_CHECK_BYTES
            || step.status() == ScanStepStatus::Complete;
        self.verify_source(strong_path_check)?;
        if strong_path_check {
            self.bytes_since_strong_path_check = 0;
        }
        Ok(DocumentScanStep {
            processed_bytes: step.processed_bytes(),
            discovered_rows: step.discovered_records(),
            status: match step.status() {
                ScanStepStatus::Progress => DocumentScanStatus::Progress,
                ScanStepStatus::Complete => DocumentScanStatus::Complete,
                ScanStepStatus::Cancelled => DocumentScanStatus::Cancelled,
            },
            progress: self.progress(),
        })
    }

    #[must_use]
    pub(crate) fn progress(&self) -> DocumentProgress {
        let progress = self.scan.progress();
        DocumentProgress {
            scanned_bytes: progress.scanned_bytes(),
            source_bytes: progress.source_bytes(),
            row_count: progress.record_count(),
            complete: progress.is_complete(),
            diagnostic: self.scan.diagnostic(),
        }
    }

    /// Reconstructs and pre-encodes one bounded viewport.
    pub(crate) fn snapshot(
        &self,
        first_row: u64,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<UiRowSnapshot, DocumentEngineError> {
        if token.is_cancelled(generation) {
            return Err(DocumentEngineError::Cancelled);
        }
        self.verify_source(true)?;
        let viewport = match read_viewport(
            &self.source,
            self.format.viewport_format(),
            self.scan.index(),
            first_row,
            self.viewport_limits,
            token,
            generation,
        ) {
            Ok(viewport) => viewport,
            Err(error) => {
                if matches!(
                    error,
                    ViewportError::SourceShrank | ViewportError::SourceChanged { .. }
                ) {
                    self.integrity_failed.set(true);
                }
                return Err(map_viewport_error(error));
            }
        };
        self.verify_source(true)?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(viewport.records().len())
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        let column_kind = if self.format.delimited_dialect().is_some() {
            UiColumnKind::Fields
        } else {
            UiColumnKind::Preview
        };
        let mut data_columns = usize::from(column_kind == UiColumnKind::Preview);
        for record in viewport.records() {
            if token.is_cancelled(generation) {
                return Err(DocumentEngineError::Cancelled);
            }
            let row = build_cached_row(record, self.format, self.projection_limits)?;
            data_columns = data_columns.max(row.data_columns());
            rows.push(row);
        }
        Ok(UiRowSnapshot {
            first_row,
            rows,
            columns: UiColumnLayout::new(column_kind, data_columns)?,
            reached_eof: viewport.reached_eof(),
            diagnostic: viewport.csv_diagnostic(),
        })
    }

    /// Clones the retained read-only handle for a literal-query session.
    ///
    /// The clone is proven against the current document snapshot before it is
    /// returned, and the returned source continues performing strong path and
    /// handle checks for the lifetime of the query.
    pub(crate) fn query_plan(&self) -> Result<DocumentQueryPlan, DocumentEngineError> {
        self.verify_source(true)?;
        let file = self
            .source
            .as_file()
            .try_clone()
            .map_err(|source| DocumentEngineError::Io {
                operation: "clone retained file handle for query",
                source,
            })?;
        let source = FileSource::from_file(file).map_err(|source| DocumentEngineError::Io {
            operation: "inspect cloned query file handle",
            source,
        })?;
        let query_source = DocumentQuerySource {
            source,
            read_cache: RefCell::new(QueryReadCache::new()?),
            path: self.path.clone(),
            source_snapshot: self.source_snapshot,
            #[cfg(windows)]
            windows_snapshot: self.windows_snapshot,
            integrity_failed: Cell::new(false),
        };
        query_source
            .verify_strong_source()
            .map_err(|source| DocumentEngineError::Io {
                operation: "validate cloned query snapshot",
                source,
            })?;
        self.verify_source(true)?;
        Ok(DocumentQueryPlan {
            source: query_source,
            format: self.format.viewport_format(),
        })
    }

    /// Revalidates the retained handle and selected path after one query work
    /// quantum, before any newly found match is published to the UI.
    pub(crate) fn validate_query_snapshot(&self) -> Result<(), DocumentEngineError> {
        self.verify_source(true)
    }

    fn verify_source(&self, strong_path_check: bool) -> Result<(), DocumentEngineError> {
        if self.integrity_failed.get() {
            return Err(DocumentEngineError::SourceIntegrityLost);
        }

        let handle_fingerprint = match SourceFingerprint::capture(&self.source) {
            Ok(fingerprint) => fingerprint,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "revalidate opened file fingerprint",
                    source,
                });
            }
        };
        self.reject_change(
            self.source_snapshot.compare(handle_fingerprint),
            handle_fingerprint.size(),
        )?;

        let entry_metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "revalidate source path entry",
                    source,
                });
            }
        };
        if !entry_metadata.is_file() || metadata_is_reparse(&entry_metadata) {
            return self.reject_change(SourceChange::IdentityChanged, entry_metadata.len());
        }
        #[cfg(windows)]
        self.verify_windows_source(strong_path_check)?;

        let path_fingerprint = match FileSource::fingerprint_path(&self.path) {
            Ok(fingerprint) => fingerprint,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "revalidate source path fingerprint",
                    source,
                });
            }
        };
        self.reject_change(
            self.source_snapshot.compare(path_fingerprint),
            path_fingerprint.size(),
        )?;

        #[cfg(not(windows))]
        let _ = strong_path_check;
        Ok(())
    }

    #[cfg(windows)]
    fn verify_windows_source(&self, strong_path_check: bool) -> Result<(), DocumentEngineError> {
        let handle_evidence = match windows_integrity::capture(self.source.as_file()) {
            Ok(evidence) => evidence,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "revalidate opened Windows file identity",
                    source,
                });
            }
        };
        self.reject_change(
            self.windows_snapshot.compare(handle_evidence),
            handle_evidence.size(),
        )?;
        if !strong_path_check {
            return Ok(());
        }

        let path_file = match open_source_file(&self.path) {
            Ok(file) => file,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "reopen source path for identity validation",
                    source,
                });
            }
        };
        let metadata = match path_file.metadata() {
            Ok(metadata) => metadata,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "inspect reopened source path",
                    source,
                });
            }
        };
        if !metadata.is_file() || metadata_is_reparse(&metadata) {
            return self.reject_change(SourceChange::IdentityChanged, metadata.len());
        }
        let path_evidence = match windows_integrity::capture(&path_file) {
            Ok(evidence) => evidence,
            Err(source) => {
                self.integrity_failed.set(true);
                return Err(DocumentEngineError::Io {
                    operation: "capture current Windows path identity",
                    source,
                });
            }
        };
        self.reject_change(
            self.windows_snapshot.compare(path_evidence),
            path_evidence.size(),
        )
    }

    fn reject_change(
        &self,
        change: SourceChange,
        actual_size: u64,
    ) -> Result<(), DocumentEngineError> {
        if change.is_changed() {
            self.integrity_failed.set(true);
            Err(DocumentEngineError::SourceChanged {
                change,
                expected_size: self.source_snapshot.size(),
                actual_size,
            })
        } else {
            Ok(())
        }
    }
}

/// Progress usable after every cooperative scan quantum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentProgress {
    scanned_bytes: u64,
    source_bytes: u64,
    row_count: u64,
    complete: bool,
    diagnostic: Option<CsvDiagnostic>,
}

impl DocumentProgress {
    #[must_use]
    pub(crate) const fn scanned_bytes(self) -> u64 {
        self.scanned_bytes
    }

    #[must_use]
    pub(crate) const fn source_bytes(self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub(crate) const fn row_count(self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub(crate) const fn is_complete(self) -> bool {
        self.complete
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn diagnostic(self) -> Option<CsvDiagnostic> {
        self.diagnostic
    }
}

/// Why one bounded worker quantum returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentScanStatus {
    Progress,
    Complete,
    Cancelled,
}

/// Scan evidence emitted after one bounded worker quantum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentScanStep {
    processed_bytes: usize,
    discovered_rows: u64,
    status: DocumentScanStatus,
    progress: DocumentProgress,
}

impl DocumentScanStep {
    #[must_use]
    pub(crate) const fn processed_bytes(self) -> usize {
        self.processed_bytes
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn discovered_rows(self) -> u64 {
        self.discovered_rows
    }

    #[must_use]
    pub(crate) const fn status(self) -> DocumentScanStatus {
        self.status
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn progress(self) -> DocumentProgress {
        self.progress
    }
}

/// Sorted, immutable, pointer-stable UTF-16 rows ready for an owner-data list.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct UiRowSnapshot {
    first_row: u64,
    rows: Vec<UiCachedRow>,
    columns: UiColumnLayout,
    reached_eof: bool,
    diagnostic: Option<CsvDiagnostic>,
}

/// Semantic shape of the bounded native data columns (excluding `Row`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiColumnKind {
    Preview,
    Fields,
}

/// Bounded column metadata transferred alongside every immutable viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UiColumnLayout {
    kind: UiColumnKind,
    data_columns: u8,
}

impl UiColumnLayout {
    fn new(kind: UiColumnKind, data_columns: usize) -> Result<Self, DocumentEngineError> {
        if data_columns > MAX_PROJECTED_FIELDS
            || (kind == UiColumnKind::Preview && data_columns != 1)
        {
            return Err(DocumentEngineError::TooManyDisplayColumns);
        }
        Ok(Self {
            kind,
            data_columns: u8::try_from(data_columns)
                .map_err(|_| DocumentEngineError::TooManyDisplayColumns)?,
        })
    }

    #[must_use]
    pub(crate) const fn preview() -> Self {
        Self {
            kind: UiColumnKind::Preview,
            data_columns: 1,
        }
    }

    #[must_use]
    pub(crate) const fn fields(data_columns: u8) -> Option<Self> {
        if data_columns as usize > MAX_PROJECTED_FIELDS {
            None
        } else {
            Some(Self {
                kind: UiColumnKind::Fields,
                data_columns,
            })
        }
    }

    #[must_use]
    pub(crate) const fn is_valid(self) -> bool {
        self.data_columns as usize <= MAX_PROJECTED_FIELDS
            && (matches!(self.kind, UiColumnKind::Fields) || self.data_columns == 1)
    }

    #[must_use]
    pub(crate) const fn kind(self) -> UiColumnKind {
        self.kind
    }

    #[must_use]
    pub(crate) const fn data_columns(self) -> u8 {
        self.data_columns
    }

    #[must_use]
    pub(crate) fn grow(self, candidate: Self) -> Option<Self> {
        if !self.is_valid() || !candidate.is_valid() || self.kind != candidate.kind {
            return None;
        }
        Some(Self {
            kind: self.kind,
            data_columns: if self.data_columns > candidate.data_columns {
                self.data_columns
            } else {
                candidate.data_columns
            },
        })
    }
}

impl UiRowSnapshot {
    #[must_use]
    pub(crate) const fn first_row(&self) -> u64 {
        self.first_row
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub(crate) const fn columns(&self) -> UiColumnLayout {
        self.columns
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn reached_eof(&self) -> bool {
        self.reached_eof
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn diagnostic(&self) -> Option<CsvDiagnostic> {
        self.diagnostic
    }

    /// Returns stable NUL-terminated storage without allocation, locks, or I/O.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn cell(&self, absolute_row: u64, subitem: usize) -> Option<&[u16]> {
        let index = self
            .rows
            .binary_search_by_key(&absolute_row, |row| row.absolute_row)
            .ok()?;
        self.rows.get(index)?.cell(subitem)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn row(&self, absolute_row: u64) -> Option<&UiCachedRow> {
        let index = self
            .rows
            .binary_search_by_key(&absolute_row, |row| row.absolute_row)
            .ok()?;
        self.rows.get(index)
    }

    /// Transfers the already encoded rows to the worker cache bridge.
    pub(crate) fn into_rows(self) -> Vec<UiCachedRow> {
        self.rows
    }
}

/// One cached row and its source/display integrity flags.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct UiCachedRow {
    absolute_row: u64,
    cells: Vec<Utf16Cell>,
    source_truncated: bool,
    display_truncated: bool,
    escaped_non_utf8: bool,
}

pub(crate) struct UiCachedRowParts {
    pub(crate) absolute_row: u64,
    pub(crate) cells: Vec<Utf16Cell>,
    pub(crate) source_truncated: bool,
    pub(crate) display_truncated: bool,
    pub(crate) escaped_non_utf8: bool,
}

impl UiCachedRow {
    fn data_columns(&self) -> usize {
        self.cells.len().saturating_sub(1)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn absolute_row(&self) -> u64 {
        self.absolute_row
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn source_truncated(&self) -> bool {
        self.source_truncated
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn display_truncated(&self) -> bool {
        self.display_truncated
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn escaped_non_utf8(&self) -> bool {
        self.escaped_non_utf8
    }

    #[cfg(test)]
    #[must_use]
    fn cell(&self, subitem: usize) -> Option<&[u16]> {
        self.cells.get(subitem).map(Utf16Cell::as_slice)
    }

    /// Moves UTF-16 cell ownership without decoding or copying source text.
    pub(crate) fn into_parts(self) -> UiCachedRowParts {
        UiCachedRowParts {
            absolute_row: self.absolute_row,
            cells: self.cells,
            source_truncated: self.source_truncated,
            display_truncated: self.display_truncated,
            escaped_non_utf8: self.escaped_non_utf8,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Utf16Cell {
    units: Box<[u16]>,
}

impl Utf16Cell {
    fn decimal(value: u64) -> Result<Self, DocumentEngineError> {
        const DIGITS: [u16; 10] = [48, 49, 50, 51, 52, 53, 54, 55, 56, 57];
        let mut encoded = [0_u16; MAX_ROW_NUMBER_UTF16_UNITS];
        let mut cursor = encoded.len();
        let mut remaining = value;
        loop {
            cursor -= 1;
            let digit = usize::try_from(remaining % 10)
                .map_err(|_| DocumentEngineError::ArithmeticOverflow)?;
            encoded[cursor] = DIGITS[digit];
            remaining /= 10;
            if remaining == 0 {
                break;
            }
        }
        let mut units = Vec::new();
        units
            .try_reserve_exact(encoded.len() - cursor + 1)
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        units.extend_from_slice(&encoded[cursor..]);
        units.push(0);
        Ok(Self {
            units: units.into_boxed_slice(),
        })
    }

    #[cfg(test)]
    #[must_use]
    fn as_slice(&self) -> &[u16] {
        &self.units
    }

    /// Moves the pointer-stable NUL-terminated payload into the worker cache.
    pub(crate) fn into_units(self) -> Box<[u16]> {
        self.units
    }
}

struct PreviewBuilder {
    units: Vec<u16>,
    limit: usize,
    truncated: bool,
    escaped_non_utf8: bool,
}

impl PreviewBuilder {
    fn new(limit: usize, input_bytes: usize) -> Result<Self, DocumentEngineError> {
        let maximum = limit
            .checked_add(1)
            .ok_or(DocumentEngineError::ArithmeticOverflow)?;
        // One input byte can expand to at most a six-unit visible control
        // escape. Reserve only the bounded amount this field can require.
        let capacity = input_bytes.saturating_mul(6).saturating_add(5).min(maximum);
        let mut units = Vec::new();
        units
            .try_reserve_exact(capacity)
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        Ok(Self {
            units,
            limit,
            truncated: false,
            escaped_non_utf8: false,
        })
    }

    fn push_bytes(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(valid) => {
                    self.push_valid(valid);
                    break;
                }
                Err(error) => {
                    let valid_length = error.valid_up_to();
                    if valid_length != 0
                        && let Ok(valid) = std::str::from_utf8(&bytes[..valid_length])
                    {
                        self.push_valid(valid);
                    }
                    let invalid_length = error.error_len().unwrap_or(bytes.len() - valid_length);
                    let invalid_end = valid_length.saturating_add(invalid_length).min(bytes.len());
                    for byte in &bytes[valid_length..invalid_end] {
                        self.push_hex_byte(*byte);
                    }
                    bytes = &bytes[invalid_end..];
                }
            }
        }
    }

    fn push_valid(&mut self, text: &str) {
        for character in text.chars() {
            match character {
                '\n' => self.push_ascii(r"\n"),
                '\r' => self.push_ascii(r"\r"),
                '\t' => self.push_ascii(r"\t"),
                '\0' => self.push_ascii(r"\0"),
                value if value.is_control() => self.push_control_escape(value),
                value => {
                    let mut encoded = [0_u16; 2];
                    self.push_units(value.encode_utf16(&mut encoded));
                }
            }
        }
    }

    fn push_control_escape(&mut self, character: char) {
        const HEX: [u16; 16] = [
            48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 65, 66, 67, 68, 69, 70,
        ];
        self.push_ascii(r"\u{");
        let mut digits = [0_u16; 6];
        let mut cursor = digits.len();
        let mut remaining = u32::from(character);
        loop {
            cursor -= 1;
            let index = usize::try_from(remaining & 0x0f).unwrap_or_default();
            digits[cursor] = HEX[index];
            remaining >>= 4;
            if remaining == 0 {
                break;
            }
        }
        self.push_units(&digits[cursor..]);
        self.push_ascii("}");
    }

    fn push_hex_byte(&mut self, byte: u8) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        self.escaped_non_utf8 = true;
        let encoded = [
            92,
            120,
            u16::from(HEX[usize::from(byte >> 4)]),
            u16::from(HEX[usize::from(byte & 0x0f)]),
        ];
        self.push_units(&encoded);
    }

    fn push_ascii(&mut self, text: &str) {
        for byte in text.bytes() {
            self.push_units(&[u16::from(byte)]);
        }
    }

    fn push_units(&mut self, units: &[u16]) {
        let available = self.limit.saturating_sub(self.units.len());
        let retained = available.min(units.len());
        self.units.extend_from_slice(&units[..retained]);
        if retained != units.len() {
            self.truncated = true;
        }
    }

    fn finish(mut self, omission_marker: bool) -> Utf16Cell {
        if omission_marker || self.truncated {
            const MARKER: &[u16] = &[32, 46, 46, 46];
            if self.limit >= MARKER.len() {
                self.units.truncate(self.limit - MARKER.len());
                self.units.extend_from_slice(MARKER);
            }
        }
        self.units.push(0);
        Utf16Cell {
            units: self.units.into_boxed_slice(),
        }
    }
}

fn build_cached_row(
    record: &RecordPreview,
    format: DocumentFormat,
    projection_limits: FieldProjectionLimits,
) -> Result<UiCachedRow, DocumentEngineError> {
    let mut source_truncated = record.is_truncated();
    let mut display_truncated = false;
    let mut escaped_non_utf8 = false;
    let mut cells = Vec::new();
    if let Some(dialect) = format.delimited_dialect() {
        let projection = project_delimited_record(record, dialect, projection_limits)?;
        source_truncated |= projection.are_fields_truncated();
        cells
            .try_reserve_exact(projection.fields().len().saturating_add(1))
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        cells.push(Utf16Cell::decimal(record.row().saturating_add(1))?);
        for field in projection.fields() {
            let mut value =
                PreviewBuilder::new(MAX_PREVIEW_UTF16_UNITS, field.decoded_bytes().len())?;
            value.push_bytes(field.decoded_bytes());
            escaped_non_utf8 |= field.utf8_status() != FieldUtf8Status::Valid;
            source_truncated |= field.is_truncated();
            display_truncated |= value.truncated;
            escaped_non_utf8 |= value.escaped_non_utf8;
            cells.push(value.finish(field.is_truncated()));
        }
    } else {
        cells
            .try_reserve_exact(2)
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        cells.push(Utf16Cell::decimal(record.row().saturating_add(1))?);
        let mut preview = PreviewBuilder::new(MAX_PREVIEW_UTF16_UNITS, record.bytes().len())?;
        preview.push_bytes(record.bytes());
        escaped_non_utf8 |= preview.escaped_non_utf8;
        display_truncated = preview.truncated;
        cells.push(preview.finish(source_truncated));
    }
    Ok(UiCachedRow {
        absolute_row: record.row(),
        cells,
        source_truncated,
        display_truncated,
        escaped_non_utf8,
    })
}

fn viewport_limits() -> Result<EngineLimits, DocumentEngineError> {
    let budget = ManagedMemoryBudget::new(
        CORE_OPERATION_BUDGET_BYTES,
        READ_BUFFER_BYTES,
        MAX_VIEWPORT_PAYLOAD_BYTES,
        CHECKPOINT_BUDGET_BYTES,
        VIEWPORT_SCRATCH_BYTES,
    )?;
    Ok(EngineLimits::new(
        READ_BUFFER_BYTES,
        MAX_VIEWPORT_ROWS,
        MAX_RECORD_PREVIEW_BYTES,
        MAX_VIEWPORT_PAYLOAD_BYTES,
        CANCELLATION_CHECK_BYTES,
        budget,
    )?)
}

pub(crate) fn infer_format(path: &Path) -> Result<DocumentFormat, DocumentEngineError> {
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or(DocumentEngineError::UnsupportedFormat)?;
    if extension.eq_ignore_ascii_case("csv") {
        Ok(DocumentFormat::Csv)
    } else if extension.eq_ignore_ascii_case("tsv") {
        Ok(DocumentFormat::Tsv)
    } else if extension.eq_ignore_ascii_case("jsonl") || extension.eq_ignore_ascii_case("ndjson") {
        Ok(DocumentFormat::Jsonl)
    } else if extension.eq_ignore_ascii_case("log") || extension.eq_ignore_ascii_case("txt") {
        Ok(DocumentFormat::Log)
    } else {
        Err(DocumentEngineError::UnsupportedFormat)
    }
}

#[cfg(windows)]
fn open_source_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

    // Excluding FILE_SHARE_WRITE prevents in-place writers from changing bytes
    // under the retained snapshot. FILE_SHARE_DELETE remains enabled so
    // editors can perform atomic replace saves; path identity checks then
    // reject the rotated object and request a clean reload.
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_DELETE.0)
        .open(path)
}

#[cfg(not(windows))]
fn open_source_file(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn validate_local_regular_path(path: &Path) -> Result<PathBuf, DocumentEngineError> {
    if path.as_os_str().is_empty() {
        return Err(DocumentEngineError::EmptyPath);
    }
    if path.as_os_str().len() > MAX_PATH_STORAGE_BYTES {
        return Err(DocumentEngineError::PathTooLong);
    }
    validate_path_syntax(path)?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| DocumentEngineError::Io {
                operation: "resolve current directory",
                source,
            })?
            .join(path)
    };
    ensure_local_volume(&absolute)?;
    let entry_metadata =
        fs::symlink_metadata(&absolute).map_err(|source| DocumentEngineError::Io {
            operation: "inspect selected path entry",
            source,
        })?;
    if metadata_is_reparse(&entry_metadata) {
        return Err(DocumentEngineError::ReparsePointUnsupported);
    }
    let canonical = fs::canonicalize(&absolute).map_err(|source| DocumentEngineError::Io {
        operation: "resolve file path",
        source,
    })?;
    validate_path_syntax(&canonical)?;
    ensure_local_volume(&canonical)?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|source| DocumentEngineError::Io {
        operation: "inspect file path",
        source,
    })?;
    if !metadata.is_file() {
        return Err(DocumentEngineError::NotRegularFile);
    }
    if metadata_is_reparse(&metadata) {
        return Err(DocumentEngineError::ReparsePointUnsupported);
    }
    Ok(canonical)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_integrity {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;

    use leanrows_core::SourceChange;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) struct FileEvidence {
        volume: u32,
        file_index: u64,
        size: u64,
        creation_time: u64,
        last_write_time: u64,
        change_time: u64,
        attributes: u32,
    }

    impl FileEvidence {
        pub(super) fn compare(self, current: Self) -> SourceChange {
            if self.volume != current.volume || self.file_index != current.file_index {
                SourceChange::IdentityChanged
            } else if current.size < self.size {
                SourceChange::Shrank
            } else if current.size > self.size {
                SourceChange::Grew
            } else if self.creation_time != current.creation_time
                || self.last_write_time != current.last_write_time
                || self.change_time != current.change_time
                || self.attributes != current.attributes
            {
                SourceChange::RevisionChanged
            } else {
                SourceChange::Unchanged
            }
        }

        pub(super) const fn size(self) -> u64 {
            self.size
        }
    }

    pub(super) fn capture(file: &File) -> io::Result<FileEvidence> {
        let handle = HANDLE(file.as_raw_handle());
        let mut legacy = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: handle is borrowed from a live File, and legacy is writable
        // for the exact structure expected by the synchronous API.
        unsafe { GetFileInformationByHandle(handle, &raw mut legacy) }.map_err(windows_error)?;

        let mut basic = FILE_BASIC_INFO::default();
        let basic_size =
            u32::try_from(std::mem::size_of::<FILE_BASIC_INFO>()).map_err(io::Error::other)?;
        // SAFETY: handle remains live; basic points to a correctly sized,
        // writable FILE_BASIC_INFO for the duration of the synchronous call.
        unsafe {
            GetFileInformationByHandleEx(handle, FileBasicInfo, (&raw mut basic).cast(), basic_size)
        }
        .map_err(windows_error)?;

        Ok(FileEvidence {
            volume: legacy.dwVolumeSerialNumber,
            file_index: (u64::from(legacy.nFileIndexHigh) << 32) | u64::from(legacy.nFileIndexLow),
            size: (u64::from(legacy.nFileSizeHigh) << 32) | u64::from(legacy.nFileSizeLow),
            creation_time: basic.CreationTime.cast_unsigned(),
            last_write_time: basic.LastWriteTime.cast_unsigned(),
            change_time: basic.ChangeTime.cast_unsigned(),
            attributes: basic.FileAttributes,
        })
    }

    fn windows_error(error: windows::core::Error) -> io::Error {
        io::Error::other(error)
    }
}

#[cfg(windows)]
fn validate_path_syntax(path: &Path) -> Result<(), DocumentEngineError> {
    use std::path::{Component, Prefix};

    let prefix = match path.components().next() {
        Some(Component::Prefix(prefix)) => Some(prefix.kind()),
        _ => None,
    };
    match prefix {
        Some(Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)) => {
            Err(DocumentEngineError::RemotePathUnsupported)
        }
        Some(Prefix::DeviceNS(_) | Prefix::Verbatim(_)) => {
            Err(DocumentEngineError::UnsupportedPathSyntax)
        }
        _ if path.to_string_lossy().starts_with("//") => {
            Err(DocumentEngineError::RemotePathUnsupported)
        }
        _ => Ok(()),
    }
}

#[cfg(not(windows))]
fn validate_path_syntax(path: &Path) -> Result<(), DocumentEngineError> {
    let path = path.to_string_lossy();
    if path.starts_with(r"\\") || path.starts_with("//") {
        Err(DocumentEngineError::RemotePathUnsupported)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn ensure_local_volume(path: &Path) -> Result<(), DocumentEngineError> {
    windows_volume::ensure_local(path)
}

#[cfg(not(windows))]
fn ensure_local_volume(_path: &Path) -> Result<(), DocumentEngineError> {
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_volume {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
    use windows::Win32::System::WindowsProgramming::{
        DRIVE_NO_ROOT_DIR, DRIVE_REMOTE, DRIVE_UNKNOWN,
    };
    use windows::core::PCWSTR;

    use super::{DocumentEngineError, MAX_WINDOWS_PATH_UNITS};

    pub(super) fn ensure_local(path: &Path) -> Result<(), DocumentEngineError> {
        let input_length = path.as_os_str().encode_wide().count();
        if input_length >= MAX_WINDOWS_PATH_UNITS {
            return Err(DocumentEngineError::PathTooLong);
        }
        let mut input = Vec::new();
        input
            .try_reserve_exact(input_length + 1)
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        input.extend(path.as_os_str().encode_wide());
        input.push(0);

        let mut root = Vec::new();
        root.try_reserve_exact(MAX_WINDOWS_PATH_UNITS)
            .map_err(|_| DocumentEngineError::AllocationUnavailable)?;
        root.resize(MAX_WINDOWS_PATH_UNITS, 0);
        // SAFETY: input is NUL-terminated, root is writable for its length, and
        // both buffers remain alive for the synchronous call.
        unsafe { GetVolumePathNameW(PCWSTR(input.as_ptr()), &mut root) }
            .map_err(|_| DocumentEngineError::VolumeQueryFailed)?;
        // SAFETY: success above writes a NUL-terminated root into root.
        let drive_type = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
        match drive_type {
            DRIVE_REMOTE => Err(DocumentEngineError::RemotePathUnsupported),
            DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => Err(DocumentEngineError::UnsupportedVolume),
            _ => Ok(()),
        }
    }
}

fn map_viewport_error(error: ViewportError) -> DocumentEngineError {
    match error {
        ViewportError::Cancelled => DocumentEngineError::Cancelled,
        other => DocumentEngineError::Viewport(other),
    }
}

/// A fail-closed document open, scan, projection, or cache-build failure.
#[derive(Debug)]
pub(crate) enum DocumentEngineError {
    EmptyPath,
    PathTooLong,
    RemotePathUnsupported,
    UnsupportedPathSyntax,
    UnsupportedVolume,
    VolumeQueryFailed,
    UnsupportedFormat,
    NotRegularFile,
    ReparsePointUnsupported,
    SourceChanged {
        change: SourceChange,
        expected_size: u64,
        actual_size: u64,
    },
    SourceIntegrityLost,
    Cancelled,
    AllocationUnavailable,
    ArithmeticOverflow,
    TooManyDisplayColumns,
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Budget(BudgetError),
    Scan(ScanSessionError),
    Viewport(ViewportError),
    Projection(FieldProjectionError),
}

impl From<BudgetError> for DocumentEngineError {
    fn from(error: BudgetError) -> Self {
        Self::Budget(error)
    }
}

impl From<ScanSessionError> for DocumentEngineError {
    fn from(error: ScanSessionError) -> Self {
        Self::Scan(error)
    }
}

impl From<FieldProjectionError> for DocumentEngineError {
    fn from(error: FieldProjectionError) -> Self {
        Self::Projection(error)
    }
}

impl fmt::Display for DocumentEngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("a file path is required"),
            Self::PathTooLong => formatter.write_str("the file path exceeds the supported limit"),
            Self::RemotePathUnsupported => {
                formatter.write_str("remote and UNC paths are not supported in LeanRows 0.1")
            }
            Self::UnsupportedPathSyntax => {
                formatter.write_str("the Windows device path syntax is not supported")
            }
            Self::UnsupportedVolume => formatter.write_str("the file volume is not locally usable"),
            Self::VolumeQueryFailed => {
                formatter.write_str("the file volume could not be validated as local")
            }
            Self::UnsupportedFormat => formatter
                .write_str("unsupported file type; use CSV, TSV, JSONL, NDJSON, LOG, or TXT"),
            Self::NotRegularFile => formatter.write_str("the selected path is not a regular file"),
            Self::ReparsePointUnsupported => {
                formatter.write_str("reparse-point and symbolic-link files are not supported")
            }
            Self::SourceChanged {
                change,
                expected_size,
                actual_size,
            } => write!(
                formatter,
                "the open document changed ({change:?}) from size {expected_size} to {actual_size}; reload it to continue"
            ),
            Self::SourceIntegrityLost => formatter.write_str(
                "the open document snapshot is no longer trustworthy; reload it to continue",
            ),
            Self::Cancelled => formatter.write_str("the document operation was cancelled"),
            Self::AllocationUnavailable => {
                formatter.write_str("bounded document cache allocation is unavailable")
            }
            Self::ArithmeticOverflow => formatter.write_str("document cache arithmetic overflowed"),
            Self::TooManyDisplayColumns => {
                formatter.write_str("document column metadata exceeded its fixed bound")
            }
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::Budget(error) => write!(formatter, "document memory plan failed: {error}"),
            Self::Scan(error) => write!(formatter, "document scan failed: {error}"),
            Self::Viewport(error) => write!(formatter, "document viewport failed: {error}"),
            Self::Projection(error) => write!(formatter, "row projection failed: {error}"),
        }
    }
}

impl std::error::Error for DocumentEngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Budget(error) => Some(error),
            Self::Scan(error) => Some(error),
            Self::Viewport(error) => Some(error),
            Self::Projection(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use leanrows_core::{CancellationToken, DocumentFormat};

    use super::{
        DocumentEngine, DocumentEngineError, DocumentScanStatus, MAX_PREVIEW_UTF16_UNITS,
        UiColumnKind, UiRowSnapshot, infer_format, validate_local_regular_path,
        validate_path_syntax,
    };

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempFixture {
        path: PathBuf,
        directory: bool,
    }

    impl TempFixture {
        fn file(extension: &str, bytes: &[u8]) -> io::Result<Self> {
            let path = unique_path(extension);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            Ok(Self {
                path,
                directory: false,
            })
        }

        fn directory(extension: &str) -> io::Result<Self> {
            let path = unique_path(extension);
            fs::create_dir(&path)?;
            Ok(Self {
                path,
                directory: true,
            })
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            if self.directory {
                let _ = fs::remove_dir(&self.path);
            } else {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    fn unique_path(extension: &str) -> PathBuf {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "leanrows-document-engine-{}-{sequence}.{extension}",
            std::process::id()
        ))
    }

    fn open(fixture: &TempFixture) -> Result<(DocumentEngine, UiRowSnapshot), DocumentEngineError> {
        let token = CancellationToken::new();
        DocumentEngine::open(fixture.path(), &token, token.snapshot())
            .map(super::OpenedDocument::into_parts)
    }

    fn cell_text(
        snapshot: &UiRowSnapshot,
        row: u64,
        subitem: usize,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let units = snapshot.cell(row, subitem).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "expected cached cell is missing")
        })?;
        let payload = units.strip_suffix(&[0]).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "cached cell is not NUL terminated",
            )
        })?;
        Ok(String::from_utf16(payload)?)
    }

    #[test]
    fn format_inference_is_explicit_and_case_insensitive() {
        let cases = [
            ("rows.csv", DocumentFormat::Csv),
            ("rows.CSV", DocumentFormat::Csv),
            ("rows.tsv", DocumentFormat::Tsv),
            ("rows.jsonl", DocumentFormat::Jsonl),
            ("rows.NDJSON", DocumentFormat::Jsonl),
            ("rows.log", DocumentFormat::Log),
            ("rows.TXT", DocumentFormat::Log),
        ];
        for (path, expected) in cases {
            assert_eq!(infer_format(Path::new(path)).ok(), Some(expected));
        }
        assert!(matches!(
            infer_format(Path::new("rows.parquet")),
            Err(DocumentEngineError::UnsupportedFormat)
        ));
        assert!(matches!(
            infer_format(Path::new("rows")),
            Err(DocumentEngineError::UnsupportedFormat)
        ));
    }

    #[test]
    fn quoted_csv_newlines_and_escaped_quotes_are_projected()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = TempFixture::file(
            "csv",
            b"name,notes\nalpha,\"line1\nline2\"\nbeta,\"say \"\"hi\"\"\"\n",
        )?;
        let (engine, first) = open(&fixture)?;
        assert_eq!(engine.format(), DocumentFormat::Csv);
        assert_eq!(engine.progress().scanned_bytes(), 0);
        assert_eq!(engine.progress().row_count(), 0);
        assert_eq!(first.len(), 3);
        assert!(first.reached_eof());
        assert_eq!(first.columns().kind(), UiColumnKind::Fields);
        assert_eq!(first.columns().data_columns(), 2);
        assert_eq!(cell_text(&first, 0, 0)?, "1");
        assert_eq!(cell_text(&first, 0, 1)?, "name");
        assert_eq!(cell_text(&first, 0, 2)?, "notes");
        assert_eq!(cell_text(&first, 1, 1)?, "alpha");
        assert_eq!(cell_text(&first, 1, 2)?, r"line1\nline2");
        assert_eq!(cell_text(&first, 2, 1)?, "beta");
        assert_eq!(cell_text(&first, 2, 2)?, "say \"hi\"");
        Ok(())
    }

    #[test]
    fn tsv_jsonl_ndjson_log_and_txt_use_expected_display_rules()
    -> Result<(), Box<dyn std::error::Error>> {
        let tsv = TempFixture::file("tsv", b"a\tb\n")?;
        let (engine, first) = open(&tsv)?;
        assert_eq!(engine.format(), DocumentFormat::Tsv);
        assert_eq!(first.columns().kind(), UiColumnKind::Fields);
        assert_eq!(first.columns().data_columns(), 2);
        assert_eq!(cell_text(&first, 0, 1)?, "a");
        assert_eq!(cell_text(&first, 0, 2)?, "b");

        let cases: [(&str, &[u8], DocumentFormat, &str); 4] = [
            ("jsonl", b"{\"a\":1}\n", DocumentFormat::Jsonl, "{\"a\":1}"),
            ("ndjson", b"{\"b\":2}\n", DocumentFormat::Jsonl, "{\"b\":2}"),
            ("log", b"INFO\tready\n", DocumentFormat::Log, r"INFO\tready"),
            ("txt", b"plain text\n", DocumentFormat::Log, "plain text"),
        ];
        for (extension, bytes, expected_format, expected_preview) in cases {
            let fixture = TempFixture::file(extension, bytes)?;
            let (engine, first) = open(&fixture)?;
            assert_eq!(engine.format(), expected_format);
            assert_eq!(first.columns().kind(), UiColumnKind::Preview);
            assert_eq!(first.columns().data_columns(), 1);
            assert_eq!(cell_text(&first, 0, 1)?, expected_preview);
        }
        Ok(())
    }

    #[test]
    fn csv_empty_trailing_quoted_escaped_and_multiline_fields_remain_independent()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = TempFixture::file(
            "csv",
            b",alpha,,\"a,b\",\"line1\nline2\",\"say \"\"hi\"\"\",\n",
        )?;
        let (_engine, first) = open(&fixture)?;
        assert_eq!(first.columns().kind(), UiColumnKind::Fields);
        assert_eq!(first.columns().data_columns(), 7);
        let expected = ["", "alpha", "", "a,b", r"line1\nline2", "say \"hi\"", ""];
        for (field, expected) in expected.into_iter().enumerate() {
            assert_eq!(cell_text(&first, 0, field.saturating_add(1))?, expected);
        }
        assert!(first.cell(0, 8).is_none());
        Ok(())
    }

    #[test]
    fn csv_field_count_is_capped_at_sixty_four_and_reports_source_truncation()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut line = (1..=65)
            .map(|field| format!("field-{field}"))
            .collect::<Vec<_>>()
            .join(",");
        line.push('\n');
        let fixture = TempFixture::file("csv", line.as_bytes())?;
        let (_engine, first) = open(&fixture)?;
        assert_eq!(first.columns().kind(), UiColumnKind::Fields);
        assert_eq!(first.columns().data_columns(), 64);
        assert_eq!(cell_text(&first, 0, 1)?, "field-1");
        assert_eq!(cell_text(&first, 0, 64)?, "field-64");
        assert!(first.cell(0, 65).is_none());
        let row = first
            .row(0)
            .ok_or_else(|| io::Error::other("bounded CSV row is missing"))?;
        assert!(row.source_truncated());
        Ok(())
    }

    #[test]
    fn invalid_utf8_is_rendered_as_exact_visible_byte_escapes()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = b"good\nbad".to_vec();
        bytes.push(0xff);
        bytes.extend_from_slice(b"tail\n");
        let fixture = TempFixture::file("log", &bytes)?;
        let (_engine, first) = open(&fixture)?;
        assert_eq!(cell_text(&first, 1, 1)?, r"bad\xFFtail");
        let row = first.row(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "invalid UTF-8 row was not cached")
        })?;
        assert_eq!(row.absolute_row(), 1);
        assert!(row.escaped_non_utf8());
        assert!(!row.source_truncated());
        assert!(!row.display_truncated());
        Ok(())
    }

    #[test]
    fn stale_cancellation_generations_stop_open_scan_and_viewport_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = TempFixture::file("log", b"one\ntwo\n")?;
        let stale_open_token = CancellationToken::new();
        let stale_open_generation = stale_open_token.snapshot();
        stale_open_token.cancel();
        assert!(matches!(
            DocumentEngine::open(fixture.path(), &stale_open_token, stale_open_generation),
            Err(DocumentEngineError::Cancelled)
        ));

        let token = CancellationToken::new();
        let opened = DocumentEngine::open(fixture.path(), &token, token.snapshot())?;
        let (mut engine, _first) = opened.into_parts();
        let stale = token.snapshot();
        token.cancel();
        let step = engine.scan_step(&token, stale)?;
        assert_eq!(step.status(), DocumentScanStatus::Cancelled);
        assert_eq!(step.processed_bytes(), 0);
        assert_eq!(step.discovered_rows(), 0);
        assert_eq!(step.progress().scanned_bytes(), 0);
        assert!(matches!(
            engine.snapshot(0, &token, stale),
            Err(DocumentEngineError::Cancelled)
        ));

        let current = token.snapshot();
        let resumed = engine.scan_step(&token, current)?;
        assert_eq!(resumed.status(), DocumentScanStatus::Complete);
        assert_eq!(resumed.progress().row_count(), 2);
        Ok(())
    }

    #[test]
    fn giant_record_preview_and_utf16_cache_stay_bounded() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut bytes = vec![b'x'; 2 * 1_024 * 1_024];
        bytes.push(b'\n');
        let fixture = TempFixture::file("log", &bytes)?;
        let (engine, first) = open(&fixture)?;
        assert_eq!(engine.progress().scanned_bytes(), 0);
        assert_eq!(first.len(), 1);
        assert!(first.reached_eof());
        let cell = first.cell(0, 1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "giant-record preview is missing")
        })?;
        assert!(cell.len() <= MAX_PREVIEW_UTF16_UNITS + 1);
        assert!(cell_text(&first, 0, 1)?.ends_with(" ..."));
        let row = first.row(0).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "giant-record row is missing")
        })?;
        assert!(row.source_truncated());
        assert!(row.display_truncated());
        Ok(())
    }

    #[test]
    fn cooperative_scan_steps_are_capped_and_reach_exact_row_count()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"row\n".repeat(20_000);
        let fixture = TempFixture::file("txt", &bytes)?;
        let token = CancellationToken::new();
        let opened = DocumentEngine::open(fixture.path(), &token, token.snapshot())?;
        let (mut engine, first) = opened.into_parts();
        assert_eq!(first.first_row(), 0);
        assert!(!first.is_empty());
        assert!(!first.reached_eof());
        assert_eq!(first.diagnostic(), None);

        let mut calls = 0_u32;
        let mut discovered = 0_u64;
        loop {
            let step = engine.scan_step(&token, token.snapshot())?;
            calls = calls.saturating_add(1);
            discovered = discovered.saturating_add(step.discovered_rows());
            assert!(step.processed_bytes() <= super::MAX_SCAN_STEP_BYTES);
            if step.status() == DocumentScanStatus::Complete {
                break;
            }
            assert_eq!(step.status(), DocumentScanStatus::Progress);
            assert!(calls < 100);
        }
        let progress = engine.progress();
        assert!(calls >= 2);
        assert_eq!(discovered, 20_000);
        assert_eq!(progress.row_count(), 20_000);
        assert_eq!(progress.scanned_bytes(), progress.source_bytes());
        assert!(progress.is_complete());
        assert_eq!(progress.diagnostic(), None);
        Ok(())
    }

    #[test]
    fn path_validation_fails_closed_without_touching_unc_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches!(
            validate_local_regular_path(Path::new("")),
            Err(DocumentEngineError::EmptyPath)
        ));
        assert!(matches!(
            validate_path_syntax(Path::new(r"\\server\share\rows.csv")),
            Err(DocumentEngineError::RemotePathUnsupported)
        ));
        assert!(matches!(
            validate_path_syntax(Path::new("//server/share/rows.csv")),
            Err(DocumentEngineError::RemotePathUnsupported)
        ));

        let directory = TempFixture::directory("csv")?;
        assert!(matches!(
            validate_local_regular_path(directory.path()),
            Err(DocumentEngineError::NotRegularFile)
        ));

        let missing = unique_path("csv");
        assert!(matches!(
            validate_local_regular_path(&missing),
            Err(DocumentEngineError::Io { .. })
        ));

        let unsupported = TempFixture::file("bin", b"data")?;
        let token = CancellationToken::new();
        assert!(matches!(
            DocumentEngine::open(unsupported.path(), &token, token.snapshot()),
            Err(DocumentEngineError::UnsupportedFormat)
        ));
        Ok(())
    }

    #[cfg(windows)]
    fn require_write_sharing_violation(
        result: io::Result<std::fs::File>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match result {
            Err(error) if error.raw_os_error() == Some(32) => Ok(()),
            Err(error) => Err(format!("expected Windows sharing violation, got {error}").into()),
            Ok(file) => {
                drop(file);
                Err("in-place writer unexpectedly opened the source".into())
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn retained_snapshot_denies_in_place_append_truncate_and_rewrite()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = TempFixture::file("log", b"original\nsecond\n")?;
        let (engine, first) = open(&fixture)?;
        assert_eq!(cell_text(&first, 0, 1)?, "original");

        require_write_sharing_violation(OpenOptions::new().append(true).open(fixture.path()))?;
        require_write_sharing_violation(
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(fixture.path()),
        )?;
        require_write_sharing_violation(OpenOptions::new().write(true).open(fixture.path()))?;

        let token = CancellationToken::new();
        let unchanged = engine.snapshot(0, &token, token.snapshot())?;
        assert_eq!(cell_text(&unchanged, 0, 1)?, "original");
        assert_eq!(cell_text(&unchanged, 1, 1)?, "second");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn atomic_replacement_is_detected_by_file_identity_and_poisoned()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = TempFixture::file("log", b"old-one\nold-two\n")?;
        let replacement = TempFixture::file("log", b"new-one\nnew-two\n")?;
        let (engine, first) = open(&original)?;
        assert_eq!(cell_text(&first, 0, 1)?, "old-one");

        // The read handle shares delete, so editors may use the common
        // write-temp + atomic-rename save strategy without corrupting the
        // retained snapshot.
        fs::remove_file(original.path())?;
        fs::rename(replacement.path(), original.path())?;

        let token = CancellationToken::new();
        match engine.snapshot(0, &token, token.snapshot()) {
            Err(DocumentEngineError::SourceChanged {
                change: leanrows_core::SourceChange::IdentityChanged,
                ..
            }) => {}
            Err(error) => return Err(format!("unexpected replacement error: {error}").into()),
            Ok(_) => return Err("atomic replacement was not detected".into()),
        }
        assert!(matches!(
            engine.snapshot(0, &token, token.snapshot()),
            Err(DocumentEngineError::SourceIntegrityLost)
        ));
        Ok(())
    }
}
