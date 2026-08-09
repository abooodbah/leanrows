//! Pure, platform-independent viewport and display-cache models.

use crate::AccessibleName;

/// Microsoft documents 100,000,000 as the virtual list-view item-count limit.
const MAX_NATIVE_ROWS: u32 = 100_000_000;

/// Maps a bounded native list-view window onto a potentially much larger file.
///
/// All absolute positions are `u64`. Arithmetic that crosses the UI/native
/// boundary is checked, and the visible count never exceeds `i32::MAX`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlidingRowWindow {
    total_rows: u64,
    first_row: u64,
    visible_rows: u32,
}

impl SlidingRowWindow {
    /// Creates a window whose visible region starts at row zero.
    #[must_use]
    pub fn new(total_rows: u64, requested_visible_rows: u32) -> Self {
        let native_rows = if requested_visible_rows > MAX_NATIVE_ROWS {
            MAX_NATIVE_ROWS
        } else {
            requested_visible_rows
        };
        let visible_rows = if total_rows < u64::from(native_rows) {
            match u32::try_from(total_rows) {
                Ok(rows) => rows,
                Err(_) => MAX_NATIVE_ROWS,
            }
        } else {
            native_rows
        };
        Self {
            total_rows,
            first_row: 0,
            visible_rows,
        }
    }

    /// Returns the total logical row count.
    #[must_use]
    pub const fn total_rows(self) -> u64 {
        self.total_rows
    }

    /// Returns the first absolute row represented by local row zero.
    #[must_use]
    pub const fn first_row(self) -> u64 {
        self.first_row
    }

    /// Returns the bounded count supplied to `LVM_SETITEMCOUNT`.
    #[must_use]
    pub const fn visible_rows(self) -> u32 {
        self.visible_rows
    }

    /// Replaces the logical row count while preserving a valid window.
    pub fn set_total_rows(&mut self, total_rows: u64) {
        self.total_rows = total_rows;
        self.visible_rows = self
            .visible_rows
            .min(u32::try_from(total_rows).unwrap_or(u32::MAX));
        self.first_row = self.first_row.min(self.max_first_row());
    }

    /// Changes the native window size and clamps it to the logical dataset.
    pub fn set_visible_rows(&mut self, requested: u64) {
        let bounded = requested.min(u64::from(MAX_NATIVE_ROWS));
        self.visible_rows = u32::try_from(bounded.min(self.total_rows)).unwrap_or(MAX_NATIVE_ROWS);
        self.first_row = self.first_row.min(self.max_first_row());
    }

    /// Moves the window to the closest valid absolute starting row.
    pub fn seek(&mut self, requested_first_row: u64) {
        self.first_row = requested_first_row.min(self.max_first_row());
    }

    /// Recenters the native window around an absolute row where possible.
    pub fn center_on(&mut self, absolute_row: u64) {
        let bounded_row = absolute_row.min(self.total_rows.saturating_sub(1));
        let requested_first = bounded_row.saturating_sub(u64::from(self.visible_rows / 2));
        self.seek(requested_first);
    }

    /// Moves the window by a signed logical-row delta using saturating bounds.
    pub fn scroll(&mut self, delta: i64) {
        let candidate = if delta.is_negative() {
            self.first_row.saturating_sub(delta.unsigned_abs())
        } else {
            self.first_row.saturating_add(delta.unsigned_abs())
        };
        self.seek(candidate);
    }

    /// Converts a native list index into an absolute logical row.
    #[must_use]
    pub fn local_to_absolute(self, local_row: i32) -> Option<u64> {
        let local = u32::try_from(local_row).ok()?;
        if local >= self.visible_rows {
            return None;
        }
        let absolute = self.first_row.checked_add(u64::from(local))?;
        (absolute < self.total_rows).then_some(absolute)
    }

