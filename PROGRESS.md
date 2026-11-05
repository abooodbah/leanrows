# Progress

Work for LeanRows desktop 0.5.0. This file is updated with every merge.

## Goal for 0.5.0

- Faster whole-file sorting, without changing its limits: 16 MiB of managed
  memory, temporary files only in `%LOCALAPPDATA%\LeanRows\sort-cache`, and the
  free-space check before a sort starts.
- LeanMark's five themes (System, Light, Dark, Dim, Dusk) with LeanMark 0.6.0's
  colour values. Windows high contrast keeps system colours.
- A theme button with a coloured dot and a tooltip that names the theme.
- A settings button and panel with theme, zoom and every keyboard shortcut,
  sorting included. `Ctrl+/` or `F1` opens it and `Escape` closes it.
- Toolbar and tabs restyled to match LeanMark.

LeanRows stays Rust and Win32, with no web view.

## Efficiency review

Reviewed on 2026-11-05 at main `866667b` (0.4.0): open, scroll, find, theme
switch, sorting, what is allocated and kept, dead code, dependencies and the
executable.

How the numbers were measured:

- "Local" is this Linux machine (4 cores, shared with other jobs), release
  build, Rust 1.97.1. The sort numbers come from the core sort engine driven
  the way `leanrows-spike --sort` drives it, with a phase timer. Local times
  vary by about 10% between runs because the machine is shared.
- The fixture is the one `tools/Measure-LeanRowsSort.ps1` writes: each row is
  `D9((row*97+13)%N),row-D9(row)` with CRLF. 10,000,000 rows is 250,000,000
  bytes. Sorted by the first column in text order.
- Windows numbers come from the repository's CI on `windows-2022`. They are
  added below as the benchmark workflow produces them.

| Place | What it costs | Measured | Proposed fix |
| --- | --- | --- | --- |
| Sort merge (`sort/session.rs`, `merge_disk`) | Merges two runs at a time. 10M rows make 1,221 runs of 8,192 rows, so the merge makes 11 passes, each reading and writing 1.28 GB. | Local, 10M rows: merging 41.7 s of 65.5 s total. | Merge up to 64 runs at a time through a winner tree. 2 passes for any file up to the 33.5M-row limit. |
| Inverse map (`sort/session.rs`, `build_maps`) | Seeks and writes 8 bytes for every row, so 2 system calls per row. This is the file position set again for every row. | Local, 1M rows: 1,000,000 `lseek` and 1,000,000 `write` calls (strace). 10M rows: 5.9 s. | Collect (row, rank) pairs in buckets of 512 Ki rows, then place each bucket in memory and write it in one go. |
| Sort comparisons (`sort/key.rs`, `Comparison::new`) | Builds four 4 KiB read buffers (16 KiB zeroed) for every comparison, even when the 32-byte key prefix decides it, which is nearly always. | Local, 10M rows: sorting runs in memory takes 15.6 s of 65.5 s. A prototype without it took the 1M sort from 4.32 s to 2.37 s. | Decide from the stored prefix and number fields first. Build the read buffers only when the full keys must be read. |
| Sort steps in the app (`worker/sorting.rs`, `advance_sort`) | After every 64 KiB work step the worker checks the source path again, which opens the path several times. A 10M sort has about 300,000 steps. | Windows only. To be measured in CI with the worker benchmark. | If it measures as real: check the path at most every 100 ms while sorting, and always before the sorted view is shown. |
| Sorted rows (`sort/storage.rs`, `SortedOrder::record_at`) | Reads the 32-byte header and the file length for every row shown, so 3 reads per row instead of 1. | Local: 0.71 µs per row, 0.09 ms per 128-row view. | None for now. Too small to matter. |
| Sort extraction (`sort/session.rs`, `extract`) | Reads each record's bytes from the source again (to find the column, then the key) after the scanner read them. | Local, 10M rows: 2.3 s of 65.5 s. | None for now. Small. |
| Run sorting in memory (`sort/session.rs`, `sort_batch`) | Merges one element per state-machine call so a comparison can stop mid-key. | After the comparison fix, local 10M: about 5 s. | For the user to decide: use the standard stable sort for a batch whose keys all fit in the 32-byte prefix. |
| Release build (`Cargo.toml`) | No release profile: no link-time optimisation and 16 code units. | Executable 918,528 bytes (0.4.0 release). | To be measured in CI: link-time optimisation and one code unit. |
| Open | The first 128-row view is read before indexing finishes; the index is a fixed 4,096 checkpoints. | No cost found. | None. |
| Scroll | A view is rebuilt from the nearest checkpoint, at most a few thousand rows away. | No cost found. | None. |
| Find | Starts on Enter or F3, not on every keystroke, with fixed buffers. | No cost found. | None. |
| Theme switch | Builds one set of 4 fonts and 5 brushes and repaints once. | No cost found. | None. |
| Dependencies | Two: `windows` 0.62.2 (Win32 calls) and `miniz_oxide` 0.9.1 (`.xlsx`). Both used. | No unused dependency. | None. |
| Dead code | The Linux build warns only about code behind `cfg(windows)`. Clippy with `-D warnings` passes on Windows. | None found. | None. |

