//! Streaming worksheet rows with a fixed number of resume checkpoints.
//!
//! A scan reads `<sheetData>` once, counts rows, and keeps at most
//! `CHECKPOINT_CAPACITY` copies of the reader state at row boundaries. A
//! viewport resumes from the nearest checkpoint at or before its first row
//! and reads forward, keeping only the requested rows.

use super::cell::{NumberKind, boolean_text, serial_text};
use super::xml::{XmlEvent, XmlTokenizer};
use super::zip::EntryStream;
use super::{
    CellContext, XLSX_MAX_COLUMNS, XLSX_MAX_ROWS, XlsxError, XlsxWorkbook, check_cancelled,
    utf8_prefix,
};
use crate::{CancellationGeneration, CancellationToken, PositionedRead};

const CHECKPOINT_CAPACITY: usize = 32;
const INITIAL_CHECKPOINT_STRIDE: u64 = 4_096;
const CANCELLATION_EVENTS: u32 = 4_096;
/// A scan step stops after this many uncompressed bytes per compressed byte
/// budget, so highly compressed rows still return in bounded time.
const OUTPUT_BUDGET_FACTOR: u64 = 16;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    BeforeData,
    InData,
    InRow,
    After,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellType {
    Number,
    Shared,
    Inline,
    Formula,
    Boolean,
    Error,
    Date,
    Other,
}

/// Per-cell parse state while a row is being collected.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag tracks one independent child element of <c>"
)]
#[derive(Debug, Default)]
struct CellState {
    column: u32,
    kind: Option<CellType>,
    style: usize,
    value: Vec<u8>,
    value_truncated: bool,
    has_value: bool,
    inline: Vec<u8>,
    inline_truncated: bool,
    in_value: bool,
    in_inline: bool,
    in_text: bool,
    phonetic_depth: u32,
}

/// Builds the cells of one collected row.
struct RowBuilder<'a> {
    context: &'a CellContext,
    limits: XlsxRowLimits,
    row: XlsxRow,
    row_bytes: usize,
}

impl RowBuilder<'_> {
    fn reset(&mut self, index: u64) {
        self.row = XlsxRow {
            index,
            cells: Vec::new(),
            truncated: false,
        };
        self.row_bytes = 0;
    }

    fn place(&mut self, column: u32, text: &[u8], source_truncated: bool) {
        let column = column as usize;
        if column >= self.limits.max_columns {
            self.row.truncated = true;
            return;
        }
        while self.row.cells.len() < column {
            self.row.cells.push(XlsxCell::default());
        }
        let room = self
            .limits
            .max_cell_bytes
            .min(self.limits.max_row_bytes.saturating_sub(self.row_bytes));
        let kept = utf8_prefix(text, room);
        let truncated = source_truncated || kept.len() < text.len();
        if kept.len() < text.len() && room < self.limits.max_cell_bytes {
            self.row.truncated = true;
        }
        self.row_bytes += kept.len();
        self.row.cells.push(XlsxCell {
            text: kept.to_vec(),
            truncated,
        });
    }
}

/// Reader state for one sheet part. A copy taken between rows can resume.
struct SheetCursor {
    stream: EntryStream,
    xml: XmlTokenizer,
    phase: Phase,
    last_row: u64,
    row: u64,
    next_column: u32,
    events: u32,
}

impl SheetCursor {
    fn new(stream: EntryStream, max_tag_bytes: usize, text_limit: usize) -> Self {
        let xml = XmlTokenizer::new(stream.name(), max_tag_bytes, text_limit);
        Self {
            stream,
            xml,
            phase: Phase::BeforeData,
            last_row: 0,
            row: 0,
            next_column: 0,
            events: 0,
        }
    }

    fn checkpoint(&self) -> Self {
        Self {
            stream: self.stream.checkpoint(),
            xml: self.xml.checkpoint(),
            phase: self.phase,
            last_row: self.last_row,
            row: self.row,
            next_column: self.next_column,
            events: 0,
        }
    }

    fn retained_bytes(&self) -> usize {
        self.stream.retained_bytes() + std::mem::size_of::<Self>()
    }

