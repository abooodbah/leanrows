use crate::{CsvDialect, ViewportFormat};

/// A row-oriented document format understood by the core engine.
///
/// JSON Lines and log files share physical line boundaries. Their content is
/// deliberately left uninterpreted by the boundary scanner.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentFormat {
    /// Comma-separated values with CSV-style quoting.
    Csv,
    /// Tab-separated values with CSV-style quoting.
    Tsv,
    /// One JSON value per physical line.
    Jsonl,
    /// Unstructured, line-oriented text.
    Log,
}

impl DocumentFormat {
    /// Returns the boundary rules used to reconstruct visible records.
    #[must_use]
    pub const fn viewport_format(self) -> ViewportFormat {
        match self {
            Self::Csv => ViewportFormat::Delimited(CsvDialect::csv()),
            Self::Tsv => ViewportFormat::Delimited(CsvDialect::tsv()),
            Self::Jsonl | Self::Log => ViewportFormat::Lines,
        }
    }

    /// Returns the CSV-compatible dialect for delimited formats.
    #[must_use]
    pub const fn delimited_dialect(self) -> Option<CsvDialect> {
        match self {
            Self::Csv => Some(CsvDialect::csv()),
            Self::Tsv => Some(CsvDialect::tsv()),
            Self::Jsonl | Self::Log => None,
        }
    }
}

impl From<DocumentFormat> for ViewportFormat {
    fn from(format: DocumentFormat) -> Self {
        format.viewport_format()
    }
}

#[cfg(test)]
mod tests {
    use super::DocumentFormat;
    use crate::{CsvDialect, ViewportFormat};

    #[test]
    fn formats_map_to_their_physical_boundary_rules() {
        assert_eq!(
            DocumentFormat::Csv.viewport_format(),
            ViewportFormat::Delimited(CsvDialect::csv())
        );
        assert_eq!(
            DocumentFormat::Tsv.viewport_format(),
            ViewportFormat::Delimited(CsvDialect::tsv())
        );
        assert_eq!(
            DocumentFormat::Jsonl.viewport_format(),
            ViewportFormat::Lines
        );
        assert_eq!(DocumentFormat::Log.viewport_format(), ViewportFormat::Lines);
        assert_eq!(
            DocumentFormat::Csv.delimited_dialect(),
            Some(CsvDialect::csv())
        );
        assert_eq!(DocumentFormat::Jsonl.delimited_dialect(), None);
    }
}
