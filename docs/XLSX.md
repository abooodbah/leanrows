# XLSX cell text

This document is the reference for how LeanRows turns an `.xlsx` worksheet into
rows of text. The desktop app follows it, and the Android port is meant to
follow it too, so both show the same text for the same file. If the code and
this document disagree, that is a bug in one of them.

The reader is read-only. It never evaluates formulas, runs macros, or follows
external links. It shows the values that the producing program stored.

The implementation is in `crates/leanrows-core/src/xlsx/`. The fixtures in
[`tests/fixtures/xlsx/`](../tests/fixtures/xlsx/README.md) were written by
LibreOffice, openpyxl and XlsxWriter, and the tests in
`crates/leanrows-core/tests/xlsx_fixtures.rs` check the text below against
them.

## Scope

- Supported: `.xlsx` (Office Open XML workbook) with stored or deflate ZIP
  entries. Elements are matched by local name and relationship types by
  suffix, so strict-namespace files should also work, but no strict fixture
  is tested yet.
- Out of scope for now: `.xlsm`, `.xlsb`, `.xls`, password-protected
  workbooks (these are OLE files, not ZIP), ZIP64 archives and multi-disk
  archives. They are refused with a message.
- Find is not available for `.xlsx` yet. Search runs over raw file bytes,
  which are compressed in a workbook.

## Sheets

1. `_rels/.rels` names the workbook part (the `officeDocument` relationship).
   If it is missing, `xl/workbook.xml` is used.
2. Sheets come in workbook order from `<sheets>` in the workbook part. Each
   `r:id` is resolved through the workbook's relationships part, for example
   `xl/_rels/workbook.xml.rels`.
3. A sheet is offered when its relationship type ends in `/worksheet` and its
   `state` is not `hidden` or `veryHidden`. Chart sheets, dialog sheets and
   macro sheets are not offered.
4. The first offered sheet opens by default. The user can switch to any other
   offered sheet. On desktop this is Next sheet and Previous sheet in the
   overflow menu, which wrap around. The status line shows
   `Sheet 2 of 3: Name`.
5. A relationship with `TargetMode="External"` is never followed. A sheet that
   points outside the package is refused when it is opened.

Example: openpyxl-features.xlsx has the sheets `Hidden first` (hidden),
`Types`, `Sparse`, `Second visible` and `Very hidden` (veryHidden). `Types`
opens by default, and the user can switch between `Types`, `Sparse` and
`Second visible`.

## Cell text

The cell type is the `t` attribute of `<c>`. The table shows the stored XML
and the displayed text.

| Type | Rule | Stored | Shown |
| --- | --- | --- | --- |
| `t="s"` | The shared string: all `<t>` text inside the `<si>`, including rich runs (`<r><t>`), in order. Phonetic runs (`<rPh>`) are left out. | `<v>3</v>` with `<si><r><t>bold</t></r><r><t> and plain</t></r></si>` | `bold and plain` |
| `t="inlineStr"` | The same rule applied to `<is>`. | `<is><t>inline text</t></is>` | `inline text` |
| `t="str"` | The text in `<v>`, a formula's string result. | `<f>"a"&amp;"b"</f><v>ab</v>` | `ab` |
| `t="b"` | `TRUE` for `1`, `FALSE` for `0`. Any other value is shown as stored. | `<v>1</v>` | `TRUE` |
| `t="e"` | The error text as stored. | `<v>#DIV/0!</v>` | `#DIV/0!` |
| `t="d"` | The stored ISO 8601 text, unchanged. | `<v>2024-02-29T00:00:00</v>` | `2024-02-29T00:00:00` |
| no `t`, or `t="n"` | A date or time format turns the serial into text (see below). Any other number is the `<v>` text exactly as stored, with no rounding or locale formatting. | `<v>-1.5E-010</v>` | `-1.5E-010` |
| any other `t` | The `<v>` text as stored. | | |

More rules:

