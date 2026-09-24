# Acceptance and assurance

LeanRows separates practical 0.2.x release qualification from longer assurance
campaigns. This prevents an unrun research-scale test from being described as a
pass while keeping the project accountable for the evidence required to ship a
useful first release.

Passing source-level tests alone does not establish startup time, total process
memory, installer behavior, or accessibility. Conversely, a sparse 10 GiB file
can establish bounded first-viewport behavior but cannot substitute for a full
scan of 10 GiB of physically allocated data.

## Evidence rules

A runtime result should record the source revision, command or harness version,
fixture description and hash where feasible, Windows build, hardware and
storage, raw observation, and pass/fail rule. Generated local evidence belongs
under `artifacts/qa/` and is not treated as a timeless product claim.

Measurements use these terms:

- **Cold baseline:** the complete discovered process tree at idle with no open
  document.
- **Document idle:** the same process-tree measurement with a named fixture
  open and the requested view stable.
- **Private bytes:** private committed virtual memory for all discovered
  LeanRows processes.
- **Working set:** resident pages for the same process tree at the sample time.
- **First viewport:** a real worker event has supplied a bounded row cache and
  the native owner-data control has served at least one displayed row.
- **Logical sparse size:** the addressable file length of an NTFS sparse
  fixture. It is not the amount of physical data scanned.

## Required for a 0.2.x tag

| ID | Release requirement | Repository evidence path |
| --- | --- | --- |
| BUILD-01 | Rust formatting, all workspace targets, and strict Clippy pass on Rust 1.97.1 with the lockfile enforced. | Root README commands and `.github/workflows/windows.yml` |
| CORE-01 | Scanner, checkpoint, viewport, query-quota, cancellation, malformed-input, and source-change tests pass. | `leanrows-core` unit and integration tests |
| APP-01 | The native window, responsive top bar, inline search, owner-data grid, flat progress rule, tab strip, status strip, accessibility names, keyboard focus, and cleanup pass the shell smoke. The progress check retains the native class, rejects marquee style, verifies a 50 percent value, and compares the rendered accent, surface, and border pixels. The tab strip check lays the shell out for two files and compares the rendered surface, selected-tab accent, and border pixels. A second ordinary launch hands its file to the running window and exits 0, reopening an open file selects its tab, and one process remains. | `leanrows.exe --smoke-test` and `tests/qa/Invoke-LeanRowsShellSmoke.ps1` in the Windows workflow |
| DOC-01 | A supported real document produces valid bounded smoke evidence, serves a native row, leaves its source unchanged, and leaves no residual process. Empty, missing, and unsupported inputs fail closed. | `tests/qa/Invoke-LeanRowsDocumentSmoke.ps1` |
| FORMAT-01 | Targeted tests cover CSV and TSV quoted newlines and escaped quotes, line formats, invalid bytes, empty input, and giant-record preview bounds. | `leanrows-win32` document-engine tests |
| SOURCE-01 | In-place append, truncate, and rewrite are denied while open; atomic replacement is detected; stale snapshot work stops. | Core source-integrity tests and Windows document-engine tests |
| MEMORY-01 | Cold and document-open process-tree memory observations are present and non-null; missing samples cannot be converted to zero. Results are disclosed without calling managed limits total RAM. | `tests/qa/Measure-LeanRowsIdle.ps1` and aggregate receipt |
| SCALE-01 | First viewports and deterministic seeks succeed on bounded synthetic fixtures, including 1 GiB and 10 GiB logical sparse fixtures, with source-integrity checks and complete cleanup. | `tests/qa/New-LeanRowsSparseFixtures.ps1` and `Test-LeanRowsRandomSeek.ps1` |
| PKG-01 | The unsigned Windows x64 package is deterministic, contains the exact allow-listed payload, validates every manifest hash, and has a separate SHA-256 file. | `scripts/Test-ReleaseEngineering.ps1` |
| INSTALL-01 | Installer validation succeeds without changing files or the registry; registration and uninstall paths pass static contract checks. | Package `install.ps1 -ValidateOnly` and release-engineering tests |
| A11Y-01 | Automated native smoke confirms accessible names, roles, focus, the System to Light to Dark to System appearance cycle, preference persistence, runtime grid repainting, and high-contrast system-color fallback. The documented keyboard path and responsive narrow/default/wide layouts are manually usable before tagging. | Native smoke and release checklist |
| NET-01 | A recorded Windows network observation finds no TCP connection or UDP endpoint during open, view, a native list seek, reload, and clean close. | `tests/qa/Observe-LeanRowsNetwork.ps1` and aggregate release receipt |

The aggregate local harness coordinates document smokes, memory observations,
deterministic seeks, sparse scale fixtures, package validation, and cleanup:

```powershell
cargo +1.97.1 build --release --locked -p leanrows-app -p leanrows-spike
.\tests\qa\Invoke-LeanRowsReleaseQa.ps1
```

Its [README](../tests/qa/README.md) defines exactly what it proves and, equally
important, what it does not prove. A deliberately reduced run must record that
the sparse scale fixtures were skipped.

## Extended assurance

The following campaigns strengthen later releases and comparative claims, but
they are not represented as completed 0.2.x release gates:

| ID | Extended campaign | Claim boundary |
| --- | --- | --- |
| FUZZ-EXT | At least 24 aggregate CPU-hours of coverage-guided fuzzing across scanners, indexes, viewport reconstruction, and query state. | No long-duration fuzz claim until a versioned receipt exists. |
| SEEK-EXT | 100,000 deterministically seeded random-row reconstructions against an independent sequential oracle. | The smaller release matrix is not described as 100,000 seeks. |
| SCALE-EXT | Full scans of real, non-sparse 1, 10, and 100 GiB corpora where storage permits. | Sparse logical files prove address-space behavior, not equivalent I/O volume. |
| PERF-EXT | Repeatable first-viewport and indexing comparisons against named tools on a recorded reference host. | No “fastest” or cross-tool RAM claim without the full protocol and raw results. |
| A11Y-EXT | Manual screen-reader coverage and automated accessibility analysis beyond the built-in smoke. | Native roles and names are not presented as universal assistive-technology certification. |
| RECOVERY-EXT | Disk-full, permission-denied, forced termination, and owned-scratch recovery campaigns. | Core quota tests do not prove every filesystem failure mode. |

Extended campaigns can become release-blocking in a later version through an
accepted architecture decision. Until then, their status remains explicit in
[Validation status](VALIDATION_STATUS.md).

## Release decision

A 0.2.x tag requires every item in the release table to pass or to have a
written, publicly reviewed exception that narrows the release claim. A failed
check is fixed or the release scope changes; it is never relabeled as a pass.
Final ZIP digests and host-specific memory results are recorded only after the
source and binary are frozen.