    fn next_event<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<Option<XmlEvent>, XlsxError> {
        self.events = self.events.wrapping_add(1);
        if self.events.is_multiple_of(CANCELLATION_EVENTS) {
            check_cancelled(token, generation)?;
        }
        loop {
            if self.stream.available().is_empty() && !self.stream.fill(source)? {
                self.xml.finish()?;
                return Ok(None);
            }
            let (used, event) = self.xml.feed(self.stream.available())?;
            self.stream.consume(used);
            if event.is_some() {
                return Ok(event);
            }
        }
    }

    fn malformed(&self, reason: &'static str) -> XlsxError {
        XlsxError::MalformedXml {
            part: self.xml.part().to_owned(),
            reason,
        }
    }

    fn start_row(&mut self, value: &mut Vec<u8>) -> Result<u64, XlsxError> {
        let number = if self.xml.attribute(b"r", false, value)? {
            parse_number(value).ok_or(XlsxError::BadRowNumber { row: 0 })?
        } else {
            self.last_row + 1
        };
        if number == 0 || number > XLSX_MAX_ROWS || number <= self.last_row {
            return Err(XlsxError::BadRowNumber { row: number });
        }
        Ok(number)
    }

    /// Reads up to the end of the next stored row and returns its number.
    /// When `builder` is set and the row number is above `collect_after`,
    /// the row's cells are turned into text. Returns `None` at the end of the
    /// sheet data.
    fn next_row<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
        mut builder: Option<(&mut RowBuilder<'_>, u64)>,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<Option<u64>, XlsxError> {
        let mut value = Vec::new();
        let mut cell = CellState::default();
        let mut collecting = false;
        loop {
            let Some(event) = self.next_event(source, token, generation)? else {
                return match self.phase {
                    Phase::BeforeData | Phase::After => {
                        self.phase = Phase::After;
                        Ok(None)
                    }
                    Phase::InData | Phase::InRow => {
                        Err(self.malformed("the sheet data is not closed"))
                    }
                };
            };
            match self.phase {
                Phase::BeforeData => {
                    if let XmlEvent::Start { empty } = event
                        && self.xml.name() == b"sheetData"
                    {
                        if empty {
                            self.phase = Phase::After;
                            return Ok(None);
                        }
                        self.phase = Phase::InData;
                    }
                }
                Phase::InData => match event {
                    XmlEvent::Start { empty } if self.xml.name() == b"row" => {
                        let number = self.start_row(&mut value)?;
                        collecting = false;
                        if let Some((row_builder, collect_after)) = builder.as_mut()
                            && number > *collect_after
                        {
                            row_builder.reset(number - 1);
                            collecting = true;
                        }
                        if empty {
                            self.last_row = number;
                            return Ok(Some(number));
                        }
                        self.row = number;
                        self.next_column = 0;
                        self.phase = Phase::InRow;
                    }
                    XmlEvent::End if self.xml.name() == b"sheetData" => {
                        self.phase = Phase::After;
                        return Ok(None);
                    }
                    _ => {}
                },
                Phase::InRow => {
                    let row_builder = match builder.as_mut() {
                        Some((row_builder, _)) if collecting => Some(&mut **row_builder),
                        _ => None,
                    };
                    if self.row_event(event, &mut cell, &mut value, row_builder)? {
                        self.last_row = self.row;
                        self.phase = Phase::InData;
                        return Ok(Some(self.row));
                    }
                }
                Phase::After => return Ok(None),
            }
        }
    }

