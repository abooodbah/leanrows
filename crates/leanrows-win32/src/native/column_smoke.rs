//! Real native workflow for hidden columns, run at the end of the sort smoke
//! on the `.columns.csv` file beside the sort fixture. It drives the actual
//! header, menu commands, worker and list, with no mock grid. The fixture
//! has four columns: a name, a word only one row holds, the sort smoke's
//! numbers, and a constant tag.
use super::sort_smoke::{click, command, expect_order, sorted, wait};
#[allow(
    clippy::wildcard_imports,
    reason = "native workflow smoke deliberately uses the actual shell functions"
)]
use super::*;
use leanrows_core::{SortDirection, SortMode};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Controls::{
    HDF_SORTDOWN, HDF_SORTUP, HDI_FORMAT, HDI_TEXT, HDITEMW, HDM_GETITEMRECT, HDM_GETITEMW,
    LVM_GETCOLUMNWIDTH, LVM_GETITEMTEXTW,
};
use windows::Win32::UI::WindowsAndMessaging::{EndMenu, WM_CONTEXTMENU};

const END_MENU_TIMER_ID: usize = 0x4c52_434d;

fn state_mut(window: HWND) -> Result<&'static mut WindowState, ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: The smoke runs on the owning UI thread, and no message is
    // dispatched while the caller holds this short-lived reference.
    unsafe { pointer.as_mut() }.ok_or_else(|| ShellError::new("no smoke state"))
}

fn header_titles(window: HWND) -> Result<Vec<String>, ShellError> {
    let state = state_mut(window)?;
    let header = list_header(state.list).ok_or_else(|| ShellError::new("no list header"))?;
    let count = header_column_count(state.list).unwrap_or_default();
    let mut titles = Vec::new();
    for index in 0..usize::try_from(count).unwrap_or_default() {
        let mut text = [0_u16; 64];
        let mut item = HDITEMW {
            mask: HDI_TEXT,
            pszText: PWSTR(text.as_mut_ptr()),
            cchTextMax: 64,
            ..Default::default()
        };
        // SAFETY: The header copies at most 64 units into this stack buffer.
        unsafe {
            SendMessageW(
                header,
                HDM_GETITEMW,
                Some(WPARAM(index)),
                Some(LPARAM((&raw mut item) as isize)),
            )
        };
        let len = text.iter().position(|u| *u == 0).unwrap_or(text.len());
        titles.push(String::from_utf16_lossy(&text[..len]));
    }
    Ok(titles)
}

fn expect_titles(window: HWND, expected: &[&str]) -> Result<(), ShellError> {
    let titles = header_titles(window)?;
    if titles != expected {
        return Err(ShellError::new(format!(
            "column headers: expected {expected:?}, got {titles:?}"
        )));
    }
    Ok(())
}

