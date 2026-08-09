use crate::{CsvDialect, RecordPreview, RecordSpan};
use core::fmt;

/// Output limits for projecting fields from one visible record prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldProjectionLimits {
    fields: usize,
    decoded_bytes_per_field: usize,
    total_decoded_bytes: usize,
}

impl FieldProjectionLimits {
    /// Creates projection limits.
    ///
    /// # Errors
    /// At least one field slot is required so an empty record remains
    /// representable.
    pub const fn new(
        max_fields: usize,
        max_decoded_bytes_per_field: usize,
        max_total_decoded_bytes: usize,
    ) -> Result<Self, FieldProjectionError> {
        if max_fields == 0 {
            return Err(FieldProjectionError::ZeroFieldLimit);
        }
        Ok(Self {
            fields: max_fields,
            decoded_bytes_per_field: max_decoded_bytes_per_field,
            total_decoded_bytes: max_total_decoded_bytes,
        })
    }

    #[must_use]
    pub const fn max_fields(self) -> usize {
        self.fields
    }

    #[must_use]
    pub const fn max_decoded_bytes_per_field(self) -> usize {
        self.decoded_bytes_per_field
    }

    #[must_use]
    pub const fn max_total_decoded_bytes(self) -> usize {
        self.total_decoded_bytes
    }
}

/// UTF-8 state of the bounded decoded byte prefix retained for a field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldUtf8Status {
    /// Every retained byte forms valid UTF-8.
    Valid,
    /// A complete, invalid UTF-8 sequence occurs in the retained bytes.
    Invalid,
    /// Truncation cut off a potentially valid multi-byte sequence.
    Incomplete,
}

/// A bounded decoded prefix plus the exact visible raw byte span of one field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldProjection {
    raw_span: RecordSpan,
    decoded_bytes: Vec<u8>,
    flags: FieldFlags,
    utf8_status: FieldUtf8Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FieldFlags(u8);

impl FieldFlags {
    const QUOTED: u8 = 1;
    const ESCAPED_QUOTES: u8 = 2;
    const RAW_TRUNCATED: u8 = 4;
    const DECODED_TRUNCATED: u8 = 8;

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

impl FieldProjection {
    /// Returns the absolute span of the raw bytes visible for this field.
    ///
    /// Quotes and escaped quote pairs remain inside this span. A truncated raw
    /// span ends at the visible record-prefix boundary.
    #[must_use]
    pub const fn raw_span(&self) -> RecordSpan {
        self.raw_span
    }

    /// Returns the bounded, unescaped value prefix.
    #[must_use]
    pub fn decoded_bytes(&self) -> &[u8] {
        &self.decoded_bytes
    }

    /// Borrows the decoded prefix as UTF-8 without allocating.
    ///
    /// # Errors
    /// Returns the standard UTF-8 error when the retained bytes are invalid or
    /// incomplete.
    pub fn utf8(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.decoded_bytes)
    }

    #[must_use]
    pub const fn was_quoted(&self) -> bool {
        self.flags.contains(FieldFlags::QUOTED)
    }

    #[must_use]
    pub const fn had_escaped_quotes(&self) -> bool {
        self.flags.contains(FieldFlags::ESCAPED_QUOTES)
    }

    /// Reports that the raw field extends beyond the visible record prefix.
    #[must_use]
    pub const fn is_raw_truncated(&self) -> bool {
        self.flags.contains(FieldFlags::RAW_TRUNCATED)
    }

    /// Reports that a decoded byte limit omitted value bytes.
    #[must_use]
    pub const fn is_decoded_truncated(&self) -> bool {
        self.flags.contains(FieldFlags::DECODED_TRUNCATED)
    }

    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.is_raw_truncated() || self.is_decoded_truncated()
    }

    #[must_use]
    pub const fn utf8_status(&self) -> FieldUtf8Status {
        self.utf8_status
    }
}

/// Bounded field projections for one visible delimited record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelimitedRecordProjection {
    record_span: RecordSpan,
    fields: Vec<FieldProjection>,
    fields_truncated: bool,
}

impl DelimitedRecordProjection {
    #[must_use]
    pub const fn record_span(&self) -> RecordSpan {
        self.record_span
    }

