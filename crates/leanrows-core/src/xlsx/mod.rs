//! Read-only `.xlsx` workbooks: stored cell values, one sheet at a time.
//!
//! The reader never evaluates formulas, runs macros, or follows external
//! relationships. Worksheet XML is streamed through a fixed decompression
//! window; only the shared-string table, the number-format table and a fixed
//! number of resume checkpoints are kept for the life of the document. The
//! display rules are written down in `docs/XLSX.md`.

mod cell;
mod sheet;
mod xml;
mod zip;

use std::fmt;
use std::io;

use self::cell::{NumberKind, builtin_kind, format_code_kind};
use self::xml::{XmlEvent, XmlTokenizer};
use self::zip::{EntryStream, ZipDirectory};
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

/// The shared-string table: each string kept up to `max_cell_bytes`.
#[derive(Debug, Default)]
struct SharedStrings {
    arena: Vec<u8>,
    /// End offset of each string in `arena`; the top bit marks truncation.
    ends: Vec<u32>,
}

const TRUNCATED_BIT: u32 = 1 << 31;

impl SharedStrings {
    fn get(&self, index: usize) -> Option<(&[u8], bool)> {
        let end = *self.ends.get(index)?;
        let start = match index.checked_sub(1) {
            Some(previous) => self.ends.get(previous)? & !TRUNCATED_BIT,
            None => 0,
        };
        let bytes = self
            .arena
            .get(start as usize..(end & !TRUNCATED_BIT) as usize)?;
        Some((bytes, end & TRUNCATED_BIT != 0))
    }
}

/// Context needed to turn a stored cell into text.
#[derive(Debug)]
struct CellContext {
    strings: SharedStrings,
    number_kinds: Vec<NumberKind>,
    date1904: bool,
}

/// An opened workbook: its directory, sheets, styles and shared strings.
#[derive(Debug)]
pub struct XlsxWorkbook {
    directory: ZipDirectory,
    limits: XlsxLimits,
    sheets: Vec<XlsxSheet>,
    context: CellContext,
}

/// Returns the longest prefix of `bytes` of at most `limit` bytes that does
/// not end inside a UTF-8 sequence.
pub(crate) fn utf8_prefix(bytes: &[u8], limit: usize) -> &[u8] {
    if bytes.len() <= limit {
        return bytes;
    }
    let mut end = limit;
    while end > 0 && bytes[end] & 0xc0 == 0x80 {
        end -= 1;
    }
    &bytes[..end]
}

/// One streamed part: decompression plus tokenizer.
struct PartCursor {
    stream: EntryStream,
    xml: XmlTokenizer,
}

impl PartCursor {
    fn open<R: PositionedRead + ?Sized>(
        source: &R,
        directory: &ZipDirectory,
        name: &str,
        limits: &XlsxLimits,
        text_limit: usize,
    ) -> Result<Option<Self>, XlsxError> {
        let Some(entry) = directory.find(name) else {
            return Ok(None);
        };
        let stream = directory.open(source, entry, limits)?;
        Ok(Some(Self {
            xml: XmlTokenizer::new(entry.name(), limits.max_tag_bytes, text_limit),
            stream,
        }))
    }

    fn next<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
    ) -> Result<Option<XmlEvent>, XlsxError> {
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
}

struct Relationship {
    id: Vec<u8>,
    kind: Vec<u8>,
    target: String,
    external: bool,
}

