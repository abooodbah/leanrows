# Validation status

**Status date:** 2026-08-09<br>
**Scope:** LeanRows 0.1.2 frozen local release candidate PASS, Windows x64;<br>
tagged CI rebuild/publication pending

LeanRows 0.1.2 contains the production core, redesigned native Win32 viewer,
deterministic packaging, installer validation, adversarial tests, and the local
release-QA harness. The frozen local candidate passed its documented pre-tag
qualification. Tagged CI must still rebuild the release commit and publish the
unsigned package.

## Repository evidence

| Area | Current repository state | Evidence |
| --- | --- | --- |
| Toolchain and quality policy | Implemented | Rust 1.97.1 pin, lockfile, formatting, all-target tests, strict Clippy, Windows CI |
| CSV and line boundaries | Implemented and covered by automated tests | Core adversarial tests and Win32 document-engine tests |
| Bounded index and viewport | Implemented and covered by automated tests | Fixed-capacity checkpoints, operation plans, giant-record tests |
| Literal query engine and inline search | Implemented and covered by core, worker, and native tests | Query integration, cancellation, quota, navigation, source rotation, and multi-process scratch-lifecycle tests |
| Source snapshot integrity | Implemented and covered by Windows-specific tests | Write-sharing denial, atomic-replacement detection, generation cancellation |
| Native UI and accessibility | Implemented; frozen 0.1.2 binary smoke passed | Warm opaque responsive top bar, inline search, virtual owner-data grid, System/Light appearance choices, high-contrast system-color fallback, DPI handling, accessibility names |
| Packaging and installer validation | Implemented | Deterministic package scripts, manifest verification, CI package job |
| Process-tree memory and scale harness | Completed; results are host-specific | `tests/qa/` aggregate JSON and Markdown receipts |
| Local network observation | Completed with a polling-bounded result | Six samples observed zero TCP and UDP endpoints during native open, seek, reload, and clean close; source integrity and process cleanup passed |
| Tagged release package | Tagged CI rebuild/publication pending | The tag workflow independently rebuilds and publishes the unsigned ZIP and checksum sidecar |

The redesign changes the native shell, not the document ownership model. The
responsive top bar and inline search continue to feed the bounded worker and
cache path, while the `SysListView32` row surface remains owner-data. Public
copy may describe these mechanisms before tagging. The frozen binary hashes,
host-specific timings, whole-process memory, installer-cycle results, and
network observation are recorded below.

## Frozen v0.1.2 local evidence

The pre-tag aggregate receipt records the frozen local candidate. A post-commit
rerun records the exact release commit before the annotated tag is created.
Earlier v0.1.1 results are not used as v0.1.2 evidence.

| Frozen evidence field | v0.1.2 value |
| --- | --- |
| Exact clean commit | The exact release commit is recorded by the post-commit receipt at `artifacts/qa/v0.1.2-post-commit/qa-receipt.json`; annotated tag `v0.1.2` must resolve to it. |
| Aggregate gate result and receipt path | 25/25 passed in the pre-tag receipt at `artifacts/qa/v0.1.2-final-local/qa-receipt.json`; the final post-commit rerun writes `artifacts/qa/v0.1.2-post-commit/qa-receipt.json`. |
| Frozen binary and unsigned package | `leanrows.exe`: 527,872 bytes, SHA-256 `8c8fd33435b1b8e6e443e4b3198382f90fe44f69ad0bb051104424dc3fd29c9f`. `LeanRows-v0.1.2-windows-x64-unsigned.zip`: SHA-256 `b0e4425793ba6bfbcaecba97763500c70d9dcf74a5a42610b17126e0d402d67f`. |
| Cold and document-open process-tree memory | Cold peak: 2,363,392 private bytes and 15,458,304 working-set bytes. Document-open peak: 3,137,536 private bytes and 16,457,728 working-set bytes. Commit bytes were not observed. |
| Deterministic seek and sparse logical-scale result | 24/24 seeks passed. First viewport: 170 ms for the 1 GiB sparse fixture and 182 ms for the 10 GiB sparse fixture; each reported 131,072 allocated bytes. These are host-specific sparse-file observations, not equivalent full-I/O measurements. |
| Disposable install/reinstall/uninstall cycle | Install and reinstall hashes matched; uninstall removed the executable and LeanRows direct defaults; the final reinstall hash matched. `.csv`, `.tsv`, `.jsonl`, and `.ndjson` were direct defaults, the protected `.log` `UserChoice` was preserved, and staging was removed. |
| Local network observation | Six polling samples observed TCP = 0 and UDP = 0, with a maximum sample-start gap of 1,541 ms. Open, seek, reload, close, and source-integrity checks passed. |
| Cleanup and residual processes | Cleanup was `true`; residual process count was 0. |

