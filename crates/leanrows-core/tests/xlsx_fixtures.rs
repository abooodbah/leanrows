//! `.xlsx` reading against workbooks written by `LibreOffice`, `openpyxl` and
//! `XlsxWriter`, plus hostile files derived from them. The fixtures and their
//! producers are listed in `tests/fixtures/xlsx/README.md`; the expected text
//! follows `docs/XLSX.md`.

use std::path::PathBuf;

use leanrows_core::{
    CancellationToken, FileSource, XlsxError, XlsxLimits, XlsxRowLimits, XlsxRows, XlsxScanStatus,
    XlsxSheetScan, XlsxSheetVisibility, XlsxWorkbook,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/xlsx")
        .join(name)
}

struct Opened {
    source: FileSource,
    workbook: XlsxWorkbook,
    token: CancellationToken,
}

fn open_with(name: &str, limits: XlsxLimits) -> Result<Opened, XlsxError> {
    let source = FileSource::open(fixture(name)).map_err(XlsxError::Io)?;
    let token = CancellationToken::new();
    let workbook = XlsxWorkbook::open(&source, limits, &token, token.snapshot())?;
    Ok(Opened {
        source,
        workbook,
        token,
    })
}

fn open(name: &str) -> Result<Opened, XlsxError> {
    open_with(name, XlsxLimits::standard())
}

impl Opened {
    fn scan(&self, sheet: usize) -> Result<XlsxSheetScan, XlsxError> {
        XlsxSheetScan::new(&self.source, &self.workbook, sheet)
    }

    fn scan_all(&self, sheet: usize) -> Result<XlsxSheetScan, XlsxError> {
        let mut scan = self.scan(sheet)?;
        loop {
            let step = scan.step(&self.source, 64 * 1_024, &self.token, self.token.snapshot())?;
            match step.status() {
                XlsxScanStatus::Complete => return Ok(scan),
                XlsxScanStatus::Progress => {}
                XlsxScanStatus::Cancelled => return Err(XlsxError::Cancelled),
            }
        }
    }

    fn read(
        &self,
        scan: &XlsxSheetScan,
        first_row: u64,
        count: usize,
    ) -> Result<XlsxRows, XlsxError> {
        scan.read_rows(
            &self.source,
            &self.workbook,
            first_row,
            count,
            XlsxRowLimits::standard(),
            &self.token,
            self.token.snapshot(),
        )
    }

    fn texts(
        &self,
        scan: &XlsxSheetScan,
        first_row: u64,
        count: usize,
    ) -> Result<Vec<Vec<String>>, XlsxError> {
        Ok(self
            .read(scan, first_row, count)?
            .rows()
            .iter()
            .map(|row| {
                row.cells()
                    .iter()
                    .map(|cell| String::from_utf8_lossy(cell.bytes()).into_owned())
                    .collect()
            })
            .collect())
    }