    /// Handles one event inside a row. Returns true at the end of the row.
    fn row_event(
        &mut self,
        event: XmlEvent,
        cell: &mut CellState,
        value: &mut Vec<u8>,
        mut builder: Option<&mut RowBuilder<'_>>,
    ) -> Result<bool, XlsxError> {
        match event {
            XmlEvent::Start { empty } => match self.xml.name() {
                b"c" => {
                    let column = if self.xml.attribute(b"r", false, value)? {
                        parse_reference(value, self.row)?
                    } else {
                        self.next_column
                    };
                    if column < self.next_column || column >= XLSX_MAX_COLUMNS {
                        return Err(XlsxError::BadCellReference { row: self.row });
                    }
                    self.next_column = column + 1;
                    if builder.is_none() {
                        return Ok(false);
                    }
                    *cell = CellState {
                        column,
                        ..CellState::default()
                    };
                    cell.kind = Some(if self.xml.attribute(b"t", false, value)? {
                        cell_type(value)
                    } else {
                        CellType::Number
                    });
                    if self.xml.attribute(b"s", false, value)? {
                        cell.style = parse_number(value)
                            .and_then(|style| usize::try_from(style).ok())
                            .ok_or(XlsxError::StyleIndex { row: self.row })?;
                    }
                    if empty && let Some(builder) = builder.as_mut() {
                        self.finish_cell(cell, builder)?;
                    }
                }
                b"v" if cell.kind.is_some() && !empty => {
                    cell.in_value = true;
                    cell.has_value = true;
                    self.xml.collect_text(true);
                }
                b"v" if cell.kind.is_some() => cell.has_value = true,
                b"is" if cell.kind.is_some() && !empty => cell.in_inline = true,
                b"rPh" if cell.in_inline && !empty => cell.phonetic_depth += 1,
                b"t" if cell.in_inline && cell.phonetic_depth == 0 && !empty => {
                    cell.in_text = true;
                    self.xml.collect_text(true);
                }
                _ => {}
            },
            XmlEvent::End => match self.xml.name() {
                b"row" => return Ok(true),
                b"v" if cell.in_value => {
                    cell.in_value = false;
                    cell.value.clear();
                    cell.value.extend_from_slice(self.xml.text());
                    cell.value_truncated = self.xml.text_truncated();
                    self.xml.collect_text(false);
                }
                b"t" if cell.in_text => {
                    cell.in_text = false;
                    cell.inline.extend_from_slice(self.xml.text());
                    cell.inline_truncated |= self.xml.text_truncated();
                    self.xml.collect_text(false);
                }
                b"rPh" if cell.in_inline => {
                    cell.phonetic_depth = cell.phonetic_depth.saturating_sub(1);
                }
                b"is" => cell.in_inline = false,
                b"c" if cell.kind.is_some() => {
                    if let Some(builder) = builder.as_mut() {
                        self.finish_cell(cell, builder)?;
                    }
                    cell.kind = None;
                }
                _ => {}
            },
            XmlEvent::Text => {}
        }
        Ok(false)
    }

    fn finish_cell(&self, cell: &CellState, builder: &mut RowBuilder<'_>) -> Result<(), XlsxError> {
        let context = builder.context;
        let row = self.row;
        let value = cell.value.as_slice();
        match cell.kind.unwrap_or(CellType::Number) {
            CellType::Shared => {
                if !cell.has_value {
                    builder.place(cell.column, b"", false);
                    return Ok(());
                }
                let (text, truncated) = parse_number(value)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| context.strings.get(index))
                    .ok_or(XlsxError::SharedStringIndex { row })?;
                builder.place(cell.column, text, truncated);
            }
            CellType::Inline => {
                builder.place(cell.column, &cell.inline, cell.inline_truncated);
            }
            CellType::Boolean => {
                let text = boolean_text(value).map_or(value, str::as_bytes);
                builder.place(cell.column, text, cell.value_truncated);
            }
            CellType::Number => {
                let kind = if cell.style == 0 && context.number_kinds.is_empty() {
                    NumberKind::Plain
                } else {
                    *context
                        .number_kinds
                        .get(cell.style)
                        .ok_or(XlsxError::StyleIndex { row })?
                };
                match serial_text(value, kind, context.date1904) {
                    Some(text) => builder.place(cell.column, text.as_bytes(), false),
                    None => builder.place(cell.column, value, cell.value_truncated),
                }
            }
            CellType::Formula | CellType::Error | CellType::Date | CellType::Other => {
                builder.place(cell.column, value, cell.value_truncated);
            }
        }
        Ok(())
    }
}

fn cell_type(value: &[u8]) -> CellType {
    match value {
        b"n" => CellType::Number,
        b"s" => CellType::Shared,
        b"inlineStr" => CellType::Inline,
        b"str" => CellType::Formula,
        b"b" => CellType::Boolean,
        b"e" => CellType::Error,
        b"d" => CellType::Date,
        _ => CellType::Other,
    }
}

