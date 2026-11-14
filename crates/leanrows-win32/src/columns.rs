//! Data columns hidden from one tab's view.

#[cfg(test)]
mod tests {
    use super::{HiddenColumns, MAX_HIDDEN_COLUMN};

    fn hidden(columns: &[usize], data_columns: u8) -> HiddenColumns {
        let mut set = HiddenColumns::NONE;
        for column in columns {
            assert!(set.hide(*column, data_columns), "column {column} hides");
        }
        set
    }

    #[test]
    fn a_hidden_middle_column_maps_headers_to_the_right_file_columns() {
        let set = hidden(&[3], 5);
        // The Row column stays first and is never a data column.
        assert_eq!(set.file_column(0, 5), Some(0));
        assert_eq!(set.file_column(1, 5), Some(1));
        assert_eq!(set.file_column(2, 5), Some(2));
        assert_eq!(set.file_column(3, 5), Some(4));
        assert_eq!(set.file_column(4, 5), Some(5));
        assert_eq!(set.file_column(5, 5), None);
        assert_eq!(set.native_column(0, 5), Some(0));
        assert_eq!(set.native_column(2, 5), Some(2));
        assert_eq!(set.native_column(3, 5), None);
        assert_eq!(set.native_column(4, 5), Some(3));
        assert_eq!(set.native_column(5, 5), Some(4));
        assert_eq!(set.native_column(6, 5), None);
        assert_eq!(set.visible(5).collect::<Vec<_>>(), [1, 2, 4, 5]);
        assert_eq!(set.visible_count(5), 4);
        assert!(set.contains(3));
        assert!(!set.contains(2));
        assert!(!set.contains(0));
    }

    #[test]
    fn several_hidden_columns_including_both_ends_keep_the_order() {
        let set = hidden(&[5, 1, 3], 6);
        assert_eq!(set.visible(6).collect::<Vec<_>>(), [2, 4, 6]);
        assert_eq!(set.file_column(1, 6), Some(2));
        assert_eq!(set.file_column(2, 6), Some(4));
        assert_eq!(set.file_column(3, 6), Some(6));
        assert_eq!(set.file_column(4, 6), None);
        for file in [2, 4, 6] {
            let native = set.native_column(file, 6);
            assert_eq!(native.and_then(|n| set.file_column(n, 6)), Some(file));
        }
        assert_eq!(set.count(), 3);
    }

    #[test]
    fn the_last_visible_column_the_row_column_and_unknown_columns_cannot_be_hidden() {
        let mut set = HiddenColumns::NONE;
        // A one-column view (text, log, JSONL preview, or a one-field CSV).
        assert!(!set.can_hide(1, 1));
        assert!(!set.hide(1, 1));
        assert!(!set.can_hide(0, 3));
        assert!(!set.can_hide(4, 3));
        assert!(set.hide(1, 2));
        assert!(!set.can_hide(2, 2));
        assert!(!set.hide(2, 2));
        // Hiding a hidden column again changes nothing.
        assert!(!set.hide(1, 2));
        assert_eq!(set, hidden(&[1], 2));
        assert_eq!(set.visible(2).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn the_sixty_fourth_column_is_the_last_one_that_can_be_hidden() {
        assert_eq!(MAX_HIDDEN_COLUMN, 64);
        let mut set = HiddenColumns::NONE;
        assert!(set.hide(64, 64));
        assert!(!set.can_hide(65, 64));
        assert!(!set.can_hide(65, u8::MAX));
        assert!(set.contains(64));
        assert!(!set.contains(65));
        assert_eq!(set.native_column(63, 64), Some(63));
        assert_eq!(set.file_column(63, 64), Some(63));
        assert_eq!(set.file_column(64, 64), None);
        for column in 2..=63 {
            assert!(set.hide(column, 64));
        }
        assert_eq!(set.visible(64).collect::<Vec<_>>(), [1]);
        assert!(!set.hide(1, 64));
        assert_eq!(set.bits(), u64::MAX - 1);
    }

    #[test]
    fn columns_found_later_in_the_file_are_visible() {
        let set = hidden(&[2], 3);
        assert_eq!(set.visible_count(3), 2);
        assert_eq!(set.visible_count(5), 4);
        assert_eq!(set.native_column(5, 5), Some(4));
        assert_eq!(set.visible(5).collect::<Vec<_>>(), [1, 3, 4, 5]);
    }

    #[test]
    fn bits_round_trip_through_the_worker_atomic() {
        let set = hidden(&[2, 7], 8);
        assert_eq!(HiddenColumns::from_bits(set.bits()), set);
        assert!(HiddenColumns::NONE.is_empty());
        assert!(!set.is_empty());
        assert_eq!(HiddenColumns::default(), HiddenColumns::NONE);
    }
}