    #[must_use]
    pub fn fields(&self) -> &[FieldProjection] {
        &self.fields
    }

    /// Reports that record-prefix truncation or the field-count cap may have
    /// omitted fields.
    #[must_use]
    pub const fn are_fields_truncated(&self) -> bool {
        self.fields_truncated
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldState {
    Start,
    Unquoted,
    Quoted,
    QuoteClosed,
}

struct FieldAccumulator {
    raw_start: usize,
    decoded: Vec<u8>,
    decoded_limit: usize,
    quoted: bool,
    had_escaped_quotes: bool,
    decoded_truncated: bool,
}

impl FieldAccumulator {
    fn new(raw_start: usize, decoded_limit: usize) -> Self {
        Self {
            raw_start,
            decoded: Vec::new(),
            decoded_limit,
            quoted: false,
            had_escaped_quotes: false,
            decoded_truncated: false,
        }
    }

    fn push_decoded(&mut self, byte: u8) {
        if self.decoded.len() < self.decoded_limit {
            self.decoded.push(byte);
        } else {
            self.decoded_truncated = true;
        }
    }

    fn finish(
        self,
        record_start: u64,
        raw_end: usize,
        raw_truncated: bool,
    ) -> Result<FieldProjection, FieldProjectionError> {
        let raw_start = record_start
            .checked_add(
                u64::try_from(self.raw_start).map_err(|_| FieldProjectionError::OffsetOverflow)?,
            )
            .ok_or(FieldProjectionError::OffsetOverflow)?;
        let raw_end = record_start
            .checked_add(u64::try_from(raw_end).map_err(|_| FieldProjectionError::OffsetOverflow)?)
            .ok_or(FieldProjectionError::OffsetOverflow)?;
        let raw_span = RecordSpan::new(raw_start, raw_end)
            .map_err(|_| FieldProjectionError::VisiblePrefixOutsideRecord)?;
        let truncated = raw_truncated || self.decoded_truncated;
        let utf8_status = classify_utf8(&self.decoded, truncated);
        let mut flags = 0;
        if self.quoted {
            flags |= FieldFlags::QUOTED;
        }
        if self.had_escaped_quotes {
            flags |= FieldFlags::ESCAPED_QUOTES;
        }
        if raw_truncated {
            flags |= FieldFlags::RAW_TRUNCATED;
        }
        if self.decoded_truncated {
            flags |= FieldFlags::DECODED_TRUNCATED;
        }
        Ok(FieldProjection {
            raw_span,
            decoded_bytes: self.decoded,
            flags: FieldFlags(flags),
            utf8_status,
        })
    }
}

/// Projects CSV/TSV fields from the bounded bytes already materialized for a
/// visible record.
///
/// The parser never reads the rest of a truncated record and never constructs
/// an unbounded string. Escaped quote pairs are unescaped only in the bounded
/// decoded prefix; the raw field span continues to cover their original bytes.
///
/// # Errors
/// Returns [`FieldProjectionError`] when limits or record offsets are invalid.
pub fn project_delimited_record(
    record: &RecordPreview,
    dialect: CsvDialect,
    limits: FieldProjectionLimits,
) -> Result<DelimitedRecordProjection, FieldProjectionError> {
    if limits.fields == 0 {
        return Err(FieldProjectionError::ZeroFieldLimit);
    }
    let visible_len =
        u64::try_from(record.bytes().len()).map_err(|_| FieldProjectionError::OffsetOverflow)?;
    let visible_end = record
        .span()
        .start()
        .checked_add(visible_len)
        .ok_or(FieldProjectionError::OffsetOverflow)?;
    if visible_end > record.span().end() {
        return Err(FieldProjectionError::VisiblePrefixOutsideRecord);
    }
    let raw_record_truncated = visible_end < record.span().end() || record.is_truncated();
    let mut fields = Vec::new();
    let mut total_remaining = limits.total_decoded_bytes;
    let mut omitted_fields = false;
    let mut state = FieldState::Start;
    let mut field = FieldAccumulator::new(0, limits.decoded_bytes_per_field.min(total_remaining));
    let mut cursor = 0;

    while cursor < record.bytes().len() {
        let byte = record.bytes()[cursor];
        match state {
            FieldState::Start if byte == dialect.quote() => {
                field.quoted = true;
                state = FieldState::Quoted;
                cursor += 1;
            }
            FieldState::Start if byte == dialect.delimiter() => {
                finish_field(
                    &mut fields,
                    &mut omitted_fields,
                    &mut total_remaining,
                    field,
                    record.span().start(),
                    cursor,
                    false,
                    limits,
                )?;
                cursor += 1;
                field = new_field(cursor, fields.len(), total_remaining, limits);
            }
            FieldState::Unquoted | FieldState::QuoteClosed if byte == dialect.delimiter() => {
                finish_field(
                    &mut fields,
                    &mut omitted_fields,
                    &mut total_remaining,
                    field,
                    record.span().start(),
                    cursor,
                    false,
                    limits,
                )?;
                cursor += 1;
                field = new_field(cursor, fields.len(), total_remaining, limits);
                state = FieldState::Start;
            }
            FieldState::Quoted if byte == dialect.quote() => {
                state = FieldState::QuoteClosed;
                cursor += 1;
            }
            FieldState::QuoteClosed if byte == dialect.quote() => {
                field.had_escaped_quotes = true;
                field.push_decoded(dialect.quote());
                state = FieldState::Quoted;
                cursor += 1;
            }
            FieldState::Start | FieldState::Unquoted | FieldState::QuoteClosed => {
                field.push_decoded(byte);
                state = FieldState::Unquoted;
                cursor += 1;
            }
            FieldState::Quoted => {
                field.push_decoded(byte);
                cursor += 1;
            }
        }
    }

    finish_field(
        &mut fields,
        &mut omitted_fields,
        &mut total_remaining,
        field,
        record.span().start(),
        record.bytes().len(),
        raw_record_truncated,
        limits,
    )?;
    Ok(DelimitedRecordProjection {
        record_span: record.span(),
        fields,
        fields_truncated: omitted_fields || raw_record_truncated,
    })
}

fn new_field(
    raw_start: usize,
    projected_count: usize,
    total_remaining: usize,
    limits: FieldProjectionLimits,
) -> FieldAccumulator {
    let decoded_limit = if projected_count < limits.fields {
        limits.decoded_bytes_per_field.min(total_remaining)
    } else {
        0
    };
    FieldAccumulator::new(raw_start, decoded_limit)
}

#[allow(clippy::too_many_arguments)]
fn finish_field(
    fields: &mut Vec<FieldProjection>,
    omitted_fields: &mut bool,
    total_remaining: &mut usize,
    field: FieldAccumulator,
    record_start: u64,
    raw_end: usize,
    raw_truncated: bool,
    limits: FieldProjectionLimits,
) -> Result<(), FieldProjectionError> {
    if fields.len() == limits.fields {
        *omitted_fields = true;
        return Ok(());
    }
    let projection = field.finish(record_start, raw_end, raw_truncated)?;
    *total_remaining = total_remaining.saturating_sub(projection.decoded_bytes.len());
    fields.push(projection);
    Ok(())
}

fn classify_utf8(bytes: &[u8], truncated: bool) -> FieldUtf8Status {
    match std::str::from_utf8(bytes) {
        Ok(_) => FieldUtf8Status::Valid,
        Err(error) if truncated && error.error_len().is_none() => FieldUtf8Status::Incomplete,
        Err(_) => FieldUtf8Status::Invalid,
    }
}

/// A malformed projection request independent of source content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldProjectionError {
    ZeroFieldLimit,
    OffsetOverflow,
    VisiblePrefixOutsideRecord,
}

impl fmt::Display for FieldProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFieldLimit => formatter.write_str("field projection limit must be non-zero"),
            Self::OffsetOverflow => formatter.write_str("field projection offset overflowed u64"),
            Self::VisiblePrefixOutsideRecord => {
                formatter.write_str("visible bytes fall outside the record span")
            }
        }
    }
}

