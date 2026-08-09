//! One bounded document worker with coalesced document, query, and event slots.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use leanrows_core::{
    CancellationGeneration, CancellationToken, LiteralMatchMode, QueryError, QuerySession,
    QuerySessionConfig, QueryStepStatus, recover_stale_query_files,
};

use crate::document_engine::{
    DocumentEngine, DocumentEngineError, DocumentProgress, DocumentQuerySource, DocumentScanStatus,
    UiColumnKind, UiColumnLayout, UiRowSnapshot,
};
use crate::{
    AccessibleName, CachedRow, CoalescingMailbox, DisplayCell, ImmutableRowCache, SendOutcome,
};

const PROGRESS_EVENT_BYTES: u64 = 4 * 1_024 * 1_024;
const QUERY_READ_BYTES: usize = 64 * 1_024;
const QUERY_CANCELLATION_CHECK_BYTES: usize = 16 * 1_024;
const QUERY_PAGE_HITS: usize = 256;
const QUERY_SCRATCH_QUOTA_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_QUERY_MESSAGE_BYTES: usize = 256;
const QUERY_CACHE_DIRECTORY: &str = "query-cache";
const QUERY_RECOVERY_LOCK: &str = ".recovery.lock";
const QUERY_ACTIVE_MARKER_PREFIX: &str = ".active-v1-";
const QUERY_ACTIVE_MARKER_SUFFIX: &str = ".lock";
const QUERY_MARKER_CREATE_ATTEMPTS: usize = 64;
const QUERY_STARTUP_LOCK_ATTEMPTS: usize = 1_000;
const QUERY_STARTUP_LOCK_RETRY: Duration = Duration::from_millis(5);
static NEXT_QUERY_ACTIVE_MARKER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct OpenCommand {
    path: PathBuf,
    serial: u64,
    generation: CancellationGeneration,
}

#[derive(Clone, Copy, Debug)]
struct ViewportCommand {
    serial: u64,
    first_row: u64,
    generation: CancellationGeneration,
}