fn parse_number(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 19 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parses an A1 reference and returns the zero-based column. The row digits
/// must match the enclosing row.
fn parse_reference(reference: &[u8], row: u64) -> Result<u32, XlsxError> {
    let letters = reference
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count();
    let error = XlsxError::BadCellReference { row };
    if letters == 0 || letters > 3 {
        return Err(error);
    }
    let mut column: u32 = 0;
    for letter in &reference[..letters] {
        column = column * 26 + u32::from(letter.to_ascii_uppercase() - b'A' + 1);
    }
    if column > XLSX_MAX_COLUMNS {
        return Err(error);
    }
    let digits = &reference[letters..];
    if !digits.is_empty() && parse_number(digits) != Some(row) {
        return Err(error);
    }
    Ok(column - 1)
}

/// Incremental row count and resume checkpoints for one sheet.
pub struct XlsxSheetScan {
    sheet: usize,
    cursor: SheetCursor,
    checkpoints: Vec<SheetCursor>,
    stride: u64,
    next_checkpoint: u64,
    compressed_total: u64,
    complete: bool,
}

impl std::fmt::Debug for XlsxSheetScan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("XlsxSheetScan")
            .field("sheet", &self.sheet)
            .field("rows", &self.cursor.last_row)
            .field("checkpoints", &self.checkpoints.len())
            .field("complete", &self.complete)
            .finish_non_exhaustive()
    }
}

