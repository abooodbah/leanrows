//! Hiding data columns from the view, and showing them again.
//!
//! A hidden column is a per-tab view setting: the native list holds the Row
//! column and the shown data columns, and `HiddenColumns` maps each list
//! column to its column in the file. Nothing is written to the file and
//! nothing is kept after the tab closes or the file is reloaded.
#[allow(
    clippy::wildcard_imports,
    reason = "private shell extension shares its parent's Win32 bindings"
)]
use super::*;
use crate::columns::{HiddenColumns, MAX_HIDDEN_COLUMN};
use windows::Win32::UI::WindowsAndMessaging::MF_POPUP;

/// Hides the column whose header was right-clicked last.
pub(super) const ID_COLUMN_HIDE: u16 = 420;
pub(super) const ID_COLUMN_SHOW_ALL: u16 = 421;
/// The More menu's Hide column entries, one per data column 1 to 64.
pub(super) const ID_COLUMN_HIDE_FIRST: u16 = 430;
pub(super) const ID_COLUMN_HIDE_LAST: u16 = 493;

/// Which column items a menu enables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MenuState {
    pub(super) hide: bool,
    pub(super) show_all: bool,
}

/// The column items for a menu opened on data column `file_column`.
pub(super) fn menu_state(
    hidden: HiddenColumns,
    columns: NativeColumns,
    file_column: usize,
) -> MenuState {
    MenuState {
        hide: hidden.can_hide(file_column, columns.data_columns),
        show_all: !hidden.is_empty(),
    }
}

/// The header title of a data column: `Column 3`, or `C` in a workbook.
pub(super) fn column_title(kind: UiColumnKind, file_column: usize) -> String {
    match kind {
        UiColumnKind::Preview => "Preview".to_owned(),
        UiColumnKind::Fields => format!("Column {file_column}"),
        UiColumnKind::Cells => sheet_column_letters(u8::try_from(file_column).unwrap_or(u8::MAX)),
    }
}

pub(super) fn hidden_status(title: &str, sorted_by_it: bool) -> String {
    let mut status = format!("Hid {title} from the view. The file is unchanged.");
    if sorted_by_it {
        status.push_str(" Rows stay sorted by it.");
    }
    status
}

/// The More menu command that hides data column `file_column`.
pub(super) fn hide_command(file_column: usize) -> Option<u16> {
    if file_column == 0 || file_column > MAX_HIDDEN_COLUMN {
        return None;
    }
    let offset = u16::try_from(file_column - 1).ok()?;
    Some(ID_COLUMN_HIDE_FIRST + offset)
}

/// The data column a More menu Hide command names.
pub(super) fn hide_command_column(command: u16) -> Option<usize> {
    (ID_COLUMN_HIDE_FIRST..=ID_COLUMN_HIDE_LAST)
        .contains(&command)
        .then(|| usize::from(command - ID_COLUMN_HIDE_FIRST) + 1)
}

/// Handles the column commands; returns false for any other command.
pub(super) fn command(window: HWND, command: u16) -> bool {
    let file_column = match command {
        ID_COLUMN_HIDE | ID_COLUMN_SHOW_ALL => None,
        _ => match hide_command_column(command) {
            Some(column) => Some(column),
            None => return false,
        },
    };
    let pointer = state_pointer_for(window);
    // SAFETY: Menu commands run on the UI thread that owns the state.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return true;
    };
    let result = if command == ID_COLUMN_SHOW_ALL {
        show_all(state)
    } else {
        hide(state, file_column.unwrap_or(state.sort_column))
    };
    let status = result.unwrap_or_else(|error| error.to_string());
    set_status_handle(state.status, &status);
    true
}

fn hide(state: &mut WindowState, file_column: usize) -> Result<String, ShellError> {
    let Some(kind) = state.columns.kind else {
        return Err(ShellError::new("Open a file first."));
    };
    let mut hidden = state.hidden_columns;
    if !hidden.hide(file_column, state.columns.data_columns) {
        return Err(ShellError::new(refusal(
            state.hidden_columns,
            state.columns,
            file_column,
        )));
    }
    apply(state, hidden)?;
    let sorted_by_it = state
        .sort
        .as_ref()
        .and_then(|sort| sort.committed)
        .is_some_and(|request| request.column + 1 == file_column);
    Ok(hidden_status(
        &column_title(kind, file_column),
        sorted_by_it,
    ))
}

