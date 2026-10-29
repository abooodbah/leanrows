//! Read-only `.xlsx` workbooks: stored cell values, one sheet at a time.
//!
//! This is the public surface only. Every entry point returns an error until
//! the reader is implemented, so the fixture tests can run and fail first.
//!
//! The reader never evaluates formulas, runs macros, or follows external
//! relationships. Worksheet XML is streamed through a fixed decompression
//! window; only the shared-string table, the number-format table and a fixed
//! number of resume checkpoints are kept for the life of the document. The
//! display rules are written down in `docs/XLSX.md`.

mod sheet;

use std::fmt;
use std::io;

use crate::{CancellationGeneration, CancellationToken, PositionedRead};

pub use self::sheet::{
    XlsxCell, XlsxRow, XlsxRowLimits, XlsxRows, XlsxScanProgress, XlsxScanStatus, XlsxScanStep,
    XlsxSheetScan,
};

/// Excel's row limit, also enforced here.
pub const XLSX_MAX_ROWS: u64 = 1_048_576;
/// Excel's column limit (`XFD`), also enforced here.
pub const XLSX_MAX_COLUMNS: u32 = 16_384;

const MIB: u64 = 1_024 * 1_024;

/// Explicit bounds for one workbook. Inputs beyond them are refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XlsxLimits {
    /// Most entries in the ZIP central directory.
    pub max_zip_entries: usize,
    /// Largest central directory read into memory.
    pub max_central_directory_bytes: usize,
    /// Largest uncompressed size of any one entry.
    pub max_entry_bytes: u64,
    /// Largest uncompressed-to-compressed ratio once an entry is past
    /// `ratio_check_after_bytes`.
    pub max_compression_ratio: u64,
    /// Uncompressed size below which the ratio is not checked.
    pub ratio_check_after_bytes: u64,
    /// Most decoded text in the shared-string table, counted in full.
    pub max_shared_string_bytes: u64,
    /// Most entries in the shared-string table.
    pub max_shared_strings: usize,
    /// Most cell formats (`cellXfs`) and custom number formats.
    pub max_styles: usize,
    /// Most sheets and most workbook relationships.
    pub max_sheets: usize,
    /// Longest single XML tag.
    pub max_tag_bytes: usize,
    /// Most bytes kept for one cell or shared string. Longer text is kept up
    /// to this length and marked as truncated.
    pub max_cell_bytes: usize,
}

impl XlsxLimits {
    /// The limits used by the application.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_zip_entries: 10_000,
            max_central_directory_bytes: 4 * 1_024 * 1_024,
            max_entry_bytes: 1_024 * MIB,
            max_compression_ratio: 200,
            ratio_check_after_bytes: MIB,
            max_shared_string_bytes: 256 * MIB,
            max_shared_strings: 16_777_216,
            max_styles: 65_536,
            max_sheets: 4_096,
            max_tag_bytes: 64 * 1_024,
            max_cell_bytes: 1_024,
        }
    }
}

impl Default for XlsxLimits {
    fn default() -> Self {
        Self::standard()
    }
}

