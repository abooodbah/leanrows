#[allow(
    clippy::wildcard_imports,
    reason = "private shell extension shares its parent's Win32 bindings"
)]
use super::*;
use leanrows_core::{SortDirection, SortMode, SortPhase, SortRequest};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Controls::{
    HDF_SORTDOWN, HDF_SORTUP, HDHITTESTINFO, HDI_FORMAT, HDITEMW, HDM_GETITEMW, HDM_HITTEST,
    HDM_SETITEMW, LVM_GETHEADER, LVN_COLUMNCLICK, NMLISTVIEW,
};
pub(super) const ID_TEXT_ASC: u16 = 410;
pub(super) const ID_TEXT_DESC: u16 = 411;
pub(super) const ID_NUMBER_ASC: u16 = 412;
pub(super) const ID_NUMBER_DESC: u16 = 413;
pub(super) const ID_SOURCE: u16 = 414;
pub(super) const ID_CANCEL: u16 = 415;
pub(super) fn next_header_sort(
    current: Option<SortRequest>,
    clicked: usize,
) -> Option<SortRequest> {
    let column = clicked.checked_sub(1)?;
    Some(match current.filter(|r| r.column == column) {
        Some(mut r) => {
            r.direction = match r.direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
            r
        }
        None => SortRequest {
            column,
            mode: SortMode::Text,
            direction: SortDirection::Ascending,
        },
    })
}
pub(super) fn handle_notification(window: HWND, lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: Every synchronous WM_NOTIFY payload starts with NMHDR.
    let notification = unsafe { &*(lparam.0 as *const NMHDR) };
    let pointer = state_pointer_for(window);
    // SAFETY: This state is owned by the UI thread and attached to the live window.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return false;
    };
    if notification.hwndFrom != state.list || notification.code != LVN_COLUMNCLICK {
        return false;
    }
    // SAFETY: LVN_COLUMNCLICK supplies an NMLISTVIEW for this list.
    let click = unsafe { &*(lparam.0 as *const NMLISTVIEW) };
    // The list shows only the visible columns, so a header's position is
    // mapped to its column in the file before sorting by it.
    let Some(column) = usize::try_from(click.iSubItem).ok().and_then(|native| {
        state
            .hidden_columns
            .file_column(native, state.columns.data_columns)
    }) else {
        return true;
    };
    state.sort_column = column;
    let current = state.sort.as_ref().and_then(|s| s.request.or(s.committed));
    submit(state, next_header_sort(current, column));
    true
}
fn submit(state: &mut WindowState, request: Option<SortRequest>) {
    let Some(worker) = &state.worker else {
        return;
    };
    let serial = worker.request_sort(request);
    let committed = state.sort.as_ref().and_then(|s| s.committed);
    state.sort = Some(crate::worker::WorkerSortState {
        serial,
        request,
        committed,
        phase: SortPhase::Extracting,
        scanned_bytes: 0,
        source_bytes: 0,
        error: None,
    });
    set_status_handle(
        state.status,
        "Preparing sort; use More > Cancel sort to stop.",
    );
}
pub(super) fn command(window: HWND, command: u16) -> bool {
    if !(ID_TEXT_ASC..=ID_CANCEL).contains(&command) {
        return false;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: Native menu commands run on the owning UI thread.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return true;
    };
    if command == ID_CANCEL {
        if let Some(worker) = &state.worker {
            worker.cancel_sort();
        }
        return true;
    }
    let request = if command == ID_SOURCE {
        None
    } else {
        let Some(column) = state.sort_column.checked_sub(1) else {
            return true;
        };
        if column >= usize::from(state.columns.data_columns) {
            return true;
        }
        Some(SortRequest {
            column,
            mode: if matches!(command, ID_NUMBER_ASC | ID_NUMBER_DESC) {
                SortMode::Numeric
            } else {
                SortMode::Text
            },
            direction: if matches!(command, ID_TEXT_DESC | ID_NUMBER_DESC) {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            },
        })
    };
    submit(state, request);
    true
}
pub(super) fn append_menu(menu: HMENU) -> Result<(), ShellError> {
    append_menu_text(menu, ID_TEXT_ASC, w!("Sort &text ascending"))?;
    append_menu_text(menu, ID_TEXT_DESC, w!("Sort text &descending"))?;
    append_menu_text(menu, ID_NUMBER_ASC, w!("Sort &numbers ascending"))?;
    append_menu_text(menu, ID_NUMBER_DESC, w!("Sort numbers d&escending"))?;
    append_menu_text(menu, ID_SOURCE, w!("Restore source &order"))?;
    append_menu_text(menu, ID_CANCEL, w!("&Cancel sort"))?;
    Ok(())
}
pub(super) fn header_menu(window: HWND, header: HWND, lparam: LPARAM) -> Result<(), ShellError> {
    let packed = lparam.0.to_le_bytes();
    let mut point = POINT {
        x: i32::from(i16::from_le_bytes([packed[0], packed[1]])),
        y: i32::from(i16::from_le_bytes([packed[2], packed[3]])),
    };
    let mut rect = RECT::default();
    // SAFETY: The HWND and stack rectangle are valid throughout this UI call.
    unsafe { GetWindowRect(header, &raw mut rect) }.map_err(menu_error)?;
    let mut column = 1;
    if point.x == -1 && point.y == -1 {
        point.x = rect.left + 20;
        point.y = rect.bottom;
    } else {
        let mut local = point;
        // SAFETY: This converts screen coordinates for the live header.
        let _ = unsafe { ScreenToClient(header, &raw mut local) };
        let mut hit = HDHITTESTINFO {
            pt: local,
            ..Default::default()
        };
        // SAFETY: The synchronous header hit test does not retain the stack pointer.
        let index = unsafe {
            SendMessageW(
                header,
                HDM_HITTEST,
                Some(WPARAM(0)),
                Some(LPARAM((&raw mut hit) as isize)),
            )
        }
        .0;
        column =
            usize::try_from(index).map_err(|_| ShellError::new("no column at this position"))?;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: UI-thread state remains attached during menu tracking.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return Ok(());
    };
    let Some(file_column) = state
        .hidden_columns
        .file_column(column, state.columns.data_columns)
    else {
        return Ok(());
    };
    state.sort_column = file_column;
    // SAFETY: The new popup is guarded until tracking returns.
    let menu = MenuGuard::new(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
    append_menu(menu.handle())?;
    super::column_view::append_header_items(menu.handle(), state, file_column)?;
    // SAFETY: All handles/coordinates are live and tracking runs synchronously.
    let selected = unsafe {
        TrackPopupMenuEx(
            menu.handle(),
            TPM_RETURNCMD.0,
            point.x,
            point.y,
            window,
            None,
        )
    };
    if let Ok(id) = u16::try_from(selected.0) {
        // The header menu also holds the column commands.
        handle_command(window, id);
    }
    Ok(())
}
pub(super) fn update_header(state: &WindowState) {
    // SAFETY: The native list is live; the synchronous message returns its header.
    let header =
        HWND(unsafe { SendMessageW(state.list, LVM_GETHEADER, None, None) }.0 as *mut c_void);
    let current = state.sort.as_ref().and_then(|s| s.committed);
    let data_columns = state.columns.data_columns;
    for column in 0..=state.hidden_columns.visible_count(data_columns) {
        let file_column = state.hidden_columns.file_column(column, data_columns);
        let mut item = HDITEMW {
            mask: HDI_FORMAT,
            ..Default::default()
        };
        // SAFETY: The bounded item index and writable stack item remain valid for both calls.
        unsafe {
            SendMessageW(
                header,
                HDM_GETITEMW,
                Some(WPARAM(column)),
                Some(LPARAM((&raw mut item) as isize)),
            );
            item.fmt.0 &= !(HDF_SORTUP.0 | HDF_SORTDOWN.0);
            if let Some(sort) = current.filter(|s| Some(s.column + 1) == file_column) {
                item.fmt.0 |= if sort.direction == SortDirection::Ascending {
                    HDF_SORTUP.0
                } else {
                    HDF_SORTDOWN.0
                };
            }
            SendMessageW(
                header,
                HDM_SETITEMW,
                Some(WPARAM(column)),
                Some(LPARAM((&raw mut item) as isize)),
            );
        }
    }
    // SAFETY: Invalidating the live header requests its existing custom painter.
    let _ = unsafe { InvalidateRect(Some(header), None, false) };
}
/// Names a sort for the status strip, marking a sorted column that is
/// hidden: its order stays, and the status is where it shows.
pub(super) fn sort_description(
    request: Option<SortRequest>,
    hidden: crate::columns::HiddenColumns,
) -> String {
    request.map_or("source order".to_owned(), |r| {
        let marker = if hidden.contains(r.column + 1) {
            " (hidden)"
        } else {
            ""
        };
        format!(
            "column {}{marker} / {:?} / {:?}",
            r.column + 1,
            r.mode,
            r.direction
        )
    })
}
pub(super) fn status(event: &WorkerEvent, hidden: crate::columns::HiddenColumns) -> Option<String> {
    let sort = event.sort.as_ref()?;
    if let Some(error) = &sort.error {
        return Some(format!("Sort failed: {error}"));
    }
    let description = sort_description(sort.request, hidden);
    Some(match sort.phase {
        SortPhase::Complete => format!(
            "Ready | {} rows | {description}",
            event.progress.available_rows
        ),
        SortPhase::Cancelled => "Sort cancelled | previous order retained".to_owned(),
        phase => format!(
            "Sorting {description} | {phase:?} | {}% | Cancel: More > Cancel sort",
            progress_percent(sort.scanned_bytes, sort.source_bytes)
        ),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_hidden_sorted_column_keeps_the_order_and_the_status_says_so() {
        let request = SortRequest {
            column: 1,
            mode: SortMode::Text,
            direction: SortDirection::Ascending,
        };
        let none = crate::columns::HiddenColumns::NONE;
        assert_eq!(
            sort_description(Some(request), none),
            "column 2 / Text / Ascending"
        );
        let mut hidden = none;
        assert!(hidden.hide(2, 3));
        assert_eq!(
            sort_description(Some(request), hidden),
            "column 2 (hidden) / Text / Ascending"
        );
        let mut other = none;
        assert!(other.hide(3, 3));
        assert_eq!(
            sort_description(Some(request), other),
            "column 2 / Text / Ascending"
        );
        assert_eq!(sort_description(None, hidden), "source order");
    }
    #[test]
    fn row_header_restores_source_and_repeated_click_keeps_numeric_mode() {
        let current = Some(SortRequest {
            column: 2,
            mode: SortMode::Numeric,
            direction: SortDirection::Ascending,
        });
        assert_eq!(next_header_sort(current, 0), None);
        assert_eq!(
            next_header_sort(current, 3),
            Some(SortRequest {
                column: 2,
                mode: SortMode::Numeric,
                direction: SortDirection::Descending
            })
        );
        assert_eq!(
            next_header_sort(current, 1),
            Some(SortRequest {
                column: 0,
                mode: SortMode::Text,
                direction: SortDirection::Ascending
            })
        );
    }
}