fn refusal(hidden: HiddenColumns, columns: NativeColumns, file_column: usize) -> &'static str {
    if file_column == 0 || file_column > usize::from(columns.data_columns) {
        "Only data columns can be hidden."
    } else if hidden.contains(file_column) {
        "That column is already hidden."
    } else {
        "The last visible column cannot be hidden."
    }
}

fn show_all(state: &mut WindowState) -> Result<String, ShellError> {
    if state.hidden_columns.is_empty() {
        return Ok("Every column is shown.".to_owned());
    }
    apply(state, HiddenColumns::NONE)?;
    // Rows fetched while columns were hidden do not hold them. They show as
    // loading until the worker sends the rows again.
    if !state.cache.skipped().is_empty()
        && let (Some(worker), Some(serial)) = (state.worker.as_ref(), state.active_serial)
    {
        let top = top_visible_row(state).unwrap_or_else(|| state.rows.first_row());
        let _ = worker.request_viewport(serial, top);
    }
    Ok(format!(
        "Showing all {} columns.",
        state.columns.data_columns
    ))
}

/// Rebuilds the list's data columns for `hidden`, keeping the width of each
/// column that stays, and tells the worker which fields to skip.
fn apply(state: &mut WindowState, hidden: HiddenColumns) -> Result<(), ShellError> {
    let before = state.hidden_columns;
    let data_columns = state.columns.data_columns;
    let mut widths_by_file = [0_i32; MAX_HIDDEN_COLUMN + 1];
    for (native, width) in column_widths(state.list).into_iter().enumerate() {
        if let Some(slot) = before
            .file_column(native, data_columns)
            .and_then(|file| widths_by_file.get_mut(file))
        {
            *slot = width;
        }
    }
    let list = state.list;
    let mut result = Ok(());
    with_list_redraw_off(list, || {
        result = rebuild(state, hidden, &widths_by_file);
    });
    // SAFETY: The list is live; this repaints it with the new columns.
    let _ = unsafe { InvalidateRect(Some(list), None, true) };
    result
}

fn rebuild(
    state: &mut WindowState,
    hidden: HiddenColumns,
    widths_by_file: &[i32],
) -> Result<(), ShellError> {
    remove_data_columns(state.list)?;
    state.hidden_columns = hidden;
    insert_existing_data_columns(
        state.list,
        state.columns,
        hidden,
        state.theme.zoom,
        &mut state.column_zoom,
    )
    .map_err(|()| ShellError::new("native data-column rebuild failed"))?;
    let data_columns = state.columns.data_columns;
    let widths: Vec<i32> = (0..=hidden.visible_count(data_columns))
        .map(|native| {
            hidden
                .file_column(native, data_columns)
                .and_then(|file| widths_by_file.get(file))
                .copied()
                .unwrap_or_default()
        })
        .collect();
    set_column_widths(state.list, &widths);
    if let Some(worker) = &state.worker {
        worker.set_hidden_columns(hidden);
    }
    sorting::update_header(state);
    Ok(())
}

fn append_column_item(
    menu: HMENU,
    command: u16,
    text: &str,
    enabled: bool,
) -> Result<(), ShellError> {
    let mut encoded: Vec<u16> = text.encode_utf16().filter(|unit| *unit != 0).collect();
    encoded.push(0);
    append_menu_item(menu, command, PCWSTR(encoded.as_ptr()), enabled)
}

/// Adds Hide column and Show all columns to a header's context menu.
pub(super) fn append_header_items(
    menu: HMENU,
    state: &WindowState,
    file_column: usize,
) -> Result<(), ShellError> {
    let items = menu_state(state.hidden_columns, state.columns, file_column);
    // SAFETY: The caller owns the popup until tracking returns.
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, None) }.map_err(menu_error)?;
    append_column_item(menu, ID_COLUMN_HIDE, "&Hide column", items.hide)?;
    append_column_item(
        menu,
        ID_COLUMN_SHOW_ALL,
        "Sho&w all columns",
        items.show_all,
    )
}

