# Validation status

**Status date:** 2026-08-09<br>
**Scope:** LeanRows 0.1.1 frozen local release candidate, Windows x64;<br>
tagged CI rebuild/publication pending

LeanRows has moved beyond the original architecture spike. The repository now
contains the production core, native Win32 document viewer, deterministic
packaging, installer validation, adversarial tests, and a local release-QA
harness. This page distinguishes measured local-candidate evidence from the
independent tagged CI rebuild and publication that remain pending.

## Repository evidence

| Area | Current repository state | Evidence |
| --- | --- | --- |
| Toolchain and quality policy | Implemented | Rust 1.97.1 pin, lockfile, formatting, all-target tests, strict Clippy, Windows CI |
| CSV and line boundaries | Implemented and covered by automated tests | Core adversarial tests and Win32 document-engine tests |
| Bounded index and viewport | Implemented and covered by automated tests | Fixed-capacity checkpoints, operation plans, giant-record tests |
| Literal query engine and native Find | Implemented and covered by core, worker, and native tests | Query integration, cancellation, quota, navigation, source-rotation, and multi-process scratch-lifecycle tests |
| Source snapshot integrity | Implemented and covered by Windows-specific tests | Write-sharing denial, atomic-replacement detection, generation cancellation |
| Native UI and accessibility | Implemented with automated smoke coverage | Win32 shell smoke and document smoke |
| Packaging and installer validation | Implemented | Deterministic package scripts, manifest verification, CI package job |
| Process-tree memory and scale harness | Implemented; results are host-specific | `tests/qa/` aggregate JSON and Markdown receipts |
| Local network observation | Completed with a polling-bounded result | Six samples observed zero TCP and UDP endpoints during native open, view, seek, reload, and clean close; source unchanged and residual process count 0 |
| Tagged release package | Tagged CI rebuild/publication pending | The tag workflow independently rebuilds and publishes the unsigned ZIP and checksum sidecar |

The repository does not commit a universal memory number or provisional binary
hash. Release qualification measures the frozen artifact and retains the raw
receipt separately from the source documentation.

The frozen local candidate's aggregate run passed 25/25 checks. First
viewports for structurally verified 1 GiB and 10 GiB sparse fixtures passed at
172 ms and 173 ms, respectively; these are host-specific observations, not
universal performance claims. The run also completed 24/24 deterministic
seeks, captured cold and document-open process-tree memory, passed
deterministic unsigned packaging and installer `-ValidateOnly`, and reported
cleanup `true`.

A disposable current-user install, reinstall, and uninstall cycle verified all
five **Open with** registrations, direct defaults for the four unprotected
extensions, and eight kind-correct saved-state values. Reinstall was
idempotent; uninstall restored a prior `.csv` handler, removed defaults with no
predecessor, preserved a simulated later `.ndjson` handler, left protected
`.log` and `.txt` choices unchanged, and left zero LeanRows-owned residue.

The local v0.1.1 candidate network run recorded six samples with zero TCP and
UDP endpoints during open, view, seek, reload, and clean close. The source was
unchanged, the residual process count was 0, and the maximum sample-start gap
was 1,905 ms. This polling-bounded check covers the discovered LeanRows process
tree, but it is not ETW event or packet capture and cannot exclude activity
between samples.

## Required release receipt

The local 0.1.1 candidate receipt reports the measured checks below. Tag CI
independently rebuilds and publishes the deterministic unsigned package. The
exact-commit requirement is satisfied by rerunning the aggregate receipt after
the release changes are cleanly committed:

- the exact clean commit identity from the post-commit rerun;
- formatting, all-target test, strict Clippy, native smoke, and packaging
  results;
- positive and negative document-smoke results;
- cold and document-open full process-tree observations;
- deterministic seek and sparse logical-scale results;
- source hashes before and after applicable runs;
- installer `-ValidateOnly` result;
- network-observation result;
- cleanup status and residual-process count; and
- every skipped or unsupported assurance item.

The aggregate local harness writes machine-readable JSON and a readable
Markdown summary below `artifacts/qa/`.

## Explicitly unclaimed extended evidence

The 0.1.1 repository does not claim completion of:

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
viewports, fixed-capacity indexing, bounded previews, snapshot integrity, and a
native interface. Numeric performance, whole-process memory, and comparative
claims require a frozen-artifact receipt with the relevant host and fixture
details.
