//! Hiding data columns from the view, and showing them again.

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
        assert!(ID_COLUMN_HIDE > super::super::sorting::ID_CANCEL);
        assert!(ID_COLUMN_SHOW_ALL > ID_COLUMN_HIDE);
        assert!(ID_COLUMN_HIDE_FIRST > ID_COLUMN_SHOW_ALL);
    }
}
