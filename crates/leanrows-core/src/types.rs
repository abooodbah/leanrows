use core::fmt;

/// An end-exclusive byte range occupied by one logical record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RecordSpan {
    start: u64,
    end: u64,
}

impl RecordSpan {
    /// Creates a validated record span.
    ///
    /// # Errors
    ///
    /// Returns [`SpanError`] when `end` precedes `start`.
    pub const fn new(start: u64, end: u64) -> Result<Self, SpanError> {
        if end < start {
            return Err(SpanError { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the first byte offset in the record.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Returns the end-exclusive byte offset of the record.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.end
    }

    /// Returns the record length in bytes.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    /// Reports whether this is an empty record.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// A sparse mapping from a logical row to its record-start byte offset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Checkpoint {
    row: u64,
    offset: u64,
}

impl Checkpoint {
    /// Creates a checkpoint. Callers must supply a record boundary.
    #[must_use]
    pub const fn new(row: u64, offset: u64) -> Self {
        Self { row, offset }
    }

    /// Returns the zero-based logical row.
    #[must_use]
    pub const fn row(self) -> u64 {
        self.row
    }

    /// Returns the byte offset at which the row begins.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }
}

/// Describes an invalid end-exclusive span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpanError {
    start: u64,
    end: u64,
}

impl fmt::Display for SpanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "span end {} precedes start {}",
            self.end, self.start
        )
    }
}

impl std::error::Error for SpanError {}
