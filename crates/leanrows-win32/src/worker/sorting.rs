#[allow(
    clippy::wildcard_imports,
    reason = "private worker extension shares the worker's document and mailbox types"
)]
use super::*;
pub(super) fn default_sort_directory() -> io::Result<PathBuf> {
    use std::sync::OnceLock;
    static DIRECTORY: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let base =
                std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
            let directory = base.join("LeanRows").join("sort-cache");
            fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            leanrows_core::recover_stale_sort_files(&directory).map_err(|e| e.to_string())?;
            Ok(directory)
        })
        .clone()
        .map_err(io::Error::other)
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
    if !runtime.coordinator.is_current(active.serial)
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
                .query_plan()
                .map_err(|e| e.to_string())?
                .into_parts();
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
                &runtime.sort_directory,
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
    if !runtime.coordinator.is_current(active.serial)
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
        &runtime.coordinator,
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
fn prepare_sort_if_current<T>(
    coordinator: &SortCoordinator,
    cancellation: &CancellationToken,
    serial: u64,
    generation: CancellationGeneration,
    prepare: impl FnOnce() -> Result<T, String>,
) -> Result<Option<T>, String> {
    if !coordinator.is_current(serial) || cancellation.is_cancelled(generation) {
        return Ok(None);
    }
    let prepared = prepare()?;
    if !coordinator.is_current(serial) || cancellation.is_cancelled(generation) {
        return Ok(None);
    }
    Ok(Some(prepared))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_viewport_cancellation_preserves_published_order() -> Result<(), String> {
        for superseded in [false, true] {
            let coordinator = SortCoordinator::new();
            let serial = coordinator.request();
            let token = CancellationToken::new();
            let generation = token.snapshot();
            let mut displayed = vec![2, 0, 1];
            let prepared =
                prepare_sort_if_current(&coordinator, &token, serial, generation, || {
                    // Cancellation arrives while the final viewport is being prepared.
                    let next = vec![1, 0, 2];
                    if superseded {
                        coordinator.request();
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
