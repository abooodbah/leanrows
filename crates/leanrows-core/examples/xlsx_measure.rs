//! Times opening, scanning and seeking one `.xlsx` sheet with the core
//! reader, and prints one JSON line. On Linux it also reports the process
//! peak resident set (`VmHWM`); elsewhere that field is null.
//!
//! ```text
//! cargo run --release -p leanrows-core --example xlsx_measure -- tests/fixtures/xlsx/xlsxwriter-large.xlsx
//! ```

use std::time::Instant;

use leanrows_core::{
    CancellationToken, FileSource, XlsxLimits, XlsxRowLimits, XlsxScanStatus, XlsxSheetScan,
    XlsxWorkbook,
};

fn peak_resident_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn milliseconds(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: xlsx_measure FILE.xlsx")?;
    let token = CancellationToken::new();
    let generation = token.snapshot();
    let limits = XlsxRowLimits::standard();

    let open_start = Instant::now();
    let source = FileSource::open(&path)?;
    let workbook = XlsxWorkbook::open(&source, XlsxLimits::standard(), &token, generation)?;
    let sheet = workbook.default_sheet().ok_or("no visible worksheet")?;
    let mut scan = XlsxSheetScan::new(&source, &workbook, sheet)?;
    let first = scan.read_rows(&source, &workbook, 0, 128, limits, &token, generation)?;
    let first_viewport_ms = milliseconds(open_start);

    let scan_start = Instant::now();
    let mut steps = 0_u64;
    while scan.step(&source, 64 * 1_024, &token, generation)?.status() != XlsxScanStatus::Complete {
        steps += 1;
    }
    let scan_ms = milliseconds(scan_start);
    let rows = scan.progress().rows();

    let middle_start = Instant::now();
    let middle = scan.read_rows(
        &source,
        &workbook,
        rows / 2,
        128,
        limits,
        &token,
        generation,
    )?;
    let middle_ms = milliseconds(middle_start);
    let last_start = Instant::now();
    let last_first_row = rows.saturating_sub(128);
    let last = scan.read_rows(
        &source,
        &workbook,
        last_first_row,
        128,
        limits,
        &token,
        generation,
    )?;
    let last_ms = milliseconds(last_start);

    println!(
        "{{\"rows\":{rows},\"shared_strings\":{},\"first_viewport_rows\":{},\"first_viewport_ms\":{first_viewport_ms:.1},\"scan_ms\":{scan_ms:.1},\"scan_steps\":{steps},\"middle_viewport_rows\":{},\"middle_viewport_ms\":{middle_ms:.1},\"last_viewport_rows\":{},\"last_viewport_ms\":{last_ms:.1},\"checkpoints\":{},\"retained_workbook_bytes\":{},\"retained_scan_bytes\":{},\"peak_resident_kib\":{}}}",
        workbook.shared_string_count(),
        first.rows().len(),
        middle.rows().len(),
        last.rows().len(),
        scan.checkpoint_count(),
        workbook.retained_bytes(),
        scan.retained_bytes(),
        peak_resident_kib().map_or_else(|| "null".to_owned(), |kib| kib.to_string()),
    );
    Ok(())
}
