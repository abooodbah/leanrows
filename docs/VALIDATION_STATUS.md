# Validation status

**Status date:** 2026-08-09<br>
**Scope:** LeanRows 0.1.3 frozen local release candidate PASS, Windows x64;<br>
clean release commit, post-commit rerun, tagged CI, and public artifact
verification pending<br>
**Current public release:** LeanRows 0.1.2

LeanRows 0.1.3 corrects System-theme resolution, restores the complete
System/Light/Dark appearance cycle, and aligns the indexing indicator with
LeanMark's flat progress rule. The frozen local candidate passed the current
source, native-shell, document, packaging, aggregate QA, and live installer
checks. Its executable is 546,816 bytes with SHA-256
`F8394AC84BFDD1E4857AF087B509088B5586172048A499E761DF0D5701F8A7A1`.
This is local pre-commit evidence. The clean release commit, post-commit rerun,
tagged CI rebuild, and public download verification remain pending.

LeanRows 0.1.2 is already published on
[GitHub Releases](https://github.com/abooodbah/leanrows/releases/tag/v0.1.2).
It is not awaiting tagged CI or publication.

## v0.1.3 repository state

| Area | Current state | Evidence boundary |
| --- | --- | --- |
| Toolchain and quality policy | Passed locally | Formatting passed; workspace all-target locked tests passed 143/143; strict Clippy passed; release app and spike builds passed. |
| CSV and line boundaries | Passed locally | Workspace tests and document smokes passed, including the small fixture and a 57,671,680-byte document. |
| Source snapshot integrity | Passed within the recorded local scope | Document, seek, reload, source-integrity, and clean-close checks passed. The corrective release does not change document ownership. |
| Native appearance | Passed on the frozen executable | Shell smoke exercised System, Light, Dark, and System, including persistence, effective runtime appearance, and grid repainting. |
| Native child rendering | Passed on the frozen executable | Documented Windows painting retained native roles, focus, input, and accessibility semantics. The exact Light capture is recorded below. |
| Progress rule | Passed on the frozen executable | Smoke retained `PROGRESS_CLASS`, rejected marquee style, verified 50 percent, and compared the expected accent, surface, and border pixels. |
| Packaging and installer | Passed locally | Release engineering, deterministic packaging, checksum, installer static checks, `ValidateOnly`, and a live upgrade/reinstall/uninstall/final-install cycle passed. |
| Process memory, sparse scale, and network observation | Passed locally | The authoritative 25-check receipt records memory, 1 and 10 GiB sparse fixtures, 24 seeks, and a polling-bounded network observation. |
| Tagged release package | Not yet published | v0.1.2 remains the current public release until the v0.1.3 tag and verified artifacts are available. |

## Native-shell verification result

The frozen v0.1.3 executable passed the strengthened shell smoke:

- It clicked the complete System to Light to Dark to System cycle and checked the
  button label, persisted preference, effective runtime appearance, and grid
  color after repainting.
- It verified that the indexing indicator retains the native progress-control
  class, does not carry marquee style, reports a 50 percent position, and
  renders the exact expected accent, surface, and border pixels.
- It continued to cover native accessibility names and roles, focus behavior,
  responsive layout bands, document controls, and deterministic cleanup.

The standalone shell smoke and the small and 57,671,680-byte document smokes
all passed against the frozen executable.

## Frozen v0.1.3 local evidence

The authoritative aggregate receipt is
`artifacts/qa/v0.1.3-final-frozen-pass/qa-receipt.json`. It passed 25/25
checks with 0 failures in 38,863 ms.

| Frozen evidence field | v0.1.3 local value |
| --- | --- |
| Source identity | The clean release commit and post-commit rerun remain pending. The receipt applies to the frozen release worktree and exact executable identified below. |
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

The 0.1.3 repository does not claim completion of:

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

Current descriptions may state the implemented mechanisms and the local
results above: progressive
viewports, fixed-capacity indexing, bounded previews, snapshot integrity, the
responsive native shell, inline literal search, the owner-data grid,
System/Light/Dark appearance, System resolution from the active Windows app
theme, high-contrast system colors, and the flat native progress rule. Numeric
performance, whole-process memory, package identity, installer outcomes,
network observations, and frozen-binary outcomes must remain tied to the
authoritative local receipt and exact artifact identities. The clean release
commit, post-commit results, tagged CI rebuild, and public artifacts are not
claimed until their separate verification is complete.
