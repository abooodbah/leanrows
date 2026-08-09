//! Command-line architecture probe for the `LeanRows` bounded-memory engine.
//!
//! This crate intentionally reports observations rather than benchmark claims.

#![forbid(unsafe_code)]

use leanrows_core::{
    AdaptiveRowIndex, CancellationToken, Checkpoint, CsvBoundaryScanner, CsvDiagnostic,
    CsvDiagnosticKind, CsvDialect, EmitControl, EngineLimits, FileSource, LineBoundaryScanner,
    ManagedMemoryBudget, PositionedRead, RecordSpan, ScanStatus, Viewport, ViewportFormat,
    read_viewport,
};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

const READ_BUFFER_BYTES: usize = 64 * 1024;
const VIEWPORT_PAYLOAD_BYTES: usize = 64 * 1024;
const CHECKPOINT_BYTES: usize = 512 * 1024;
const SCRATCH_BYTES: usize = 128 * 1024;
const MANAGED_LIMIT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECORD_PREVIEW_BYTES: usize = 2 * 1024;
const CANCELLATION_CHECK_BYTES: usize = 64 * 1024;
const INITIAL_ROW_STRIDE: u64 = 2_048;
const DEFAULT_VIEWPORT_RECORDS: usize = 20;
const MAX_VIEWPORT_RECORDS: usize = 100;
const HELP: &str = concat!(
    "LeanRows architecture-validation spike\n\n",
    "Usage:\n",
    "  leanrows-spike --scan FILE [--format csv|tsv|jsonl|log] [--row N] [--count N]\n\n",
    "Options:\n",
    "  --scan FILE    File to inspect (required).\n",
    "  --format NAME  Record format. Inferred from .csv, .tsv/.tab, .jsonl/.ndjson,\n",
    "                 or .log when omitted.\n",
    "  --row N        Zero-based first row for an optional bounded viewport.\n",
    "  --count N      Viewport row count, from 1 through 100 (default: 20).\n",
    "  -h, --help     Show this help.\n\n",
    "The JSON output contains observations from this run. Managed-memory values cover\n",
    "configured engine-owned buffers only, not total process memory.",
);

/// Error category used to select a stable process exit code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// Invalid command-line input; maps to exit code 2.
    Usage,
    /// File, scanner, or viewport failure; maps to exit code 1.
    Runtime,
}

/// A concise command-line failure with a stable category.
#[derive(Debug)]
pub struct AppError {
    kind: ErrorKind,
    message: String,
}

impl AppError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Usage,
            message: message.into(),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Runtime,
            message: message.into(),
        }
    }

    /// Returns the conventional command-line exit code for this failure.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self.kind {
            ErrorKind::Usage => 2,
            ErrorKind::Runtime => 1,
        }
    }

    /// Returns whether the error came from command-line usage or execution.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputFormat {
    Csv,
    Tsv,
    Jsonl,
    Log,
}