/// Adds a Hide column submenu that lists each shown column, and Show all
/// columns, to the More menu, so both work from the keyboard.
pub(super) fn append_overflow_items(menu: HMENU, state: &WindowState) -> Result<(), ShellError> {
    let columns = state.columns;
    let hidden = state.hidden_columns;
    // SAFETY: The new popup stays under this guard until the parent owns it.
    let submenu = MenuGuard::new(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
    let mut any = false;
    if let Some(kind) = columns.kind {
        for file_column in hidden.visible(columns.data_columns) {
            let Some(id) = hide_command(file_column) else {
                continue;
            };
            let enabled = hidden.can_hide(file_column, columns.data_columns);
            any |= enabled;
            append_column_item(
                submenu.handle(),
                id,
                &column_title(kind, file_column),
                enabled,
            )?;
        }
    }
    let flags = if any { MF_POPUP } else { MF_POPUP | MF_GRAYED };
    // SAFETY: The parent takes ownership of the submenu once this succeeds.
    unsafe { AppendMenuW(menu, flags, submenu.handle().0.addr(), w!("Hide co&lumn")) }
        .map_err(menu_error)?;
    submenu.release();
    append_column_item(
        menu,
        ID_COLUMN_SHOW_ALL,
        "Sho&w all columns",
        !hidden.is_empty(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ID_COLUMN_HIDE, ID_COLUMN_HIDE_FIRST, ID_COLUMN_HIDE_LAST, ID_COLUMN_SHOW_ALL, MenuState,
        column_title, hidden_status, hide_command, hide_command_column, menu_state,
    };
    use crate::columns::HiddenColumns;
    use crate::document_engine::UiColumnKind;
    use crate::native::NativeColumns;

    const FIELDS: NativeColumns = NativeColumns {
        kind: Some(UiColumnKind::Fields),
        data_columns: 3,
    };

    #[test]
    fn hide_is_offered_only_for_a_data_column_that_is_not_the_last_one_shown() {
        let none = HiddenColumns::NONE;
        let offered = MenuState {
            hide: true,
            show_all: false,
        };
        assert_eq!(menu_state(none, FIELDS, 2), offered);
        assert_eq!(menu_state(none, FIELDS, 3), offered);
        // The Row column, a column past the data, and one-column views.
        let nothing = MenuState {
            hide: false,
            show_all: false,
        };
        assert_eq!(menu_state(none, FIELDS, 0), nothing);
        assert_eq!(menu_state(none, FIELDS, 4), nothing);
        assert_eq!(menu_state(none, NativeColumns::preview(), 1), nothing);
        let cells = NativeColumns {
            kind: Some(UiColumnKind::Cells),
            data_columns: 2,
        };
        assert_eq!(menu_state(none, cells, 1), offered);

        let mut hidden = none;
        assert!(hidden.hide(1, 3));
        assert_eq!(
            menu_state(hidden, FIELDS, 2),
            MenuState {
                hide: true,
                show_all: true,
            }
        );
        assert!(hidden.hide(2, 3));
        assert_eq!(
            menu_state(hidden, FIELDS, 3),
            MenuState {
                hide: false,
                show_all: true,
            }
        );
    }

    #[test]
    fn messages_use_the_header_title_and_mention_the_sort() {
        assert_eq!(column_title(UiColumnKind::Fields, 3), "Column 3");
        assert_eq!(column_title(UiColumnKind::Cells, 28), "AB");
        assert_eq!(column_title(UiColumnKind::Preview, 1), "Preview");
        assert_eq!(
            hidden_status("Column 3", false),
            "Hid Column 3 from the view. The file is unchanged."
        );
        assert_eq!(
            hidden_status("C", true),
            "Hid C from the view. The file is unchanged. Rows stay sorted by it."
        );
    }

    #[test]
    fn overflow_menu_ids_cover_each_hideable_column_and_stay_clear_of_others() {
        assert_eq!(hide_command(1), Some(ID_COLUMN_HIDE_FIRST));
        assert_eq!(hide_command(64), Some(ID_COLUMN_HIDE_LAST));
        assert_eq!(hide_command(0), None);
        assert_eq!(hide_command(65), None);
        assert_eq!(hide_command_column(ID_COLUMN_HIDE_FIRST + 2), Some(3));
        assert_eq!(hide_command_column(ID_COLUMN_HIDE_LAST), Some(64));
        assert_eq!(hide_command_column(ID_COLUMN_HIDE_LAST + 1), None);
        assert_eq!(hide_command_column(ID_COLUMN_SHOW_ALL), None);
        // Above the sort commands (410 to 415); the accelerator table ends at 200.
        const {
            assert!(ID_COLUMN_HIDE > super::super::sorting::ID_CANCEL);
            assert!(ID_COLUMN_SHOW_ALL > ID_COLUMN_HIDE);
            assert!(ID_COLUMN_HIDE_FIRST > ID_COLUMN_SHOW_ALL);
        }
    }
}