fn resolve_target(base_directory: &str, target: &str) -> String {
    let joined = if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_owned()
    } else {
        format!("{base_directory}{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn directory_of(part: &str) -> &str {
    match part.rfind('/') {
        Some(position) => &part[..=position],
        None => "",
    }
}

fn relationships_part(part: &str) -> String {
    let (directory, file) = match part.rfind('/') {
        Some(position) => (&part[..=position], &part[position + 1..]),
        None => ("", part),
    };
    format!("{directory}_rels/{file}.rels")
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn check_cancelled(
    token: &CancellationToken,
    generation: CancellationGeneration,
) -> Result<(), XlsxError> {
    if token.is_cancelled(generation) {
        Err(XlsxError::Cancelled)
    } else {
        Ok(())
    }
}

fn read_relationships<R: PositionedRead + ?Sized>(
    source: &R,
    directory: &ZipDirectory,
    part: &str,
    limits: &XlsxLimits,
) -> Result<Vec<Relationship>, XlsxError> {
    let Some(mut cursor) = PartCursor::open(source, directory, part, limits, 0)? else {
        return Ok(Vec::new());
    };
    let base = directory_of(part.trim_end_matches(".rels"));
    // The source part lives one level above the _rels folder.
    let base = base.strip_suffix("_rels/").unwrap_or(base).to_owned();
    let mut relationships = Vec::new();
    let mut value = Vec::new();
    while let Some(event) = cursor.next(source)? {
        if !matches!(event, XmlEvent::Start { .. }) || cursor.xml.name() != b"Relationship" {
            continue;
        }
        if relationships.len() >= limits.max_sheets.saturating_mul(4) {
            return Err(XlsxError::TooManySheets {
                limit: limits.max_sheets,
            });
        }
        let id = if cursor.xml.attribute(b"Id", false, &mut value)? {
            value.clone()
        } else {
            continue;
        };
        let kind = if cursor.xml.attribute(b"Type", false, &mut value)? {
            value.clone()
        } else {
            Vec::new()
        };
        let target = if cursor.xml.attribute(b"Target", false, &mut value)? {
            lossy(&value)
        } else {
            String::new()
        };
        let external = cursor.xml.attribute(b"TargetMode", false, &mut value)?
            && value.eq_ignore_ascii_case(b"External");
        let target = if external {
            target
        } else {
            resolve_target(&base, &target)
        };
        relationships.push(Relationship {
            id,
            kind,
            target,
            external,
        });
    }
    Ok(relationships)
}

impl XlsxWorkbook {
    /// Reads the package structure, sheet list, number formats and shared
    /// strings. Worksheet data is not read here.
    ///
    /// # Errors
    ///
    /// Returns [`XlsxError`] when the file is not a supported workbook, is
    /// beyond a limit, is malformed, or the operation is cancelled.
    #[allow(
        clippy::too_many_lines,
        reason = "package, workbook, styles and strings are read in dependency order"
    )]
    pub fn open<R: PositionedRead + ?Sized>(
        source: &R,
        limits: XlsxLimits,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<Self, XlsxError> {
        check_cancelled(token, generation)?;
        let directory = ZipDirectory::read(source, &limits)?;

        let package = read_relationships(source, &directory, "_rels/.rels", &limits)?;
        let workbook_part = match package
            .iter()
            .find(|relationship| relationship.kind.ends_with(b"/officeDocument"))
        {
            Some(relationship) if relationship.external => {
                return Err(XlsxError::ExternalRelationship {
                    target: relationship.target.clone(),
                });
            }
            Some(relationship) => relationship.target.clone(),
            None => "xl/workbook.xml".to_owned(),
        };
        let relationships = read_relationships(
            source,
            &directory,
            &relationships_part(&workbook_part),
            &limits,
        )?;

        let mut cursor = PartCursor::open(source, &directory, &workbook_part, &limits, 0)?
            .ok_or_else(|| XlsxError::MissingPart {
                name: workbook_part.clone(),
            })?;
        let mut sheets = Vec::new();
        let mut date1904 = false;
        let mut value = Vec::new();
        while let Some(event) = cursor.next(source)? {
            if !matches!(event, XmlEvent::Start { .. }) {
                continue;
            }
            match cursor.xml.name() {
                b"workbookPr" => {
                    date1904 = cursor.xml.attribute(b"date1904", false, &mut value)?
                        && (value == b"1" || value.eq_ignore_ascii_case(b"true"));
                }
                b"sheet" => {
                    if sheets.len() >= limits.max_sheets {
                        return Err(XlsxError::TooManySheets {
                            limit: limits.max_sheets,
                        });
                    }
                    let name = if cursor.xml.attribute(b"name", false, &mut value)? {
                        lossy(&value)
                    } else {
                        String::new()
                    };
                    let visibility = if cursor.xml.attribute(b"state", false, &mut value)? {
                        match value.as_slice() {
                            b"hidden" => XlsxSheetVisibility::Hidden,
                            b"veryHidden" => XlsxSheetVisibility::VeryHidden,
                            _ => XlsxSheetVisibility::Visible,
                        }
                    } else {
                        XlsxSheetVisibility::Visible
                    };
                    let relationship = if cursor.xml.attribute(b"id", true, &mut value)? {
                        relationships.iter().find(|candidate| candidate.id == value)
                    } else {
                        None
                    };
                    let worksheet = relationship
                        .is_some_and(|relationship| relationship.kind.ends_with(b"/worksheet"));
                    let (part, external_target) = match relationship {
                        Some(relationship) if relationship.external => {
                            (None, Some(relationship.target.clone()))
                        }
                        Some(relationship) => (Some(relationship.target.clone()), None),
                        None => (None, None),
                    };
                    sheets.push(XlsxSheet {
                        name,
                        visibility,
                        worksheet,
                        part,
                        external_target,
                    });
                }
                _ => {}
            }
        }
        check_cancelled(token, generation)?;

        let number_kinds = match relationships
            .iter()
            .find(|relationship| relationship.kind.ends_with(b"/styles") && !relationship.external)
        {
            Some(relationship) => read_styles(source, &directory, &relationship.target, &limits)?,
            None => Vec::new(),
        };
        let strings = match relationships.iter().find(|relationship| {
            relationship.kind.ends_with(b"/sharedStrings") && !relationship.external
        }) {
            Some(relationship) => read_shared_strings(
                source,
                &directory,
                &relationship.target,
                &limits,
                token,
                generation,
            )?,
            None => SharedStrings::default(),
        };
        Ok(Self {
            directory,
            limits,
            sheets,
            context: CellContext {
                strings,
                number_kinds,
                date1904,
            },
        })
    }

    /// Sheets in workbook order, including hidden ones.
    #[must_use]
    pub fn sheets(&self) -> &[XlsxSheet] {
        &self.sheets
    }

    /// The sheet that opens by default: the first visible worksheet.
    #[must_use]
    pub fn default_sheet(&self) -> Option<usize> {
        self.sheets.iter().position(XlsxSheet::is_offered)
    }

    /// True when the workbook uses the 1904 date system.
    #[must_use]
    pub const fn uses_1904_dates(&self) -> bool {
        self.context.date1904
    }

    /// Number of shared strings.
    #[must_use]
    pub fn shared_string_count(&self) -> usize {
        self.context.strings.ends.len()
    }

    /// Bytes retained for shared strings and number formats.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.context.strings.arena.capacity()
            + self.context.strings.ends.capacity() * std::mem::size_of::<u32>()
            + self.context.number_kinds.capacity() * std::mem::size_of::<NumberKind>()
    }

    /// The limits this workbook was opened with.
    #[must_use]
    pub const fn limits(&self) -> XlsxLimits {
        self.limits
    }

    fn open_sheet_stream<R: PositionedRead + ?Sized>(
        &self,
        source: &R,
        sheet: usize,
    ) -> Result<EntryStream, XlsxError> {
        let sheet = self.sheets.get(sheet).ok_or(XlsxError::SheetOutOfRange)?;
        if let Some(target) = &sheet.external_target {
            return Err(XlsxError::ExternalRelationship {
                target: target.clone(),
            });
        }
        if !sheet.worksheet {
            return Err(XlsxError::NotWorksheet);
        }
        let part = sheet.part.as_deref().ok_or(XlsxError::NotWorksheet)?;
        let entry = self
            .directory
            .find(part)
            .ok_or_else(|| XlsxError::MissingPart {
                name: part.to_owned(),
            })?;
        self.directory.open(source, entry, &self.limits)
    }
}