/// Why a workbook was refused or could not be read.
#[derive(Debug)]
#[non_exhaustive]
pub enum XlsxError {
    /// The file does not start like a ZIP archive.
    NotZip,
    /// The file is an OLE compound file: a legacy `.xls` or an encrypted
    /// (password-protected) workbook.
    LegacyOrEncrypted,
    /// The file ends early.
    Truncated,
    /// ZIP64 archives are not supported.
    Zip64Unsupported,
    /// An entry is encrypted.
    EncryptedEntry {
        /// Entry name.
        name: String,
    },
    /// An entry uses a method other than stored or deflate.
    UnsupportedCompression {
        /// Entry name.
        name: String,
        /// ZIP method number.
        method: u16,
    },
    /// The archive has more entries than allowed.
    TooManyEntries {
        /// The limit.
        limit: usize,
    },
    /// Two entries have the same name.
    DuplicateEntry {
        /// Entry name.
        name: String,
    },
    /// The ZIP structure is inconsistent.
    MalformedZip(&'static str),
    /// An entry is larger than allowed.
    EntryTooLarge {
        /// Entry name.
        name: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// An entry compresses better than any real workbook part.
    CompressionRatio {
        /// Entry name.
        name: String,
        /// The ratio limit.
        limit: u64,
    },
    /// An entry inflates to a size other than its header says.
    SizeMismatch {
        /// Entry name.
        name: String,
    },
    /// An entry fails its CRC-32 check.
    ChecksumMismatch {
        /// Entry name.
        name: String,
    },
    /// An entry's compressed data is invalid.
    CorruptData {
        /// Entry name.
        name: String,
    },
    /// A required part is missing.
    MissingPart {
        /// Part name.
        name: String,
    },
    /// A relationship points outside the package. It is never followed.
    ExternalRelationship {
        /// The relationship target.
        target: String,
    },
    /// The chosen sheet is not a worksheet (for example a chart sheet).
    NotWorksheet,
    /// The workbook has no worksheet to show.
    NoWorksheet,
    /// The sheet index is out of range.
    SheetOutOfRange,
    /// A part has a DOCTYPE declaration.
    Doctype {
        /// Part name.
        part: String,
    },
    /// A part is not UTF-8.
    UnsupportedEncoding {
        /// Part name.
        part: String,
    },
    /// A part is not well-formed enough to read.
    MalformedXml {
        /// Part name.
        part: String,
        /// What was wrong.
        reason: &'static str,
    },
    /// The shared-string table has too many entries.
    TooManySharedStrings {
        /// The limit.
        limit: usize,
    },
    /// The shared-string table has too much text.
    SharedStringsTooLarge {
        /// The limit in bytes.
        limit: u64,
    },
    /// The style table is too large.
    TooManyStyles {
        /// The limit.
        limit: usize,
    },
    /// The workbook has too many sheets or relationships.
    TooManySheets {
        /// The limit.
        limit: usize,
    },
    /// A row number is zero, beyond 1,048,576, or not after the previous row.
    BadRowNumber {
        /// The row number as found.
        row: u64,
    },
    /// A cell reference is invalid, out of order, or beyond column XFD.
    BadCellReference {
        /// The row of the cell.
        row: u64,
    },
    /// A shared-string index is out of range.
    SharedStringIndex {
        /// The row of the cell.
        row: u64,
    },
    /// A style index is out of range.
    StyleIndex {
        /// The row of the cell.
        row: u64,
    },
    /// Reading the file failed.
    Io(io::Error),
    /// A bounded allocation failed.
    AllocationFailed,
    /// The operation was cancelled.
    Cancelled,
}

impl fmt::Display for XlsxError {
    #[allow(
        clippy::too_many_lines,
        reason = "one message per error variant reads best as one match"
    )]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotZip => {
                formatter.write_str("the file is not an .xlsx workbook (no ZIP header)")
            }
            Self::LegacyOrEncrypted => formatter.write_str(
                "the file is a legacy .xls or a password-protected workbook; neither is supported",
            ),
            Self::Truncated => {
                formatter.write_str("the workbook ends early; the file may be truncated")
            }
            Self::Zip64Unsupported => formatter.write_str("ZIP64 workbooks are not supported"),
            Self::EncryptedEntry { name } => write!(formatter, "the part {name} is encrypted"),
            Self::UnsupportedCompression { name, method } => write!(
                formatter,
                "the part {name} uses unsupported ZIP compression method {method}"
            ),
            Self::TooManyEntries { limit } => {
                write!(formatter, "the workbook has more than {limit} ZIP entries")
            }
            Self::DuplicateEntry { name } => {
                write!(formatter, "the workbook has two parts named {name}")
            }
            Self::MalformedZip(reason) => {
                write!(formatter, "the ZIP structure is invalid: {reason}")
            }
            Self::EntryTooLarge { name, limit } => write!(
                formatter,
                "the part {name} is larger than the {} MiB limit",
                limit / MIB
            ),
            Self::CompressionRatio { name, limit } => write!(
                formatter,
                "the part {name} expands more than {limit} times; it looks like a ZIP bomb"
            ),
            Self::SizeMismatch { name } => write!(
                formatter,
                "the part {name} does not match the size in its ZIP header"
            ),
            Self::ChecksumMismatch { name } => {
                write!(formatter, "the part {name} fails its CRC-32 check")
            }
            Self::CorruptData { name } => {
                write!(formatter, "the part {name} has invalid compressed data")
            }
            Self::MissingPart { name } => write!(formatter, "the workbook has no part {name}"),
            Self::ExternalRelationship { target } => write!(
                formatter,
                "the workbook points to an external target ({target}); it is not followed"
            ),
            Self::NotWorksheet => formatter.write_str("the sheet is not a worksheet"),
            Self::NoWorksheet => formatter.write_str("the workbook has no visible worksheet"),
            Self::SheetOutOfRange => formatter.write_str("the sheet does not exist"),
            Self::Doctype { part } => write!(
                formatter,
                "the part {part} has a DOCTYPE declaration; it is refused"
            ),
            Self::UnsupportedEncoding { part } => {
                write!(formatter, "the part {part} is not UTF-8")
            }
            Self::MalformedXml { part, reason } => {
                write!(formatter, "the part {part} is not valid XML: {reason}")
            }
            Self::TooManySharedStrings { limit } => {
                write!(
                    formatter,
                    "the workbook has more than {limit} shared strings"
                )
            }
            Self::SharedStringsTooLarge { limit } => write!(
                formatter,
                "the shared strings hold more than {} MiB of text",
                limit / MIB
            ),
            Self::TooManyStyles { limit } => {
                write!(formatter, "the workbook has more than {limit} cell formats")
            }
            Self::TooManySheets { limit } => write!(
                formatter,
                "the workbook has more than {limit} sheets or relationships"
            ),
            Self::BadRowNumber { row } => write!(
                formatter,
                "row number {row} is out of order or beyond 1,048,576"
            ),
            Self::BadCellReference { row } => write!(
                formatter,
                "row {row} has an invalid, out-of-order or out-of-range cell reference"
            ),
            Self::SharedStringIndex { row } => write!(
                formatter,
                "row {row} refers to a shared string that does not exist"
            ),
            Self::StyleIndex { row } => {
                write!(
                    formatter,
                    "row {row} refers to a cell format that does not exist"
                )
            }
            Self::Io(error) => write!(formatter, "reading the workbook failed: {error}"),
            Self::AllocationFailed => formatter.write_str("bounded workbook allocation failed"),
            Self::Cancelled => formatter.write_str("the workbook operation was cancelled"),
        }
    }
}

