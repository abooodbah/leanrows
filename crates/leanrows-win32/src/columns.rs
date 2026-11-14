//! Data columns hidden from one tab's view.
//!
//! Hiding a column changes only what the grid shows. The file is never
//! written, and `LeanRows` never held whole columns in memory to begin with:
//! the worker skips decoding a hidden field for the rows it fetches, and the
//! list shows the Row column plus the columns that are still visible.
//!
//! Data columns are numbered from 1, as in the `Column N` headers; 0 is the
//! Row column, which cannot be hidden. A viewport keeps at most 64 data
//! columns, so one bit per column is enough and the set never grows with the
//! file.

/// The highest data column a viewport holds, and so the highest one that can
/// be hidden. It equals the document engine's 64-field projection cap.
pub(crate) const MAX_HIDDEN_COLUMN: usize = 64;

/// A set of hidden data columns, one bit per column.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HiddenColumns(u64);

impl HiddenColumns {
    /// Every column shown.
    pub(crate) const NONE: Self = Self(0);

    /// Rebuilds a set from `bits`, as stored in the worker's atomic.
    #[must_use]
    pub(crate) const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The set as bits: bit `n - 1` is data column `n`.
    #[must_use]
    pub(crate) const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether data column `file_column` is hidden. The Row column never is.
    #[must_use]
    pub(crate) const fn contains(self, file_column: usize) -> bool {
        match Self::bit(file_column) {
            Some(bit) => self.0 & bit != 0,
            None => false,
        }
    }

    /// How many columns are hidden.
    #[must_use]
    pub(crate) const fn count(self) -> usize {
        self.0.count_ones() as usize
    }

    /// How many of the first `data_columns` data columns are shown.
    #[must_use]
    pub(crate) fn visible_count(self, data_columns: u8) -> usize {
        self.visible(data_columns).count()
    }

    /// Whether `file_column` can be hidden now: it is a data column of this
    /// view, it is shown, and another data column stays shown.
    #[must_use]
    pub(crate) fn can_hide(self, file_column: usize, data_columns: u8) -> bool {
        file_column >= 1
            && file_column <= usize::from(data_columns)
            && file_column <= MAX_HIDDEN_COLUMN
            && !self.contains(file_column)
            && self.visible_count(data_columns) > 1
    }

    /// Hides `file_column` when `can_hide` allows it, and says whether it did.
    pub(crate) fn hide(&mut self, file_column: usize, data_columns: u8) -> bool {
        if !self.can_hide(file_column, data_columns) {
            return false;
        }
        if let Some(bit) = Self::bit(file_column) {
            self.0 |= bit;
        }
        true
    }

    /// The data column shown under list column `native_column`. List column
    /// 0 is the Row column; `None` means there is no such list column.
    #[must_use]
    pub(crate) fn file_column(self, native_column: usize, data_columns: u8) -> Option<usize> {
        if native_column == 0 {
            return Some(0);
        }
        self.visible(data_columns).nth(native_column - 1)
    }

    /// The list column that shows data column `file_column`, or `None` when
    /// it is hidden or not a column of this view.
    #[must_use]
    pub(crate) fn native_column(self, file_column: usize, data_columns: u8) -> Option<usize> {
        if file_column == 0 {
            return Some(0);
        }
        if file_column > usize::from(data_columns) || self.contains(file_column) {
            return None;
        }
        Some(
            self.visible(data_columns)
                .take_while(|c| *c < file_column)
                .count()
                + 1,
        )
    }

    /// The shown data columns of a view with `data_columns` columns, in order.
    pub(crate) fn visible(self, data_columns: u8) -> impl Iterator<Item = usize> {
        (1..=usize::from(data_columns)).filter(move |column| !self.contains(*column))
    }

    const fn bit(file_column: usize) -> Option<u64> {
        if file_column == 0 || file_column > MAX_HIDDEN_COLUMN {
            None
        } else {
            Some(1 << (file_column - 1))
        }
    }
}

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