fn read_styles<R: PositionedRead + ?Sized>(
    source: &R,
    directory: &ZipDirectory,
    part: &str,
    limits: &XlsxLimits,
) -> Result<Vec<NumberKind>, XlsxError> {
    let Some(mut cursor) = PartCursor::open(source, directory, part, limits, 0)? else {
        return Err(XlsxError::MissingPart {
            name: part.to_owned(),
        });
    };
    let mut custom: Vec<(u32, NumberKind)> = Vec::new();
    let mut format_ids: Vec<u32> = Vec::new();
    let mut in_formats = false;
    let mut in_cell_formats = false;
    let mut value = Vec::new();
    while let Some(event) = cursor.next(source)? {
        let name = cursor.xml.name();
        match event {
            XmlEvent::Start { empty } => match name {
                b"numFmts" => in_formats = !empty,
                b"cellXfs" => in_cell_formats = !empty,
                b"numFmt" if in_formats => {
                    if custom.len() >= limits.max_styles {
                        return Err(XlsxError::TooManyStyles {
                            limit: limits.max_styles,
                        });
                    }
                    let id = if cursor.xml.attribute(b"numFmtId", false, &mut value)? {
                        parse_u32(&value)
                    } else {
                        None
                    };
                    let kind = if cursor.xml.attribute(b"formatCode", false, &mut value)? {
                        format_code_kind(&value)
                    } else {
                        NumberKind::Plain
                    };
                    if let Some(id) = id {
                        custom.push((id, kind));
                    }
                }
                b"xf" if in_cell_formats => {
                    if format_ids.len() >= limits.max_styles {
                        return Err(XlsxError::TooManyStyles {
                            limit: limits.max_styles,
                        });
                    }
                    let id = if cursor.xml.attribute(b"numFmtId", false, &mut value)? {
                        parse_u32(&value).unwrap_or(0)
                    } else {
                        0
                    };
                    format_ids.push(id);
                }
                _ => {}
            },
            XmlEvent::End => match name {
                b"numFmts" => in_formats = false,
                b"cellXfs" => in_cell_formats = false,
                _ => {}
            },
            XmlEvent::Text => {}
        }
    }
    Ok(format_ids
        .into_iter()
        .map(|id| {
            custom
                .iter()
                .find(|(custom_id, _)| *custom_id == id)
                .map_or_else(|| builtin_kind(id), |(_, kind)| *kind)
        })
        .collect())
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes).ok()?.trim().parse().ok()
}

