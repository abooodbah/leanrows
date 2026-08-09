# LeanRows local release QA

This directory contains dependency-free Windows PowerShell evidence tooling for
the LeanRows v0.1 application. The aggregate harness uses only disposable,
synthetic local files and invokes executables directly without a command shell.

## Run the aggregate harness

Build both release entry points first:

    cargo build --release --locked -p leanrows-app -p leanrows-spike

Then run:

    & ./tests/qa/Invoke-LeanRowsReleaseQa.ps1

The harness writes a timestamped directory below artifacts/qa containing a
human-readable Markdown receipt, a machine-readable JSON receipt, individual
document/memory/seek observations, and a verified unsigned Windows package.
It exits nonzero when any required local check fails closed.

By default it creates 1 GiB and 10 GiB logical NTFS sparse fixtures. They occupy
far less physical disk than their logical sizes and are deleted after the run.
Use -SkipSparseScaleFixtures only for a deliberately reduced run; the receipt
records the scale check as not run.

The Windows GitHub Actions workflow uses that reduced profile with 12
deterministic architecture-probe seeks and writes evidence to
artifacts/ci-qa. The evidence is uploaded as its own 14-day Actions artifact;
it is never included in the exact top-level release ZIP and checksum set.
The reduced profile still runs NET-01 against the generated small local log
fixture; skipping sparse scale does not skip network observation.

The default, non-skipped harness remains the local release gate before a tag is
created. That gate constructs and opens both the 1 GiB and 10 GiB logical NTFS
sparse fixtures. The reduced CI profile is useful per-commit regression
evidence, but it does not replace the full local sparse-scale receipt.

## What each script proves

| Script | Bounded evidence |
| --- | --- |
| Invoke-LeanRowsDocumentSmoke.ps1 | Opens a real file through the native worker, validates the documented JSON contract, attests a rendered owner-data row, checks source integrity, and requires no residual process. |
| Measure-LeanRowsIdle.ps1 | Samples the complete discovered process tree at cold or document-open idle. Missing observations remain explicit with null peaks and can never become a zero-memory result. |
| New-LeanRowsSparseFixtures.ps1 | Creates sentinel-marked, structurally verified sparse logical-size fixtures with deterministic prefix/tail descriptors. |
| Observe-LeanRowsNetwork.ps1 | Launches the GUI directly with `UseShellExecute=false` and `CreateNoWindow=true`, polls local TCP/UDP endpoint tables for the complete discovered process tree, exercises open/view, a native list-view seek, reload, clean close, source integrity, and residual-process checks, and persists only counts/protocol/state. |
| Test-LeanRowsRandomSeek.ps1 | Repeats deterministically seeded CLI viewport seeks across CSV, JSONL, and log fixtures and verifies source hashes before and after. |
| Invoke-LeanRowsReleaseQa.ps1 | Coordinates fixtures, positive and negative document smokes, network observation, memory checks, random seeks, package validation, cleanup, and receipt generation. |

## Deliberate limitations

Sparse logical-size files do not substitute for complete scans of real,
non-sparse 1, 10, or 100 GiB corpora. Their full SHA-256 hashes are deliberately
not calculated; the manifest records exact logical structure, allocated bytes,
and prefix/suffix descriptor hashes.

NET-01 is high-frequency local polling with `Get-NetTCPConnection` and
`Get-NetUDPEndpoint`; it records the requested cadence and observed start gap.
It is not ETW packet/event capture and cannot prove that no activity occurred
between samples. Raw addresses, ports, and endpoint objects are never written
to evidence. A missing cmdlet, failed sample, observed TCP connection or UDP
endpoint, sample-start gap beyond the configured bound, incomplete UI action,
changed source, forced cleanup, or residual process fails the gate closed.
The supplied fixture must be bounded and located below a system-temporary root
whose `.support_tool_fixture_root` sentinel contains `synthetic_self_test`.

The harness does not claim a manual screen-reader audit, source-change race
safety, 24 CPU-hours of fuzzing, signed artifacts, or macOS/Linux behavior. It
also records private bytes and working set but does not silently substitute
either value for process commit. These gaps remain explicit in every aggregate
receipt.

Installer exercise is static plus ValidateOnly. It does not write registry
associations or install LeanRows. Disposable fixture deletion is permitted
only for a system-temporary directory with the leanrows-qa- prefix and the
.leanrows_qa_fixture_root sentinel.

Rust workspace tests, formatting, and Clippy are separate CI gates. Their
results are deliberately not inferred into or claimed by the QA receipt.