impl InputFormat {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value.to_ascii_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "tsv" => Ok(Self::Tsv),
            "jsonl" => Ok(Self::Jsonl),
            "log" => Ok(Self::Log),
            _ => Err(AppError::usage(
                "--format must be one of: csv, tsv, jsonl, log",
            )),
        }
    }

    fn infer(path: &Path) -> Result<Self, AppError> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("csv") => Ok(Self::Csv),
            Some("tsv" | "tab") => Ok(Self::Tsv),
            Some("jsonl" | "ndjson") => Ok(Self::Jsonl),
            Some("log") => Ok(Self::Log),
            _ => Err(AppError::usage(
                "could not infer the format; pass --format csv|tsv|jsonl|log",
            )),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Jsonl => "jsonl",
            Self::Log => "log",
        }
    }

    const fn viewport_format(self) -> ViewportFormat {
        match self {
            Self::Csv => ViewportFormat::Delimited(CsvDialect::csv()),
            Self::Tsv => ViewportFormat::Delimited(CsvDialect::tsv()),
            Self::Jsonl | Self::Log => ViewportFormat::Lines,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ViewportRequest {
    row: u64,
    count: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    path: PathBuf,
    format: InputFormat,
    viewport: Option<ViewportRequest>,
}

enum ParsedCommand {
    Help,
    Scan(Arguments),
}

/// Executes the architecture probe and returns either help text or one JSON object.
///
/// Arguments must not include the executable name. The scan reads fixed-size chunks,
/// never the whole input at once. Output measurements describe this invocation only.
///
/// # Errors
/// Returns [`AppError`] for invalid arguments, file access failures, scan failures,
/// or viewport reconstruction failures.
pub fn run<I>(arguments: I) -> Result<String, AppError>
where
    I: IntoIterator<Item = OsString>,
{
    let command = parse_arguments(arguments)?;
    let ParsedCommand::Scan(arguments) = command else {
        return Ok(HELP.to_owned());
    };

    let started = Instant::now();
    let budget = managed_budget()?;
    let source = FileSource::open(&arguments.path)
        .map_err(|error| AppError::runtime(format!("could not open input: {error}")))?;
    let scan = scan_source(&source, arguments.format)?;
    let viewport = match arguments.viewport {
        Some(request) => {
            let limits = EngineLimits::new(
                READ_BUFFER_BYTES,
                request.count,
                MAX_RECORD_PREVIEW_BYTES,
                VIEWPORT_PAYLOAD_BYTES,
                CANCELLATION_CHECK_BYTES,
                budget,
            )
            .map_err(|error| {
                AppError::runtime(format!("invalid managed-memory configuration: {error:?}"))
            })?;
            let token = CancellationToken::new();
            let generation = token.snapshot();
            Some(
                read_viewport(
                    &source,
                    arguments.format.viewport_format(),
                    &scan.index,
                    request.row,
                    limits,
                    &token,
                    generation,
                )
                .map_err(|error| {
                    AppError::runtime(format!("viewport reconstruction failed: {error:?}"))
                })?,
            )
        }
        None => None,
    };
    let elapsed_ms = started.elapsed().as_millis();
    Ok(render_summary(
        arguments.format,
        &scan,
        arguments.viewport,
        viewport.as_ref(),
        budget,
        elapsed_ms,
    ))
}

fn parse_arguments<I>(arguments: I) -> Result<ParsedCommand, AppError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut iterator = arguments.into_iter();
    let mut path = None;
    let mut format = None;
    let mut row = None;
    let mut count = None;
    let mut saw_any = false;

    while let Some(argument) = iterator.next() {
        saw_any = true;
        let option = argument
            .to_str()
            .ok_or_else(|| AppError::usage("option names must be valid Unicode"))?;
        match option {
            "-h" | "--help" => return Ok(ParsedCommand::Help),
            "--scan" => {
                reject_duplicate(path.is_some(), "--scan")?;
                path = Some(PathBuf::from(next_value(&mut iterator, "--scan")?));
            }
            "--format" => {
                reject_duplicate(format.is_some(), "--format")?;
                let value = next_value(&mut iterator, "--format")?;
                let text = value
                    .to_str()
                    .ok_or_else(|| AppError::usage("--format must be valid Unicode"))?;
                format = Some(InputFormat::parse(text)?);
            }
            "--row" => {
                reject_duplicate(row.is_some(), "--row")?;
                row = Some(parse_u64(&next_value(&mut iterator, "--row")?, "--row")?);
            }
            "--count" => {
                reject_duplicate(count.is_some(), "--count")?;
                let parsed = parse_usize(&next_value(&mut iterator, "--count")?, "--count")?;
                if !(1..=MAX_VIEWPORT_RECORDS).contains(&parsed) {
                    return Err(AppError::usage(format!(
                        "--count must be between 1 and {MAX_VIEWPORT_RECORDS}"
                    )));
                }
                count = Some(parsed);
            }
            _ => return Err(AppError::usage(format!("unknown option: {option}"))),
        }
    }

    if !saw_any {
        return Err(AppError::usage("missing --scan FILE; use --help for usage"));
    }
    let path = path.ok_or_else(|| AppError::usage("missing required --scan FILE"))?;
    let format = match format {
        Some(value) => value,
        None => InputFormat::infer(&path)?,
    };
    let viewport = if row.is_some() || count.is_some() {
        Some(ViewportRequest {
            row: row.unwrap_or(0),
            count: count.unwrap_or(DEFAULT_VIEWPORT_RECORDS),
        })
    } else {
        None
    };
    Ok(ParsedCommand::Scan(Arguments {
        path,
        format,
        viewport,
    }))
}

fn next_value<I>(iterator: &mut I, option: &str) -> Result<OsString, AppError>
where
    I: Iterator<Item = OsString>,
{
    iterator
        .next()
        .ok_or_else(|| AppError::usage(format!("missing value for {option}")))
}

fn reject_duplicate(present: bool, option: &str) -> Result<(), AppError> {
    if present {
        Err(AppError::usage(format!("duplicate option: {option}")))
    } else {
        Ok(())
    }
}

fn parse_u64(value: &OsString, option: &str) -> Result<u64, AppError> {
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| AppError::usage(format!("{option} requires a non-negative integer")))
}