    /// Converts an absolute logical row into the current native list index.
    #[must_use]
    pub fn absolute_to_local(self, absolute_row: u64) -> Option<i32> {
        if absolute_row >= self.total_rows {
            return None;
        }
        let local = absolute_row.checked_sub(self.first_row)?;
        if local >= u64::from(self.visible_rows) {
            return None;
        }
        i32::try_from(local).ok()
    }

    const fn max_first_row(self) -> u64 {
        self.total_rows.saturating_sub(self.visible_rows as u64)
    }
}

/// One immutable, NUL-terminated UTF-16 display cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayCell {
    utf16: Box<[u16]>,
}

impl DisplayCell {
    /// Encodes a cell once, outside the list-view notification callback.
    #[must_use]
    pub fn new(text: &str) -> Self {
        let mut utf16: Vec<u16> = text.encode_utf16().filter(|unit| *unit != 0).collect();
        utf16.push(0);
        Self {
            utf16: utf16.into_boxed_slice(),
        }
    }

    /// Takes ownership of validated, pre-encoded display text without copying
    /// its UTF-16 payload.
    pub(crate) fn from_nul_terminated_utf16(utf16: Box<[u16]>) -> Option<Self> {
        let (terminator, text) = utf16.split_last()?;
        if *terminator != 0 || text.contains(&0) {
            return None;
        }
        Some(Self { utf16 })
    }

    /// Returns a stable pointer-ready slice including its terminal NUL.
    #[must_use]
    pub fn as_utf16(&self) -> &[u16] {
        &self.utf16
    }
}

/// A pre-encoded immutable row ready for native display callbacks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedRow {
    absolute_row: u64,
    cells: Box<[DisplayCell]>,
    accessible_name: AccessibleName,
}

impl CachedRow {
    /// Creates an immutable row from already separated display values.
    #[must_use]
    pub fn new(absolute_row: u64, cells: Vec<DisplayCell>) -> Self {
        let accessible_name =
            AccessibleName::from_utf16_cells(absolute_row, cells.iter().map(DisplayCell::as_utf16));
        Self::new_with_accessible_name(absolute_row, cells, accessible_name)
    }

    /// Creates an immutable row with a purpose-built semantic accessible name.
    #[must_use]
    pub fn new_with_accessible_name(
        absolute_row: u64,
        cells: Vec<DisplayCell>,
        accessible_name: AccessibleName,
    ) -> Self {
        Self {
            absolute_row,
            cells: cells.into_boxed_slice(),
            accessible_name,
        }
    }
}

/// Sorted immutable cache swapped only on the UI thread.
///
/// Lookup performs no allocation and no I/O, which keeps
/// `LVN_GETDISPINFOW` deterministic and bounded.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImmutableRowCache {
    rows: Box<[CachedRow]>,
}

impl ImmutableRowCache {
    /// Sorts a new cache block once before it can be queried by the UI.
    #[must_use]
    pub fn new(mut rows: Vec<CachedRow>) -> Self {
        rows.sort_unstable_by_key(|row| row.absolute_row);
        Self {
            rows: rows.into_boxed_slice(),
        }
    }

    /// Returns persistent UTF-16 storage for one cached cell.
    #[must_use]
    pub fn cell(&self, absolute_row: u64, subitem: usize) -> Option<&[u16]> {
        self.row(absolute_row)?
            .cells
            .get(subitem)
            .map(DisplayCell::as_utf16)
    }

    /// Distinguishes a cached row with fewer fields from an uncached row.
    #[must_use]
    pub fn contains_row(&self, absolute_row: u64) -> bool {
        self.row(absolute_row).is_some()
    }

    /// Returns the retained cell count for a cached row without allocation.
    #[must_use]
    pub fn cell_count(&self, absolute_row: u64) -> Option<usize> {
        Some(self.row(absolute_row)?.cells.len())
    }

    /// Returns the precomposed accessible name for one cached row.
    #[must_use]
    pub fn accessible_name(&self, absolute_row: u64) -> Option<&AccessibleName> {
        Some(&self.row(absolute_row)?.accessible_name)
    }