- Formulas are never evaluated. The cached `<v>` is shown. A cell with no
  `<v>`, or an empty `<v>`, is empty. Example: openpyxl stores `=1+2` as
  `<f>1+2</f><v></v>`, which is shown empty. LibreOffice stores the same
  formula as `<f>1+2</f><v>3</v>`, which is shown as `3`.
- Text is kept as stored, including leading and trailing spaces, tabs and line
  breaks. The only change is standard XML line-end handling: a raw CR LF or a
  lone CR in the XML becomes LF. A `&#13;` reference is kept as CR.
- `_xHHHH_` escapes (for example XlsxWriter writes `a_x0001_b` for a control
  character) are not decoded. They are shown as stored. Excel would decode
  them; this may change later, and both ports should change together.
- The desktop table shows control characters as visible escapes, for example
  a line break as `\n` and a tab as `\t`, exactly as it does for CSV. This is
  display only; the cell text above is what the rules produce.
- The five predefined XML entities and numeric character references are
  decoded. Any other entity reference is an error.

### Dates and times

A numeric cell is a date or time when its style says so: the `s` attribute is
an index into `<cellXfs>` in the styles part, and that `<xf>` has a
`numFmtId`. A missing `s` means index 0.

- A `numFmtId` defined in `<numFmts>` uses its `formatCode`. Otherwise the
  built-in ids are used: 14 to 17 are dates, 18 to 21 and 45 to 47 are times,
  22 is a date with time. All other built-in ids are plain numbers.
- A `formatCode` is examined letter by letter, ignoring case. Text in double
  quotes, the character after a backslash, the character after `_` or `*`,
  and anything inside `[...]` is skipped. The exception is a bracket that
  holds only `h`, only `m` or only `s` (for example `[h]` or `[mm]`), which
  counts as a time part. Then `h` or `s` is a time part, `d` or `y` is a date
  part, and `m` is a date part only when there is no time part.
- Date part only: `YYYY-MM-DD`. Date and time part: `YYYY-MM-DD HH:MM:SS`.
  Time part only: `HH:MM:SS`, from the fraction of the day; whole days are
  dropped, so `[h]:mm:ss` with value 1.5 shows `12:00:00`.
- The serial is rounded to the nearest second first, then split into days and
  seconds. `45351.999999` with `yyyy-mm-dd hh:mm:ss` is `2024-03-01 00:00:00`.
- 1900 system (the default): Excel treats 1900 as a leap year. LeanRows
  shows what Excel shows. Serial 0 is `1900-01-00`, 1 is `1900-01-01`, 59 is
  `1900-02-28`, 60 is `1900-02-29`, and 61 is `1900-03-01`. From 61 on, day
  `n` is 1899-12-30 plus `n` days.
- 1904 system (`<workbookPr date1904="1"/>` or `"true"`): day `n` is
  1904-01-01 plus `n` days.
- A negative serial, a serial whose date would be after 9999-12-31, or a value
  that is not a number is shown as the stored text. Example: `-1` with
  `yyyy-mm-dd` shows `-1`.
- Percent, currency, thousands separators and every other number format are
  not applied. `0.25` with `0.00%` shows `0.25`.

| Format | Stored | Shown |
| --- | --- | --- |
| `yyyy-mm-dd` (openpyxl) | `45351` | `2024-02-29` |
| `yyyy-mm-dd h:mm:ss` | `45351.57326388889` | `2024-02-29 13:45:30` |
| built-in 21 `h:mm:ss` | `0.2710069444444445` | `06:30:15` |
| `dd/mm/yyyy` | `36525` | `1999-12-31` |
| built-in 14 | `36526` | `2000-01-01` |
| `"Day "0` | `12345` | `12345` |
| `[h]:mm:ss` | `1.5` | `12:00:00` |
| `yyyy-mm-dd`, 1904 system | `0` | `1904-01-01` |

## Layout