Separate source-quality commands, run outside the aggregate receipt, also
passed: Rust formatting, the full workspace test run (133 tests), strict
Clippy, the release build, and the product-site validator. These results are
not attributed to the 25-check aggregate receipt.

The network result is polling-bounded rather than an ETW event trace or packet
capture. It cannot exclude activity between samples.

Tag CI must independently rebuild the same commit, rerun its documented gates,
and publish the deterministic unsigned package. The aggregate local harness
writes machine-readable JSON and a readable Markdown summary below
`artifacts/qa/`.

## Historical v0.1.1 local-candidate evidence

The evidence in this section belongs to the earlier 0.1.1 local candidate. It
demonstrates that the QA and installer harnesses produced bounded, reproducible
results, but it does not qualify the redesigned 0.1.2 binary.

The v0.1.1 aggregate run passed 25/25 checks. First viewports for structurally
verified 1 GiB and 10 GiB sparse fixtures passed at 172 ms and 173 ms,
respectively; these were host-specific observations, not universal performance
claims. The run also completed 24/24 deterministic seeks, captured cold and
document-open process-tree memory, passed deterministic unsigned packaging and
installer `-ValidateOnly`, and reported cleanup `true`.

A disposable current-user install, reinstall, and uninstall cycle verified all
five **Open with** registrations, direct defaults for the four unprotected
extensions, and eight kind-correct saved-state values. Reinstall was
idempotent; uninstall restored a prior `.csv` handler, removed defaults with no
predecessor, preserved a simulated later `.ndjson` handler, left protected
`.log` and `.txt` choices unchanged, and left zero LeanRows-owned residue.

The v0.1.1 network run recorded six samples with zero TCP and UDP endpoints
during open, view, seek, reload, and clean close. The source was unchanged, the
residual process count was 0, and the maximum sample-start gap was 1,905 ms.
This polling-bounded check covered the discovered LeanRows process tree, but it
was not ETW event or packet capture and could not exclude activity between
samples.

## Required release receipt

The 0.1.2 local receipt must record:

- the exact clean commit identity from the post-commit rerun;
- formatting, all-target test, strict Clippy, native smoke, and packaging
  results;
- positive and negative document-smoke results;
- cold and document-open full process-tree observations;
- deterministic seek and sparse logical-scale results;
- source hashes before and after applicable runs;
- installer `-ValidateOnly` and disposable lifecycle results;
- network-observation result;
- cleanup status and residual-process count; and
- every skipped or unsupported assurance item.

## Explicitly unclaimed extended evidence

The 0.1.2 repository does not claim completion of:

- a 24 CPU-hour fuzzing campaign;
- 100,000 oracle-backed random seeks;
- complete scans of real, non-sparse 10 GiB or 100 GiB corpora;
- third-party comparative performance leadership;
- Authenticode signing;
- a comprehensive manual screen-reader certification; or
- macOS, Linux, ARM64, or x86 release support.

These items remain in the extended assurance program described in
[Acceptance](ACCEPTANCE.md). Their absence does not become a hidden pass.

## Claim discipline

Public descriptions may state the implemented mechanisms: progressive
viewports, fixed-capacity indexing, bounded previews, snapshot integrity, the
warm opaque responsive shell, inline literal search, the owner-data grid,
System and Light appearance choices, and the high-contrast system-color
fallback. Numeric performance, whole-process memory, comparative claims, and
frozen-binary outcomes require a final v0.1.2 receipt with the relevant host
and fixture details.
