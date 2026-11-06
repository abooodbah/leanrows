# Changelog

All notable changes to LeanRows are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.1] - 2026-11-06

Zooming is fast, and the lists have thin scroll bars.

### Changed

- A zoom step sets the column widths with the list's redraw off, gives the new
  fonts to the rows and their titles only, and paints once. It no longer
  rebuilds and repaints the whole window.
- Ctrl+wheel notches already queued are taken as one zoom change.
- Each column keeps the width it was designed at, so zooming in and out no
  longer makes widths drift. A width set by hand becomes the new basis.
- Columns of a file opened while zoomed are sized for that zoom.
- The rows and the settings shortcut list have thin scroll bars in the theme's
  colours, without arrow buttons, as in LeanMark. High contrast keeps the
  system scroll bars.
- The settings shortcut list fits its width: long actions wrap onto a second
  line and the list has no horizontal scroll bar.

### Measured

On the maintainer's laptop, 0.5.0 and 0.5.1 built and measured the same way:

| Measure | 0.5.0 | 0.5.1 |
| --- | --- | --- |
| Zoom step, six-column 2 MB file (median) | 285 ms | 31 ms |
| Zoom step, 10M-row file (median) | 115 ms | 21 ms |
| 8 Ctrl+wheel notches at once, 2 MB file | 1,234 ms | 22.5 ms |
| Page Down, 10M-row file | 20.4 ms | 21.0 ms |
| One wheel notch, 10M-row file | 2.45 ms | 2.07 ms |
| Start to input idle, the 10M-row file | 428 ms | 425 ms |
| Peak working set, the 10M-row file open | 18.0 MiB | 18.0 MiB |
| Executable size, local build | 986,112 bytes | 1,024,512 bytes |

## [0.5.0] - 2026-11-05

LeanRows takes on LeanMark's look, and sorting a large file is several times
faster.

### Added

- LeanMark's Dim and Dusk themes, so the theme button cycles System, Light,
  Dark, Dim and Dusk with LeanMark 0.6.0's colours. Windows high contrast still
  replaces every theme with system colours.
- The theme button shows the current theme as LeanMark's coloured dot, and its
  tooltip names the theme, such as "System (Dark)". `Ctrl+Shift+T` changes the
  theme.
- A settings button and panel, opened with `Ctrl+/` or `F1` and closed with
  `Escape`: the five themes, the zoom, and the list of every keyboard shortcut
  and mouse action, sorting included.
- Zoom for the rows with LeanMark's steps from 50% to 300%: `Ctrl+Plus`,
  `Ctrl+Minus`, `Ctrl+0`, or `Ctrl` with the mouse wheel. Only the rows and
  their column titles zoom. The level is remembered.

### Changed

- The toolbar and tabs follow LeanMark: ghost buttons that show a hover, a
  bordered Open, a rounded search field with a focus ring, the "LR" wordmark,
  a border below the bar, and tabs without separators whose close mark shows
  on the selected or hovered tab.
- Sorting merges up to 64 sorted runs at a time instead of two.
- Sorting builds its row-to-rank map with sequential writes instead of a seek
  and a small write for every row.
- Sorting decides most comparisons from the stored key prefix without setting
  up readers for the full keys.
- Dark's focus ring uses LeanMark's colour.

### Measured