impl std::error::Error for XlsxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Whether a sheet is offered to the user.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XlsxSheetVisibility {
    /// Shown as a tab in Excel.
    Visible,
    /// `state="hidden"`.
    Hidden,
    /// `state="veryHidden"`.
    VeryHidden,
}

/// One entry of the workbook's `<sheets>` list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XlsxSheet {
    name: String,
    visibility: XlsxSheetVisibility,
    worksheet: bool,
    part: Option<String>,
    external_target: Option<String>,
}

impl XlsxSheet {
    /// The sheet name as stored.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The sheet state.
    #[must_use]
    pub const fn visibility(&self) -> XlsxSheetVisibility {
        self.visibility
    }

    /// True for a worksheet; false for chart, dialog and macro sheets.
    #[must_use]
    pub const fn is_worksheet(&self) -> bool {
        self.worksheet
    }

    /// True when the sheet is a visible worksheet, the only kind offered.
    #[must_use]
    pub const fn is_offered(&self) -> bool {
        self.worksheet && matches!(self.visibility, XlsxSheetVisibility::Visible)
    }
}

const NOT_IMPLEMENTED: XlsxError =
    XlsxError::MalformedZip("the .xlsx reader is not implemented yet");

/// An opened workbook.
#[derive(Debug)]
pub struct XlsxWorkbook {
    sheets: Vec<XlsxSheet>,
}

impl XlsxWorkbook {
    /// Reads the package structure.
    ///
    /// # Errors
    ///
    /// Always fails until the reader is implemented.
    pub fn open<R: PositionedRead + ?Sized>(
        _source: &R,
        _limits: XlsxLimits,
        _token: &CancellationToken,
        _generation: CancellationGeneration,
    ) -> Result<Self, XlsxError> {
        Err(NOT_IMPLEMENTED)
    }

    /// Sheets in workbook order.
    #[must_use]
    pub fn sheets(&self) -> &[XlsxSheet] {
        &self.sheets
    }

    /// The sheet that opens by default.
    #[must_use]
    pub const fn default_sheet(&self) -> Option<usize> {
        None
    }

    /// True when the workbook uses the 1904 date system.
    #[must_use]
    pub const fn uses_1904_dates(&self) -> bool {
        false
    }

    /// Number of shared strings.
    #[must_use]
    pub const fn shared_string_count(&self) -> usize {
        0
    }
}