impl XlsxSheetScan {
    /// Prepares a scan of one sheet. The sheet part's declared sizes are
    /// checked here.
    ///
    /// # Errors
    ///
    /// Returns [`XlsxError`] when the sheet is not a readable worksheet or
    /// its part is beyond a limit.
    pub fn new<R: PositionedRead + ?Sized>(
        source: &R,
        workbook: &XlsxWorkbook,
        sheet: usize,
    ) -> Result<Self, XlsxError> {
        let stream = workbook.open_sheet_stream(source, sheet)?;
        let compressed_total = stream_compressed_total(workbook, sheet);
        let limits = workbook.limits();
        Ok(Self {
            sheet,
            cursor: SheetCursor::new(stream, limits.max_tag_bytes, limits.max_cell_bytes),
            checkpoints: Vec::new(),
            stride: INITIAL_CHECKPOINT_STRIDE,
            next_checkpoint: INITIAL_CHECKPOINT_STRIDE,
            compressed_total,
            complete: false,
        })
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
            rows: self.cursor.last_row,
            compressed_read: self.cursor.stream.consumed(),
            compressed_total: self.compressed_total,
            complete: self.complete,
        }
    }

    /// Bytes retained by the scan state and its checkpoints.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.cursor.retained_bytes()
            + self
                .checkpoints
                .iter()
                .map(SheetCursor::retained_bytes)
                .sum::<usize>()
    }

    /// Number of resume checkpoints held.
    #[must_use]
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// Reads rows until about `max_compressed_bytes` of compressed input has
    /// been used.
    ///
    /// # Errors
    ///
    /// Returns [`XlsxError`] for malformed or over-limit sheet data.
    pub fn step<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
        max_compressed_bytes: usize,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<XlsxScanStep, XlsxError> {
        let start_consumed = self.cursor.stream.consumed();
        let start_produced = self.cursor.stream.produced();
        let start_rows = self.cursor.last_row;
        let budget = max_compressed_bytes as u64;
        let step = |scan: &Self, status| XlsxScanStep {
            status,
            consumed_bytes: usize::try_from(scan.cursor.stream.consumed() - start_consumed)
                .unwrap_or(usize::MAX),
            discovered_rows: scan.cursor.last_row - start_rows,
        };
        if self.complete {
            return Ok(step(self, XlsxScanStatus::Complete));
        }
        loop {
            if token.is_cancelled(generation) {
                return Ok(step(self, XlsxScanStatus::Cancelled));
            }
            let row = match self.cursor.next_row(source, None, token, generation) {
                Ok(row) => row,
                Err(XlsxError::Cancelled) => return Ok(step(self, XlsxScanStatus::Cancelled)),
                Err(error) => return Err(error),
            };
            let Some(row) = row else {
                self.complete = true;
                return Ok(step(self, XlsxScanStatus::Complete));
            };
            if row >= self.next_checkpoint {
                self.add_checkpoint();
            }
            let consumed = self.cursor.stream.consumed() - start_consumed;
            let produced = self.cursor.stream.produced() - start_produced;
            if consumed >= budget || produced >= budget.saturating_mul(OUTPUT_BUDGET_FACTOR) {
                return Ok(step(self, XlsxScanStatus::Progress));
            }
        }
    }

    fn add_checkpoint(&mut self) {
        if self.checkpoints.len() == CHECKPOINT_CAPACITY {
            // Keep every second checkpoint and double the stride.
            let mut index = 0;
            self.checkpoints.retain(|_| {
                index += 1;
                index % 2 == 0
            });
            self.stride = self.stride.saturating_mul(2);
        }
        self.checkpoints.push(self.cursor.checkpoint());
        self.next_checkpoint = self.cursor.last_row.saturating_add(self.stride);
    }

    /// Reads up to `max_rows` rows starting at zero-based `first_row`.
    /// Missing rows inside the sheet are returned empty.
    ///
    /// # Errors
    ///
    /// Returns [`XlsxError`] for malformed sheet data, a cancelled
    /// generation, or a read failure.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors read_viewport: source, position, bounds and cancellation"
    )]
    pub fn read_rows<R: PositionedRead + ?Sized>(
        &self,
        source: &R,
        workbook: &XlsxWorkbook,
        first_row: u64,
        max_rows: usize,
        limits: XlsxRowLimits,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<XlsxRows, XlsxError> {
        check_cancelled(token, generation)?;
        let resume = self
            .checkpoints
            .iter()
            .rev()
            .find(|checkpoint| checkpoint.last_row <= first_row);
        let mut cursor = if let Some(checkpoint) = resume {
            checkpoint.checkpoint()
        } else {
            let workbook_limits = workbook.limits();
            SheetCursor::new(
                workbook.open_sheet_stream(source, self.sheet)?,
                workbook_limits.max_tag_bytes,
                workbook_limits.max_cell_bytes,
            )
        };
        let mut builder = RowBuilder {
            context: &workbook.context,
            limits,
            row: XlsxRow::default(),
            row_bytes: 0,
        };
        let mut rows: Vec<XlsxRow> = Vec::new();
        rows.try_reserve_exact(max_rows.min(1_024))
            .map_err(|_| XlsxError::AllocationFailed)?;
        let mut next_index = first_row;
        let mut reached_end = false;
        while rows.len() < max_rows {
            let Some(number) =
                cursor.next_row(source, Some((&mut builder, first_row)), token, generation)?
            else {
                reached_end = true;
                break;
            };
            let index = number - 1;
            if index < first_row {
                continue;
            }
            while next_index < index && rows.len() < max_rows {
                rows.push(XlsxRow {
                    index: next_index,
                    ..XlsxRow::default()
                });
                next_index += 1;
            }
            if rows.len() == max_rows {
                break;
            }
            rows.push(std::mem::take(&mut builder.row));
            next_index = index + 1;
        }
        Ok(XlsxRows {
            first_row,
            rows,
            reached_end,
        })
    }
}

fn stream_compressed_total(workbook: &XlsxWorkbook, sheet: usize) -> u64 {
    workbook
        .sheets
        .get(sheet)
        .and_then(|sheet| sheet.part.as_deref())
        .and_then(|part| workbook.directory.find(part))
        .map_or(0, super::zip::ZipEntry::compressed_bytes)
}

#[cfg(test)]
mod tests {
    use super::parse_reference;

    #[test]
    fn a1_references_map_to_zero_based_columns() {
        assert_eq!(parse_reference(b"A1", 1).ok(), Some(0));
        assert_eq!(parse_reference(b"z7", 7).ok(), Some(25));
        assert_eq!(parse_reference(b"AA3", 3).ok(), Some(26));
        assert_eq!(parse_reference(b"XFD9", 9).ok(), Some(16_383));
        assert!(parse_reference(b"XFE9", 9).is_err());
        assert!(parse_reference(b"A2", 1).is_err());
        assert!(parse_reference(b"12", 12).is_err());
    }
}
