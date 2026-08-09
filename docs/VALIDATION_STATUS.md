# Validation status

**Qualification date:** 2026-08-09<br>
**Scope:** LeanRows 0.1.0, Windows x64

LeanRows has moved beyond the original architecture spike. The repository now
contains the production core, native Win32 document viewer, deterministic
packaging, installer validation, adversarial tests, and a local release-QA
harness. This page distinguishes implemented evidence from final artifact
evidence that can only be attached after a source revision is frozen.

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
| Local network observation | Implemented with a polling-bounded result | Native open, view, seek, reload, and clean-close workflow observed zero TCP and UDP endpoints across six samples |
| Final release ZIP digest | Recorded by the tag workflow after source freeze | GitHub Release ZIP and `.sha256` attachment |

The repository does not commit a universal memory number or provisional binary
hash. Release qualification measures the frozen artifact and retains the raw
receipt separately from the source documentation.

The measured network run requested a 50 ms polling interval and recorded six
samples, with a 1,157 ms maximum sample-start gap. This check covers the
discovered LeanRows process tree, but it is not ETW event or packet capture and
cannot exclude activity between samples.

## Required release receipt

The final 0.1 receipt must report, without substituting missing values:

- the exact commit and executable SHA-256;
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
Markdown summary below `artifacts/qa/`. The CI tag job independently rebuilds
and publishes the deterministic unsigned package.

## Explicitly unclaimed extended evidence

The 0.1 repository does not claim completion of:

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