fn read_shared_strings<R: PositionedRead + ?Sized>(
    source: &R,
    directory: &ZipDirectory,
    part: &str,
    limits: &XlsxLimits,
    token: &CancellationToken,
    generation: CancellationGeneration,
) -> Result<SharedStrings, XlsxError> {
    let Some(mut cursor) =
        PartCursor::open(source, directory, part, limits, limits.max_cell_bytes)?
    else {
        return Err(XlsxError::MissingPart {
            name: part.to_owned(),
        });
    };
    let mut strings = SharedStrings::default();
    let mut in_item = false;
    let mut phonetic_depth = 0_u32;
    let mut collecting = false;
    let mut item_start = 0_usize;
    let mut item_truncated = false;
    let mut total_text = 0_u64;
    let mut events = 0_u32;
    while let Some(event) = cursor.next(source)? {
        events = events.wrapping_add(1);
        if events.is_multiple_of(4_096) {
            check_cancelled(token, generation)?;
        }
        match event {
            XmlEvent::Start { empty } => match cursor.xml.name() {
                b"si" => {
                    in_item = true;
                    phonetic_depth = 0;
                    item_start = strings.arena.len();
                    item_truncated = false;
                    if empty {
                        in_item = false;
                        push_string(&mut strings, limits, item_start, false)?;
                    }
                }
                b"rPh" if !empty => phonetic_depth += 1,
                b"t" if in_item && phonetic_depth == 0 && !empty => {
                    cursor.xml.collect_text(true);
                    collecting = true;
                }
                _ => {}
            },
            XmlEvent::End => match cursor.xml.name() {
                b"t" if collecting => {
                    total_text += cursor.xml.text_total();
                    if total_text > limits.max_shared_string_bytes {
                        return Err(XlsxError::SharedStringsTooLarge {
                            limit: limits.max_shared_string_bytes,
                        });
                    }
                    let used = strings.arena.len() - item_start;
                    let room = limits.max_cell_bytes.saturating_sub(used);
                    let text = cursor.xml.text();
                    let kept = utf8_prefix(text, room);
                    if kept.len() < text.len() || cursor.xml.text_truncated() {
                        item_truncated = true;
                    }
                    strings
                        .arena
                        .try_reserve(kept.len())
                        .map_err(|_| XlsxError::AllocationFailed)?;
                    strings.arena.extend_from_slice(kept);
                    cursor.xml.collect_text(false);
                    collecting = false;
                }
                b"rPh" => phonetic_depth = phonetic_depth.saturating_sub(1),
                b"si" if in_item => {
                    in_item = false;
                    push_string(&mut strings, limits, item_start, item_truncated)?;
                }
                _ => {}
            },
            XmlEvent::Text => {}
        }
    }
    Ok(strings)
}

fn push_string(
    strings: &mut SharedStrings,
    limits: &XlsxLimits,
    start: usize,
    truncated: bool,
) -> Result<(), XlsxError> {
    if strings.ends.len() >= limits.max_shared_strings {
        return Err(XlsxError::TooManySharedStrings {
            limit: limits.max_shared_strings,
        });
    }
    let end = u32::try_from(strings.arena.len())
        .ok()
        .filter(|end| end & TRUNCATED_BIT == 0)
        .ok_or(XlsxError::SharedStringsTooLarge {
            limit: limits.max_shared_string_bytes,
        })?;
    debug_assert!(start <= end as usize);
    strings
        .ends
        .try_reserve(1)
        .map_err(|_| XlsxError::AllocationFailed)?;
    strings
        .ends
        .push(if truncated { end | TRUNCATED_BIT } else { end });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{relationships_part, resolve_target, utf8_prefix};

    #[test]
    fn relationship_targets_resolve_like_opc_part_names() {
        assert_eq!(
            resolve_target("xl/", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve_target("xl/", "/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(resolve_target("xl/sub/", "../styles.xml"), "xl/styles.xml");
        assert_eq!(
            relationships_part("xl/workbook.xml"),
            "xl/_rels/workbook.xml.rels"
        );
    }

    #[test]
    fn utf8_prefix_never_splits_a_character() {
        let text = "aé€".as_bytes();
        assert_eq!(utf8_prefix(text, 2), b"a");
        assert_eq!(utf8_prefix(text, 3), "aé".as_bytes());
        assert_eq!(utf8_prefix(text, 10), text);
    }
}
