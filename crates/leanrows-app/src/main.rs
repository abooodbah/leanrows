#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;

use leanrows_win32::{
    DocumentSmokeColumnKind, DocumentSmokePhase, ShellOptions, ShellOutcome, run_shell,
    show_startup_error,
};

const MAX_DOCUMENT_SMOKE_JSON_BYTES: usize = 64 * 1_024;
const USAGE: &str = "usage: leanrows [--smoke-test | --document-smoke-test] [--] [FILE...]";

/// A parsed command line.
#[derive(Debug, Default, Eq, PartialEq)]
struct Arguments {
    paths: Vec<PathBuf>,
    smoke_test: bool,
    document_smoke_test: bool,
}

/// A usage error. `automation` is true when an automation flag was present,
/// so the diagnostic goes to stderr instead of a dialog.
#[derive(Debug, Eq, PartialEq)]
struct UsageError {
    automation: bool,
}

fn main() {
    let arguments = match parse_arguments(std::env::args_os().skip(1)) {
        Ok(arguments) => arguments,
        Err(error) => print_usage_and_exit(error.automation),
    };
    let Arguments {
        paths,
        smoke_test,
        document_smoke_test,
    } = arguments;
    let automation = smoke_test || document_smoke_test;

    match run_shell(ShellOptions {
        initial_paths: paths,
        smoke_test,
        document_smoke_test,
    }) {
        Ok(outcome) => {
            if smoke_test && !shell_smoke_passed(&outcome) {
                eprintln!("native shell smoke verification did not complete");
                std::process::exit(1);
            }
            if document_smoke_test {
                let Some(document) = outcome.document_smoke.as_ref() else {
                    eprintln!("document smoke produced no evidence");
                    std::process::exit(1);
                };
                if !shell_smoke_passed(&outcome) {
                    eprintln!("document smoke did not preserve native shell verification");
                    std::process::exit(1);
                }
                let phase = match document.phase {
                    DocumentSmokePhase::ViewportReady => "viewport_ready",
                    DocumentSmokePhase::Scanning => "scanning",
                    DocumentSmokePhase::Complete => "complete",
                };
                let table = match document.column_kind {
                    DocumentSmokeColumnKind::Preview => String::new(),
                    DocumentSmokeColumnKind::Fields => format!(
                        ",\"table\":{{\"column_count\":{},\"sample\":{}}}",
                        document.data_columns,
                        json_string_array(&document.sample_cells),
                    ),
                };
                let json = format!(
                    "{{\"schema\":\"leanrows.document-smoke\",\"version\":1,\"source_bytes\":{},\"cache\":{{\"first_row\":{},\"row_count\":{}}},\"row\":{{\"number\":\"{}\",\"preview\":\"{}\"}}{},\"scan\":{{\"scanned_bytes\":{},\"indexed_rows\":{},\"available_rows\":{},\"complete\":{},\"phase\":\"{}\"}},\"shell\":{{\"controls\":{},\"accessibility\":{},\"keyboard_focus\":{},\"system_colors\":{}}}}}",
                    document.source_bytes,
                    document.cache_first_row,
                    document.cached_rows,
                    escape_json(&document.first_row),
                    escape_json(&document.preview),
                    table,
                    document.scanned_bytes,
                    document.indexed_rows,
                    document.available_rows,
                    document.scan_complete,
                    phase,
                    outcome.smoke_controls_verified,
                    outcome.smoke_accessibility_verified,
                    outcome.smoke_keyboard_focus_verified,
                    outcome.smoke_system_colors_verified,
                );
                if json.len() > MAX_DOCUMENT_SMOKE_JSON_BYTES {
                    eprintln!("document smoke evidence exceeded its output bound");
                    std::process::exit(1);
                }
                println!("{json}");
            }
        }
        Err(error) => {
            report_startup_error(automation, &format!("LeanRows could not start: {error}"));
            std::process::exit(1);
        }
    }
}