- The cell position comes from the `r` attribute in A1 style. A cell with no
  `r` is in the column after the previous cell. Cells in a row must be in
  increasing column order, and the digits of `r` must match the row;
  otherwise the file is refused.
- A row's number comes from `r` on `<row>`; a row with no `r` follows the
  previous row. Rows must be in increasing order, or the file is refused.
- Missing rows between stored rows are shown as empty rows, so row numbers
  match the sheet. The row count is the number of the last stored `<row>`,
  even if that row has no cells. Nothing is shown after it.
- Each row is as wide as its last stored cell, including a stored cell with no
  value. Missing cells before it are empty.
- Columns are titled with the sheet's letters, `A`, `B`, `C` and so on. The
  first row is a normal row, not a header, as for CSV.

Example from the `Sparse` sheet of openpyxl-features.xlsx, which stores `A1`,
`C3`, `J10`, `BR12` and `A1048576`:

| Row | A | B | C |
| --- | --- | --- | --- |
| 1 | top-left | | |
| 2 | | | |
| 3 | | | C3 |

The row count is 1,048,576 and row 1,048,576 shows `last row` in column A.

## Limits

Inputs beyond these limits are refused with a message. Nothing past a limit is
read.

| Limit | Value |
| --- | --- |
| Rows | 1,048,576 (Excel's limit) |
| Columns | 16,384, column `XFD` (Excel's limit) |
| Uncompressed size of one ZIP entry | 1 GiB |
| Uncompressed to compressed ratio of one entry, once past 1 MiB | 200 |
| ZIP entries | 10,000 |
| Central directory | 4 MiB |
| Shared-string text in total, counted in full | 256 MiB |
| Shared strings | 16,777,216 |
| Cell formats (`cellXfs`) and custom number formats | 65,536 each |
| Sheets | 4,096 |
| One XML tag | 64 KiB |

The size and ratio limits are checked against the ZIP headers before an entry
is read, and again on the bytes actually produced while it is read. Headers
that lie do not get past them. The CRC-32 and size of each entry are checked
when it ends.

XML with a DOCTYPE is refused, so no entity can be declared. XML that is not
UTF-8 is refused.

Display bounds (desktop, the same as for CSV): at most 64 columns per row, 1
KiB of text per cell and 4 KiB per row. Text beyond these is cut at a
character boundary and the cell or row is marked as truncated. Shared strings
are kept up to 1 KiB each. Cells beyond the 64th column are not shown and the
row is marked as truncated.

## Memory and speed

The worksheet XML is streamed through a 32 KiB decompression window. It is
never held in memory. The shared-string table (up to 1 KiB per string), the
number-format table and at most 32 resume checkpoints (about 44 KiB each) are
kept while the document is open. A viewport resumes from the nearest
checkpoint at or before its first row and reads forward.

Measured on the 120,001-row fixture `xlsxwriter-large.xlsx` (4,202,904 bytes,
27.8 MB of sheet XML, 120,010 shared strings), release build, Linux, Intel
Xeon 2.1 GHz, three runs:

| Step | Result |
| --- | --- |
| Open, read shared strings, first 128 rows | 42 to 55 ms |
| Scan the whole sheet | 448 to 455 ms |
| 128 rows from the middle, after the scan | 10 to 12 ms |
| 128 rows at the end, after the scan | 5 to 6 ms |
| Retained shared strings and formats | 2.5 MiB |
| Retained scan state and 29 checkpoints | 1.3 MiB |
| Peak resident set of the measuring process | 5.6 MiB |

The same data saved as CSV by LibreOffice (5,724,493 bytes) is scanned by the
CSV engine in 28 to 30 ms on the same host, so a workbook scan is about 15
times slower than the equivalent CSV. The command is:

```sh
cargo run --release -p leanrows-core --example xlsx_measure -- tests/fixtures/xlsx/xlsxwriter-large.xlsx
```

These numbers are from one host and are not a Windows working-set
measurement.