#[derive(Debug)]
enum QueryCommand {
    Start {
        document_serial: u64,
        query_serial: u64,
        needle: Vec<u8>,
        mode: LiteralMatchMode,
        generation: CancellationGeneration,
    },
    Navigate {
        document_serial: u64,
        query_serial: u64,
        direction: QueryDirection,
        generation: CancellationGeneration,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueryDirection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerQueryPhase {
    Searching,
    MatchReady,
    Complete,
    QuotaExceeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkerQueryState {
    pub(crate) query_serial: u64,
    pub(crate) phase: WorkerQueryPhase,
    pub(crate) scanned_bytes: u64,
    pub(crate) source_bytes: u64,
    pub(crate) stored_hits: u64,
    pub(crate) active_hit_index: Option<u64>,
    pub(crate) active_row: Option<u64>,
    pub(crate) message: String,
}

impl WorkerQueryState {
    fn is_internally_consistent(&self) -> bool {
        let active_pair_matches = self.active_hit_index.is_some() == self.active_row.is_some();
        let active_hit_exists = self
            .active_hit_index
            .is_none_or(|index| index < self.stored_hits);
        let phase_matches = match self.phase {
            WorkerQueryPhase::MatchReady => self.active_hit_index.is_some(),
            WorkerQueryPhase::Searching
            | WorkerQueryPhase::Complete
            | WorkerQueryPhase::QuotaExceeded
            | WorkerQueryPhase::Failed => true,
        };
        self.query_serial != 0
            && self.scanned_bytes <= self.source_bytes
            && self.message.len() <= MAX_QUERY_MESSAGE_BYTES
            && active_pair_matches
            && active_hit_exists
            && phase_matches
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerProgress {
    pub(crate) scanned_bytes: u64,
    pub(crate) source_bytes: u64,
    pub(crate) indexed_rows: u64,
    pub(crate) available_rows: u64,
    pub(crate) complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerPhase {
    ViewportReady,
    Scanning,
    Complete,
    Failed,
}

#[derive(Debug)]
pub(crate) struct WorkerEvent {
    pub(crate) path: PathBuf,
    pub(crate) result: WorkerResult,
    pub(crate) serial: u64,
    pub(crate) cache: Arc<ImmutableRowCache>,
    pub(crate) cache_first_row: u64,
    pub(crate) cached_rows: u32,
    pub(crate) columns: Option<UiColumnLayout>,
    pub(crate) progress: WorkerProgress,
    pub(crate) phase: WorkerPhase,
    pub(crate) source_truncated_rows: u32,
    pub(crate) display_truncated_rows: u32,
    pub(crate) escaped_non_utf8_rows: u32,
    pub(crate) query: Option<WorkerQueryState>,
}

impl WorkerEvent {
    fn is_internally_consistent(&self) -> bool {
        let counts_fit = self.source_truncated_rows <= self.cached_rows
            && self.display_truncated_rows <= self.cached_rows
            && self.escaped_non_utf8_rows <= self.cached_rows;
        let cache_anchor_exists =
            self.cached_rows == 0 || self.cache.cell(self.cache_first_row, 0).is_some();
        let columns_match = match self.columns {
            Some(columns) => {
                let retained_cells = self
                    .cache
                    .cell_count(self.cache_first_row)
                    .unwrap_or(1)
                    .saturating_sub(1);
                columns.is_valid() && retained_cells <= usize::from(columns.data_columns())
            }
            None => self.cached_rows == 0,
        };
        let progress_is_ordered = self.progress.scanned_bytes <= self.progress.source_bytes
            && self.progress.indexed_rows <= self.progress.available_rows;
        let phase_matches = match self.phase {
            WorkerPhase::Complete => {
                self.progress.complete && matches!(&self.result, WorkerResult::Ready { .. })
            }
            WorkerPhase::ViewportReady | WorkerPhase::Scanning => {
                matches!(&self.result, WorkerResult::Ready { .. })
            }
            WorkerPhase::Failed => matches!(&self.result, WorkerResult::Failed { .. }),
        };
        let query_is_valid = self
            .query
            .as_ref()
            .is_none_or(WorkerQueryState::is_internally_consistent);
        counts_fit
            && cache_anchor_exists
            && columns_match
            && progress_is_ordered
            && phase_matches
            && query_is_valid
    }
}

#[derive(Debug)]
pub(crate) enum WorkerResult {
    Ready { size: u64 },
    Failed { message: String },
}

pub(crate) struct Worker {
    opens: Arc<CoalescingMailbox<OpenCommand>>,
    viewports: Arc<CoalescingMailbox<ViewportCommand>>,
    queries: Arc<CoalescingMailbox<QueryCommand>>,
    signals: Arc<CoalescingMailbox<()>>,
    events: Arc<CoalescingMailbox<WorkerEvent>>,
    cancellation: Arc<CancellationToken>,
    query_cancellation: Arc<CancellationToken>,
    latest_serial: Arc<AtomicU64>,
    active_serial: Arc<AtomicU64>,
    latest_query_serial: Arc<AtomicU64>,
    query_sequence: AtomicU64,
    query_active_marker: Option<QueryActiveMarker>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(crate) fn start(wake_ui: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Self> {
        let scratch = prepare_default_query_scratch()?;
        Self::start_with_query_scratch(wake_ui, scratch, QUERY_SCRATCH_QUOTA_BYTES)
    }

    #[cfg(test)]
    fn start_for_query_test(
        wake_ui: Arc<dyn Fn() + Send + Sync>,
        scratch_directory: &Path,
        scratch_quota_bytes: u64,
    ) -> io::Result<Self> {
        let scratch = prepare_query_scratch(scratch_directory.to_path_buf())?;
        Self::start_with_query_scratch(wake_ui, scratch, scratch_quota_bytes)
    }

    fn start_with_query_scratch(
        wake_ui: Arc<dyn Fn() + Send + Sync>,
        scratch: QueryScratchSetup,
        query_scratch_quota_bytes: u64,
    ) -> io::Result<Self> {
        let QueryScratchSetup {
            directory,
            active_marker,
        } = scratch;
        let opens = Arc::new(CoalescingMailbox::<OpenCommand>::new());
        let viewports = Arc::new(CoalescingMailbox::<ViewportCommand>::new());
        let queries = Arc::new(CoalescingMailbox::<QueryCommand>::new());
        let signals = Arc::new(CoalescingMailbox::<()>::new());
        let events = Arc::new(CoalescingMailbox::<WorkerEvent>::new());
        let cancellation = Arc::new(CancellationToken::new());
        let query_cancellation = Arc::new(CancellationToken::new());
        let latest_serial = Arc::new(AtomicU64::new(0));
        let active_serial = Arc::new(AtomicU64::new(0));
        let latest_query_serial = Arc::new(AtomicU64::new(0));
        let runtime = WorkerRuntime {
            opens: Arc::clone(&opens),
            viewports: Arc::clone(&viewports),
            queries: Arc::clone(&queries),
            signals: Arc::clone(&signals),
            events: Arc::clone(&events),
            cancellation: Arc::clone(&cancellation),
            query_cancellation: Arc::clone(&query_cancellation),
            latest_serial: Arc::clone(&latest_serial),
            active_serial: Arc::clone(&active_serial),
            latest_query_serial: Arc::clone(&latest_query_serial),
            query_scratch_directory: directory,
            query_scratch_quota_bytes,
            wake_ui,
        };
        let thread = thread::Builder::new()
            .name(String::from("leanrows-document"))
            .spawn(move || run_worker(&runtime))?;
        Ok(Self {
            opens,
            viewports,
            queries,
            signals,
            events,
            cancellation,
            query_cancellation,
            latest_serial,
            active_serial,
            latest_query_serial,
            query_sequence: AtomicU64::new(0),
            query_active_marker: Some(active_marker),
            thread: Some(thread),
        })
    }

    /// Cancels the current generation and coalesces this path over older opens.
    pub(crate) fn submit_path(&self, path: PathBuf) -> u64 {
        let previous = self.latest_serial.fetch_add(1, Ordering::AcqRel);
        let serial = previous.wrapping_add(1);
        let generation = self.cancellation.cancel();
        self.active_serial.store(0, Ordering::Release);
        self.query_cancellation.cancel();
        self.latest_query_serial.store(0, Ordering::Release);
        let _ = self.viewports.try_take();
        let _ = self.queries.try_take();
        let _ = self.events.try_take();
        if self
            .opens
            .send(OpenCommand {
                path,
                serial,
                generation,
            })
            .is_ok()
        {
            let _ = self.signals.send(());
        }
        serial
    }

    /// Coalesces an absolute viewport request for the current document.
    ///
    /// Returns false without cancellation or I/O when the serial is stale.
    #[allow(dead_code)]
    pub(crate) fn request_viewport(&self, serial: u64, first_row: u64) -> bool {
        if self.latest_serial.load(Ordering::Acquire) != serial {
            return false;
        }
        let generation = self.cancellation.cancel();
        if self
            .viewports
            .send(ViewportCommand {
                serial,
                first_row,
                generation,
            })
            .is_err()
        {
            return false;
        }
        let _ = self.signals.send(());
        true
    }

    /// Starts a bounded literal query for the current document snapshot.
    ///
    /// The returned nonzero serial identifies the query. A stale document
    /// serial is rejected without allocating, cancelling, or touching disk.
    pub(crate) fn start_query(
        &self,
        document_serial: u64,
        needle: Vec<u8>,
        case_sensitive: bool,
    ) -> Option<u64> {
        if self.latest_serial.load(Ordering::Acquire) != document_serial
            || self.active_serial.load(Ordering::Acquire) != document_serial
        {
            return None;
        }
        let query_serial = next_nonzero_serial(&self.query_sequence);
        let generation = self.query_cancellation.cancel();
        self.latest_query_serial
            .store(query_serial, Ordering::Release);
        let _ = self.events.try_take();
        let command = QueryCommand::Start {
            document_serial,
            query_serial,
            needle,
            mode: if case_sensitive {
                LiteralMatchMode::Exact
            } else {
                LiteralMatchMode::AsciiCaseInsensitive
            },
            generation,
        };
        if self.queries.send(command).is_err() {
            self.latest_query_serial.store(0, Ordering::Release);
            return None;
        }
        let _ = self.signals.send(());
        Some(query_serial)
    }

    /// Requests one demand-driven query navigation operation.
    ///
    /// Stale document and query serials are rejected before cancellation.
    pub(crate) fn navigate_query(
        &self,
        document_serial: u64,
        query_serial: u64,
        direction: QueryDirection,
    ) -> bool {
        if self.latest_serial.load(Ordering::Acquire) != document_serial
            || self.latest_query_serial.load(Ordering::Acquire) != query_serial
        {
            return false;
        }
        let generation = self.query_cancellation.cancel();
        let _ = self.events.try_take();
        if self
            .queries
            .send(QueryCommand::Navigate {
                document_serial,
                query_serial,
                direction,
                generation,
            })
            .is_err()
        {
            return false;
        }
        let _ = self.signals.send(());
        true
    }

    pub(crate) fn poll(&self) -> Option<WorkerEvent> {
        loop {
            let event = self.events.try_take()?;
            let latest_query = self.latest_query_serial.load(Ordering::Acquire);
            let query_is_current = match (latest_query, event.query.as_ref()) {
                (0, None) => true,
                (serial, Some(query)) => query.query_serial == serial,
                _ => false,
            };
            if event.serial == self.latest_serial.load(Ordering::Acquire) && query_is_current {
                debug_assert!(event.is_internally_consistent());
                return Some(event);
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.query_cancellation.cancel();
        self.opens.close();
        self.viewports.close();
        self.queries.close();
        self.signals.close();
        self.events.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = self.query_active_marker.take();
    }
}

struct ActiveDocument {
    path: PathBuf,
    serial: u64,
    engine: DocumentEngine,
    cache: CacheState,
    scan_complete: bool,
    unreported_bytes: u64,
    query: Option<ActiveQuery>,
}

struct ActiveQuery {
    serial: u64,
    generation: CancellationGeneration,
    session: Option<QuerySession<DocumentQuerySource>>,
    demand_hit: Option<u64>,
    unreported_bytes: u64,
    state: WorkerQueryState,
}

struct CacheState {
    cache: Arc<ImmutableRowCache>,
    first_row: u64,
    row_count: u32,
    columns: UiColumnLayout,
    source_truncated_rows: u32,
    display_truncated_rows: u32,
    escaped_non_utf8_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheConversionError {
    AllocationUnavailable,
    InvalidUtf16Cell,
    InconsistentColumns,
    TooManyRows,
}

impl fmt::Display for CacheConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded UTF-16 row cache conversion failed")
    }
}

impl std::error::Error for CacheConversionError {}

struct WorkerRuntime {
    opens: Arc<CoalescingMailbox<OpenCommand>>,
    viewports: Arc<CoalescingMailbox<ViewportCommand>>,
    queries: Arc<CoalescingMailbox<QueryCommand>>,
    signals: Arc<CoalescingMailbox<()>>,
    events: Arc<CoalescingMailbox<WorkerEvent>>,
    cancellation: Arc<CancellationToken>,
    query_cancellation: Arc<CancellationToken>,
    latest_serial: Arc<AtomicU64>,
    active_serial: Arc<AtomicU64>,
    latest_query_serial: Arc<AtomicU64>,
    query_scratch_directory: PathBuf,
    query_scratch_quota_bytes: u64,
    wake_ui: Arc<dyn Fn() + Send + Sync>,
}

fn run_worker(runtime: &WorkerRuntime) {
    let mut active = None;
    while runtime.signals.wait_take().is_some() {
        loop {
            let _ = runtime.signals.try_take();
            if let Some(command) = runtime.opens.try_take() {
                if !replace_document(runtime, &mut active, command) {
                    return;
                }
                continue;
            }
            if let Some(request) = runtime.viewports.try_take() {
                if !apply_viewport_request(runtime, active.as_mut(), request) {
                    return;
                }
                continue;
            }
            if let Some(command) = runtime.queries.try_take() {
                if !apply_query_command(runtime, active.as_mut(), command) {
                    return;
                }
                continue;
            }
            let Some(document) = active.as_mut() else {
                break;
            };
            if document
                .query
                .as_ref()
                .is_some_and(|query| query.demand_hit.is_some())
            {
                if !advance_query_once(runtime, document) {
                    return;
                }
                continue;
            }
            if document.scan_complete {
                break;
            }
            if !scan_once(runtime, document) {
                return;
            }
        }
    }
}

fn replace_document(
    runtime: &WorkerRuntime,
    active: &mut Option<ActiveDocument>,
    command: OpenCommand,
) -> bool {
    *active = None;
    let opened =
        match DocumentEngine::open(&command.path, &runtime.cancellation, command.generation) {
            Ok(opened) => opened,
            Err(DocumentEngineError::Cancelled) => return true,
            Err(error) => {
                return emit_failed_open(runtime, command.path, command.serial, error.to_string());
            }
        };
    let (engine, first_snapshot) = opened.into_parts();
    let cache = match convert_cache(first_snapshot) {
        Ok(cache) => cache,
        Err(error) => {
            return emit_failed_open(runtime, command.path, command.serial, error.to_string());
        }
    };
    let mut document = ActiveDocument {
        path: command.path,
        serial: command.serial,
        engine,
        cache,
        scan_complete: false,
        unreported_bytes: 0,
        query: None,
    };
    document.scan_complete = document.engine.progress().is_complete();
    runtime
        .active_serial
        .store(document.serial, Ordering::Release);
    let sent = emit_active(runtime, &document, WorkerPhase::ViewportReady, None);
    *active = Some(document);
    sent
}

fn apply_viewport_request(
    runtime: &WorkerRuntime,
    active: Option<&mut ActiveDocument>,
    request: ViewportCommand,
) -> bool {
    let Some(document) = active else {
        return true;
    };
    if document.serial != request.serial
        || runtime.latest_serial.load(Ordering::Acquire) != request.serial
    {
        return true;
    }
    let snapshot =
        match document
            .engine
            .snapshot(request.first_row, &runtime.cancellation, request.generation)
        {
            Ok(snapshot) => snapshot,
            Err(DocumentEngineError::Cancelled) => return true,
            Err(error) => {
                invalidate_query(runtime, document);
                return emit_active(
                    runtime,
                    document,
                    WorkerPhase::Failed,
                    Some(error.to_string()),
                );
            }
        };
    document.cache = match convert_cache(snapshot) {
        Ok(mut cache) => {
            let Some(columns) = document.cache.columns.grow(cache.columns) else {
                invalidate_query(runtime, document);
                return emit_active(
                    runtime,
                    document,
                    WorkerPhase::Failed,
                    Some(CacheConversionError::InconsistentColumns.to_string()),
                );
            };
            cache.columns = columns;
            cache
        }
        Err(error) => {
            invalidate_query(runtime, document);
            return emit_active(
                runtime,
                document,
                WorkerPhase::Failed,
                Some(error.to_string()),
            );
        }
    };
    emit_active(runtime, document, WorkerPhase::ViewportReady, None)
}

fn apply_query_command(
    runtime: &WorkerRuntime,
    active: Option<&mut ActiveDocument>,
    command: QueryCommand,
) -> bool {
    let Some(document) = active else {
        return true;
    };
    match command {
        QueryCommand::Start {
            document_serial,
            query_serial,
            needle,
            mode,
            generation,
        } => start_document_query(
            runtime,
            document,
            document_serial,
            query_serial,
            &needle,
            mode,
            generation,
        ),
        QueryCommand::Navigate {
            document_serial,
            query_serial,
            direction,
            generation,
        } => navigate_document_query(
            runtime,
            document,
            document_serial,
            query_serial,
            direction,
            generation,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn start_document_query(
    runtime: &WorkerRuntime,
    document: &mut ActiveDocument,
    document_serial: u64,
    query_serial: u64,
    needle: &[u8],
    mode: LiteralMatchMode,
    generation: CancellationGeneration,
) -> bool {
    if document.serial != document_serial
        || runtime.latest_serial.load(Ordering::Acquire) != document_serial
        || runtime.latest_query_serial.load(Ordering::Acquire) != query_serial
    {
        return true;
    }

    document.query = None;
    let session: Result<QuerySession<DocumentQuerySource>, String> = (|| {
        let plan = document
            .engine
            .query_plan()
            .map_err(|error| error.to_string())?;
        let (source, format) = plan.into_parts();
        let config = QuerySessionConfig::new(
            format,
            QUERY_READ_BYTES,
            QUERY_CANCELLATION_CHECK_BYTES,
            QUERY_PAGE_HITS,
            runtime.query_scratch_quota_bytes,
        )
        .map_err(|error| error.to_string())?;
        QuerySession::new(
            source,
            needle,
            mode,
            config,
            &runtime.query_scratch_directory,
        )
        .map_err(|error| error.to_string())
    })();

    document.query = Some(match session {
        Ok(session) => {
            let progress = session.progress();
            ActiveQuery {
                serial: query_serial,
                generation,
                session: Some(session),
                demand_hit: Some(0),
                unreported_bytes: 0,
                state: WorkerQueryState {
                    query_serial,
                    phase: WorkerQueryPhase::Searching,
                    scanned_bytes: progress.scanned_bytes(),
                    source_bytes: progress.source_bytes(),
                    stored_hits: progress.stored_hits(),
                    active_hit_index: None,
                    active_row: None,
                    message: bounded_query_message(String::from("Searching this document...")),
                },
            }
        }
        Err(error) => failed_query(
            query_serial,
            generation,
            document.engine.progress().source_bytes(),
            format!("Search could not start: {error}"),
        ),
    });
    emit_active(runtime, document, current_document_phase(document), None)
}

fn navigate_document_query(
    runtime: &WorkerRuntime,
    document: &mut ActiveDocument,
    document_serial: u64,
    query_serial: u64,
    direction: QueryDirection,
    generation: CancellationGeneration,
) -> bool {
    if document.serial != document_serial
        || runtime.latest_serial.load(Ordering::Acquire) != document_serial
        || runtime.latest_query_serial.load(Ordering::Acquire) != query_serial
    {
        return true;
    }
    let Some(query) = document
        .query
        .as_mut()
        .filter(|query| query.serial == query_serial)
    else {
        return true;
    };
    query.generation = generation;
    query.demand_hit = None;
    query.unreported_bytes = 0;
    let result = match direction {
        QueryDirection::Previous => navigate_to_previous_hit(query),
        QueryDirection::Next => navigate_to_next_hit(query),
    };
    if let Err(error) = result {
        fail_query(query, format!("Search navigation failed: {error}"));
    }
    emit_active(runtime, document, current_document_phase(document), None)
}

fn navigate_to_previous_hit(query: &mut ActiveQuery) -> Result<(), QueryError> {
    if query.session.is_none() {
        return Ok(());
    }
    update_query_progress(query);
    let previous = query
        .session
        .as_ref()
        .and_then(|session| session.previous_hit_index(query.state.active_hit_index));
    if let Some(previous) = previous {
        activate_query_hit(query, previous)?;
    } else if query.state.active_hit_index.is_some() {
        query.state.phase = WorkerQueryPhase::MatchReady;
        query.state.message = bounded_query_message(String::from("Already at the first match."));
    } else {
        query.state.phase = WorkerQueryPhase::Complete;
        query.state.message = bounded_query_message(String::from("No previous match is stored."));
    }
    Ok(())
}

fn navigate_to_next_hit(query: &mut ActiveQuery) -> Result<(), QueryError> {
    if query.session.is_none() {
        return Ok(());
    }
    update_query_progress(query);
    let target = query
        .state
        .active_hit_index
        .and_then(|index| index.checked_add(1))
        .unwrap_or(0);
    let (hit_count, status) = query
        .session
        .as_ref()
        .map(|session| (session.hit_count(), session.status()))
        .ok_or(QueryError::ScratchCorrupt)?;
    if target < hit_count {
        return activate_query_hit(query, target);
    }
    match status {
        QueryStepStatus::Running | QueryStepStatus::Cancelled => {
            query.demand_hit = Some(target);
            query.state.phase = WorkerQueryPhase::Searching;
            query.state.message =
                bounded_query_message(String::from("Searching for the next match..."));
        }
        QueryStepStatus::Complete => set_query_complete(query),
        QueryStepStatus::QuotaExceeded(details) => set_query_quota_exceeded(query, details),
    }
    Ok(())
}

fn advance_query_once(runtime: &WorkerRuntime, document: &mut ActiveDocument) -> bool {
    let should_emit = {
        let Some(query) = document.query.as_mut() else {
            return true;
        };
        let Some(target) = query.demand_hit else {
            return true;
        };
        let before = query
            .session
            .as_ref()
            .map_or(0, |session| session.progress().scanned_bytes());
        let outcome = match query.session.as_mut() {
            Some(session) => session.advance(
                QUERY_READ_BYTES,
                &runtime.query_cancellation,
                query.generation,
            ),
            None => return true,
        };
        let source_validation = document.engine.validate_query_snapshot();
        match (outcome, source_validation) {
            (_, Err(error)) => {
                runtime.active_serial.store(0, Ordering::Release);
                fail_query(
                    query,
                    format!("Search stopped because the source changed: {error}"),
                );
                true
            }
            (Ok(status), Ok(())) => {
                update_query_progress(query);
                query.unreported_bytes = query
                    .unreported_bytes
                    .saturating_add(query.state.scanned_bytes.saturating_sub(before));
                if query.state.stored_hits > target {
                    match activate_query_hit(query, target) {
                        Ok(()) => true,
                        Err(error) => {
                            fail_query(query, format!("Search result could not be read: {error}"));
                            true
                        }
                    }
                } else {
                    match status {
                        QueryStepStatus::Running => {
                            if query.unreported_bytes >= PROGRESS_EVENT_BYTES {
                                query.unreported_bytes = 0;
                                true
                            } else {
                                false
                            }
                        }
                        QueryStepStatus::Cancelled => false,
                        QueryStepStatus::Complete => {
                            set_query_complete(query);
                            true
                        }
                        QueryStepStatus::QuotaExceeded(details) => {
                            set_query_quota_exceeded(query, details);
                            true
                        }
                    }
                }
            }
            (Err(error), Ok(())) => {
                fail_query(
                    query,
                    format!("Search failed: {error}. Reopen the file and try again."),
                );
                true
            }
        }
    };
    !should_emit || emit_active(runtime, document, current_document_phase(document), None)
}

fn activate_query_hit(query: &mut ActiveQuery, index: u64) -> Result<(), QueryError> {
    let hit = query
        .session
        .as_mut()
        .ok_or(QueryError::ScratchCorrupt)?
        .read_hit(index)?
        .ok_or(QueryError::ScratchCorrupt)?;
    update_query_progress(query);
    query.demand_hit = None;
    query.unreported_bytes = 0;
    query.state.phase = WorkerQueryPhase::MatchReady;
    query.state.active_hit_index = Some(index);
    query.state.active_row = Some(hit.row());
    query.state.message = bounded_query_message(format!(
        "Match {} ({} stored).",
        index.saturating_add(1),
        query.state.stored_hits
    ));
    Ok(())
}

fn update_query_progress(query: &mut ActiveQuery) {
    let Some(session) = query.session.as_ref() else {
        return;
    };
    let progress = session.progress();
    query.state.scanned_bytes = progress.scanned_bytes();
    query.state.source_bytes = progress.source_bytes();
    query.state.stored_hits = progress.stored_hits();
}

fn set_query_complete(query: &mut ActiveQuery) {
    query.demand_hit = None;
    query.state.phase = WorkerQueryPhase::Complete;
    query.state.message = bounded_query_message(if query.state.stored_hits == 0 {
        String::from("No matches found.")
    } else {
        String::from("No more matches. Use Previous to revisit stored results.")
    });
}

fn set_query_quota_exceeded(query: &mut ActiveQuery, details: leanrows_core::QueryQuotaExceeded) {
    query.demand_hit = None;
    query.state.phase = WorkerQueryPhase::QuotaExceeded;
    query.state.message = bounded_query_message(format!(
        "Search reached its {}-byte hit-cache limit. Use a more specific search.",
        details.quota_bytes()
    ));
}

fn fail_query(query: &mut ActiveQuery, message: String) {
    query.demand_hit = None;
    query.session = None;
    query.state.phase = WorkerQueryPhase::Failed;
    query.state.message = bounded_query_message(message);
}

fn failed_query(
    query_serial: u64,
    generation: CancellationGeneration,
    source_bytes: u64,
    message: String,
) -> ActiveQuery {
    ActiveQuery {
        serial: query_serial,
        generation,
        session: None,
        demand_hit: None,
        unreported_bytes: 0,
        state: WorkerQueryState {
            query_serial,
            phase: WorkerQueryPhase::Failed,
            scanned_bytes: 0,
            source_bytes,
            stored_hits: 0,
            active_hit_index: None,
            active_row: None,
            message: bounded_query_message(message),
        },
    }
}

fn invalidate_query(runtime: &WorkerRuntime, document: &mut ActiveDocument) {
    runtime.active_serial.store(0, Ordering::Release);
    runtime.query_cancellation.cancel();
    runtime.latest_query_serial.store(0, Ordering::Release);
    let _ = runtime.queries.try_take();
    document.query = None;
}

fn current_document_phase(document: &ActiveDocument) -> WorkerPhase {
    if document.scan_complete {
        WorkerPhase::Complete
    } else {
        WorkerPhase::Scanning
    }
}

fn bounded_query_message(mut message: String) -> String {
    if message.len() <= MAX_QUERY_MESSAGE_BYTES {
        return message;
    }
    let mut boundary = MAX_QUERY_MESSAGE_BYTES;
    while !message.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    message.truncate(boundary);
    message
}

fn scan_once(runtime: &WorkerRuntime, document: &mut ActiveDocument) -> bool {
    let generation = runtime.cancellation.snapshot();
    let step = match document.engine.scan_step(&runtime.cancellation, generation) {
        Ok(step) => step,
        Err(DocumentEngineError::Cancelled) => return true,
        Err(error) => {
            document.scan_complete = true;
            invalidate_query(runtime, document);
            return emit_active(
                runtime,
                document,
                WorkerPhase::Failed,
                Some(error.to_string()),
            );
        }
    };
    document.unreported_bytes = document
        .unreported_bytes
        .saturating_add(u64::try_from(step.processed_bytes()).unwrap_or(u64::MAX));
    match step.status() {
        DocumentScanStatus::Complete => {
            document.scan_complete = true;
            document.unreported_bytes = 0;
            emit_active(runtime, document, WorkerPhase::Complete, None)
        }
        DocumentScanStatus::Progress if document.unreported_bytes >= PROGRESS_EVENT_BYTES => {
            document.unreported_bytes = 0;
            emit_active(runtime, document, WorkerPhase::Scanning, None)
        }
        DocumentScanStatus::Cancelled | DocumentScanStatus::Progress => true,
    }
}

fn emit_failed_open(runtime: &WorkerRuntime, path: PathBuf, serial: u64, message: String) -> bool {
    runtime.active_serial.store(0, Ordering::Release);
    runtime.query_cancellation.cancel();
    runtime.latest_query_serial.store(0, Ordering::Release);
    let _ = runtime.queries.try_take();
    let event = WorkerEvent {
        path,
        result: WorkerResult::Failed { message },
        serial,
        cache: Arc::new(ImmutableRowCache::default()),
        cache_first_row: 0,
        cached_rows: 0,
        columns: None,
        progress: WorkerProgress::default(),
        phase: WorkerPhase::Failed,
        source_truncated_rows: 0,
        display_truncated_rows: 0,
        escaped_non_utf8_rows: 0,
        query: None,
    };
    emit_event(runtime, event)
}

fn emit_active(
    runtime: &WorkerRuntime,
    document: &ActiveDocument,
    phase: WorkerPhase,
    failure: Option<String>,
) -> bool {
    let progress = worker_progress(document.engine.progress(), &document.cache);
    let result = failure.map_or(
        WorkerResult::Ready {
            size: progress.source_bytes,
        },
        |message| WorkerResult::Failed { message },
    );
    let event = WorkerEvent {
        path: document.path.clone(),
        result,
        serial: document.serial,
        cache: Arc::clone(&document.cache.cache),
        cache_first_row: document.cache.first_row,
        cached_rows: document.cache.row_count,
        columns: Some(document.cache.columns),
        progress,
        phase,
        source_truncated_rows: document.cache.source_truncated_rows,
        display_truncated_rows: document.cache.display_truncated_rows,
        escaped_non_utf8_rows: document.cache.escaped_non_utf8_rows,
        query: document.query.as_ref().map(|query| query.state.clone()),
    };
    emit_event(runtime, event)
}

fn emit_event(runtime: &WorkerRuntime, event: WorkerEvent) -> bool {
    if runtime.latest_serial.load(Ordering::Acquire) != event.serial {
        return true;
    }
    match runtime.events.send(event) {
        Ok(SendOutcome::Accepted) => {
            (runtime.wake_ui)();
            true
        }
        Ok(SendOutcome::Replaced) => true,
        Err(_) => false,
    }
}

fn worker_progress(progress: DocumentProgress, cache: &CacheState) -> WorkerProgress {
    let cached_end = cache.first_row.saturating_add(u64::from(cache.row_count));
    WorkerProgress {
        scanned_bytes: progress.scanned_bytes(),
        source_bytes: progress.source_bytes(),
        indexed_rows: progress.row_count(),
        available_rows: progress.row_count().max(cached_end),
        complete: progress.is_complete(),
    }
}

fn convert_cache(snapshot: UiRowSnapshot) -> Result<CacheState, CacheConversionError> {
    let first_row = snapshot.first_row();
    let row_count = u32::try_from(snapshot.len()).map_err(|_| CacheConversionError::TooManyRows)?;
    let columns = snapshot.columns();
    let encoded_rows = snapshot.into_rows();
    let mut rows = Vec::new();
    rows.try_reserve_exact(encoded_rows.len())
        .map_err(|_| CacheConversionError::AllocationUnavailable)?;
    let mut source_truncated_rows = 0_u32;
    let mut display_truncated_rows = 0_u32;
    let mut escaped_non_utf8_rows = 0_u32;
    for row in encoded_rows {
        let parts = row.into_parts();
        source_truncated_rows =
            source_truncated_rows.saturating_add(u32::from(parts.source_truncated));
        display_truncated_rows =
            display_truncated_rows.saturating_add(u32::from(parts.display_truncated));
        escaped_non_utf8_rows =
            escaped_non_utf8_rows.saturating_add(u32::from(parts.escaped_non_utf8));
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(parts.cells.len())
            .map_err(|_| CacheConversionError::AllocationUnavailable)?;
        for cell in parts.cells {
            cells.push(
                DisplayCell::from_nul_terminated_utf16(cell.into_units())
                    .ok_or(CacheConversionError::InvalidUtf16Cell)?,
            );
        }
        if cells.len().saturating_sub(1) > usize::from(columns.data_columns()) {
            return Err(CacheConversionError::InconsistentColumns);
        }
        let accessible_name = match columns.kind() {
            UiColumnKind::Preview => AccessibleName::from_preview_utf16(
                parts.absolute_row,
                cells
                    .get(1)
                    .ok_or(CacheConversionError::InconsistentColumns)?
                    .as_utf16(),
            ),
            UiColumnKind::Fields => AccessibleName::from_utf16_cells(
                parts.absolute_row,
                cells.iter().skip(1).map(DisplayCell::as_utf16),
            ),
        };
        rows.push(CachedRow::new_with_accessible_name(
            parts.absolute_row,
            cells,
            accessible_name,
        ));
    }
    Ok(CacheState {
        cache: Arc::new(ImmutableRowCache::new(rows)),
        first_row,
        row_count,
        columns,
        source_truncated_rows,
        display_truncated_rows,
        escaped_non_utf8_rows,
    })
}

struct QueryScratchSetup {
    directory: PathBuf,
    active_marker: QueryActiveMarker,
}

struct QueryActiveMarker {
    handle: Option<File>,
    path: PathBuf,
    directory: PathBuf,
}

impl QueryActiveMarker {
    fn create(directory: &Path) -> io::Result<Self> {
        for _ in 0..QUERY_MARKER_CREATE_ATTEMPTS {
            let path = directory.join(fresh_query_active_marker_name());
            debug_assert_eq!(path.parent(), Some(directory));
            match create_active_marker_file(&path) {
                Ok(handle) => {
                    let metadata = fs::symlink_metadata(&path)?;
                    if !metadata.is_file() || worker_metadata_is_reparse(&metadata) {
                        drop(handle);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "new LeanRows query active marker is not a plain file",
                        ));
                    }
                    return Ok(Self {
                        handle: Some(handle),
                        path,
                        directory: directory.to_path_buf(),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a unique LeanRows query active marker",
        ))
    }
}

impl Drop for QueryActiveMarker {
    fn drop(&mut self) {
        // Closing the share-denied handle first makes the marker classifiable
        // as stale. Cleanup remains confined to this exact, generated child.
        drop(self.handle.take());
        let _ = remove_owned_active_marker(&self.directory, &self.path);
    }
}

fn prepare_default_query_scratch() -> io::Result<QueryScratchSetup> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is required for the LeanRows query cache",
        )
    })?;
    #[cfg(not(windows))]
    let base = std::env::temp_dir().into_os_string();
    let application = PathBuf::from(base).join("LeanRows");
    ensure_plain_directory(&application)?;
    prepare_query_scratch(application.join(QUERY_CACHE_DIRECTORY))
}

fn prepare_query_scratch(directory: PathBuf) -> io::Result<QueryScratchSetup> {
    ensure_plain_directory(&directory)?;
    let directory = fs::canonicalize(directory)?;
    ensure_plain_directory(&directory)?;
    let _startup_lock = acquire_query_startup_lock(&directory)?;
    let another_worker_is_live = retire_stale_active_markers(&directory)?;
    let active_marker = QueryActiveMarker::create(&directory)?;
    if !another_worker_is_live {
        recover_stale_query_files(&directory).map_err(|error| {
            io::Error::other(format!("LeanRows query-cache recovery failed: {error}"))
        })?;
    }
    Ok(QueryScratchSetup {
        directory,
        active_marker,
    })
}

fn ensure_plain_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || worker_metadata_is_reparse(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "LeanRows query-cache path is not a plain directory",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => {}
            Err(race) if race.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        },
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || worker_metadata_is_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LeanRows query-cache path is not a plain directory",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn acquire_query_startup_lock(directory: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    let path = directory.join(QUERY_RECOVERY_LOCK);
    for attempt in 0..QUERY_STARTUP_LOCK_ATTEMPTS {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) if is_windows_sharing_violation(&error) => {
                if attempt + 1 == QUERY_STARTUP_LOCK_ATTEMPTS {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out serializing LeanRows query-cache startup",
                    ));
                }
                thread::sleep(QUERY_STARTUP_LOCK_RETRY);
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "timed out serializing LeanRows query-cache startup",
    ))
}

#[cfg(not(windows))]
fn acquire_query_startup_lock(directory: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(QUERY_RECOVERY_LOCK))
}

#[cfg(windows)]
fn create_active_marker_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(path)
}

#[cfg(not(windows))]
fn create_active_marker_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(windows)]
fn open_active_marker_exclusive(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
}

#[cfg(windows)]
fn is_windows_sharing_violation(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(32 | 33))
}