fn item_text(window: HWND, rank: usize, native_column: i32) -> Result<String, ShellError> {
    let state = state_mut(window)?;
    let mut text = [0_u16; 64];
    let mut item = LVITEMW {
        iSubItem: native_column,
        pszText: PWSTR(text.as_mut_ptr()),
        cchTextMax: 64,
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
    Ok(String::from_utf16_lossy(&text[..len]))
}

fn expect_item(window: HWND, rank: usize, native: i32, expected: &str) -> Result<(), ShellError> {
    let actual = item_text(window, rank, native)?;
    if actual != expected {
        return Err(ShellError::new(format!(
            "row {rank} header {native}: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

/// Hides a column through the header menu's command, as a right-click on
/// that header would.
fn hide_from_header(window: HWND, file_column: usize) -> Result<(), ShellError> {
    state_mut(window)?.sort_column = file_column;
    command(window, column_view::ID_COLUMN_HIDE);
    Ok(())
}

/// The list columns whose header shows a sort arrow.
fn sort_arrows(window: HWND) -> Result<Vec<usize>, ShellError> {
    let state = state_mut(window)?;
    let header = list_header(state.list).ok_or_else(|| ShellError::new("no list header"))?;
    let count = header_column_count(state.list).unwrap_or_default();
    let mut arrows = Vec::new();
    for index in 0..usize::try_from(count).unwrap_or_default() {
        let mut item = HDITEMW {
            mask: HDI_FORMAT,
            ..Default::default()
        };
        // SAFETY: The header fills this stack item synchronously.
        unsafe {
            SendMessageW(
                header,
                HDM_GETITEMW,
                Some(WPARAM(index)),
                Some(LPARAM((&raw mut item) as isize)),
            )
        };
        if item.fmt.0 & (HDF_SORTUP.0 | HDF_SORTDOWN.0) != 0 {
            arrows.push(index);
        }
    }
    Ok(arrows)
}

fn column_width(window: HWND, native: usize) -> Result<isize, ShellError> {
    let state = state_mut(window)?;
    // SAFETY: LVM_GETCOLUMNWIDTH carries a column index and no pointers.
    Ok(unsafe { SendMessageW(state.list, LVM_GETCOLUMNWIDTH, Some(WPARAM(native)), None) }.0)
}

unsafe extern "system" fn end_menu(_: HWND, _: u32, _: usize, _: u32) {
    // SAFETY: Ends the menu this thread is tracking, if any.
    let _ = unsafe { EndMenu() };
}

/// Right-clicks list column `native` through the header's real hit test.
/// A timer closes the menu if it opens, so no command is chosen.
fn right_click_header(window: HWND, native: usize) -> Result<(), ShellError> {
    let state = state_mut(window)?;
    let header = list_header(state.list).ok_or_else(|| ShellError::new("no list header"))?;
    let mut rect = RECT::default();
    // SAFETY: The item rectangle is writable and retained only for this call.
    unsafe {
        SendMessageW(
            header,
            HDM_GETITEMRECT,
            Some(WPARAM(native)),
            Some(LPARAM((&raw mut rect) as isize)),
        )
    };
    let mut point = POINT {
        x: rect.left.midpoint(rect.right),
        y: rect.top.midpoint(rect.bottom),
    };
    // SAFETY: The header is live and the point is a writable stack value.
    let _ = unsafe { ClientToScreen(header, &raw mut point) };
    let packed = ((point.y & 0xffff) << 16) | (point.x & 0xffff);
    // SAFETY: The timer belongs to this window and is killed below; the
    // context-menu message carries only packed screen coordinates.
    unsafe {
        SetTimer(Some(window), END_MENU_TIMER_ID, 200, Some(end_menu));
        SendMessageW(
            header,
            WM_CONTEXTMENU,
            Some(WPARAM(header.0.addr())),
            Some(LPARAM(packed as isize)),
        );
        let _ = KillTimer(Some(window), END_MENU_TIMER_ID);
    }
    Ok(())
}

fn expect_status(window: HWND, fragment: &str) -> Result<(), ShellError> {
    let status = control_text(state_mut(window)?.status);
    if !status.contains(fragment) {
        return Err(ShellError::new(format!(
            "status {status:?} does not say {fragment:?}"
        )));
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "one ordered native workflow checks hiding, sorting, copy, tabs, find and showing"
)]
pub(super) fn run(window: HWND, sort_fixture: &Path, path: &Path) -> Result<(), ShellError> {
    open_document(window, path.to_path_buf())?;
    wait(window, |s| {
        s.rows.total_rows() == 4 && s.cache.cell(3, 0).is_some()
    })?;
    expect_titles(
        window,
        &["Row", "Column 1", "Column 2", "Column 3", "Column 4"],
    )?;

    // A middle column: the headers after it move left and keep their data,
    // and a width set by hand stays with its column.
    {
        let state = state_mut(window)?;
        set_column_widths(state.list, &[0, 151]);
    }
    hide_from_header(window, 2)?;
    expect_titles(window, &["Row", "Column 1", "Column 3", "Column 4"])?;
    expect_item(window, 0, 1, "z")?;
    expect_item(window, 0, 2, "10")?;
    expect_item(window, 0, 3, "keep")?;
    expect_status(window, "Hid Column 2 from the view. The file is unchanged.")?;
    if column_width(window, 1)? != 151 {
        return Err(ShellError::new("hiding a column lost another's width"));
    }
    // Rows fetched from now on skip the hidden field.
    {
        let state = state_mut(window)?;
        if let (Some(worker), Some(serial)) = (state.worker.as_ref(), state.active_serial) {
            worker.request_viewport(serial, 0);
        }
    }
    wait(window, |s| s.cache.skipped().contains(2))?;
    expect_item(window, 0, 2, "10")?;
    // A right-click on the second data header picks file column 3.
    right_click_header(window, 2)?;
    if state_mut(window)?.sort_column != 3 {
        return Err(ShellError::new(
            "a right-click on a shifted header picked the wrong column",
        ));
    }

    // A click on the second data header sorts file column 3, not column 2:
    // text order of 10, 2, ...993, ...992 is rows 1, 2, 4, 3.
    click(window, 2)?;
    sorted(window, SortMode::Text, SortDirection::Ascending, 2)?;
    expect_order(window, [0, 1, 3, 2])?;
    expect_item(window, 1, 2, "2")?;
    if sort_arrows(window)? != [2] {
        return Err(ShellError::new(
            "the sort arrow is not on the sorted header",
        ));
    }

    // Hiding the sorted column keeps the order.
    command(window, column_view::ID_COLUMN_HIDE_FIRST + 2);
    expect_titles(window, &["Row", "Column 1", "Column 4"])?;
    expect_status(window, "Rows stay sorted by it.")?;
    expect_order(window, [0, 1, 3, 2])?;
    if !sort_arrows(window)?.is_empty() {
        return Err(ShellError::new("a sort arrow moved to another header"));
    }
    {
        let state = state_mut(window)?;
        let committed = state.sort.as_ref().and_then(|s| s.committed);
        if committed.map(|r| r.column) != Some(2) {
            return Err(ShellError::new("hiding the sorted column changed the sort"));
        }
        let copied = build_copy_text(
            state.cache.as_ref(),
            state.rows,
            state.columns,
            state.hidden_columns,
            &[0, 1],
        )?;
        let expected: Vec<u16> = "1\tz\tkeep\r\n2\ta\tkeep\0".encode_utf16().collect();
        if copied != expected {
            return Err(ShellError::new(format!(
                "copy with hidden columns: {}",
                String::from_utf16_lossy(&copied)
            )));
        }
    }

    // The last visible column stays.
    hide_from_header(window, 1)?;
    expect_titles(window, &["Row", "Column 4"])?;
    hide_from_header(window, 4)?;
    expect_titles(window, &["Row", "Column 4"])?;
    expect_status(window, "last visible column")?;

    // Each tab keeps its own hidden columns.
    open_document(window, sort_fixture.to_path_buf())?;
    expect_titles(window, &["Row", "Column 1", "Column 2"])?;
    open_document(window, path.to_path_buf())?;
    expect_titles(window, &["Row", "Column 4"])?;
    expect_order(window, [0, 1, 3, 2])?;

    // Find still matches text in hidden columns: "second" is only in column 2.
    start_or_navigate_find(
        state_mut(window)?,
        b"second".to_vec(),
        true,
        QueryDirection::Next,
    )?;
    wait(window, |s| {
        s.revealed_match.is_some() && first_selected_row(s) == Some(1)
    })?;
    expect_status(window, "hidden columns are searched too")?;

    // Show all columns brings back every header and refetches their cells.
    command(window, column_view::ID_COLUMN_SHOW_ALL);
    expect_titles(
        window,
        &["Row", "Column 1", "Column 2", "Column 3", "Column 4"],
    )?;
    wait(window, |s| {
        s.cache.skipped().is_empty() && s.cache.cell(1, 2).is_some()
    })?;
    expect_item(window, 1, 2, "second")?;
    expect_item(window, 1, 3, "2")?;

    // Reloading the file shows every column again.
    hide_from_header(window, 2)?;
    expect_titles(window, &["Row", "Column 1", "Column 3", "Column 4"])?;
    command(window, ID_FILE_RELOAD);
    wait(window, |s| {
        s.rows.total_rows() == 4 && s.cache.cell(3, 0).is_some()
    })?;
    expect_titles(
        window,
        &["Row", "Column 1", "Column 2", "Column 3", "Column 4"],
    )?;
    expect_item(window, 0, 2, "first")?;
    Ok(())
}
