# LeanRows Microsoft Store listing

Paste each field into Partner Center > your app > Submission > Store listings > English (United States). Every value below was checked against the README and the app on 2026-10-24. The version, file types, description, features and "What's new" were updated for 0.3.0 on 2026-10-30 from the README and the 0.3.0 release notes.

## Product

- Product name (reserve this exact name): LeanRows
- Publisher display name: Lean Apps Studio
- Short title (50 characters or fewer): LeanRows: large CSV viewer
- Version in this pack: 0.3.0 (package version 0.3.0.0)
- File types it opens: CSV, TSV, JSON Lines, NDJSON, and log files (.csv, .tsv, .jsonl, .ndjson, .log). Excel workbooks (.xlsx) open from inside the app; the package does not register .xlsx.

## Short description

Open very large CSV, TSV, JSON Lines, and log files without loading them into memory.

(85 characters.)

## Description

LeanRows is a read-only viewer for large row-based files: CSV, TSV, JSON Lines, NDJSON, and log files.

It reads the file in blocks as you move through it and keeps only the visible rows in memory, so memory use depends on the window, not the file size. On the test machine, a 1.5 GB CSV with 20 million rows was fully indexed with a peak working set of 18.4 MB. The first rows appear before indexing reaches the end of the file.

Excel workbooks (.xlsx) open read-only, one worksheet at a time. LeanRows shows the values the workbook stores, with columns titled A, B, C. Formulas are not calculated, and macros and external links are never run or followed.

You can search for literal text, jump to a row number, and copy selected rows. Each file opens in its own tab in one window. CSV and TSV records are read correctly even when quoted fields contain line breaks. Bytes that are not valid text are shown escaped, and very long values are marked as shortened instead of being changed silently.

LeanRows never edits or rewrites the file it shows. It has no telemetry, no update checker, no account, and makes no network connections. It is a native Windows program and does not use a browser engine.

LeanRows is free and open source under the MIT License.

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

LeanRows now opens Excel .xlsx workbooks, read-only, one worksheet at a time. Find does not work in workbooks yet.

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