    pub(crate) fn accessible_entries(&self) -> impl Iterator<Item = (u64, &AccessibleName)> {
        self.rows
            .iter()
            .map(|row| (row.absolute_row, &row.accessible_name))
    }

    fn row(&self, absolute_row: u64) -> Option<&CachedRow> {
        let index = self
            .rows
            .binary_search_by_key(&absolute_row, |row| row.absolute_row)
            .ok()?;
        self.rows.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessibleName, CachedRow, DisplayCell, ImmutableRowCache, SlidingRowWindow};

    #[test]
    fn mapping_is_checked_at_documented_native_limit_and_u64_boundaries() {
        let mut window = SlidingRowWindow::new(u64::MAX, u32::MAX);
        assert_eq!(window.visible_rows(), 100_000_000);
        window.seek(u64::MAX);

        assert_eq!(window.first_row(), u64::MAX - 100_000_000);
        assert_eq!(window.local_to_absolute(99_999_999), Some(u64::MAX - 1));
        assert_eq!(window.local_to_absolute(100_000_000), None);
        assert_eq!(window.local_to_absolute(-1), None);
        assert_eq!(window.absolute_to_local(u64::MAX - 1), Some(99_999_999));
        assert_eq!(window.absolute_to_local(u64::MAX), None);
    }

    #[test]
    fn recentering_clamps_at_both_dataset_edges() {
        let mut window = SlidingRowWindow::new(u64::MAX, 100_000_000);
        window.center_on(50_000_000);
        assert_eq!(window.first_row(), 0);
        assert_eq!(window.absolute_to_local(50_000_000), Some(50_000_000));

        window.center_on(u64::MAX - 1);
        assert_eq!(window.first_row(), u64::MAX - 100_000_000);
        assert_eq!(window.absolute_to_local(u64::MAX - 1), Some(99_999_999));
    }

    #[test]
    fn window_scroll_saturates_and_clamps() {
        let mut window = SlidingRowWindow::new(250, 25);
        window.scroll(i64::MAX);
        assert_eq!(window.first_row(), 225);
        window.scroll(i64::MIN);
        assert_eq!(window.first_row(), 0);
    }

    #[test]
    fn cache_is_sorted_and_lookup_is_allocation_free() {
        let cache = ImmutableRowCache::new(vec![
            CachedRow::new(7, vec![DisplayCell::new("seven")]),
            CachedRow::new(2, vec![DisplayCell::new("two")]),
        ]);

        assert_eq!(cache.cell(2, 0), Some(&[116, 119, 111, 0][..]));
        assert_eq!(cache.cell(7, 1), None);
        assert_eq!(cache.cell(8, 0), None);
        assert!(cache.contains_row(7));
        assert!(!cache.contains_row(8));
        assert_eq!(cache.cell_count(7), Some(1));
        assert_eq!(cache.cell_count(8), None);

        let name = cache.accessible_name(2).map(AccessibleName::as_utf16);
        let expected: Vec<u16> = "Row 3; Column 1: two\0".encode_utf16().collect();
        assert_eq!(name, Some(expected.as_slice()));
    }

    #[test]
    fn preencoded_cell_transfer_is_zero_copy_and_fails_closed() {
        let encoded = vec![65_u16, 66, 0].into_boxed_slice();
        let original_pointer = encoded.as_ptr();
        let cell = DisplayCell::from_nul_terminated_utf16(encoded);
        assert!(cell.is_some());
        assert_eq!(
            cell.as_ref().map(DisplayCell::as_utf16),
            Some(&[65, 66, 0][..])
        );
        assert_eq!(
            cell.as_ref().map(|value| value.as_utf16().as_ptr()),
            Some(original_pointer)
        );
        assert!(DisplayCell::from_nul_terminated_utf16(vec![65].into_boxed_slice()).is_none());
        assert!(
            DisplayCell::from_nul_terminated_utf16(vec![65, 0, 66, 0].into_boxed_slice()).is_none()
        );
    }
}
