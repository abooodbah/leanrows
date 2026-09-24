# Validation status

**Status date:** 2026-09-24<br>
**Scope:** LeanRows 0.2.0 release candidate, Windows x64;<br>
local frozen verification complete; tagged-CI, public-package, and Pages
verification pending<br>
**Current public release:** LeanRows 0.1.3 until the `v0.2.0` tag publishes

LeanRows 0.2.0 opens every file in one window, each in its own tab, cuts memory
when several files are open, and fixes white file name, file details, and
Match case backgrounds in the Dark appearance. The frozen local candidate
passed the source, native-shell, document, packaging, aggregate QA, and live
installer checks. That local executable is 618,496 bytes with SHA-256
`D0D24FCA9730BE6C7A0E1385AAB5CB6F08C8D495A595FC32E4066F583BC88A8D`.
The release commit, tagged CI rebuild, public package, installed public
executable, and product site are verified separately after tagging.

## v0.2.0 repository state

| Area | Current state | Evidence boundary |
| --- | --- | --- |
| Toolchain and quality policy | Passed locally | Formatting passed; workspace all-target locked tests passed 158/158; strict Clippy passed; release app and spike builds passed. |
| CSV and line boundaries | Passed locally | Workspace tests and document smokes passed. The document engine is unchanged from 0.1.3. |
| Source snapshot integrity | Passed within the recorded local scope | Document, seek, reload, source-integrity, and clean-close checks passed. Each tab owns its own snapshot handle. |
| Tabs and one window | Passed on the frozen executable | The shell smoke painted a two-tab strip and checked a second launch handing its file to the running window. Separate scripted checks switched, clicked, and closed tabs and restored scroll position and selection. |
| Native child rendering | Passed on the frozen executable | The shell smoke sampled the file name, file details, and Match case backgrounds on first presentation. The exact Light capture is recorded below. |
| Packaging and installer | Passed locally | Release engineering, deterministic packaging, checksum, installer static checks, `ValidateOnly`, and a live upgrade/reinstall/uninstall/final-install cycle passed. |
| Process memory, sparse scale, and network observation | Passed locally | The 25-check receipt records memory, 1 and 10 GiB sparse fixtures, 24 seeks, and a polling-bounded network observation. |
| Tagged release package and site | Pending | Recorded after tag `v0.2.0` publishes. |

## Native-shell verification result

The frozen v0.2.0 executable passed the extended shell smoke:

- It laid the shell out as it is with two open files, checked that the tab
  strip held both tabs with the second selected, and compared the rendered
  surface, selected-tab accent, and border pixels. It then restored the
  one-file layout and checked that the strip was hidden again.
- It sampled the file name, file details, and Match case backgrounds on first
  presentation. With the old brush conversion put back, this check failed in 3
  and 5 of two batches of 10 runs; with the fix it passed every run.
- A second ordinary launch exited 0 and its file became the active tab,
  reopening the first file selected its tab again, one process remained, and
  the two-tab window exited 0 on `WM_CLOSE`.
- It kept the existing checks: the System to Light to Dark to System cycle,
  the progress rule's class, value, and pixels, accessibility names and roles,
  focus behavior, responsive layout bands, and deterministic cleanup.

## Frozen v0.2.0 local evidence

The aggregate receipt is
`artifacts/qa/v0.2.0-final-frozen-pass/qa-receipt.json`. It passed 25/25
checks with 0 failures in 53,651 ms.