    fn column(&self, sheet: usize, column: usize) -> Result<Vec<String>, XlsxError> {
        let scan = self.scan(sheet)?;
        Ok(self
            .read(&scan, 0, 128)?
            .rows()
            .iter()
            .map(|row| row.cell_text(column).to_owned())
            .collect())
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn libreoffice_conversion_of_the_csv_smoke_fixture_shows_the_same_fields() -> TestResult {
    let opened = open("libreoffice-document-smoke.xlsx")?;
    let sheet = opened.workbook.default_sheet().ok_or("no default sheet")?;
    let scan = opened.scan_all(sheet)?;
    assert_eq!(scan.progress().rows(), 1);
    assert!(scan.progress().is_complete());
    let rows = opened.read(&scan, 0, 128)?;
    assert!(rows.reached_end());
    assert_eq!(rows.rows().len(), 1);
    // The CSV's empty fourth field is not stored by LibreOffice, so the row
    // ends at its last stored cell.
    assert_eq!(
        opened.texts(&scan, 0, 128)?,
        vec![strings(&["alpha", "quoted, value", "line1\nline2"])]
    );
    Ok(())
}

#[test]
fn sheets_keep_workbook_order_and_hidden_sheets_are_not_offered() -> TestResult {
    for name in ["openpyxl-features.xlsx", "libreoffice-resaved.xlsx"] {
        let opened = open(name)?;
        let names: Vec<&str> = opened
            .workbook
            .sheets()
            .iter()
            .map(leanrows_core::XlsxSheet::name)
            .collect();
        assert_eq!(
            names,
            [
                "Hidden first",
                "Types",
                "Sparse",
                "Second visible",
                "Very hidden"
            ],
            "{name}"
        );
        assert_eq!(opened.workbook.default_sheet(), Some(1), "{name}");
        let offered: Vec<usize> = opened
            .workbook
            .sheets()
            .iter()
            .enumerate()
            .filter(|(_, sheet)| sheet.is_offered())
            .map(|(index, _)| index)
            .collect();
        assert_eq!(offered, [1, 2, 3], "{name}");
        assert_eq!(
            opened.workbook.sheets()[0].visibility(),
            XlsxSheetVisibility::Hidden
        );
    }
    let opened = open("openpyxl-features.xlsx")?;
    assert_eq!(
        opened.workbook.sheets()[4].visibility(),
        XlsxSheetVisibility::VeryHidden
    );
    Ok(())
}

const OPENPYXL_TYPES: [&str; 26] = [
    "value",
    "plain text",
    "42",
    "0.3",
    "-1.5e-10",
    "TRUE",
    "FALSE",
    "2024-02-29",
    "2024-02-29 13:45:30",
    "06:30:15",
    "1999-12-31",
    "2000-01-01",
    "0.25",
    "12345",
    "#DIV/0!",
    "",
    "line1\nline2\ttab  two spaces ",
    "12:00:00",
    "bold and plain",
    "1900-02-28",
    "1900-02-29",
    "1900-03-01",
    "1900-01-00",
    "-1",
    "2024-03-01 00:00:00",
    "007",
];

#[test]
fn openpyxl_cells_follow_the_documented_text_rules() -> TestResult {
    let opened = open("openpyxl-features.xlsx")?;
    assert_eq!(opened.column(1, 1)?, strings(&OPENPYXL_TYPES));
    assert_eq!(opened.column(1, 0)?[15], "formula without cached value");
    Ok(())
}

#[test]
fn libreoffice_resave_of_the_same_workbook_shows_the_same_text() -> TestResult {
    let opened = open("libreoffice-resaved.xlsx")?;
    let mut expected = strings(&OPENPYXL_TYPES);
    // LibreOffice writes its own number text and computes the formula.
    expected[4] = "-1.5E-010".to_owned();
    expected[15] = "3".to_owned();
    assert_eq!(opened.column(1, 1)?, expected);
    Ok(())
}

#[test]
fn missing_rows_and_cells_are_filled_so_row_numbers_match_the_sheet() -> TestResult {
    let opened = open("openpyxl-features.xlsx")?;
    let scan = opened.scan_all(2)?;
    assert_eq!(scan.progress().rows(), 1_048_576);
    let rows = opened.read(&scan, 0, 12)?;
    let texts = opened.texts(&scan, 0, 12)?;
    assert_eq!(texts[0], strings(&["top-left"]));
    assert!(texts[1].is_empty());
    assert_eq!(texts[2], strings(&["", "", "C3"]));
    assert_eq!(texts[9].len(), 10);
    assert_eq!(texts[9][9], "J10");
    // BR12 is column 70, beyond the 64 shown columns.
    assert!(texts[11].is_empty());
    assert!(rows.rows()[11].is_truncated());
    assert!(!rows.rows()[9].is_truncated());
    for (offset, row) in rows.rows().iter().enumerate() {
        assert_eq!(row.index(), offset as u64);
    }

    let last = opened.read(&scan, 1_048_575, 128)?;
    assert_eq!(last.rows().len(), 1);
    assert_eq!(last.rows()[0].index(), 1_048_575);
    assert_eq!(last.rows()[0].cell_text(0), "last row");
    assert!(last.reached_end());
    assert!(opened.read(&scan, 1_048_576, 128)?.rows().is_empty());
    Ok(())
}

#[test]
fn the_1904_date_system_is_honoured() -> TestResult {
    let opened = open("openpyxl-1904.xlsx")?;
    assert!(opened.workbook.uses_1904_dates());
    assert_eq!(
        opened.column(0, 0)?,
        strings(&["2024-02-29", "1904-01-01", "1904-01-02 12:00:00"])
    );
    Ok(())
}

#[test]
fn xlsxwriter_cached_formula_values_rich_text_and_numbers_are_shown() -> TestResult {
    let opened = open("xlsxwriter-features.xlsx")?;
    assert_eq!(opened.workbook.default_sheet(), Some(0));
    let scan = opened.scan(0)?;
    let texts = opened.texts(&scan, 0, 128)?;
    let first: Vec<String> = texts
        .iter()
        .map(|row| row.first().cloned().unwrap_or_default())
        .collect();
    assert_eq!(
        first,
        strings(&[
            "bold and plain",
            "2",
            "ab",
            "TRUE",
            "#DIV/0!",
            "FALSE",
            "2023-07-14 09:05:00",
            "23:59:59",
            "a_x0001_b",
            "1E+20",
            "3.14159",
            "",
            "0",
            "",
        ])
    );
    assert_eq!(texts[11], strings(&["", "", "", "only D12"]));
    // A styled blank cell still ends the row.
    assert_eq!(texts[13], strings(&["", "", ""]));
    assert_eq!(opened.workbook.shared_string_count(), 5);
    Ok(())
}

#[test]
fn inline_and_rich_inline_strings_are_read() -> TestResult {
    let opened = open("xlsxwriter-inline-strings.xlsx")?;
    let scan = opened.scan(0)?;
    assert_eq!(
        opened.texts(&scan, 0, 128)?,
        vec![
            strings(&["inline text", "rich inline"]),
            strings(&[" leading and trailing ", "7"]),
        ]
    );
    assert_eq!(opened.workbook.shared_string_count(), 0);
    Ok(())
}

fn large_row(row: u64, date: &str, category: &str) -> Vec<String> {
    let whole = row / 4;
    let amount = match row % 4 {
        0 => whole.to_string(),
        1 => format!("{whole}.25"),
        2 => format!("{whole}.5"),
        _ => format!("{whole}.75"),
    };
    vec![
        row.to_string(),
        format!("name-{row:06}"),
        amount,
        date.to_owned(),
        if row.is_multiple_of(2) {
            "TRUE"
        } else {
            "FALSE"
        }
        .to_owned(),
        category.to_owned(),
    ]
}

#[test]
fn a_large_sheet_is_counted_and_read_from_bounded_checkpoints() -> TestResult {
    let opened = open("xlsxwriter-large.xlsx")?;
    assert_eq!(opened.workbook.shared_string_count(), 120_010);
    let scan = opened.scan_all(0)?;
    assert_eq!(scan.progress().rows(), 120_001);
    assert!(scan.checkpoint_count() > 4);
    assert!(scan.checkpoint_count() <= 32);
    assert!(scan.retained_bytes() < 2 * 1_024 * 1_024);

    assert_eq!(
        opened.texts(&scan, 0, 2)?,
        vec![
            strings(&["id", "name", "amount", "date", "flag", "category"]),
            large_row(1, "2020-01-02", "south"),
        ]
    );
    let middle = opened.texts(&scan, 60_000, 3)?;
    assert_eq!(middle[0], large_row(60_000, "2024-05-19", "north"));
    let last = opened.read(&scan, 119_990, 128)?;
    assert_eq!(last.rows().len(), 11);
    assert!(last.reached_end());
    assert_eq!(
        opened.texts(&scan, 120_000, 1)?[0],
        large_row(120_000, "2028-10-05", "north")
    );

    // A scan with no checkpoints reads the same rows from the start.
    let fresh = opened.scan(0)?;
    assert_eq!(fresh.checkpoint_count(), 0);
    assert_eq!(opened.texts(&fresh, 60_000, 3)?, middle);
    Ok(())
}

#[test]
fn scan_steps_are_bounded_by_the_compressed_budget() -> TestResult {
    let opened = open("xlsxwriter-large.xlsx")?;
    let mut scan = opened.scan(0)?;
    let mut steps = 0_u32;
    loop {
        let step = scan.step(
            &opened.source,
            64 * 1_024,
            &opened.token,
            opened.token.snapshot(),
        )?;
        steps += 1;
        // One row can overshoot the budget by at most its own size.
        assert!(step.consumed_bytes() <= 64 * 1_024 + 16 * 1_024);
        if step.status() == XlsxScanStatus::Complete {
            break;
        }
        assert!(steps < 1_000);
    }
    assert!(steps > 10);
    Ok(())
}

#[test]
fn cancelled_generations_stop_open_and_scan() -> TestResult {
    let source = FileSource::open(fixture("xlsxwriter-large.xlsx"))?;
    let token = CancellationToken::new();
    let stale = token.snapshot();
    token.cancel();
    assert!(matches!(
        XlsxWorkbook::open(&source, XlsxLimits::standard(), &token, stale),
        Err(XlsxError::Cancelled)
    ));
    let opened = open("xlsxwriter-large.xlsx")?;
    let mut scan = opened.scan(0)?;
    let stale = opened.token.snapshot();
    opened.token.cancel();
    let step = scan.step(&opened.source, 64 * 1_024, &opened.token, stale)?;
    assert_eq!(step.status(), XlsxScanStatus::Cancelled);
    Ok(())
}

#[test]
fn a_zip_bomb_entry_over_one_gib_is_refused_before_reading() -> TestResult {
    let opened = open("hostile-bomb-size.xlsx")?;
    assert!(matches!(
        opened.scan(0),
        Err(XlsxError::EntryTooLarge { .. })
    ));
    Ok(())
}

#[test]
fn a_zip_bomb_with_an_extreme_ratio_is_refused_before_reading() -> TestResult {
    let opened = open("hostile-bomb-ratio.xlsx")?;
    assert!(matches!(
        opened.scan(0),
        Err(XlsxError::CompressionRatio { .. })
    ));
    Ok(())
}

#[test]
fn lying_zip_sizes_are_caught_while_streaming() -> TestResult {
    let opened = open("hostile-size-lie.xlsx")?;
    let error = opened.scan_all(0).err().ok_or("the lie was not detected")?;
    assert!(
        matches!(error, XlsxError::CompressionRatio { .. }),
        "{error}"
    );
    Ok(())
}

#[test]
fn a_truncated_workbook_is_refused() {
    assert!(matches!(
        open("hostile-truncated.xlsx"),
        Err(XlsxError::Truncated)
    ));
}

#[test]
fn a_doctype_with_external_entities_is_refused() -> TestResult {
    let opened = open("hostile-doctype-xxe.xlsx")?;
    let scan = opened.scan(0)?;
    assert!(matches!(
        opened.read(&scan, 0, 128),
        Err(XlsxError::Doctype { .. })
    ));
    Ok(())
}

#[test]
fn a_billion_laughs_shared_string_table_is_refused() {
    assert!(matches!(
        open("hostile-billion-laughs.xlsx"),
        Err(XlsxError::Doctype { .. })
    ));
}

#[test]
fn an_external_worksheet_target_is_not_followed() -> TestResult {
    let opened = open("hostile-external-sheet.xlsx")?;
    assert!(matches!(
        opened.scan(0),
        Err(XlsxError::ExternalRelationship { .. })
    ));
    Ok(())
}

#[test]
fn a_csv_with_an_xlsx_name_is_refused() -> TestResult {
    let source = FileSource::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/document-smoke.csv"),
    )?;
    let token = CancellationToken::new();
    assert!(matches!(
        XlsxWorkbook::open(&source, XlsxLimits::standard(), &token, token.snapshot()),
        Err(XlsxError::NotZip)
    ));
    Ok(())
}

#[test]
fn shared_string_text_beyond_the_limit_is_refused() {
    let limits = XlsxLimits {
        max_shared_string_bytes: 100,
        ..XlsxLimits::standard()
    };
    assert!(matches!(
        open_with("libreoffice-resaved.xlsx", limits),
        Err(XlsxError::SharedStringsTooLarge { .. })
    ));
}

#[test]
fn long_text_is_kept_up_to_the_cell_limit_and_marked() -> TestResult {
    let limits = XlsxLimits {
        max_cell_bytes: 4,
        ..XlsxLimits::standard()
    };
    let opened = open_with("libreoffice-resaved.xlsx", limits)?;
    let scan = opened.scan(1)?;
    let rows = opened.read(&scan, 1, 1)?;
    let cell = &rows.rows()[0].cells()[1];
    assert_eq!(cell.text(), Some("plai"));
    assert!(cell.is_truncated());
    Ok(())
}
