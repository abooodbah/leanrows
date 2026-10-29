# XLSX fixtures

These workbooks were written by real producers. The hostile files start from
one of them and are changed the way an attacker would change them, with
Python's `zipfile` module. [`generate.py`](generate.py) writes all of them.

The expected text for each file is in
`crates/leanrows-core/tests/xlsx_fixtures.rs`, and the rules are in
[`docs/XLSX.md`](../../../docs/XLSX.md). The Android port can copy this
directory as it is.

Producers used for the committed files, all on Linux:

- LibreOffice 24.2.7.2 (build 420), headless `soffice --convert-to xlsx`
- openpyxl 3.1.5 on Python 3.13.17
- XlsxWriter 3.2.9 on Python 3.13.17
- Python 3.13.17 `zipfile` (hostile files and the phonetic edit only)

LibreOffice and openpyxl store timestamps, so a rerun of `generate.py` writes
different bytes. The hashes below are for the committed files.

## Workbooks

| File | Producer | What it covers | Bytes | SHA-256 |
| --- | --- | --- | --- | --- |
| `libreoffice-document-smoke.xlsx` | LibreOffice 24.2.7.2, from `tests/fixtures/document-smoke.csv` | Shared strings, a quoted comma, a line break inside a cell, a trailing empty CSV field that is not stored | 5,570 | `831cc6928395e569e3c225b72f11a46bcd9952d7f77999aefa7a170032811216` |
| `openpyxl-features.xlsx` | openpyxl 3.1.5 | Five sheets (hidden first, veryHidden last), inline strings, rich inline text, numbers, booleans, an error, dates, times and custom formats, 1900 serials 0 and 59 to 61, a formula without a cached value, whitespace, sparse rows and columns up to row 1,048,576 and column BR | 7,589 | `fa1c87f04e9e223b2542211731a8d29975af76e2cca74a849f2ed6bae527faa8` |
| `libreoffice-resaved.xlsx` | LibreOffice 24.2.7.2, reading `openpyxl-features.xlsx` | The same content written by LibreOffice: shared strings with rich runs, its own number formats, booleans as cached `TRUE()` formulas, the formula with its cached value | 11,296 | `7e231aec5241464ad67cab19063808abf2759982d1c73e152fb361f403e67802` |
| `openpyxl-1904.xlsx` | openpyxl 3.1.5 | The 1904 date system | 4,908 | `a5a20c0e7351ee26d5a12ce41a691e2492532e4365291486a70f19769a57c5c6` |
| `xlsxwriter-features.xlsx` | XlsxWriter 3.2.9 | Rich shared strings, formulas with cached number, string, boolean and error values, a `_x0001_` escape, leading empty columns, a styled blank cell | 6,145 | `d61d635797ff170fc66da010ec66f34ab046280927da1f135dcd1dd0d31ac3f9` |
| `xlsxwriter-inline-strings.xlsx` | XlsxWriter 3.2.9, `constant_memory` mode | Inline and rich inline strings, leading and trailing spaces | 5,096 | `2250f8634ba07ac7b1a5b31730a11173ccbf32f4cca827a4839cc3784136b86c` |
| `excel-style-phonetic.xlsx` | `libreoffice-document-smoke.xlsx`, edited by `generate.py` | The first shared string gets a phonetic run (`<rPh>`) and `<phoneticPr>` in the form Excel writes for East Asian text. None of the producers here writes phonetic runs, so this one part is edited by hand. | 5,480 | `777231bc4a677d30cbeedc9ed12d8a9481f5043918735e64d7654e1a2ade9faf` |
| `xlsxwriter-large.xlsx` | XlsxWriter 3.2.9 | 120,001 rows by 6 columns, 120,010 shared strings, 27.8 MB of sheet XML | 4,202,904 | `889ee37320572ea8d53e8a979f9b9d8d253a4d79e9a0d15808dde3b59b126fc7` |

## Hostile files

| File | Made from | Change | Expected result | Bytes | SHA-256 |
| --- | --- | --- | --- | --- | --- |
| `hostile-bomb-size.xlsx` | `libreoffice-document-smoke.xlsx` | Sheet replaced with 1,100 MiB of empty rows, deflate level 9 | Refused: entry over 1 GiB | 1,684,738 | `87d5edc6bdcfe67ab571817e103b3961d43109b1c53eeb3281610ea021e0fdc9` |
| `hostile-bomb-ratio.xlsx` | `libreoffice-document-smoke.xlsx` | Sheet replaced with 100 MiB of empty rows, about 667:1 | Refused: ratio over 200 | 157,286 | `1eeb764ffb5d9f8c79f73a5fb428f04ac6c5a6253899c66b87fc27a4c8ed6039` |
| `hostile-size-lie.xlsx` | `hostile-bomb-ratio.xlsx` | Both ZIP headers claim 100 times the compressed size | Refused while streaming: ratio over 200 | 157,286 | `152c28879659634201c0d302043a7e2c8362040470e38836740b8845b2445ea7` |
| `hostile-crc-mismatch.xlsx` | `libreoffice-document-smoke.xlsx` | Sheet stored uncompressed, then one byte changed (shared-string index 0 to 1) | Refused at the end of the scan: CRC-32 mismatch | 6,950 | `9e89eb480e11c7cf0d338039fe52a95a9ef079f5af9ad44a2a2feeb899232ecb` |
| `hostile-truncated.xlsx` | `openpyxl-features.xlsx` | First 60 percent of the bytes | Refused: truncated | 4,553 | `3e1f50ea349468a5e81e64f28e66be0dbda9062ba1b37682e05b81f85b6c6974` |
| `hostile-doctype-xxe.xlsx` | `libreoffice-document-smoke.xlsx` | Sheet XML with a DOCTYPE declaring `file:///etc/passwd` and `http://` external entities, used in a cell | Refused: DOCTYPE | 5,515 | `1fe5fdacfe90c86ab807e5dc28595fd3e47f607b2abfd11570e29310b3c64334` |
| `hostile-billion-laughs.xlsx` | `libreoffice-document-smoke.xlsx` | Shared strings with a ten-level nested entity DOCTYPE | Refused: DOCTYPE | 5,520 | `d9c7a16ea7c7561c4dab31de8e5c21a53a3bed01747220641d5af11cd44d9cd2` |
| `hostile-external-sheet.xlsx` | `libreoffice-document-smoke.xlsx` | The worksheet relationship points to `http://example.invalid/sheet1.xml` with `TargetMode="External"` | Refused: external target, not fetched | 5,432 | `55a1c12efd3687789671061642dcaa2dfa1acec55bea9c07c8207924b213e4ca` |
