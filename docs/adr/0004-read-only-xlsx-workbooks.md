# ADR 0004: Read-only XLSX workbooks

- **Status:** Proposed
- **Date:** 2026-10-29

## Context

Users ask to inspect `.xlsx` files the same way as CSV. A workbook is a ZIP
package of compressed XML parts. ADR 0001 assumes that positioned reads of the
source feed one boundary scanner directly; a workbook needs decompression and
an XML reader in between, and a hostile package can hide a very large part
behind a small file.

The roadmap keeps spreadsheet formulas and editing out of scope. Showing the
values a producer stored is within the viewer's purpose.

## Decision

LeanRows reads `.xlsx` workbooks read-only and shows stored cell values as
text, one worksheet at a time.

- The ZIP central directory is parsed in `leanrows-core`. Only stored and
  deflate entries on one disk are accepted; ZIP64, encryption and legacy OLE
  files are refused.
- Deflate uses `miniz_oxide` (no unsafe code, default features off) into a
  fixed 32 KiB window. Entry sizes and the compression ratio are checked
  against the headers and again on the produced bytes.
- A small streaming XML tokenizer reads the parts. It refuses any DOCTYPE and
  decodes only predefined entities and character references.
- Formulas are never evaluated, macros are never run, and external
  relationships are never followed.
- The worksheet is scanned once to count rows. At most 32 copies of the reader
  state are kept as resume checkpoints. A viewport resumes from the nearest
  one and keeps only its rows.
- The shared-string table is kept in memory, up to 1 KiB per string and
  256 MiB of text in total.
- The cell-text rules are written down in `docs/XLSX.md` so that other ports
  can show the same text.

## Consequences

- One new dependency, `miniz_oxide` 0.9.1, with its dependency `adler2` 2.0.1.
- Memory for an open workbook grows with its shared-string table, within the
  stated limit. Worksheet XML is never held in memory.
- A workbook scan is slower than a CSV scan of the same data, because the XML
  is larger and must be decompressed and tokenized.
- Literal Find does not work on workbooks, because it searches raw file bytes.

## Validation

`crates/leanrows-core/tests/xlsx_fixtures.rs` checks the documented text
against workbooks written by LibreOffice, openpyxl and XlsxWriter, and checks
that the hostile fixtures in `tests/fixtures/xlsx/` are refused. The release
QA harness opens `libreoffice-document-smoke.xlsx` through the native
document smoke.
