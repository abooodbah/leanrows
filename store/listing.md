# LeanRows Microsoft Store listing

Paste each field into Partner Center > your app > Submission > Store listings > English (United States). Every value below was checked against the README and the app on 2026-10-24. The version, file types, description, features and "What's new" were updated for 0.3.0 on 2026-10-30 from the README and the 0.3.0 release notes. The version and "What's new" were updated for 0.4.0 on 2026-11-02 and for 0.5.0 on 2026-11-06, and for 0.5.1 on 2026-11-07. For 0.6.0 on 2026-11-14 the version, description, features and "What's new" were updated; the description is the AlternativeTo full description with the hide-columns line.

## Product

- Product name (reserve this exact name): LeanRows
- Publisher display name: Lean Apps Studio
- Short title (50 characters or fewer): LeanRows: large CSV viewer
- Version in this pack: 0.6.0 (package version 0.6.0.0)
- File types it opens: CSV, TSV, JSON Lines, NDJSON, and log files (.csv, .tsv, .jsonl, .ndjson, .log). Excel workbooks (.xlsx) open from inside the app; the package does not register .xlsx.

## Short description

Open very large CSV, TSV, JSON Lines, and log files without loading them into memory.

(85 characters.)

## Description

Open CSV files too big for Excel. Excel stops at 1,048,576 rows. LeanRows opens CSV, TSV, JSONL, NDJSON, log and text files of any size on Windows, plus Excel .xlsx workbooks one sheet at a time.

It never loads the whole file. A 1.5 GB CSV with 20 million rows was read to the end at a peak of 18.4 MB of memory, so it stays usable on a work laptop that is short on RAM.

Click a column header to sort the whole file, as text or as numbers.
Hide columns you do not need, per tab, without changing the file.
Search the whole file, jump to any row, and open several files as tabs.
A single program of about 1 MB. No installer, no admin rights, no runtime. Read-only, so your file is never changed.
No telemetry and no network access.
Five themes, including two softer dark themes.

Free and MIT licensed. Install from the Microsoft Store, winget, Scoop or Chocolatey, or download the portable ZIP from GitHub.

## Product features

- Opens CSV and TSV files with millions of rows
- Memory use stays small because only visible rows are kept
- First rows appear while the rest of the file is still being indexed
- Quote-aware CSV and TSV, including line breaks inside quoted fields
- JSON Lines, NDJSON, log, and text files shown one line per row
- Excel .xlsx workbooks: the stored values, one worksheet at a time
- Find text, forward and backward, with an optional case-insensitive mode
- Go to any row number
- Copy selected rows as tab-separated text
- Hide columns you do not need, per tab, without changing the file
- Each file opens in its own tab in one window
- Strictly read-only: the source file is never changed
- Light, dark, and system themes; high contrast supported
- No network access and no telemetry
- Free and open source (MIT License)

## Search terms (7)

- csv viewer
- large csv
- tsv
- jsonl
- log viewer
- big data file
- read only

## Category

- Category: Productivity
- Subcategory or secondary category: Utilities & tools

## What's new in this version

You can now hide columns you do not need. Right-click a column header and choose Hide column, or use the More menu; Show all columns brings them back. The file is never changed, each tab keeps its own hidden columns, and hidden columns are left out when you copy rows.

## Copyright and trademark info

Copyright (c) 2026 Abdulfatah Bahbouh. Released under the MIT License.

## Links

- Website: https://abooodbah.github.io/leanrows/
- Support contact: https://github.com/abooodbah/leanrows/issues
- Privacy policy: https://github.com/abooodbah/leanrows#privacy-policy

## Additional information

- Pricing: Free. No in-app purchases, no ads, no subscriptions.
- Personal information: the app does not access, collect, or transmit personal information.
- System requirements: Windows 10 version 1809 or later, or Windows 11, x64. No other runtime is needed.

## Screenshots

Upload the files in `store/screenshots/` in this order (Desktop, 1920x1080). Each one is the packaged app running on Windows 11 with public sample files from this repository or generated sample data.

| File | Caption |
| --- | --- |
| 01-weather-csv.png | A CSV file of weather station readings. |
| 02-two-million-rows.png | A 147 MB CSV with 2,000,001 rows, fully indexed. |
| 03-tabs-log.png | A CSV, a JSON Lines file, and a log file open as tabs. |
| 04-jsonl-light.png | A JSON Lines file in the light theme. |