fn parse_usize(value: &OsString, option: &str) -> Result<usize, AppError> {
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| AppError::usage(format!("{option} requires a non-negative integer")))
}

fn managed_budget() -> Result<ManagedMemoryBudget, AppError> {
    ManagedMemoryBudget::new(
        MANAGED_LIMIT_BYTES,
        READ_BUFFER_BYTES,
        VIEWPORT_PAYLOAD_BYTES,
        CHECKPOINT_BYTES,
        SCRATCH_BYTES,
    )
    .map_err(|error| AppError::runtime(format!("invalid managed-memory configuration: {error:?}")))
}

struct ScanResult {
    file_size_bytes: u64,
    record_count: u64,
    diagnostic: Option<CsvDiagnostic>,
    index: AdaptiveRowIndex,
}

struct ScanCollector {
    record_count: u64,
    index: AdaptiveRowIndex,
    error: Option<AppError>,
}

impl ScanCollector {
    fn accept(&mut self, span: RecordSpan) -> EmitControl {
        let checkpoint = Checkpoint::new(self.record_count, span.start());
        if let Err(error) = self.index.observe(checkpoint) {
            self.error = Some(AppError::runtime(format!(
                "checkpoint indexing failed: {error:?}"
            )));
            return EmitControl::Stop;
        }
        let Some(next) = self.record_count.checked_add(1) else {
            self.error = Some(AppError::runtime("record count overflowed u64"));
            return EmitControl::Stop;
        };
        self.record_count = next;
        EmitControl::Continue
    }