impl std::error::Error for FieldProjectionError {}

#[cfg(test)]
mod tests {
    use super::{FieldProjectionLimits, FieldUtf8Status, project_delimited_record};
    use crate::{
        AdaptiveRowIndex, CancellationToken, CsvDialect, EngineLimits, ManagedMemoryBudget,
        PositionedRead, RecordSpan, Viewport, ViewportFormat, read_viewport,
    };
    use std::io;

    struct MemorySource(Vec<u8>);

    impl PositionedRead for MemorySource {
        fn size(&self) -> io::Result<u64> {
            u64::try_from(self.0.len()).map_err(io::Error::other)
        }

        fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            if start >= self.0.len() {
                return Ok(0);
            }
            let count = destination.len().min(self.0.len() - start);
            destination[..count].copy_from_slice(&self.0[start..start + count]);
            Ok(count)
        }
    }

    fn viewport(
        input: Vec<u8>,
        preview_bytes: usize,
    ) -> Result<Viewport, Box<dyn std::error::Error>> {
        let source = MemorySource(input);
        let index = AdaptiveRowIndex::new(8, 1)?;
        let token = CancellationToken::new();
        let payload = preview_bytes.max(1);
        let budget = ManagedMemoryBudget::new(8_192, 127, payload, 512, 512)?;
        let limits = EngineLimits::new(127, 1, preview_bytes, payload, 11, budget)?;
        Ok(read_viewport(
            &source,
            ViewportFormat::Delimited(CsvDialect::csv()),
            &index,
            0,
            limits,
            &token,
            token.snapshot(),
        )?)
    }

    #[test]
    fn escaped_quotes_are_decoded_while_raw_spans_remain_exact()
    -> Result<(), Box<dyn std::error::Error>> {
        let visible = viewport(b"\"a\"\"b\",plain,\n".to_vec(), 64)?;
        let record = &visible.records()[0];
        let limits = FieldProjectionLimits::new(8, 64, 128)?;
        let projected = project_delimited_record(record, CsvDialect::csv(), limits)?;
        assert_eq!(projected.fields().len(), 3);
        assert_eq!(projected.fields()[0].raw_span(), RecordSpan::new(0, 6)?);
        assert_eq!(projected.fields()[0].decoded_bytes(), b"a\"b");
        assert!(projected.fields()[0].was_quoted());
        assert!(projected.fields()[0].had_escaped_quotes());
        assert_eq!(projected.fields()[1].raw_span(), RecordSpan::new(7, 12)?);
        assert_eq!(projected.fields()[2].raw_span(), RecordSpan::new(13, 13)?);
        assert!(!projected.are_fields_truncated());
        Ok(())
    }

    #[test]
    fn giant_field_projection_stays_within_both_visible_and_decoded_caps()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut input = vec![b'x'; 1024 * 1024];
        input.push(b'\n');
        let visible = viewport(input, 64)?;
        let record = &visible.records()[0];
        assert_eq!(record.bytes().len(), 64);
        assert!(record.is_truncated());

        let limits = FieldProjectionLimits::new(2, 8, 8)?;
        let projected = project_delimited_record(record, CsvDialect::csv(), limits)?;
        assert_eq!(projected.fields().len(), 1);
        let field = &projected.fields()[0];
        assert_eq!(field.decoded_bytes().len(), 8);
        assert_eq!(field.raw_span(), RecordSpan::new(0, 64)?);
        assert!(field.is_raw_truncated());
        assert!(field.is_decoded_truncated());
        assert!(projected.are_fields_truncated());
        Ok(())
    }

    #[test]
    fn invalid_and_truncation_cut_utf8_are_reported_distinctly()
    -> Result<(), Box<dyn std::error::Error>> {
        let invalid = viewport(vec![b'\"', 0xff, b'\"', b'\n'], 16)?;
        let limits = FieldProjectionLimits::new(2, 8, 8)?;
        let projected = project_delimited_record(&invalid.records()[0], CsvDialect::csv(), limits)?;
        assert_eq!(
            projected.fields()[0].utf8_status(),
            FieldUtf8Status::Invalid
        );
        assert!(projected.fields()[0].utf8().is_err());

        let incomplete = viewport("é\n".as_bytes().to_vec(), 16)?;
        let one_byte = FieldProjectionLimits::new(2, 1, 1)?;
        let projected =
            project_delimited_record(&incomplete.records()[0], CsvDialect::csv(), one_byte)?;
        assert_eq!(
            projected.fields()[0].utf8_status(),
            FieldUtf8Status::Incomplete
        );
        assert!(projected.fields()[0].is_decoded_truncated());
        Ok(())
    }
}
