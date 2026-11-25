# LeanRows Microsoft Store listing

Paste each field into Partner Center > your app > Submission > Store listings > English (United States). Every value below was checked against the README and the app on 2026-10-24. The version, file types, description, features and "What's new" were updated for 0.3.0 on 2026-10-30 from the README and the 0.3.0 release notes. The version and "What's new" were updated for 0.4.0 on 2026-11-02 and for 0.5.0 on 2026-11-06, and for 0.5.1 on 2026-11-07. For 0.6.0 on 2026-11-14 the version, description, features and "What's new" were updated; the description is the AlternativeTo full description with the hide-columns line. For 0.6.1 on 2026-11-15 the version and "What's new" were updated; the package now shows the reserved name LeanRows CSV Viewer. For 0.7.0 on 2026-11-25 the version, description, features and "What's new" were updated, and "What's new" is given for the Japanese, Simplified Chinese and Traditional Chinese listings too.

## Product

- Product name (reserve this exact name): LeanRows CSV Viewer (reserved 2026-11; the MSIX manifest DisplayName is set to it, and the package identity, executable and window title stay LeanRows)
- Publisher display name: Lean Apps Studio
- Short title (50 characters or fewer): LeanRows: large CSV viewer
- Version in this pack: 0.7.0 (package version 0.7.0.0)
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
The interface is in English, Simplified Chinese or Japanese.

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
- Interface in English, Simplified Chinese and Japanese
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

Paste each paragraph into the listing of its language.

English (United States):

The interface is now available in Simplified Chinese and Japanese. LeanRows follows your Windows display language, and Settings has a Language choice: System, English, 简体中文 or 日本語. Menus, the toolbar, find, go to row, sorting, the status bar, messages and the keyboard shortcut list are translated. Your data is shown as before.

Japanese (ja-JP):

インターフェースが簡体字中国語と日本語に対応しました。LeanRows は Windows の表示言語に従い、設定の [言語] で [システム]、English、简体中文、日本語 を選べます。メニュー、ツールバー、検索、行へ移動、並べ替え、ステータス バー、メッセージ、キーボード ショートカットの一覧が翻訳されています。データの表示はこれまでどおりです。

Chinese (Simplified, zh-Hans):

界面现已提供简体中文和日语版本。LeanRows 默认跟随 Windows 显示语言，也可以在设置的“语言”中选择：跟随系统、English、简体中文或日本語。菜单、工具栏、查找、转到行、排序、状态栏、提示信息以及键盘快捷键列表均已翻译。数据的显示方式保持不变。

Chinese (Traditional, zh-Hant):

介面現已提供簡體中文與日文版本。LeanRows 預設跟隨 Windows 顯示語言，也可以在設定的「語言」中選擇：跟隨系統、English、简体中文或日本語。繁體中文的 Windows 會維持英文介面，可在設定中改選简体中文。選單、工具列、尋找、移至列、排序、狀態列、訊息以及鍵盤快速鍵清單均已翻譯。資料的顯示方式維持不變。

The three translations come from #59. They were checked by back translation, not by a native speaker.

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
