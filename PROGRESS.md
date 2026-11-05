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

| Item | Pull request | Result |
| --- | --- | --- |
| 1. Review and plan | [#27](https://github.com/abooodbah/leanrows/pull/27) | This file. |
| 2. Benchmark workflow | [#28](https://github.com/abooodbah/leanrows/pull/28) | `.github/workflows/benchmark.yml` and `tools/Compare-LeanRowsBuilds.ps1`: main and the branch on one `windows-2022` runner. |
| 3. Merge up to 64 runs at a time | [#29](https://github.com/abooodbah/leanrows/pull/29) | 10M-row sort on Windows CI: core engine 95.6 s to 82.7 s, app worker 112.5 s to 91.4 s. |
| 4. Inverse map with sequential writes | [#31](https://github.com/abooodbah/leanrows/pull/31) | 10M-row sort on Windows CI: core engine 89.4 s to 50.1 s, app worker 107.9 s to 64.7 s. 1M rows: 2,084,481 system calls to 86,321. |
| 8. Theme button with dot and tooltip | [#32](https://github.com/abooodbah/leanrows/pull/32) | LeanMark's dot in a grey ring, the tooltip "Theme: System (Dark) (Ctrl+Shift+T)", Ctrl+Shift+T. |
| 7. Dim and Dusk themes | [#30](https://github.com/abooodbah/leanrows/pull/30) | Five themes with LeanMark 0.6.0's values; start time, memory and size unchanged. |

## In progress

- 5. Comparisons decided from stored fields (branch `sort-comparison-fast-path`).
- 9. Zoom for the rows (branch `grid-zoom`).

## Remaining

Items 6 and 10 to 14 of the plan.

## Known gaps

- This machine has no Windows. The app, its tests and every Windows number
  come from CI. Nothing is run by hand on a Windows desktop. The Windows code
  is type-checked and linted here with the `x86_64-pc-windows-msvc` standard
  library (`cargo clippy --target x86_64-pc-windows-msvc`), which needs no
  linker.
- Local Linux sort times are noisy: the machine is shared, and the same 10M
  sort of main took 60.7 s, 63.7 s and 99.4 s in three runs.
- Merging 64 runs at a time holds up to 4 MiB of read pages during a sort.
  The sort's managed peak went from 2.62 MiB to 6.64 MiB, still within its
  16 MiB limit, and the probe's working set during the sort from 5.5 MiB to
  9.4 MiB. Opening a file is unchanged.
- Mutation checks for Win32 code run through the Windows workflow on the
  branches `mutation/theme-1` to `mutation/theme-5`. This session cannot
  delete branches, so those five stay on GitHub for the user to delete.
- PR bodies: the GitHub gateway in this session adds a "Generated by" footer
  to every PR it creates or edits. The bodies of #27 and #28 were emptied on
  an earlier instruction, which removed that footer; #28's body was then
  written again and has the footer. Footers are left as the gateway sets
  them from #29 on. Squash commit messages carry the full text without it.

## Measured numbers

All Windows numbers are from `windows-2022` CI runners. Each benchmark run
builds main and the branch and measures both on the same runner, so compare
numbers within one run, not across runs.

| What | main | branch | Where |
| --- | --- | --- | --- |
| Executable size, #29 | 918,528 bytes | 917,504 bytes | [run 41983511717](https://github.com/abooodbah/leanrows/actions/runs/41983511717) |
| Start to input idle, 250 MB CSV, #29 | 67 ms | 66 ms | same run |
| Peak working set, 250 MB CSV open, #29 | 16.2 MiB | 16.2 MiB | same run |
| 10M-row sort, core engine, #29 | 95.6 s | 82.7 s | same run |
| 10M-row sort, app worker, #29 | 112.5 s | 91.4 s | same run |
| 10M-row sort, core engine, #31 | 89.4 s | 50.1 s | [run 41986835023](https://github.com/abooodbah/leanrows/actions/runs/41986835023) |
| 10M-row sort, app worker, #31 | 107.9 s | 64.7 s | same run |
| Sort managed memory peak, #31 | 6.64 MiB | 6.58 MiB | same run |
| Executable size, #31 | 917,504 bytes | 928,768 bytes | same run |
| System calls, 1M-row sort, #31 | 2,084,481 (1,006,231 `write`, 1,000,000 `lseek`) | 86,321 (6,231 `write`, 1,100 `pwrite64`, no `lseek`) | Local, `strace -c` |
| Executable size, #32 | 916,480 bytes | 929,280 bytes | [run 41987029313](https://github.com/abooodbah/leanrows/actions/runs/41987029313) |
| Start to input idle, no file, #32 | 59 ms | 60 ms | same run |
| Start to input idle, 250 MB CSV, #32 | 90 ms | 94 ms | same run |
| Peak working set, 250 MB CSV open, #32 | 16.2 MiB | 16.3 MiB | same run |
| Executable size, #30 | 917,504 bytes | 916,480 bytes | [run 41985452878](https://github.com/abooodbah/leanrows/actions/runs/41985452878) |
| Start to input idle, no file, #30 | 56 ms | 56 ms | same run |
| Start to input idle, 250 MB CSV, #30 | 86 ms | 88 ms | same run |
| Peak working set, 250 MB CSV open, #30 | 16.2 MiB | 16.2 MiB | same run |

Main before any change (0.4.0 code):

| What | Value | Where |
| --- | --- | --- |
| Executable size | 918,528 bytes | 0.4.0 release (`docs/VALIDATION_STATUS.md`) |
| Workspace tests on Windows CI | 223 (app 5, core 34 + 6 + 6 + 15 + 13 + 23, spike 7, win32 114) | [CI run 41524019415](https://github.com/abooodbah/leanrows/actions/runs/41524019415) |
| Workspace tests on Linux | 160 (win32 has 52 here; its Win32 tests do not build on Linux) | Local |
| 10M-row text sort, core engine | 60.7 s (`leanrows-spike --sort`), 65.5 s (phase timer) | Local |
| 1M-row text sort, core engine | 4.3 s to 4.5 s | Local |