#[cfg(windows)]
fn retire_stale_active_markers(directory: &Path) -> io::Result<bool> {
    let mut live_marker_exists = false;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !is_query_active_marker_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        if path.parent() != Some(directory) {
            live_marker_exists = true;
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || worker_metadata_is_reparse(&metadata) {
            // A recognized but unsafe candidate is never removed and blocks
            // recovery, which is safer than touching another process's state.
            live_marker_exists = true;
            continue;
        }
        match open_active_marker_exclusive(&path) {
            Ok(handle) => {
                if !handle.metadata()?.is_file() {
                    live_marker_exists = true;
                    continue;
                }
                drop(handle);
                remove_owned_active_marker(directory, &path)?;
            }
            Err(error) if is_windows_sharing_violation(&error) => {
                live_marker_exists = true;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(live_marker_exists)
}

#[cfg(not(windows))]
fn retire_stale_active_markers(directory: &Path) -> io::Result<bool> {
    for entry in fs::read_dir(directory)? {
        if is_query_active_marker_name(&entry?.file_name()) {
            // The native release is Windows-only. Portable builds fail closed
            // by skipping recovery whenever an active-marker candidate exists.
            return Ok(true);
        }
    }
    Ok(false)
}

fn remove_owned_active_marker(directory: &Path, path: &Path) -> io::Result<()> {
    if path.parent() != Some(directory)
        || !path.file_name().is_some_and(is_query_active_marker_name)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "query active-marker cleanup escaped its owned directory",
        ));
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || worker_metadata_is_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "query active-marker cleanup target is not a plain file",
        ));
    }
    fs::remove_file(path)
}