/// Files may follow `--` when their names start with `--`. Automation takes
/// at most one file, and the document smoke needs exactly one.
fn parse_arguments(arguments: impl IntoIterator<Item = OsString>) -> Result<Arguments, UsageError> {
    let arguments: Vec<OsString> = arguments.into_iter().collect();
    let automation = arguments
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| argument == "--smoke-test" || argument == "--document-smoke-test");
    let usage = UsageError { automation };
    let mut parsed = Arguments::default();
    let mut options_ended = false;
    for argument in arguments {
        if options_ended {
            parsed.paths.push(PathBuf::from(argument));
        } else if argument == "--" {
            options_ended = true;
        } else if argument == "--smoke-test" {
            parsed.smoke_test = true;
        } else if argument == "--document-smoke-test" {
            parsed.document_smoke_test = true;
        } else if argument.to_str().is_some_and(|text| text.starts_with("--")) {
            return Err(usage);
        } else {
            parsed.paths.push(PathBuf::from(argument));
        }
    }
    let conflicting = parsed.smoke_test && parsed.document_smoke_test;
    let missing_document = parsed.document_smoke_test && parsed.paths.len() != 1;
    let too_many_files = automation && parsed.paths.len() > 1;
    if conflicting || missing_document || too_many_files {
        return Err(usage);
    }
    Ok(parsed)
}

fn json_string_array(values: &[String]) -> String {
    let mut json = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push('"');
        json.push_str(&escape_json(value));
        json.push('"');
    }
    json.push(']');
    json
}

fn shell_smoke_passed(outcome: &ShellOutcome) -> bool {
    outcome.smoke_controls_verified
        && outcome.smoke_accessibility_verified
        && outcome.smoke_keyboard_focus_verified
        && outcome.smoke_system_colors_verified
}

fn escape_json(value: &str) -> String {
    use std::fmt::Write as _;

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if control <= '\u{1F}' => {
                let _ = write!(escaped, "\\u{:04X}", u32::from(control));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

fn report_startup_error(automation: bool, message: &str) {
    if automation {
        eprintln!("{message}");
    } else {
        show_startup_error(message);
    }
}

fn print_usage_and_exit(automation: bool) -> ! {
    report_startup_error(automation, USAGE);
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{Arguments, UsageError, escape_json, json_string_array, parse_arguments};

    fn parse(arguments: &[&str]) -> Result<Arguments, UsageError> {
        parse_arguments(arguments.iter().map(OsString::from))
    }

    #[test]
    fn ordinary_launches_accept_any_number_of_files() {
        assert_eq!(parse(&[]), Ok(Arguments::default()));
        assert_eq!(
            parse(&["a.csv", "b.log", "c.txt"]).map(|parsed| parsed.paths),
            Ok(vec![
                PathBuf::from("a.csv"),
                PathBuf::from("b.log"),
                PathBuf::from("c.txt")
            ])
        );
        assert_eq!(
            parse(&["--", "--smoke-test", "-x.csv"]),
            Ok(Arguments {
                paths: vec![PathBuf::from("--smoke-test"), PathBuf::from("-x.csv")],
                ..Arguments::default()
            })
        );
    }

    #[test]
    fn usage_errors_report_automation_on_stderr_only_for_automation() {
        assert_eq!(parse(&["--bogus"]), Err(UsageError { automation: false }));
        assert_eq!(
            parse(&["--bogus", "--smoke-test"]),
            Err(UsageError { automation: true })
        );
        assert_eq!(
            parse(&["--smoke-test", "--document-smoke-test"]),
            Err(UsageError { automation: true })
        );
        assert_eq!(
            parse(&["first.csv", "second.csv", "--smoke-test"]),
            Err(UsageError { automation: true })
        );
        assert_eq!(
            parse(&["--document-smoke-test"]),
            Err(UsageError { automation: true })
        );
        assert_eq!(
            parse(&["--document-smoke-test", "rows.csv"]),
            Ok(Arguments {
                paths: vec![PathBuf::from("rows.csv")],
                document_smoke_test: true,
                ..Arguments::default()
            })
        );
    }

    #[test]
    fn smoke_json_text_is_single_line_and_escaped() {
        assert_eq!(
            escape_json("quote=\" slash=\\ line=\n tab=\t control=\u{1F}"),
            "quote=\\\" slash=\\\\ line=\\n tab=\\t control=\\u001F"
        );
    }

    #[test]
    fn table_samples_are_bounded_json_strings() {
        assert_eq!(
            json_string_array(&[
                String::from("a,b"),
                String::from("line1\\nline2"),
                String::new()
            ]),
            "[\"a,b\",\"line1\\\\nline2\",\"\"]"
        );
    }
}