    fn take_error(&mut self) -> Result<(), AppError> {
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn scan_source(source: &FileSource, format: InputFormat) -> Result<ScanResult, AppError> {
    let file_size_bytes = source
        .size()
        .map_err(|error| AppError::runtime(format!("could not read input size: {error}")))?;
    let checkpoint_capacity = CHECKPOINT_BYTES / std::mem::size_of::<Checkpoint>();
    let index =
        AdaptiveRowIndex::new(checkpoint_capacity, INITIAL_ROW_STRIDE).map_err(|error| {
            AppError::runtime(format!("could not create checkpoint index: {error:?}"))
        })?;
    let mut collector = ScanCollector {
        record_count: 0,
        index,
        error: None,
    };
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];

    let diagnostic = match format {
        InputFormat::Csv | InputFormat::Tsv => {
            let dialect = if format == InputFormat::Csv {
                CsvDialect::csv()
            } else {
                CsvDialect::tsv()
            };
            let mut scanner = CsvBoundaryScanner::new(dialect);
            while scanner.offset() < file_size_bytes {
                let count = read_chunk(source, scanner.offset(), file_size_bytes, &mut buffer)?;
                let progress = scanner
                    .feed(&buffer[..count], |span| collector.accept(span))
                    .map_err(|error| AppError::runtime(format!("scan failed: {error}")))?;
                collector.take_error()?;
                if progress.status() != ScanStatus::Complete || progress.consumed() != count {
                    return Err(AppError::runtime(
                        "scanner stopped before the input boundary",
                    ));
                }
            }
            let finish = scanner.finish(|span| collector.accept(span));
            collector.take_error()?;
            if finish.stopped() {
                return Err(AppError::runtime("scanner stopped during finalization"));
            }
            finish.diagnostic()
        }
        InputFormat::Jsonl | InputFormat::Log => {
            let mut scanner = LineBoundaryScanner::new();
            while scanner.offset() < file_size_bytes {
                let count = read_chunk(source, scanner.offset(), file_size_bytes, &mut buffer)?;
                let progress = scanner
                    .feed(&buffer[..count], |span| collector.accept(span))
                    .map_err(|error| AppError::runtime(format!("scan failed: {error}")))?;
                collector.take_error()?;
                if progress.status() != ScanStatus::Complete || progress.consumed() != count {
                    return Err(AppError::runtime(
                        "scanner stopped before the input boundary",
                    ));
                }
            }
            let finish = scanner.finish(|span| collector.accept(span));
            collector.take_error()?;
            if finish.stopped() {
                return Err(AppError::runtime("scanner stopped during finalization"));
            }
            None
        }
    };

    Ok(ScanResult {
        file_size_bytes,
        record_count: collector.record_count,
        diagnostic,
        index: collector.index,
    })
}

fn read_chunk(
    source: &FileSource,
    offset: u64,
    file_size_bytes: u64,
    buffer: &mut [u8],
) -> Result<usize, AppError> {
    let remaining = file_size_bytes - offset;
    let request = usize::try_from(remaining)
        .unwrap_or(usize::MAX)
        .min(buffer.len());
    let count = source
        .read_at(offset, &mut buffer[..request])
        .map_err(|error| AppError::runtime(format!("input read failed: {error}")))?;
    if count == 0 {
        return Err(AppError::runtime(
            "input became shorter than its opening snapshot",
        ));
    }
    Ok(count)
}

fn render_summary(
    format: InputFormat,
    scan: &ScanResult,
    request: Option<ViewportRequest>,
    viewport: Option<&Viewport>,
    budget: ManagedMemoryBudget,
    elapsed_ms: u128,
) -> String {
    let mut output = String::with_capacity(1_024);
    output.push_str("{\"schema_version\":1,\"status\":\"observed\",\"format\":");
    append_json_string(&mut output, format.name());
    output.push_str(",\"file_size_bytes\":");
    output.push_str(&scan.file_size_bytes.to_string());
    output.push_str(",\"record_count\":");
    output.push_str(&scan.record_count.to_string());
    output.push_str(",\"elapsed_ms\":");
    output.push_str(&elapsed_ms.to_string());
    output.push_str(",\"elapsed_kind\":\"wall_clock_observation\",\"malformed_csv\":");
    append_diagnostic(&mut output, scan.diagnostic);
    output.push_str(
        ",\"managed_memory\":{\"scope\":\"configured_engine_owned_buffers_only\",\"limit_bytes\":",
    );
    output.push_str(&budget.limit_bytes().to_string());
    output.push_str(",\"committed_bytes\":");
    output.push_str(&budget.committed_bytes().to_string());
    output.push_str(",\"read_buffers_bytes\":");
    output.push_str(&budget.read_buffers_bytes().to_string());
    output.push_str(",\"viewport_payload_bytes\":");
    output.push_str(&budget.viewport_payload_bytes().to_string());
    output.push_str(",\"checkpoint_bytes\":");
    output.push_str(&budget.checkpoint_bytes().to_string());
    output.push_str(",\"scratch_bytes\":");
    output.push_str(&budget.scratch_bytes().to_string());
    output.push_str("},\"viewport\":");
    append_viewport(&mut output, request, viewport);
    output.push('}');
    output
}

fn append_diagnostic(output: &mut String, diagnostic: Option<CsvDiagnostic>) {
    let Some(diagnostic) = diagnostic else {
        output.push_str("null");
        return;
    };
    output.push_str("{\"kind\":");
    let kind = match diagnostic.kind() {
        CsvDiagnosticKind::UnterminatedQuotedField => "unterminated_quoted_field",
    };
    append_json_string(output, kind);
    output.push_str(",\"offset\":");
    output.push_str(&diagnostic.offset().to_string());
    output.push_str(",\"record_start\":");
    output.push_str(&diagnostic.record_start().to_string());
    output.push('}');
}

fn append_viewport(
    output: &mut String,
    request: Option<ViewportRequest>,
    viewport: Option<&Viewport>,
) {
    let (Some(request), Some(viewport)) = (request, viewport) else {
        output.push_str("null");
        return;
    };
    output.push_str("{\"requested_row\":");
    output.push_str(&request.row.to_string());
    output.push_str(",\"requested_count\":");
    output.push_str(&request.count.to_string());
    output.push_str(",\"returned_count\":");
    output.push_str(&viewport.records().len().to_string());
    output.push_str(",\"reached_eof\":");
    output.push_str(if viewport.reached_eof() {
        "true"
    } else {
        "false"
    });
    output.push_str(",\"records\":[");
    for (position, record) in viewport.records().iter().enumerate() {
        if position != 0 {
            output.push(',');
        }
        output.push_str("{\"row\":");
        output.push_str(&record.row().to_string());
        output.push_str(",\"start_byte\":");
        output.push_str(&record.span().start().to_string());
        output.push_str(",\"end_byte\":");
        output.push_str(&record.span().end().to_string());
        output.push_str(",\"truncated\":");
        output.push_str(if record.is_truncated() {
            "true"
        } else {
            "false"
        });
        output.push_str(",\"preview_utf8_lossy\":");
        append_json_string(output, &String::from_utf8_lossy(record.bytes()));
        output.push('}');
    }
    output.push_str("]}");
}

fn append_json_string(output: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control <= '\u{1f}' => {
                let code = control as usize;
                output.push_str("\\u00");
                output.push(char::from(HEX[(code >> 4) & 0x0f]));
                output.push(char::from(HEX[code & 0x0f]));
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::{
        AppError, ErrorKind, InputFormat, ParsedCommand, append_json_string, parse_arguments, run,
    };
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    struct TempFile(PathBuf);

    impl TempFile {
        fn create(extension: &str, contents: &[u8]) -> Result<Self, std::io::Error> {
            let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "leanrows-spike-test-{}-{sequence}.{extension}",
                std::process::id()
            );
            let path = std::env::temp_dir().join(name);
            fs::write(&path, contents)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _result = fs::remove_file(&self.0);
        }
    }

    fn strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_inferred_format_and_default_viewport_count() -> Result<(), Box<dyn Error>> {
        let command = parse_arguments(strings(&["--scan", "sample.CSV", "--row", "7"]))?;
        let ParsedCommand::Scan(arguments) = command else {
            return Err("expected scan command".into());
        };
        assert_eq!(arguments.format, InputFormat::Csv);
        let viewport = arguments.viewport.ok_or("expected viewport")?;
        assert_eq!(viewport.row, 7);
        assert_eq!(viewport.count, 20);
        Ok(())
    }

    #[test]
    fn rejects_unknown_and_duplicate_options() {
        let unknown = parse_arguments(strings(&["--wat"]));
        assert!(matches!(unknown, Err(ref error) if error.kind() == ErrorKind::Usage));
        let duplicate = parse_arguments(strings(&["--scan", "one.csv", "--scan", "two.csv"]));
        assert!(matches!(duplicate, Err(ref error) if error.kind() == ErrorKind::Usage));
    }

    #[test]
    fn escapes_json_control_characters() {
        let mut output = String::new();
        append_json_string(&mut output, "a\n\"b\\c\u{0001}");
        assert_eq!(output, "\"a\\n\\\"b\\\\c\\u0001\"");
    }

    #[test]
    fn scans_multiline_csv_and_reconstructs_a_viewport() -> Result<(), Box<dyn Error>> {
        let file = TempFile::create("csv", b"a,b\n\"multi\nline\",2\nlast,3\n")?;
        let output = run(vec![
            OsString::from("--scan"),
            file.path().as_os_str().to_owned(),
            OsString::from("--row"),
            OsString::from("1"),
            OsString::from("--count"),
            OsString::from("2"),
        ])?;
        assert!(output.contains("\"record_count\":3"));
        assert!(output.contains("\"malformed_csv\":null"));
        assert!(output.contains("\"returned_count\":2"));
        assert!(output.contains("multi\\nline"));
        assert!(!output.contains(&file.path().to_string_lossy().to_string()));
        Ok(())
    }

    #[test]
    fn reports_unterminated_csv_quote_without_rejecting_raw_access() -> Result<(), Box<dyn Error>> {
        let file = TempFile::create("csv", b"ok,1\n\"open\nrecord")?;
        let output = run(vec![
            OsString::from("--scan"),
            file.path().as_os_str().to_owned(),
        ])?;
        assert!(output.contains("\"record_count\":2"));
        assert!(output.contains("\"kind\":\"unterminated_quoted_field\""));
        Ok(())
    }

    #[test]
    fn explicit_format_allows_an_unknown_extension() -> Result<(), AppError> {
        let file = TempFile::create("data", b"{\"n\":1}\n{\"n\":2}\n")
            .map_err(|error| AppError::runtime(error.to_string()))?;
        let output = run(vec![
            OsString::from("--scan"),
            file.path().as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("jsonl"),
        ])?;
        assert!(output.contains("\"format\":\"jsonl\""));
        assert!(output.contains("\"record_count\":2"));
        Ok(())
    }
}