On one `windows-2022` GitHub Actions runner, 0.4.0 and 0.5.0 built and measured
the same way ([run 42051703441](https://github.com/abooodbah/leanrows/actions/runs/42051703441)),
with a 10,000,000-row, 250,000,000-byte CSV sorted by its first column in text
order:

| Measure | 0.4.0 | 0.5.0 |
| --- | --- | --- |
| Sort, core engine | 102.1 s | 16.1 s |
| Sort, the app's worker | not measured | 14.1 s |
| Sort, memory charged to its 16 MiB limit | 2.6 MiB | 6.6 MiB |
| Sort, temporary disk at its peak | 2,441 MiB | 2,441 MiB |
| Start to input idle, no file | 59 ms | 62 ms |
| Start to input idle, the CSV open | 91 ms | 104 ms |
| Peak working set, the CSV open | 16.2 MiB | 16.3 MiB |
| Executable size | 918,528 bytes | 986,624 bytes |

The sort uses 4 MiB more memory while it merges, for the 64 read pages. The
earlier maintainer's-laptop figure of about 6 minutes for 10 million rows was
measured in the app; this runner is faster, so compare the two columns, not
the laptop.

## [0.4.0] - 2026-11-02

Column sorting, contributed by @w3bgr3p in #9.

### Added

- Whole-document column sorting through native header clicks and text/numeric
  menu commands, with exact decimal comparison and original-row tie ordering.
- Cancellable external merge sorting with process-wide 16 MiB managed-memory
  and 8 GiB scratch quotas; original Row, Find and Go to row semantics retained.
- Native sort-workflow QA and nonsparse resource benchmarks through the existing
  Rust engine.
- A sort that would not fit on the drive, or in the 8 GiB sort limit, is refused
  before it starts with a message that says how much space it needs.

### Fixed

- Sorting, or restoring source order, in one tab no longer cancels a sort
  running or waiting in another tab; the second sort waits its turn.

### Known gaps

- Sorting does not work on `.xlsx` workbooks yet. It reports that instead of
  sorting, as Find does.

## [0.3.0] - 2026-10-29

### Added

- Read-only `.xlsx` workbooks. LeanRows shows the stored cell values of one
  worksheet at a time, with columns titled A, B, C. The first visible sheet
  opens; Next sheet and Previous sheet in the overflow menu switch between
  visible sheets. Formulas are shown as their cached values and are never
  evaluated. The rules and limits are in `docs/XLSX.md`.
- Workbooks that look like ZIP bombs, are truncated, contain a DOCTYPE, or
  point to external sheets are refused with a message.

### Known gaps

- Find does not work on `.xlsx` files yet.
- The installer and the Store package do not register `.xlsx`.

## [0.2.0] - 2026-09-24

### Added

- Open files now share one window, each in its own tab. Opening a file from
  Explorer or the command line while LeanRows is running adds a tab to the
  running window instead of starting another process. Opening a file that is
  already open selects its tab.
- A tab strip appears when two or more files are open. Click a tab to show it,
  and click its close mark or middle-click it to close it. `Ctrl+Tab` and
  `Ctrl+Shift+Tab` (or `Ctrl+PgDn` and `Ctrl+PgUp`) move between tabs,
  `Ctrl+1` to `Ctrl+8` pick a tab by position, `Ctrl+9` picks the last tab,
  and `Ctrl+W` or `Ctrl+F4` closes the current tab. The overflow menu has
  Close tab and Next tab.
- Each tab keeps its own scroll position, selected row, column widths, and Find
  state.
- The Open dialog accepts several files, and several supported files can be
  dropped at once.

### Changed

- Five open files now run in one process instead of five. Measured with the
  QA fixtures, five files use 3.8 MiB private memory and an 18.5 MiB working
  set, compared with 14.7 MiB and 88.1 MiB for five 0.1.3 windows. With one
  file open, the two versions measure within 0.1 MiB of each other.
- Minimizing the window returns its working set to Windows. In the same
  measurement it drops from 18.5 MiB to 0.8 MiB, and pages come back as the
  window is used.
- Starting LeanRows again while it is running brings the running window
  forward instead of opening a second, empty window.
- A window with no open file no longer starts a document worker thread. The
  worker starts with the first file.
- At most 32 files can be open at once, so memory stays bounded.
- The command line accepts several files. An unknown option that starts with
  `--` is now a usage error, and `--` ends option parsing for file names that
  start with `--`.

### Fixed

- Fixed the file name, file details, and Match case controls showing as white
  blocks in the top bar, most visibly in the Dark appearance. Their background
  brush was returned through a conversion that failed for some Windows brush
  handles, so the controls were left unpainted. The problem dated from the
  0.1.2 redesign and appeared in about half of launches.

## [0.1.3] - 2026-08-09

### Fixed

- Fixed System appearance so it follows the active Windows app theme instead
  of always resolving to Light.
- Restored the complete System, Light, and Dark appearance cycle, including
  persistence of an explicit Dark selection.
- Applied the selected appearance consistently to native child controls while
  retaining their native roles, keyboard behavior, and accessibility
  semantics.

### Changed

- Replaced the platform progress animation with a flat, two-DIP rule below the
  top bar. Its accent prefix tracks indexing progress without a marquee.
- Strengthened the native shell smoke to exercise the full appearance cycle,
  persistence and runtime repainting, and the progress control's class, value,
  style, and rendered pixels.

## [0.1.2] - 2026-08-09

### Added

- Added a responsive native top bar that keeps file identity, inline literal
  search, and primary document actions available across narrow, default, and
  wide window layouts.
- Added System and Light appearance choices, with a system-color fallback when
  Windows high contrast is active.

### Changed

- Reworked the Windows shell around warm, opaque surfaces, restrained rules,
  deliberate spacing, and one blue action accent to align with the LeanMark
  visual family.
- Replaced the modal Find entry point with an inline search field, Match case
  toggle, and previous/next controls while retaining `F3` and `Shift+F3`
  navigation.
- Preserved the owner-data grid and bounded worker/cache model beneath the new
  shell, so the visual redesign does not introduce a whole-file UI model.

## [0.1.1] - 2026-08-09

### Fixed

- Fixed the per-user installer failure that occurred when PowerShell bound the
  empty name required for a registry default value.
- Fixed registry-key creation so repeated writes preserve earlier sibling
  values, and opened direct-default keys writable for exact uninstall
  restoration or removal.

### Changed

- The installer now records and sets a direct per-user default for supported
  extensions that have no protected Explorer `UserChoice`, while continuing to
  register all supported extensions in **Open with**.
- Reinstallation preserves the original recorded handler, and uninstallation
  restores or removes only direct defaults that still point to LeanRows.
- Protected `UserChoice` selections, later user changes, and `.txt`
  associations remain untouched.

## [0.1.0] - 2026-08-09

### Added

- Native Windows x64 viewer for CSV, TSV, JSONL, NDJSON, log, and text files.
- Progressive first-viewport loading with background indexing.
- Quote-aware CSV and TSV record scanning across fixed-size read blocks.
- Independent native columns for bounded CSV and TSV fields.
- Fixed-capacity adaptive row checkpoints and bounded viewport previews.
- Demand-driven literal Find with next/previous navigation, exact or ASCII-only
  case-insensitive byte matching, bounded result pages, and a 64 MiB scratch
  quota.
- Checked absolute-row navigation, bounded tab-delimited clipboard copy, and
  supported-file drag-and-drop.
- Read-only snapshot handles with source identity and revision checks.
- Visible escaping of invalid UTF-8 bytes and explicit preview truncation.
- Keyboard navigation, native accessibility names, system colors, and
  per-monitor DPI support.
- Deterministic unsigned portable ZIP, package manifest, SHA-256 checksum, and
  optional per-user installer with Start menu and Installed Apps registration.
- Automated core, adversarial-input, native-shell, document, packaging, and QA
  harnesses.

### Security

- Remote and UNC paths, device paths, reparse points, symbolic links, and
  non-regular files are rejected.
- In-place source writes are not shared while a snapshot is open; atomic
  replacement is detected before stale work can continue.
- File content is displayed as inert native text and no network or telemetry
  path is included.

[0.5.1]: https://github.com/abooodbah/leanrows/releases/tag/v0.5.1
[0.5.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.5.0
[0.4.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.4.0
[0.3.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.3.0
[0.2.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.2.0
[0.1.3]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.3
[0.1.2]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.2
[0.1.1]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.1
[0.1.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.0
