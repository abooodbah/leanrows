#[allow(
    clippy::wildcard_imports,
    reason = "private worker extension shares the worker's document and mailbox types"
)]
use super::*;
#[cfg(windows)]
use crate::native::free_disk_bytes;
#[cfg(not(windows))]
use crate::native_stub::free_disk_bytes;
/// Prepares the sort folder and removes stale sort data once per process.
///
/// A failure is not cached: it fails only the sort that asked, and the next
/// sort tries again. Opening and viewing files never depend on this folder.
pub(super) fn default_sort_directory() -> io::Result<PathBuf> {
    use std::sync::OnceLock;
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    if let Some(directory) = DIRECTORY.get() {
        return Ok(directory.clone());
    }
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    let directory = base.join("LeanRows").join("sort-cache");
    fs::create_dir_all(&directory)?;
    leanrows_core::recover_stale_sort_files(&directory).map_err(io::Error::other)?;
    Ok(DIRECTORY.get_or_init(|| directory).clone())
}
pub(super) fn start_sort(
    runtime: &WorkerRuntime,
    document: &mut ActiveDocument,
    command: SortCommand,
) {
    if command.document_serial != document.serial
        || command.serial != runtime.latest_sort_serial.load(Ordering::Acquire)
    {
        return;
    }
    document.sort = None;
    document.reveal_rank = None;
    let committed = document.sort_state.as_ref().and_then(|s| s.committed);
    document.sort_state = Some(WorkerSortState {
        serial: command.serial,
        request: command.request,
        committed,
        phase: SortPhase::Extracting,
        scanned_bytes: 0,
        source_bytes: document.engine.progress().source_bytes(),
        error: None,
    });
    if let Some(request) = command.request {
        document.sort = Some(ActiveSort {
            serial: command.serial,
            request,
            generation: command.generation,
            session: None,
            source_memory: None,
            permit: None,
            last_emit: std::time::Instant::now(),
        });
    } else {
        let generation = runtime.cancellation.snapshot();
        match document
            .engine
            .snapshot(0, &runtime.cancellation, generation)
            .and_then(|s| {
                convert_cache(s).map_err(|e| DocumentEngineError::Io {
                    operation: "restore source rows",
                    source: io::Error::other(e),
                })
            }) {
            Ok(mut cache) => {
                if let Some(columns) = document.cache.columns.grow(cache.columns) {
                    cache.columns = columns;
                }
                document.cache = cache;
                document.order = None;
                if let Some(state) = &mut document.sort_state {
                    state.phase = SortPhase::Complete;
                    state.committed = None;
                }
            }
            Err(e) => sort_failed(document, e.to_string()),
        }
    }
    let _ = emit_active(runtime, document, current_document_phase(document), None);
}
/// Peak temporary disk per row: a merge pass holds two 128-byte copies of
/// every row's sort record.
const SORT_SCRATCH_BYTES_PER_ROW: u64 = 256;
/// Space a sort leaves free on the drive at its peak.
const SORT_FREE_SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// Rows the sort will read: the indexed count, or, while indexing is still
/// running, that count scaled up to the whole file.
fn expected_rows(rows: u64, complete: bool, scanned: u64, source: u64) -> u64 {
    if complete || scanned == 0 || scanned >= source {
        return rows;
    }
    let scaled = u128::from(rows) * u128::from(source) / u128::from(scanned);
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// Refuses a sort before it starts when its temporary data cannot fit.
fn check_sort_space(rows: u64, quota_left: u64, quota: u64, free: u64) -> Result<(), String> {
    let needed = rows.saturating_mul(SORT_SCRATCH_BYTES_PER_ROW);
    if needed > quota {
        return Err(format!(
            "This file is too large to sort: it needs about {} of temporary disk space, more than the {} sort limit",
            size_text(needed),
            size_text(quota)
        ));
    }
    if needed > quota_left {
        return Err(
            "Other sorted views are using the sort space; restore source order in one of them and try again"
                .to_owned(),
        );
    }
    let wanted = needed.saturating_add(SORT_FREE_SPACE_MARGIN);
    if wanted > free {
        return Err(format!(
            "Not enough free disk space to sort: it needs about {} and the drive has {} free",
            size_text(wanted),
            size_text(free)
        ));
    }
    Ok(())
}

/// Sizes as Windows Explorer shows them (binary units, labelled GB and MB).
fn size_text(bytes: u64) -> String {
    const MB: u128 = 1 << 20;
    const GB: u128 = 1 << 30;
    let bytes = u128::from(bytes);
    if bytes >= GB {
        let tenths = (bytes * 10).div_ceil(GB);
        format!("{}.{} GB", tenths / 10, tenths % 10)
    } else {
        format!("{} MB", bytes.div_ceil(MB))
    }
}

/// Applies `check_sort_space` to this document, the shared sort ledger and
/// the drive that holds the sort folder.
fn check_space_for(
    runtime: &WorkerRuntime,
    progress: DocumentProgress,
    directory: &Path,
) -> Result<(), String> {
    let resources = runtime.coordinator.resources();
    check_sort_space(
        expected_rows(
            progress.row_count(),
            progress.is_complete(),
            progress.scanned_bytes(),
            progress.source_bytes(),
        ),
        resources
            .disk_limit()
            .saturating_sub(resources.disk_bytes()),
        resources.disk_limit(),
        // If Windows cannot report free space, the sort still runs and fails
        // cleanly should the drive fill.
        free_disk_bytes(directory).unwrap_or(u64::MAX),
    )
}

fn sort_failed(document: &mut ActiveDocument, message: String) {
    document.sort = None;
    if let Some(state) = &mut document.sort_state {
        state.phase = SortPhase::Cancelled;
        state.error = Some(bounded_query_message(message));
    }
}
pub(super) fn advance_sort(runtime: &WorkerRuntime, document: &mut ActiveDocument) {
    let Some(active) = &mut document.sort else {
        return;
    };
    if !is_latest(&runtime.latest_sort_serial, active.serial)
        || runtime.sort_cancellation.is_cancelled(active.generation)
    {
        document.sort = None;
        if let Some(state) = &mut document.sort_state {
            state.phase = SortPhase::Cancelled;
        }
        let _ = emit_active(runtime, document, current_document_phase(document), None);
        return;
    }
    if active.permit.is_none() {
        active.permit = runtime.coordinator.try_acquire(active.serial);
        if active.permit.is_none() {
            std::thread::sleep(Duration::from_millis(2));
            return;
        }
    }
    if active.session.is_none() {
        let result = (|| {
            active.source_memory = Some(
                runtime
                    .coordinator
                    .resources()
                    .reserve_memory(64 * 1024)
                    .map_err(|e| e.to_string())?,
            );
            let (source, format) = document
                .engine
                .sort_plan()
                .map_err(|e| e.to_string())?
                .into_parts();
            let directory = default_sort_directory().map_err(|e| e.to_string())?;
            check_space_for(runtime, document.engine.progress(), &directory)?;
            SortSession::new(
                source,
                SortConfig {
                    format,
                    request: active.request,
                    batch_capacity: 8192,
                    merge_fan_in: 2,
                    step_bytes: 64 * 1024,
                },
                runtime.coordinator.resources(),
                &directory,
            )
            .map_err(|e| e.to_string())
        })();
        match result {
            Ok(session) => active.session = Some(session),
            Err(e) => {
                sort_failed(document, e);
                let _ = emit_active(runtime, document, current_document_phase(document), None);
                return;
            }
        }
    }
    let Some(session) = active.session.as_mut() else {
        return;
    };
    let result = session
        .advance(&runtime.sort_cancellation, active.generation)
        .and_then(|step| {
            document
                .engine
                .validate_query_snapshot()
                .map_err(|e| leanrows_core::SortError::Io(io::Error::other(e)))?;
            Ok(step)
        });
    match result {
        Err(e) => {
            sort_failed(document, e.to_string());
            let _ = emit_active(runtime, document, current_document_phase(document), None);
        }
        Ok(step) => {
            if let Some(state) = &mut document.sort_state {
                state.phase = step.phase;
                state.scanned_bytes = step.scanned_bytes;
            }
            if step.phase == SortPhase::Complete {
                complete_sort(runtime, document);
            } else if active.last_emit.elapsed() >= Duration::from_millis(100) {
                active.last_emit = std::time::Instant::now();
                let _ = emit_active(runtime, document, current_document_phase(document), None);
            }
        }
    }
}
fn complete_sort(runtime: &WorkerRuntime, document: &mut ActiveDocument) {
    let Some(mut active) = document.sort.take() else {
        return;
    };
    if !is_latest(&runtime.latest_sort_serial, active.serial)
        || runtime.sort_cancellation.is_cancelled(active.generation)
    {
        sort_failed(document, "sorting was cancelled".to_owned());
        let _ = emit_active(runtime, document, current_document_phase(document), None);
        return;
    }
    let Some(session) = active.session.take() else {
        return;
    };
    let result = prepare_sort_if_current(
        &runtime.latest_sort_serial,
        &runtime.sort_cancellation,
        active.serial,
        active.generation,
        || {
            document
                .engine
                .validate_query_snapshot()
                .map_err(|e| e.to_string())?;
            let order = session.into_order().map_err(|e| e.to_string())?;
            let snapshot = document
                .engine
                .snapshot_ordered(
                    &order,
                    0,
                    &runtime.cancellation,
                    runtime.cancellation.snapshot(),
                )
                .map_err(|e| e.to_string())?;
            let mut cache = convert_cache(snapshot).map_err(|e| e.to_string())?;
            cache.columns = document
                .cache
                .columns
                .grow(cache.columns)
                .ok_or_else(|| "sorted columns changed shape".to_owned())?;
            document
                .engine
                .validate_query_snapshot()
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((order, cache))
        },
    );
    match result {
        Ok(Some((order, cache))) => {
            document.cache = cache;
            document.order = Some(order);
            document.reveal_rank = None;
            if let Some(state) = &mut document.sort_state {
                state.phase = SortPhase::Complete;
                state.committed = Some(active.request);
            }
        }
        Ok(None) => sort_failed(document, "sorting was cancelled".to_owned()),
        Err(e) => sort_failed(document, e),
    }
    let _ = emit_active(runtime, document, current_document_phase(document), None);
}
/// Whether `serial` is still this tab's newest sort request. Sorts in other
/// tabs never supersede it.
fn is_latest(latest: &AtomicU64, serial: u64) -> bool {
    latest.load(Ordering::Acquire) == serial
}
fn prepare_sort_if_current<T>(
    latest: &AtomicU64,
    cancellation: &CancellationToken,
    serial: u64,
    generation: CancellationGeneration,
    prepare: impl FnOnce() -> Result<T, String>,
) -> Result<Option<T>, String> {
    if !is_latest(latest, serial) || cancellation.is_cancelled(generation) {
        return Ok(None);
    }
    let prepared = prepare()?;
    if !is_latest(latest, serial) || cancellation.is_cancelled(generation) {
        return Ok(None);
    }
    Ok(Some(prepared))
}
#[cfg(test)]
mod tests {
    use super::*;
    const QUOTA: u64 = 8 * 1024 * 1024 * 1024;
    #[test]
    fn sort_space_check_refuses_one_byte_short_of_free_space() {
        let rows = 1_000_000;
        let needed = rows * 256 + 64 * 1024 * 1024;
        assert_eq!(check_sort_space(rows, QUOTA, QUOTA, needed), Ok(()));
        let refused = check_sort_space(rows, QUOTA, QUOTA, needed - 1);
        assert!(
            refused
                .as_ref()
                .is_err_and(|m| m.contains("free disk space")),
            "{refused:?}"
        );
    }
    #[test]
    fn sort_space_check_refuses_files_over_the_sort_limit() {
        let rows = QUOTA / 256;
        assert_eq!(check_sort_space(rows, QUOTA, QUOTA, u64::MAX), Ok(()));
        let refused = check_sort_space(rows + 1, QUOTA, QUOTA, u64::MAX);
        assert!(
            refused.as_ref().is_err_and(|m| m.contains("too large")),
            "{refused:?}"
        );
    }
    #[test]
    fn sort_space_check_refuses_when_other_sorted_views_hold_the_space() {
        let rows = 1_000;
        assert_eq!(check_sort_space(rows, rows * 256, QUOTA, u64::MAX), Ok(()));
        let refused = check_sort_space(rows, rows * 256 - 1, QUOTA, u64::MAX);
        assert!(
            refused.as_ref().is_err_and(|m| m.contains("restore")),
            "{refused:?}"
        );
    }
    #[test]
    fn expected_rows_scales_a_partial_index_to_the_whole_file() {
        assert_eq!(expected_rows(10, true, 50, 100), 10);
        assert_eq!(expected_rows(10, false, 50, 100), 20);
        assert_eq!(expected_rows(10, false, 0, 100), 10);
        assert_eq!(expected_rows(u64::MAX, false, 1, 2), u64::MAX);
    }
    #[test]
    fn sizes_read_like_explorer() {
        assert_eq!(size_text(QUOTA), "8.0 GB");
        assert_eq!(size_text(256_000_000), "245 MB");
        assert_eq!(size_text(2_560_000_000), "2.4 GB");
    }
    #[test]
    fn final_viewport_cancellation_preserves_published_order() -> Result<(), String> {
        for superseded in [false, true] {
            let latest = AtomicU64::new(7);
            let serial = 7;
            let token = CancellationToken::new();
            let generation = token.snapshot();
            let mut displayed = vec![2, 0, 1];
            let prepared = prepare_sort_if_current(&latest, &token, serial, generation, || {
                // Cancellation arrives while the final viewport is being prepared.
                let next = vec![1, 0, 2];
                if superseded {
                    latest.store(8, Ordering::Release);
                } else {
                    token.cancel();
                }
                Ok(next)
            })?;
            if let Some(next) = prepared {
                displayed = next;
            }
            assert_eq!(displayed, vec![2, 0, 1]);
        }
        Ok(())
    }
}
