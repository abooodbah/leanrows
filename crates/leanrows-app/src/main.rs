#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use std::path::PathBuf;

use leanrows_win32::{
    DocumentSmokeColumnKind, DocumentSmokePhase, ShellOptions, ShellOutcome, run_shell,
    show_startup_error,
};

const MAX_DOCUMENT_SMOKE_JSON_BYTES: usize = 64 * 1_024;
const USAGE: &str = "usage: leanrows [--smoke-test | --document-smoke-test] [FILE]";

fn main() {
    let automation_requested = std::env::args_os()
        .skip(1)
        .any(|argument| argument == "--smoke-test" || argument == "--document-smoke-test");
    let mut smoke_test = false;
    let mut document_smoke_test = false;
    let mut initial_path: Option<PathBuf> = None;

    for argument in std::env::args_os().skip(1) {
        if argument == "--smoke-test" {
            smoke_test = true;
        } else if argument == "--document-smoke-test" {
            document_smoke_test = true;
        } else if initial_path.is_none() {
            initial_path = Some(PathBuf::from(argument));
        } else {
            print_usage_and_exit(automation_requested);
        }
    }
    if smoke_test && document_smoke_test {
        print_usage_and_exit(true);
    }
    if document_smoke_test && initial_path.is_none() {
        print_usage_and_exit(true);
    }
    let automation = automation_requested;

    match run_shell(ShellOptions {
        initial_path,
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
    use super::{escape_json, json_string_array};

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