## Plan

Each item is its own branch and pull request, merged when CI is green. Tests
are committed on their own before the change. Every pull request body has a
mutation table and before and after numbers from main and the branch,
measured the same way.

1. This file: review and plan.
2. A benchmark workflow on `windows-2022` that builds main and the branch on
   the same runner and reports executable size, start time, peak working set
   with a large file open, and the 10M-row sort time (core engine and the
   app's worker).
3. Sorting: merge up to 64 runs at a time.
4. Sorting: build the inverse map with sequential writes.
5. Sorting: decide comparisons from the stored prefix before reading keys.
6. Further efficiency changes only if the CI numbers show them: the worker's
   path check while sorting, and the release profile.
7. Dim and Dusk themes with LeanMark's values; the theme button cycles all
   five; the choice is saved as before.
8. Theme button with a coloured dot and a tooltip that names the theme.
9. Zoom for the rows (Ctrl+Plus, Ctrl+Minus, Ctrl+0), LeanMark's zoom steps,
   saved between runs. Only the grid zooms, as in LeanMark only the document
   zooms.
10. Settings button and panel: theme, zoom and the full shortcut list,
    sorting included. `Ctrl+/` or `F1` opens it, `Escape` closes it.
11. Toolbar and tabs restyled to match LeanMark. The pull request lists what
    cannot be identical in Win32.
12. Version 0.5.0: version files, CHANGELOG.md, README.md, CITATION.cff,
    `docs/releases/v0.5.0.md`.
13. Annotated tag `v0.5.0`, the release workflow and the Store package
    workflow for the tag.
14. Post-release: `site/`, `store/listing.md`, `docs/VALIDATION_STATUS.md`.

## Done

Nothing merged yet.

## In progress

- Review and plan (this file).

## Remaining

Items 2 to 14 of the plan.

## Known gaps

- This machine has no Windows. The app, its tests and every Windows number
  come from CI. Nothing is run by hand on a Windows desktop.

## Measured numbers

Main before any change (0.4.0 code):

| What | Value | Where |
| --- | --- | --- |
| Executable size | 918,528 bytes | 0.4.0 release (`docs/VALIDATION_STATUS.md`) |
| Workspace tests on Windows CI | 223 (app 5, core 34 + 6 + 6 + 15 + 13 + 23, spike 7, win32 114) | [CI run 41524019415](https://github.com/abooodbah/leanrows/actions/runs/41524019415) |
| Workspace tests on Linux | 160 (win32 has 52 here; its Win32 tests do not build on Linux) | Local |
| 10M-row text sort, core engine | 60.7 s (`leanrows-spike --sort`), 65.5 s (phase timer) | Local |
| 1M-row text sort, core engine | 4.3 s to 4.5 s | Local |