| Frozen evidence field | v0.2.0 local value |
| --- | --- |
| Source identity | This receipt applies to the pre-commit frozen worktree and local executable identified below. The release commit and tag are verified separately after publication. |
| Executable | 618,496 bytes; version `0.2.0.0`; SHA-256 `D0D24FCA9730BE6C7A0E1385AAB5CB6F08C8D495A595FC32E4066F583BC88A8D`. |
| Source and build gates | Formatting passed; workspace all-target locked tests passed 158/158; strict Clippy passed; release app and spike builds passed; shell smoke, document smokes, and release engineering passed. |
| Aggregate result | 25/25 checks passed, 0 failed, in 53,651 ms. |
| Cold process tree | Peak 2,445,312 private bytes and 17,625,088 working-set bytes across 3 samples; input-idle startup attestation took 122 ms. Commit bytes were not observed. |
| Document process tree | Peak 3,346,432 private bytes and 18,833,408 working-set bytes across 3 samples; startup attestation took 145 ms. Commit bytes were not observed. |
| Sparse logical scale | The 1 GiB and 10 GiB sparse fixtures each reported 131,072 allocated bytes and returned first viewports in 318 ms and 299 ms. These are sparse-file observations, not equivalent full-I/O measurements. |
| Deterministic seeks | 24/24 seeks passed across 3 fixtures with seed `20260809`. |
| Network observation | Six polling samples observed TCP = 0 and UDP = 0, with a maximum 1,288 ms sample-start gap and a maximum process-tree size of 1. Open, view, native seek, reload with a durable view reset, source integrity, clean close, and zero-residual-process checks passed. |
| Unsigned package | ZIP SHA-256 `912AB4ACF7BCBEC30EBAA593CEA19ED8C07DA343B3A2112A71BD1E88E556504E`; installer static, `ValidateOnly`, package, checksum, and release-engineering checks passed. The aggregate run did not change the persistent installation. |
| Live installer lifecycle | The baseline was an installed pre-release build of `main` labeled 0.1.3.0. Validation, upgrade, reinstall, uninstall, and final install all exited 0, and the reinstall state was identical. Uninstall removed the owned files, registry entries, shortcut, and **Open with** registrations and restored four direct defaults to absent. Final install left the frozen v0.2.0 executable installed, set four direct defaults to LeanRows, and registered five **Open with** entries. The protected Notepad `.log` choice and `.txt` were unchanged throughout. |
| Installer command contract | The installer intentionally creates no App Paths key. Absolute ProgID and Applications open commands and the Start menu shortcut were verified. |
| Application capture | `site/assets/leanrows-app.png` is 1,180 x 720, 63,395 bytes, and SHA-256 `90DE75F43C8DD66AC4AD11A1EF10749CB78BCF5DFF45CBBDE6B96DF72A582650`. It captures the frozen executable in Light appearance with the regenerated 120-row, six-field, 7,600-byte fixture active and a second synthetic file in another tab. The saved Dark preference was restored afterwards. |
| Cleanup | The aggregate fixture root was removed and residual process count was 0. The live installer cycle left the frozen v0.2.0 installation intentionally present. |

## Memory with several files open

These observations come from a separate harness that launches LeanRows once per
file, as Explorer does, waits 6 s, and sums every process started from the same
executable path. The 0.2.0 column is the frozen executable above; the 0.1.3
column is the published 0.1.3 executable. Five files were opened from the QA
fixtures, two rounds per build; the rounds agreed within 0.05 MiB private.

| Five files open | 0.1.3 | 0.2.0 |
| --- | --- | --- |
| Processes | 5 | 1 |
| Private memory | 14.74 to 14.75 MiB | 3.84 to 3.85 MiB |
| Working set | 88.02 to 88.10 MiB | 18.50 to 18.51 MiB |
| Working set 4 s after minimizing | 88.05 to 88.10 MiB | 0.77 to 0.78 MiB |

With one file open, 0.1.3 measured 3.23 to 3.28 MiB private and 17.98 to 18.03
MiB working set, and 0.2.0 measured 3.28 to 3.30 MiB private and 18.09 to 18.10
MiB working set. On a generated 20,000,001-row, 1,586,666,734-byte CSV indexed
to completion, peak working set was 18.10 to 18.12 MiB for 0.1.3 and 18.15 to
18.18 MiB for 0.2.0, two runs each. These are host-specific observations.

## Historical v0.1.3 local evidence

The authoritative aggregate receipt is
`artifacts/qa/v0.1.3-final-frozen-pass/qa-receipt.json`. It passed 25/25
checks with 0 failures in 38,863 ms.

