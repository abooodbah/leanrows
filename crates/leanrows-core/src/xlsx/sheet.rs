//! Worksheet row types. The scan is not implemented yet.

use super::{NOT_IMPLEMENTED, XlsxError, XlsxWorkbook};
use crate::{CancellationGeneration, CancellationToken, PositionedRead};

/// Display bounds for one viewport row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XlsxRowLimits {
    /// Most columns kept per row; later cells mark the row truncated.
    pub max_columns: usize,
    /// Most UTF-8 bytes kept per cell.
    pub max_cell_bytes: usize,
    /// Most UTF-8 bytes kept per row across its cells.
    pub max_row_bytes: usize,
}

impl XlsxRowLimits {
    /// The bounds used by the application, matching its CSV projection.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_columns: 64,
            max_cell_bytes: 1_024,
            max_row_bytes: 4 * 1_024,
        }
    }
}

/// One displayed cell.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XlsxCell {
    text: Vec<u8>,
    truncated: bool,
}

impl XlsxCell {
    /// The cell text as UTF-8 bytes (invalid sequences are passed through).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.text
    }

    /// The cell text, if it is valid UTF-8.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.text).ok()
    }

    /// True when the stored text is longer than what is kept.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// One displayed row. Rows not stored in the sheet are present and empty.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XlsxRow {
    index: u64,
    cells: Vec<XlsxCell>,
    truncated: bool,
}

impl XlsxRow {
    /// Zero-based row index; the sheet row number is `index + 1`.
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Cells from column A to the last stored cell, within the column limit.
    #[must_use]
    pub fn cells(&self) -> &[XlsxCell] {
        &self.cells
    }

    /// Text of one cell, or `""` beyond the row's width.
    #[must_use]
    pub fn cell_text(&self, column: usize) -> &str {
        self.cells
            .get(column)
            .and_then(XlsxCell::text)
            .unwrap_or("")
    }

    /// True when cells beyond the column limit or row byte limit were dropped
    /// or shortened.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Rows returned for one viewport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XlsxRows {
    first_row: u64,
    rows: Vec<XlsxRow>,
    reached_end: bool,
}

impl XlsxRows {
    /// The first requested row index.
    #[must_use]
    pub const fn first_row(&self) -> u64 {
        self.first_row
    }

    /// The rows, consecutive from `first_row`.
    #[must_use]
    pub fn rows(&self) -> &[XlsxRow] {
        &self.rows
    }

    /// Moves the rows out.
    #[must_use]
    pub fn into_rows(self) -> Vec<XlsxRow> {
        self.rows
    }

    /// True when the end of the sheet data was reached.
    #[must_use]
    pub const fn reached_end(&self) -> bool {
        self.reached_end
    }
}

/// Why a scan step returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XlsxScanStatus {
    /// The step budget was used; call again.
    Progress,
    /// The whole sheet has been read.
    Complete,
    /// The generation was cancelled.
    Cancelled,
}

/// The result of one scan step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XlsxScanStep {
    status: XlsxScanStatus,
    consumed_bytes: usize,
    discovered_rows: u64,
}

impl XlsxScanStep {
    /// Why the step returned.
    #[must_use]
    pub const fn status(self) -> XlsxScanStatus {
        self.status
    }

    /// Compressed bytes read in this step.
    #[must_use]
    pub const fn consumed_bytes(self) -> usize {
        self.consumed_bytes
    }

    /// Rows added to the row count in this step.
    #[must_use]
    pub const fn discovered_rows(self) -> u64 {
        self.discovered_rows
    }
}

/// Scan progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XlsxScanProgress {
    rows: u64,
    compressed_read: u64,
    compressed_total: u64,
    complete: bool,
}

impl XlsxScanProgress {
    /// Row count so far: the number of the last stored row.
    #[must_use]
    pub const fn rows(self) -> u64 {
        self.rows
    }

    /// Compressed sheet bytes read.
    #[must_use]
    pub const fn compressed_read(self) -> u64 {
        self.compressed_read
    }

    /// Compressed size of the sheet part.
    #[must_use]
    pub const fn compressed_total(self) -> u64 {
        self.compressed_total
    }

    /// True when the whole sheet has been read.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.complete
    }
}

/// Incremental row count for one sheet.
#[derive(Debug)]
pub struct XlsxSheetScan {
    sheet: usize,
}

impl XlsxSheetScan {
    /// Prepares a scan of one sheet.
    ///
    /// # Errors
    ///
    /// Always fails until the reader is implemented.
    pub fn new<R: PositionedRead + ?Sized>(
        _source: &R,
        _workbook: &XlsxWorkbook,
        _sheet: usize,
    ) -> Result<Self, XlsxError> {
        Err(NOT_IMPLEMENTED)
    }

    /// The sheet index this scan reads.
    #[must_use]
    pub const fn sheet(&self) -> usize {
        self.sheet
    }

    /// Current progress.
    #[must_use]
    pub const fn progress(&self) -> XlsxScanProgress {
        XlsxScanProgress {
            rows: 0,
            compressed_read: 0,
            compressed_total: 0,
            complete: false,
        }
    }

    /// Bytes retained by the scan.
    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        0
    }

    /// Number of resume checkpoints held.
    #[must_use]
    pub const fn checkpoint_count(&self) -> usize {
        0
    }

    /// Reads rows.
    ///
    /// # Errors
    ///
    /// Always fails until the reader is implemented.
    pub fn step<R: PositionedRead + ?Sized>(
        &mut self,
        _source: &R,
        _max_compressed_bytes: usize,
        _token: &CancellationToken,
        _generation: CancellationGeneration,
    ) -> Result<XlsxScanStep, XlsxError> {
        Err(NOT_IMPLEMENTED)
    }

    /// Reads up to `max_rows` rows.
    ///
    /// # Errors
    ///
    /// Always fails until the reader is implemented.
    #[allow(clippy::too_many_arguments, reason = "mirrors read_viewport")]
    pub fn read_rows<R: PositionedRead + ?Sized>(
        &self,
        _source: &R,
        _workbook: &XlsxWorkbook,
        _first_row: u64,
        _max_rows: usize,
        _limits: XlsxRowLimits,
        _token: &CancellationToken,
        _generation: CancellationGeneration,
    ) -> Result<XlsxRows, XlsxError> {
        Err(NOT_IMPLEMENTED)
    }
}
