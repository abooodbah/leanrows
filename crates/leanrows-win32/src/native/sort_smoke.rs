//! Real native control smoke; no mock sorter or separate grid.
#[allow(
    clippy::wildcard_imports,
    reason = "native workflow smoke deliberately uses the actual shell functions"
)]
use super::*;
use leanrows_core::{SortDirection, SortMode, SortPhase};
use std::time::{Duration, Instant};
use windows::Win32::UI::Controls::{HDM_GETITEMRECT, LVM_GETITEMTEXTW};
use windows::Win32::UI::WindowsAndMessaging::{
    PM_REMOVE, PeekMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_QUIT,
};
fn wait(window: HWND, predicate: impl Fn(&WindowState) -> bool) -> Result<(), ShellError> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let pointer = state_pointer_for(window);
        // SAFETY: The state remains attached on this thread until smoke cleanup.
        let state = unsafe { pointer.as_ref() }
            .ok_or_else(|| ShellError::new("smoke window was destroyed"))?;
        if predicate(state) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(ShellError::new(format!(
                "sort smoke timed out: {}",
                control_text(state.status)
            )));
        }
        let mut message = MSG::default();
        // SAFETY: The stack message is writable and this pumps the owning UI queue.
        while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                return Err(ShellError::new("smoke window exited early"));
            }
            // SAFETY: PeekMessage supplied a valid message to the owning thread.
            unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn click(window: HWND, column: usize) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: The live UI state supplies its list handle.
    let state = unsafe { pointer.as_ref() }.ok_or_else(|| ShellError::new("no smoke state"))?;
    // SAFETY: This synchronous message returns the list's live header.
    let header =
        HWND(unsafe { SendMessageW(state.list, LVM_GETHEADER, None, None) }.0 as *mut c_void);
    let mut rect = RECT::default();
    // SAFETY: The item rectangle is writable and retained only for this call.
    unsafe {
        SendMessageW(
            header,
            HDM_GETITEMRECT,
            Some(WPARAM(column)),
            Some(LPARAM((&raw mut rect) as isize)),
        )
    };
    let x = rect.left.midpoint(rect.right);
    let y = rect.top.midpoint(rect.bottom);
    let packed = (y << 16) | (x & 0xffff);
    // SAFETY: Scalar mouse messages exercise the original native header hit testing.
    unsafe {
        SendMessageW(
            header,
            WM_LBUTTONDOWN,
            Some(WPARAM(1)),
            Some(LPARAM(packed as isize)),
        );
        SendMessageW(
            header,
            WM_LBUTTONUP,
            Some(WPARAM(0)),
            Some(LPARAM(packed as isize)),
        );
    }
    Ok(())
}
fn command(window: HWND, command: u16) {
    // SAFETY: The scalar command follows the same native menu-command path.
    unsafe { SendMessageW(window, WM_COMMAND, Some(WPARAM(usize::from(command))), None) };
}
fn expect_order(window: HWND, expected: [u64; 4]) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Synchronous smoke inspection runs on the list's owning thread.
    let state = unsafe { pointer.as_ref() }.ok_or_else(|| ShellError::new("no smoke state"))?;
    for (rank, row) in expected.into_iter().enumerate() {
        let mut text = [0_u16; 32];
        let mut item = LVITEMW {
            iSubItem: 0,
            pszText: PWSTR(text.as_mut_ptr()),
            cchTextMax: 32,
            ..Default::default()
        };
        // SAFETY: The same-process owner-data callback fills this bounded stack buffer.
        unsafe {
            SendMessageW(
                state.list,
                LVM_GETITEMTEXTW,
                Some(WPARAM(rank)),
                Some(LPARAM((&raw mut item) as isize)),
            )
        };
        let len = text.iter().position(|u| *u == 0).unwrap_or(text.len());
        let actual =
            String::from_utf16(&text[..len]).map_err(|e| ShellError::new(e.to_string()))?;
        if actual != (row + 1).to_string() {
            return Err(ShellError::new(format!(
                "native sorted row {rank}: expected {}, got {actual}",
                row + 1
            )));
        }
    }
    Ok(())
}
fn sorted(
    window: HWND,
    mode: SortMode,
    direction: SortDirection,
    column: usize,
) -> Result<(), ShellError> {
    wait(window, |s| {
        s.sort.as_ref().is_some_and(|state| {
            state.phase == SortPhase::Complete
                && state.committed.is_some_and(|r| {
                    r.mode == mode && r.direction == direction && r.column == column
                })
        })
    })
    .map_err(|e| {
        ShellError::new(format!(
            "waiting for {mode:?} {direction:?} column {column}: {e}"
        ))
    })
}
#[allow(
    clippy::too_many_lines,
    reason = "one ordered native workflow checks sort, navigation and tab state"
)]
pub(super) fn run(window: HWND, paths: Vec<PathBuf>) -> Result<(), ShellError> {
    let path = paths
        .into_iter()
        .next()
        .ok_or_else(|| ShellError::new("sort smoke needs a fixture"))?;
    open_document(window, path.clone())?;
    wait(window, |s| {
        s.rows.total_rows() == 4 && s.cache.cell(3, 0).is_some()
    })?;
    click(window, 1)?;
    sorted(window, SortMode::Text, SortDirection::Ascending, 0)?;
    expect_order(window, [1, 3, 2, 0])?;
    click(window, 1)?;
    sorted(window, SortMode::Text, SortDirection::Descending, 0)?;
    expect_order(window, [0, 2, 3, 1])?;
    {
        let pointer = state_pointer_for(window);
        // SAFETY: No message is dispatched while this mutable reference is held.
        let state = unsafe { pointer.as_mut() }.ok_or_else(|| ShellError::new("no smoke state"))?;
        state.sort_column = 2;
    }
    command(window, sorting::ID_NUMBER_ASC);
    sorted(window, SortMode::Numeric, SortDirection::Ascending, 1)?;
    expect_order(window, [1, 0, 3, 2])?;
    {
        let pointer = state_pointer_for(window);
        // SAFETY: Navigation is submitted on the owning UI thread.
        let state = unsafe { pointer.as_mut() }.ok_or_else(|| ShellError::new("no smoke state"))?;
        if begin_reveal(state, 4).is_ok() {
            return Err(ShellError::new("out-of-range source row was accepted"));
        }
        begin_reveal(state, 2)?;
    }
    wait(window, |s| first_selected_row(s) == Some(3))?;
    if copy_selected_rows(window)?.0 != 1 {
        return Err(ShellError::new("sorted copy selected incorrect rows"));
    }
    {
        let pointer = state_pointer_for(window);
        // SAFETY: The real inline Find workflow owns this short-lived state access.
        let state = unsafe { pointer.as_mut() }.ok_or_else(|| ShellError::new("no smoke state"))?;
        start_or_navigate_find(state, b"z,10".to_vec(), true, QueryDirection::Next)?;
    }
    wait(window, |s| {
        s.revealed_match.is_some() && first_selected_row(s) == Some(1)
    })?;
    // Restore the viewport so all four owner-data rows can be inspected.
    {
        let pointer = state_pointer_for(window);
        // SAFETY: The UI owns the worker handle and document serial.
        let state = unsafe { pointer.as_mut() }.ok_or_else(|| ShellError::new("no smoke state"))?;
        state.active_find = None;
        state.revealed_match = None;
        if let Some(worker) = &state.worker {
            worker.request_viewport(state.active_serial.unwrap_or(0), 0);
        }
    }
    wait(window, |s| s.cache.cell(0, 0).is_some())?;
    let secondary = path.with_extension("sort-tab.log");
    open_document(window, secondary.clone())?;
    wait(window, |s| {
        s.rows.total_rows() == 1 && s.cache.cell(0, 0).is_some()
    })?;
    open_document(window, path.clone())?;
    expect_order(window, [1, 0, 3, 2])?;
    command(window, sorting::ID_NUMBER_DESC);
    sorted(window, SortMode::Numeric, SortDirection::Descending, 1)?;
    expect_order(window, [2, 3, 0, 1])?;
    command(window, sorting::ID_SOURCE);
    wait(window, |s| {
        s.sort
            .as_ref()
            .is_some_and(|v| v.phase == SortPhase::Complete && v.committed.is_none())
    })?;
    expect_order(window, [0, 1, 2, 3])?;
    {
        let pointer = state_pointer_for(window);
        // SAFETY: The smoke owns this UI-thread state and selects an existing rank.
        let state = unsafe { pointer.as_ref() }.ok_or_else(|| ShellError::new("no smoke state"))?;
        select_absolute_row(state, 3)?;
    }
    let background_coordinator = crate::sort_coordinator::SortCoordinator::shared();
    let background_gate = background_coordinator
        .try_acquire(background_coordinator.request())
        .ok_or_else(|| ShellError::new("no background scheduling gate"))?;
    click(window, 1)?;
    open_document(window, secondary)?;
    drop(background_gate);
    wait(window, |s| {
        s.background_tabs.iter().any(|t| {
            t.path.as_ref() == Some(&path)
                && t.sort
                    .as_ref()
                    .is_some_and(|v| v.phase == SortPhase::Complete)
        })
    })?;
    open_document(window, path)?;
    expect_order(window, [1, 3, 2, 0])?;
    {
        let pointer = state_pointer_for(window);
        // SAFETY: Selection inspection runs synchronously on the UI thread.
        let state = unsafe { pointer.as_ref() }.ok_or_else(|| ShellError::new("no smoke state"))?;
        if first_selected_row(state).is_some() {
            return Err(ShellError::new(
                "background sort retained a selection for the wrong source row",
            ));
        }
    }
    command(window, sorting::ID_SOURCE);
    wait(window, |s| {
        s.sort
            .as_ref()
            .is_some_and(|v| v.phase == SortPhase::Complete && v.committed.is_none())
    })?;
    expect_order(window, [0, 1, 2, 3])?;
    // Hold the actual process permit to exercise cancellation while another tab
    // still owns sorting resources, without racing a four-row sort's completion.
    let coordinator = crate::sort_coordinator::SortCoordinator::shared();
    let held = coordinator
        .try_acquire(coordinator.request())
        .ok_or_else(|| ShellError::new("sort smoke could not reserve its scheduling gate"))?;
    click(window, 1)?;
    command(window, sorting::ID_CANCEL);
    wait(window, |s| {
        s.sort
            .as_ref()
            .is_some_and(|v| v.phase == SortPhase::Cancelled)
    })?;
    expect_order(window, [0, 1, 2, 3])?;
    drop(held);
    Ok(())
}