fn is_query_active_marker_name(name: &std::ffi::OsStr) -> bool {
    let Some(encoded) = name
        .to_str()
        .and_then(|name| name.strip_prefix(QUERY_ACTIVE_MARKER_PREFIX))
        .and_then(|name| name.strip_suffix(QUERY_ACTIVE_MARKER_SUFFIX))
    else {
        return false;
    };
    encoded.len() == 32
        && encoded
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn fresh_query_active_marker_name() -> String {
    let elapsed = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(error) => error.duration().as_nanos(),
    };
    let sequence = u128::from(NEXT_QUERY_ACTIVE_MARKER.fetch_add(1, Ordering::Relaxed));
    let process = u128::from(std::process::id()) << 64;
    let owner = elapsed.rotate_left(31) ^ process ^ sequence;
    format!("{QUERY_ACTIVE_MARKER_PREFIX}{owner:032x}{QUERY_ACTIVE_MARKER_SUFFIX}")
}

#[cfg(windows)]
fn worker_metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn worker_metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn next_nonzero_serial(sequence: &AtomicU64) -> u64 {
    loop {
        let serial = sequence.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        if serial != 0 {
            return serial;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
    use std::time::{Duration, Instant};

    use leanrows_core::{
        CancellationToken, LiteralMatchMode, QUERY_HIT_ENTRY_BYTES, QUERY_SCRATCH_HEADER_BYTES,
        QuerySession, QuerySessionConfig,
    };

    use super::{
        QUERY_ACTIVE_MARKER_PREFIX, QUERY_ACTIVE_MARKER_SUFFIX, QueryDirection, Worker,
        WorkerEvent, WorkerPhase, WorkerQueryPhase, WorkerQueryState, WorkerResult,
        is_query_active_marker_name, prepare_query_scratch, remove_owned_active_marker,
    };
    use crate::document_engine::UiColumnKind;

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempFixture(PathBuf);

    impl TempFixture {
        fn file(extension: &str, bytes: &[u8]) -> io::Result<Self> {
            let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "leanrows-worker-{}-{sequence}.{extension}",
                std::process::id()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> io::Result<Self> {
            let base = fs::canonicalize(std::env::temp_dir())?;
            for _ in 0..64 {
                let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!(
                    "LeanRows-worker-query-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not reserve a worker query test directory",
            ))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let Ok(base) = fs::canonicalize(std::env::temp_dir()) else {
                return;
            };
            let owned_name = self
                .0
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("LeanRows-worker-query-"));
            if owned_name && self.0.parent() == Some(base.as_path()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[derive(Default)]
    struct FirstWakeGate {
        state: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    impl FirstWakeGate {
        fn callback(&self) {
            let mut state = lock_unpoisoned(&self.state);
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = match self.changed.wait(state) {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
            }
        }

        fn wait_until_seen(&self, timeout: Duration) -> bool {
            let state = lock_unpoisoned(&self.state);
            if state.0 {
                return true;
            }
            match self
                .changed
                .wait_timeout_while(state, timeout, |value| !value.0)
            {
                Ok((value, _)) => value.0,
                Err(poisoned) => poisoned.into_inner().0.0,
            }
        }

        fn release(&self) {
            let mut state = lock_unpoisoned(&self.state);
            state.1 = true;
            self.changed.notify_all();
        }

        fn arm(&self) {
            let mut state = lock_unpoisoned(&self.state);
            *state = (false, false);
        }
    }

    fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn wait_event(worker: &Worker, timeout: Duration) -> io::Result<WorkerEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = worker.poll() {
                return Ok(event);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "worker event timed out",
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_complete(worker: &Worker, timeout: Duration) -> io::Result<WorkerEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = wait_event(worker, remaining)?;
            if event.phase == WorkerPhase::Complete {
                return Ok(event);
            }
        }
    }

    fn wait_query<F>(
        worker: &Worker,
        query_serial: u64,
        timeout: Duration,
        predicate: F,
    ) -> io::Result<WorkerEvent>
    where
        F: Fn(&WorkerQueryState) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = wait_event(worker, remaining)?;
            if event
                .query
                .as_ref()
                .is_some_and(|query| query.query_serial == query_serial && predicate(query))
            {
                return Ok(event);
            }
        }
    }

    fn query_worker(scratch: &TempDirectory, quota: u64) -> io::Result<Worker> {
        Worker::start_for_query_test(Arc::new(|| {}), scratch.path(), quota)
    }

    fn query_hit_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let is_hit = path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("leanrows-query-v1-") && name.ends_with(".hits")
            });
            if is_hit {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    fn active_marker_paths(directory: &Path) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.file_name().is_some_and(is_query_active_marker_name) {
                paths.push(path);
            }
        }
        paths.sort_unstable();
        Ok(paths)
    }

    fn create_valid_stale_query_file(
        scratch: &Path,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        use crate::document_engine::DocumentEngine;

        let fixture = TempFixture::file("log", b"hit\n")?;
        let token = CancellationToken::new();
        let (engine, _) =
            DocumentEngine::open(fixture.path(), &token, token.snapshot())?.into_parts();
        let (source, format) = engine.query_plan()?.into_parts();
        let config =
            QuerySessionConfig::new(format, 64 * 1_024, 16 * 1_024, 256, 64 * 1_024 * 1_024)?;
        let session = QuerySession::new(source, b"hit", LiteralMatchMode::Exact, config, scratch)?;
        let owned_path = session.scratch_path().to_path_buf();
        let owned_bytes = fs::read(&owned_path)?;
        drop(session);
        fs::write(&owned_path, owned_bytes)?;
        Ok(owned_path)
    }

    fn marker_name(hex_digit: char) -> String {
        format!(
            "{QUERY_ACTIVE_MARKER_PREFIX}{}{QUERY_ACTIVE_MARKER_SUFFIX}",
            hex_digit.to_string().repeat(32)
        )
    }

    fn cell_text(event: &WorkerEvent, row: u64, column: usize) -> io::Result<String> {
        let cell = event.cache.cell(row, column).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "expected worker cache cell is missing",
            )
        })?;
        let payload = cell.strip_suffix(&[0]).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "worker cache cell has no NUL")
        })?;
        String::from_utf16(payload).map_err(io::Error::other)
    }

    #[test]
    fn first_viewport_is_published_before_multi_megabyte_scan_completes()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"row\n".repeat(1_100_000);
        let fixture = TempFixture::file("log", &bytes)?;
        let gate = Arc::new(FirstWakeGate::default());
        let callback_gate = Arc::clone(&gate);
        let worker = Worker::start(Arc::new(move || callback_gate.callback()))?;
        let serial = worker.submit_path(fixture.path().to_path_buf());
        assert!(gate.wait_until_seen(Duration::from_secs(5)));

        let first = worker.poll().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "first viewport event is missing")
        })?;
        assert_eq!(first.serial, serial);
        assert_eq!(first.phase, WorkerPhase::ViewportReady);
        assert_eq!(first.cached_rows, 128);
        assert!(!first.progress.complete);
        assert_eq!(first.progress.scanned_bytes, 0);
        assert!(first.progress.available_rows >= 128);
        assert_eq!(cell_text(&first, 0, 0)?, "1");
        assert_eq!(cell_text(&first, 0, 1)?, "row");
        gate.release();

        let complete = wait_complete(&worker, Duration::from_secs(10))?;
        assert_eq!(complete.progress.indexed_rows, 1_100_000);
        assert!(complete.progress.complete);
        assert!(Arc::ptr_eq(&first.cache, &complete.cache));
        assert_eq!(complete.cached_rows, 128);
        Ok(())
    }

    #[test]
    fn csv_worker_keeps_fields_independent_and_never_infers_a_header()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = TempFixture::file(
            "csv",
            b"header-like,\"quoted, value\",,\"line1\nline2\",\"say \"\"hi\"\"\",\n",
        )?;
        let worker = Worker::start(Arc::new(|| {}))?;
        worker.submit_path(fixture.path().to_path_buf());
        let event = wait_complete(&worker, Duration::from_secs(5))?;
        let columns = event
            .columns
            .ok_or_else(|| io::Error::other("CSV column metadata is missing"))?;
        assert_eq!(columns.kind(), UiColumnKind::Fields);
        assert_eq!(columns.data_columns(), 6);
        assert_eq!(cell_text(&event, 0, 0)?, "1");
        assert_eq!(cell_text(&event, 0, 1)?, "header-like");
        assert_eq!(cell_text(&event, 0, 2)?, "quoted, value");
        assert_eq!(cell_text(&event, 0, 3)?, "");
        assert_eq!(cell_text(&event, 0, 4)?, r"line1\nline2");
        assert_eq!(cell_text(&event, 0, 5)?, "say \"hi\"");
        assert_eq!(cell_text(&event, 0, 6)?, "");
        assert_eq!(event.source_truncated_rows, 0);
        assert!(event.is_internally_consistent());
        Ok(())
    }

    #[test]
    fn delimited_columns_cap_grow_only_and_reset_for_a_log_replacement()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut wide = (1..=65)
            .map(|field| format!("field-{field}"))
            .collect::<Vec<_>>()
            .join(",");
        wide.push('\n');
        wide.push_str(&"narrow\n".repeat(200));
        let csv = TempFixture::file("csv", wide.as_bytes())?;
        let log = TempFixture::file("log", b"plain preview\n")?;
        let worker = Worker::start(Arc::new(|| {}))?;
        let serial = worker.submit_path(csv.path().to_path_buf());
        let initial = wait_complete(&worker, Duration::from_secs(5))?;
        let initial_columns = initial
            .columns
            .ok_or_else(|| io::Error::other("wide CSV column metadata is missing"))?;
        assert_eq!(initial_columns.kind(), UiColumnKind::Fields);
        assert_eq!(initial_columns.data_columns(), 64);
        assert_eq!(initial.source_truncated_rows, 1);
        assert!(initial.cache.cell(0, 65).is_none());

        assert!(worker.request_viewport(serial, 150));
        let narrowed = loop {
            let event = wait_event(&worker, Duration::from_secs(5))?;
            if event.cache_first_row == 150 {
                break event;
            }
        };
        let narrowed_columns = narrowed
            .columns
            .ok_or_else(|| io::Error::other("narrow viewport metadata is missing"))?;
        assert_eq!(narrowed_columns.kind(), UiColumnKind::Fields);
        assert_eq!(narrowed_columns.data_columns(), 64);
        assert_eq!(cell_text(&narrowed, 150, 1)?, "narrow");
        assert!(narrowed.cache.cell(150, 2).is_none());

        worker.submit_path(log.path().to_path_buf());
        let replacement = wait_complete(&worker, Duration::from_secs(5))?;
        let replacement_columns = replacement
            .columns
            .ok_or_else(|| io::Error::other("log column metadata is missing"))?;
        assert_eq!(replacement_columns.kind(), UiColumnKind::Preview);
        assert_eq!(replacement_columns.data_columns(), 1);
        assert_eq!(cell_text(&replacement, 0, 1)?, "plain preview");
        Ok(())
    }

    #[test]
    fn replacement_events_keep_cache_and_wake_only_for_an_empty_slot()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = TempFixture::file("txt", &b"row\n".repeat(20_000))?;
        let wakes = Arc::new(AtomicU64::new(0));
        let callback_wakes = Arc::clone(&wakes);
        let worker = Worker::start(Arc::new(move || {
            callback_wakes.fetch_add(1, Ordering::Relaxed);
        }))?;
        worker.submit_path(fixture.path().to_path_buf());
        std::thread::sleep(Duration::from_millis(100));

        let event = worker.poll().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "replacement event is missing")
        })?;
        assert_eq!(event.phase, WorkerPhase::Complete);
        assert_eq!(event.cached_rows, 128);
        assert_eq!(cell_text(&event, 0, 1)?, "row");
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        std::thread::sleep(Duration::from_millis(20));
        assert!(worker.poll().is_none());
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn rapid_open_replacement_cancels_and_filters_stale_document_events()
    -> Result<(), Box<dyn std::error::Error>> {
        let first_fixture = TempFixture::file("log", &b"old\n".repeat(600_000))?;
        let second_fixture = TempFixture::file("log", b"new\n")?;
        let worker = Worker::start(Arc::new(|| {}))?;
        let stale_serial = worker.submit_path(first_fixture.path().to_path_buf());
        let current_serial = worker.submit_path(second_fixture.path().to_path_buf());
        assert_ne!(stale_serial, current_serial);
        assert!(!worker.request_viewport(stale_serial, 10));

        let event = wait_complete(&worker, Duration::from_secs(5))?;
        assert_eq!(event.serial, current_serial);
        assert_eq!(event.path, second_fixture.path());
        assert_eq!(cell_text(&event, 0, 1)?, "new");
        assert_eq!(event.progress.indexed_rows, 1);
        Ok(())
    }

    #[test]
    fn viewport_requests_coalesce_to_latest_absolute_row() -> Result<(), Box<dyn std::error::Error>>
    {
        let fixture = TempFixture::file("txt", &b"value\n".repeat(200))?;
        let gate = Arc::new(FirstWakeGate::default());
        let callback_gate = Arc::clone(&gate);
        let worker = Worker::start(Arc::new(move || callback_gate.callback()))?;
        let serial = worker.submit_path(fixture.path().to_path_buf());
        assert!(gate.wait_until_seen(Duration::from_secs(5)));
        let _first = worker.poll().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "initial viewport event is missing")
        })?;

        assert!(worker.request_viewport(serial, 10));
        assert!(worker.request_viewport(serial, 20));
        assert!(worker.request_viewport(serial, 30));
        gate.release();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let event = wait_event(&worker, deadline.saturating_duration_since(Instant::now()))?;
            if event.cache_first_row == 30 {
                assert_eq!(event.serial, serial);
                assert_eq!(cell_text(&event, 30, 0)?, "31");
                assert_eq!(cell_text(&event, 30, 1)?, "value");
                break;
            }
        }
        Ok(())
    }

    #[test]
    fn empty_invalid_utf8_and_failed_files_have_bounded_cache_events()
    -> Result<(), Box<dyn std::error::Error>> {
        let empty = TempFixture::file("csv", b"")?;
        let worker = Worker::start(Arc::new(|| {}))?;
        worker.submit_path(empty.path().to_path_buf());
        let empty_event = wait_complete(&worker, Duration::from_secs(5))?;
        assert!(matches!(
            empty_event.result,
            WorkerResult::Ready { size: 0 }
        ));
        assert_eq!(empty_event.cached_rows, 0);
        assert_eq!(empty_event.progress.indexed_rows, 0);

        let mut invalid_bytes = b"bad".to_vec();
        invalid_bytes.push(0xff);
        invalid_bytes.push(b'\n');
        let invalid = TempFixture::file("log", &invalid_bytes)?;
        worker.submit_path(invalid.path().to_path_buf());
        let invalid_event = wait_complete(&worker, Duration::from_secs(5))?;
        assert_eq!(cell_text(&invalid_event, 0, 1)?, r"bad\xFF");
        assert_eq!(invalid_event.escaped_non_utf8_rows, 1);

        let missing = std::env::temp_dir().join(format!(
            "leanrows-worker-missing-{}-{}.csv",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let missing_serial = worker.submit_path(missing);
        assert!(
            worker
                .start_query(missing_serial, b"must not start".to_vec(), true)
                .is_none()
        );
        let failed = wait_event(&worker, Duration::from_secs(5))?;
        assert_eq!(failed.phase, WorkerPhase::Failed);
        assert_eq!(failed.cached_rows, 0);
        match &failed.result {
            WorkerResult::Failed { message } => assert!(!message.is_empty()),
            WorkerResult::Ready { .. } => {
                return Err(io::Error::other("missing file unexpectedly opened").into());
            }
        }
        Ok(())
    }

    #[test]
    fn query_is_demand_driven_navigable_and_retained_on_document_progress()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"Alpha\n");
        bytes.extend_from_slice(&b"padding\n".repeat(700_000));
        bytes.extend_from_slice(b"ALPHA\n");
        let fixture = TempFixture::file("log", &bytes)?;
        let gate = Arc::new(FirstWakeGate::default());
        let callback_gate = Arc::clone(&gate);
        let worker = Worker::start_for_query_test(
            Arc::new(move || callback_gate.callback()),
            scratch.path(),
            64 * 1_024 * 1_024,
        )?;
        let document_serial = worker.submit_path(fixture.path().to_path_buf());
        assert!(gate.wait_until_seen(Duration::from_secs(5)));
        let first = worker
            .poll()
            .ok_or_else(|| io::Error::other("initial document event is missing"))?;
        assert_eq!(first.phase, WorkerPhase::ViewportReady);

        let query_serial = worker
            .start_query(document_serial, b"alpha".to_vec(), false)
            .ok_or_else(|| io::Error::other("current query was rejected"))?;
        gate.release();
        let first_match = wait_query(&worker, query_serial, Duration::from_secs(15), |query| {
            query.phase == WorkerQueryPhase::MatchReady && query.active_hit_index == Some(0)
        })?;
        let first_state = first_match
            .query
            .as_ref()
            .ok_or_else(|| io::Error::other("first query state is missing"))?;
        assert_eq!(first_state.active_row, Some(0));
        assert!(first_state.scanned_bytes < first_state.source_bytes);
        let progress_deadline = Instant::now() + Duration::from_secs(15);
        let retained_progress = loop {
            let event = wait_event(
                &worker,
                progress_deadline.saturating_duration_since(Instant::now()),
            )?;
            let retains_match = event.query.as_ref().is_some_and(|query| {
                query.query_serial == query_serial
                    && query.phase == WorkerQueryPhase::MatchReady
                    && query.active_hit_index == Some(0)
            });
            if retains_match && (event.progress.scanned_bytes > 0 || event.progress.complete) {
                break event;
            }
        };
        assert!(retained_progress.query.is_some());

        assert!(worker.navigate_query(document_serial, query_serial, QueryDirection::Next));
        let second_match = wait_query(&worker, query_serial, Duration::from_secs(20), |query| {
            query.active_hit_index == Some(1)
        })?;
        let second_state = second_match
            .query
            .as_ref()
            .ok_or_else(|| io::Error::other("second query state is missing"))?;
        assert_eq!(second_state.phase, WorkerQueryPhase::MatchReady);
        assert_eq!(second_state.active_row, Some(700_001));

        assert!(worker.navigate_query(document_serial, query_serial, QueryDirection::Previous));
        let previous = wait_query(&worker, query_serial, Duration::from_secs(5), |query| {
            query.active_hit_index == Some(0)
        })?;
        assert_eq!(
            previous.query.as_ref().and_then(|query| query.active_row),
            Some(0)
        );
        Ok(())
    }

    #[test]
    fn query_honors_csv_record_edges_multiline_fields_and_raw_invalid_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let csv = TempFixture::file("csv", b"row,value\n1,\"Needle\ninside\"\n2,end\n")?;
        let worker = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        let document_serial = worker.submit_path(csv.path().to_path_buf());
        let _ = wait_complete(&worker, Duration::from_secs(5))?;

        let multiline = worker
            .start_query(document_serial, b"needle\nINSIDE".to_vec(), false)
            .ok_or_else(|| io::Error::other("multiline query was rejected"))?;
        let matched = wait_query(&worker, multiline, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::MatchReady
        })?;
        assert_eq!(
            matched.query.as_ref().and_then(|query| query.active_row),
            Some(1)
        );

        let crossing = worker
            .start_query(document_serial, b"inside\"\n2".to_vec(), true)
            .ok_or_else(|| io::Error::other("record-edge query was rejected"))?;
        let no_crossing = wait_query(&worker, crossing, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::Complete
        })?;
        let no_crossing_state = no_crossing
            .query
            .as_ref()
            .ok_or_else(|| io::Error::other("record-edge query state is missing"))?;
        assert_eq!(no_crossing_state.stored_hits, 0);
        assert_eq!(no_crossing_state.active_row, None);

        let invalid = TempFixture::file("log", &[0xff, b'A', 0xfe, b'\n'])?;
        let invalid_document = worker.submit_path(invalid.path().to_path_buf());
        let _ = wait_complete(&worker, Duration::from_secs(5))?;
        let raw_query = worker
            .start_query(invalid_document, vec![0xff, b'a', 0xfe], false)
            .ok_or_else(|| io::Error::other("raw-byte query was rejected"))?;
        let raw_match = wait_query(&worker, raw_query, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::MatchReady
        })?;
        assert_eq!(
            raw_match.query.as_ref().and_then(|query| query.active_row),
            Some(0)
        );
        Ok(())
    }

    #[test]
    fn no_match_empty_query_and_stale_serials_fail_without_stale_navigation()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let fixture = TempFixture::file("log", b"alpha\nbeta\n")?;
        let worker = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        let document_serial = worker.submit_path(fixture.path().to_path_buf());
        let _ = wait_complete(&worker, Duration::from_secs(5))?;
        assert!(
            worker
                .start_query(document_serial.wrapping_add(1), b"x".to_vec(), true)
                .is_none()
        );

        let missing = worker
            .start_query(document_serial, b"missing".to_vec(), true)
            .ok_or_else(|| io::Error::other("no-match query was rejected"))?;
        let complete = wait_query(&worker, missing, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::Complete
        })?;
        let complete_state = complete
            .query
            .as_ref()
            .ok_or_else(|| io::Error::other("no-match state is missing"))?;
        assert_eq!(complete_state.stored_hits, 0);
        assert_eq!(complete_state.active_hit_index, None);
        assert!(!worker.navigate_query(
            document_serial,
            missing.wrapping_add(1),
            QueryDirection::Next
        ));

        let superseded = worker
            .start_query(document_serial, b"alpha".to_vec(), true)
            .ok_or_else(|| io::Error::other("replacement query was rejected"))?;
        let empty = worker
            .start_query(document_serial, Vec::new(), true)
            .ok_or_else(|| io::Error::other("empty query command was rejected"))?;
        assert!(!worker.navigate_query(document_serial, superseded, QueryDirection::Next));
        let failed = wait_query(&worker, empty, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::Failed
        })?;
        assert!(
            failed
                .query
                .as_ref()
                .is_some_and(|query| !query.message.is_empty())
        );
        assert!(query_hit_files(scratch.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn query_quota_is_exact_and_scratch_is_removed_when_worker_drops()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let fixture = TempFixture::file("log", b"x x x\n")?;
        let quota = QUERY_SCRATCH_HEADER_BYTES + 2 * QUERY_HIT_ENTRY_BYTES;
        let worker = query_worker(&scratch, quota)?;
        let document_serial = worker.submit_path(fixture.path().to_path_buf());
        let _ = wait_complete(&worker, Duration::from_secs(5))?;
        let query_serial = worker
            .start_query(document_serial, b"x".to_vec(), true)
            .ok_or_else(|| io::Error::other("quota query was rejected"))?;
        let _ = wait_query(&worker, query_serial, Duration::from_secs(5), |query| {
            query.active_hit_index == Some(0)
        })?;
        assert!(worker.navigate_query(document_serial, query_serial, QueryDirection::Next));
        let _ = wait_query(&worker, query_serial, Duration::from_secs(5), |query| {
            query.active_hit_index == Some(1)
        })?;
        assert!(worker.navigate_query(document_serial, query_serial, QueryDirection::Next));
        let quota_event = wait_query(&worker, query_serial, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::QuotaExceeded
        })?;
        let quota_state = quota_event
            .query
            .as_ref()
            .ok_or_else(|| io::Error::other("quota state is missing"))?;
        assert_eq!(quota_state.stored_hits, 2);
        assert!(quota_state.message.contains("hit-cache limit"));
        assert_eq!(query_hit_files(scratch.path())?.len(), 1);
        drop(worker);
        assert!(query_hit_files(scratch.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn document_replacement_cancels_query_and_removes_its_scratch()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let first = TempFixture::file("log", &b"old value\n".repeat(100_000))?;
        let replacement = TempFixture::file("log", b"new value\n")?;
        let gate = Arc::new(FirstWakeGate::default());
        gate.release();
        let callback_gate = Arc::clone(&gate);
        let worker = Worker::start_for_query_test(
            Arc::new(move || callback_gate.callback()),
            scratch.path(),
            64 * 1_024 * 1_024,
        )?;
        let first_serial = worker.submit_path(first.path().to_path_buf());
        let _ = wait_complete(&worker, Duration::from_secs(10))?;
        gate.arm();
        let query_serial = worker
            .start_query(first_serial, b"never present".to_vec(), true)
            .ok_or_else(|| io::Error::other("cancellable query was rejected"))?;
        assert!(gate.wait_until_seen(Duration::from_secs(5)));
        let searching = worker
            .poll()
            .ok_or_else(|| io::Error::other("searching event is missing"))?;
        assert!(searching.query.as_ref().is_some_and(|query| {
            query.query_serial == query_serial && query.phase == WorkerQueryPhase::Searching
        }));
        assert_eq!(query_hit_files(scratch.path())?.len(), 1);

        let replacement_serial = worker.submit_path(replacement.path().to_path_buf());
        assert!(!worker.navigate_query(first_serial, query_serial, QueryDirection::Next));
        gate.release();
        let current = wait_complete(&worker, Duration::from_secs(5))?;
        assert_eq!(current.serial, replacement_serial);
        assert_eq!(current.path, replacement.path());
        assert!(current.query.is_none());
        assert!(query_hit_files(scratch.path())?.is_empty());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn atomic_source_rotation_cannot_publish_a_match_from_stale_path_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let original = TempFixture::file("log", b"old needle\n")?;
        let replacement = TempFixture::file("log", b"new content\n")?;
        let gate = Arc::new(FirstWakeGate::default());
        gate.release();
        let callback_gate = Arc::clone(&gate);
        let worker = Worker::start_for_query_test(
            Arc::new(move || callback_gate.callback()),
            scratch.path(),
            64 * 1_024 * 1_024,
        )?;
        let document_serial = worker.submit_path(original.path().to_path_buf());
        let _ = wait_complete(&worker, Duration::from_secs(5))?;

        gate.arm();
        let query_serial = worker
            .start_query(document_serial, b"needle".to_vec(), true)
            .ok_or_else(|| io::Error::other("rotation query was rejected"))?;
        assert!(gate.wait_until_seen(Duration::from_secs(5)));
        let searching = worker
            .poll()
            .ok_or_else(|| io::Error::other("rotation query did not start"))?;
        assert!(searching.query.as_ref().is_some_and(|query| {
            query.query_serial == query_serial && query.phase == WorkerQueryPhase::Searching
        }));

        fs::remove_file(original.path())?;
        fs::rename(replacement.path(), original.path())?;
        gate.release();
        let failed = wait_query(&worker, query_serial, Duration::from_secs(5), |query| {
            query.phase == WorkerQueryPhase::Failed
        })?;
        let failed_state = failed
            .query
            .as_ref()
            .ok_or_else(|| io::Error::other("rotation failure state is missing"))?;
        assert_eq!(failed_state.active_hit_index, None);
        assert_eq!(failed_state.active_row, None);
        assert!(failed_state.message.contains("source changed"));
        assert!(query_hit_files(scratch.path())?.is_empty());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn leader_exit_with_follower_alive_never_allows_third_worker_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let leader = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert_eq!(active_marker_paths(scratch.path())?.len(), 1);
        let protected_scratch = create_valid_stale_query_file(scratch.path())?;

        let follower = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(protected_scratch.exists());
        assert_eq!(active_marker_paths(scratch.path())?.len(), 2);
        drop(leader);
        assert_eq!(active_marker_paths(scratch.path())?.len(), 1);

        let third = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(protected_scratch.exists());
        assert_eq!(active_marker_paths(scratch.path())?.len(), 2);
        drop(third);
        drop(follower);
        assert!(active_marker_paths(scratch.path())?.is_empty());

        let recovery = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(!protected_scratch.exists());
        drop(recovery);
        assert!(active_marker_paths(scratch.path())?.is_empty());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn crash_stale_marker_is_retired_before_no_live_worker_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let crashed_marker = scratch.path().join(marker_name('a'));
        fs::write(&crashed_marker, b"")?;
        let stale_scratch = create_valid_stale_query_file(scratch.path())?;

        let worker = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(!crashed_marker.exists());
        assert!(!stale_scratch.exists());
        assert_eq!(active_marker_paths(scratch.path())?.len(), 1);
        drop(worker);
        assert!(active_marker_paths(scratch.path())?.is_empty());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_startups_serialize_and_all_retain_unique_live_markers()
    -> Result<(), Box<dyn std::error::Error>> {
        const WORKERS: usize = 8;

        let scratch = TempDirectory::new()?;
        let stale_scratch = create_valid_stale_query_file(scratch.path())?;
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (ready_sender, ready_receiver) = mpsc::channel();
        let mut threads = Vec::new();
        for _ in 0..WORKERS {
            let directory = scratch.path().to_path_buf();
            let thread_release = Arc::clone(&release);
            let sender = ready_sender.clone();
            threads.push(std::thread::spawn(move || -> io::Result<()> {
                let setup = match prepare_query_scratch(directory) {
                    Ok(setup) => {
                        let _ = sender.send(Ok(()));
                        setup
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error.to_string()));
                        return Err(error);
                    }
                };
                let (released, changed) = &*thread_release;
                let state = lock_unpoisoned(released);
                let _state = match changed.wait_while(state, |released| !*released) {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                drop(setup);
                Ok(())
            }));
        }
        drop(ready_sender);

        let mut startup_error = None;
        for _ in 0..WORKERS {
            match ready_receiver.recv_timeout(Duration::from_secs(10))? {
                Ok(()) => {}
                Err(error) => startup_error = Some(error),
            }
        }
        if let Some(error) = startup_error {
            let (released, changed) = &*release;
            *lock_unpoisoned(released) = true;
            changed.notify_all();
            for thread in threads {
                let _ = thread.join();
            }
            return Err(error.into());
        }

        assert!(!stale_scratch.exists());
        assert_eq!(active_marker_paths(scratch.path())?.len(), WORKERS);
        let (released, changed) = &*release;
        *lock_unpoisoned(released) = true;
        changed.notify_all();
        for thread in threads {
            thread
                .join()
                .map_err(|_| io::Error::other("query startup test thread panicked"))??;
        }
        assert!(active_marker_paths(scratch.path())?.is_empty());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn active_marker_cleanup_is_confined_and_unsafe_candidates_block_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let outside = TempDirectory::new()?;
        let outside_marker = outside.path().join(marker_name('b'));
        fs::write(&outside_marker, b"")?;
        assert!(remove_owned_active_marker(scratch.path(), &outside_marker).is_err());
        assert!(outside_marker.exists());

        let unsafe_candidate = scratch.path().join(marker_name('c'));
        fs::create_dir(&unsafe_candidate)?;
        let protected_scratch = create_valid_stale_query_file(scratch.path())?;
        let worker = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(unsafe_candidate.is_dir());
        assert!(protected_scratch.exists());
        assert_eq!(active_marker_paths(scratch.path())?.len(), 2);
        drop(worker);
        assert_eq!(
            active_marker_paths(scratch.path())?,
            vec![unsafe_candidate.clone()]
        );

        fs::remove_dir(&unsafe_candidate)?;
        let recovery = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(!protected_scratch.exists());
        drop(recovery);
        assert!(active_marker_paths(scratch.path())?.is_empty());
        assert!(outside_marker.exists());
        Ok(())
    }

    #[test]
    fn worker_startup_recovers_only_valid_engine_owned_stale_query_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = TempDirectory::new()?;
        let owned_path = create_valid_stale_query_file(scratch.path())?;
        let unrelated = scratch.path().join("notes.txt");
        fs::write(&unrelated, b"keep")?;
        let malformed = scratch
            .path()
            .join("leanrows-query-v1-00000000000000000000000000000000.hits");
        fs::write(&malformed, b"partial")?;

        let worker = query_worker(&scratch, 64 * 1_024 * 1_024)?;
        assert!(!owned_path.exists());
        assert!(unrelated.exists());
        assert!(malformed.exists());
        drop(worker);
        Ok(())
    }

    #[test]
    fn drop_cancels_inflight_open_and_joins_worker() -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = vec![b'x'; 8 * 1_024 * 1_024];
        bytes.push(b'\n');
        let fixture = TempFixture::file("log", &bytes)?;
        let callback_reached = Arc::new(AtomicBool::new(false));
        let callback_flag = Arc::clone(&callback_reached);
        let worker = Worker::start(Arc::new(move || {
            callback_flag.store(true, Ordering::Release);
        }))?;
        worker.submit_path(fixture.path().to_path_buf());
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_secs(5));
        Ok(())
    }
}