| Frozen evidence field | v0.1.3 local value |
| --- | --- |
| Source identity | This receipt applies to the pre-commit frozen worktree and local executable identified below. Release commit `9e4d99cc1760feed1882eaef8b56164b4a02274f` and tag `v0.1.3` are verified separately in the published-release section. |
| Executable | 546,816 bytes; SHA-256 `F8394AC84BFDD1E4857AF087B509088B5586172048A499E761DF0D5701F8A7A1`. |
| Source and build gates | Formatting passed; workspace all-target locked tests passed 143/143; strict Clippy passed; release app and spike builds passed; shell smoke, document smokes, and release engineering passed. |
| Aggregate result | 25/25 checks passed, 0 failed, in 38,863 ms. |
| Cold process tree | Peak 2,514,944 private bytes and 17,661,952 working-set bytes across 3 samples; input-idle startup attestation took 105 ms. Commit bytes were not observed. |
| Document process tree | Peak 3,321,856 private bytes and 18,702,336 working-set bytes across 3 samples; startup attestation took 163 ms. Commit bytes were not observed. |
| Sparse logical scale | The 1 GiB and 10 GiB sparse fixtures each reported 131,072 allocated bytes and returned 128-row first viewports in 340 ms and 262 ms. These are sparse-file observations, not equivalent full-I/O measurements. |
| Deterministic seeks | 24/24 seeks passed across 3 fixtures with seed `20260809`. |
| Network observation | Five polling samples observed TCP = 0 and UDP = 0, with a maximum 1,322 ms sample-start gap and maximum process-tree size of 1. A native seek moved the top index from 0 to 998 for target row 1,024. Reload produced durable reset evidence at top index 0 and repopulated 128 rows; source integrity, clean close, and zero-residual-process checks passed. |
| Unsigned package | ZIP SHA-256 `BCD32B49FEA25E4079782403AF03366AC1C38DB1DD907B6B9920503DD396A8A3`; installer static, `ValidateOnly`, package, checksum, and release-engineering checks passed. The aggregate run did not change the persistent installation. |
| Live installer lifecycle | An installed v0.1.2 baseline was upgraded. v0.1.3 install, reinstall, uninstall, and final install all exited 0; reinstall state was identical. Uninstall removed owned files, registry entries, shortcut, and **Open with** registrations, and restored four direct defaults to absent. Final install left the exact v0.1.3 executable installed, set four direct defaults to LeanRows, and registered five **Open with** entries. The protected Notepad `.log` choice was preserved throughout. |
| Installer command contract | The installer intentionally creates no App Paths key. Absolute ProgID and Applications open commands and the Start menu shortcut were verified. |
| Application capture | `site/assets/leanrows-app.png` is 1,180 x 720, 67,555 bytes, and SHA-256 `40B5C9B9358FAC0A0882005333876255F551997D8EBB84C8A050F3DE8284C011`. It captures the exact frozen executable in Light appearance with the synthetic 120-row, six-field fixture. |
| Cleanup | The aggregate fixture root was removed and residual process count was 0. The live installer cycle left the final v0.1.3 installation intentionally present, with no process or temporary residue. |

Two earlier fail-closed receipts remain preserved. They exposed a race in the
network observer's requirement to capture a transient empty viewport during
reload. The harness now checks durable reset evidence instead: the top index
returned to 0 and the viewport repopulated to 128 rows. The authoritative final
receipt passed that condition. This correction does not relabel either earlier
receipt as a pass.

The network result is polling-bounded rather than an ETW event trace or packet
capture. It cannot exclude activity between samples.

## Published v0.1.3 verification

The public release is anchored to commit
`9e4d99cc1760feed1882eaef8b56164b4a02274f` and annotated tag `v0.1.3`.
The clean post-commit receipt at
`artifacts/qa/v0.1.3-post-commit/qa-receipt.json` passed 25/25 checks. Tagged
workflow
[run 31355194118](https://github.com/abooodbah/leanrows/actions/runs/31355194118)
completed with every job green and published the release and Pages site.

| Published evidence field | v0.1.3 result |
| --- | --- |
| Release | [LeanRows v0.1.3](https://github.com/abooodbah/leanrows/releases/tag/v0.1.3) contains exactly the unsigned Windows x64 ZIP and its checksum sidecar. |
| Public package | The downloaded sidecar records ZIP SHA-256 `9BC275C6D4784637C5ED63174A580B1DA5401ABC5A024A4B76DED205AEFFB0B7`, and the downloaded ZIP matched it. |
| Public executable | The extracted and installed `leanrows.exe` is 546,816 bytes, reports version `0.1.3.0`, and has SHA-256 `79DD102A9A8BDC9A0A591893F1FC10BC1A7EF8BDBF33F453E5A6DBE2FE1ACB0B`. |
| Downloaded-package checks | The public assets passed `Test-ReleaseEngineering.ps1`, installer `ValidateOnly`, installation, and the native shell smoke. The smoke completed in 182 ms with zero residual processes. |
| Installed associations | Four direct per-user defaults point to LeanRows and all five **Open with** registrations are present. The protected Notepad `.log` choice remained unchanged. |
| Installed appearance | The installed application retained System appearance. |
| Product site | GitHub Pages deployed successfully and returned HTTP 200 with the v0.1.3 application capture. |

The local frozen executable and package remain identified by
`F8394AC84BFDD1E4857AF087B509088B5586172048A499E761DF0D5701F8A7A1`
and
`BCD32B49FEA25E4079782403AF03366AC1C38DB1DD907B6B9920503DD396A8A3`,
respectively. They are local evidence and are not the public download hashes.
The public executable and package identities are the values in the table
above.

This status update is a post-release documentation change on `main`; it is
not part of the `v0.1.3` tag.

## Historical v0.1.2 local qualification

The table below preserves the pre-tag v0.1.2 local observations already
recorded by the project. These values belong only to that candidate and do not
qualify v0.1.3. Public v0.1.2 artifact identity is established by the checksum
sidecar attached to its GitHub release.

| Historical evidence field | v0.1.2 local value |
| --- | --- |
| Aggregate gate result | 25/25 passed in `artifacts/qa/v0.1.2-final-local/qa-receipt.json`; the post-commit harness wrote `artifacts/qa/v0.1.2-post-commit/qa-receipt.json`. |
| Local frozen binary and unsigned package | `leanrows.exe`: 527,872 bytes, SHA-256 `8c8fd33435b1b8e6e443e4b3198382f90fe44f69ad0bb051104424dc3fd29c9f`. Local ZIP: SHA-256 `b0e4425793ba6bfbcaecba97763500c70d9dcf74a5a42610b17126e0d402d67f`. |
| Cold and document-open process-tree memory | Cold peak: 2,363,392 private bytes and 15,458,304 working-set bytes. Document-open peak: 3,137,536 private bytes and 16,457,728 working-set bytes. Commit bytes were not observed. |
| Deterministic seek and sparse logical scale | 24/24 seeks passed. First viewport: 170 ms for the 1 GiB sparse fixture and 182 ms for the 10 GiB sparse fixture; each reported 131,072 allocated bytes. These are host-specific sparse-file observations, not equivalent full-I/O measurements. |
| Disposable installer cycle | Install and reinstall hashes matched; uninstall removed the executable and LeanRows direct defaults; the final reinstall hash matched. `.csv`, `.tsv`, `.jsonl`, and `.ndjson` were direct defaults, while the protected `.log` `UserChoice` was preserved. |
| Local network observation | Six polling samples observed TCP = 0 and UDP = 0, with a maximum sample-start gap of 1,541 ms. Open, seek, reload, close, source-integrity, and cleanup checks passed. |
| Cleanup | Cleanup was `true`; residual process count was 0. |

Separate v0.1.2 source-quality commands also recorded formatting, 133 workspace
tests, strict Clippy, a release build, and product-site validation as passing.
These results were outside the 25-check aggregate receipt. The network result
was polling-bounded rather than an ETW trace or packet capture, so it could not
exclude activity between samples.

## Historical v0.1.1 local-candidate evidence

The earlier v0.1.1 aggregate run passed 25/25 checks. It completed 24/24
deterministic seeks and recorded first viewports of 172 ms and 173 ms for
structurally verified 1 GiB and 10 GiB sparse fixtures. Those host-specific
observations are not universal performance claims and are not evidence for
v0.1.3.

Its disposable installer cycle verified all five **Open with** registrations,
direct defaults for four unprotected extensions, restoration of a prior
`.csv` handler, preservation of later user choices, and zero LeanRows-owned
residue. Its six-sample polling-bounded network run observed no TCP or UDP
endpoints and left no residual process.

## Explicitly unclaimed extended evidence

The 0.2.0 repository does not claim completion of:

- a 24 CPU-hour fuzzing campaign;
- 100,000 oracle-backed random seeks;
- complete scans of real, non-sparse 10 GiB or 100 GiB corpora;
- third-party comparative performance leadership;
- Authenticode signing;
- comprehensive manual screen-reader certification; or
- macOS, Linux, ARM64, or x86 release support.

These items remain in the extended assurance program described in
[Acceptance](ACCEPTANCE.md). Their absence does not become a hidden pass.

## Claim discipline

Current descriptions may state the implemented mechanisms and verified results
above: progressive viewports, fixed-capacity indexing, bounded previews,
snapshot integrity, tabs in one window, the responsive native shell, inline
literal search, the owner-data grid, System/Light/Dark appearance, System
resolution from the active Windows app theme, high-contrast system colors, and
the flat native progress rule. Numeric performance, whole-process memory, package identity,
installer outcomes, network observations, and binary outcomes must remain tied
to their recorded receipts and exact artifact identities. In particular, the
local frozen hashes and tagged public hashes describe different artifacts and
must not be interchanged.
