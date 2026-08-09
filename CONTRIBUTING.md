# Contributing to LeanRows

LeanRows welcomes focused bug reports, reproducible performance findings,
documentation improvements, tests, and implementation changes that preserve
its read-only and bounded-memory design.

## Before opening an issue

- Search existing issues for the same behavior.
- Use the bug form for reproducible defects and include the LeanRows version,
  Windows version, file format, and the smallest safe fixture or generator that
  demonstrates the problem.
- Do not attach confidential data. Prefer a synthetic fixture and include its
  SHA-256 digest when exact bytes matter.
- Use [private vulnerability reporting](https://github.com/abooodbah/leanrows/security/advisories/new)
  for suspected security issues. Do not disclose them in a public issue.

## Development setup

LeanRows targets Windows x64 and pins Rust 1.97.1. The Windows SDK resource
compiler (`rc.exe`) is required for release builds of the native application.

```powershell
rustup toolchain install 1.97.1 --profile minimal --component rustfmt,clippy
cargo +1.97.1 build --workspace --all-targets --locked
```

Run the required checks before submitting a pull request:

```powershell
cargo +1.97.1 fmt --all -- --check
cargo +1.97.1 test --workspace --all-targets --locked
cargo +1.97.1 clippy --workspace --all-targets --locked -- -D warnings
.\scripts\Test-ReleaseEngineering.ps1
```

Changes to the native shell should also run the release executable smoke test.
Changes to file handling, viewport logic, or packaging should use the relevant
harness described in [tests/qa/README.md](tests/qa/README.md).

## Design constraints

Contributions should preserve these project boundaries:

- source files remain read-only and are never used for scratch storage;
- file-size-dependent structures require explicit, checked bounds;
- UI callbacks do not parse records, perform file I/O, or wait on background
  work;
- worker requests are cancellable and stale document generations fail closed;
- malformed or invalid input is visible rather than silently repaired;
- supported file content remains inert; and
- network access, telemetry, bundled web runtimes, and helper processes are not
  introduced without an explicit architecture decision.

`leanrows-core` forbids unsafe Rust. Unsafe Win32 interop belongs in the native
boundary and must include a concrete safety justification.

## Pull requests

Keep each pull request narrow enough to review and test as one change. Add or
update tests for observable behavior, update public documentation when the
product boundary changes, and distinguish measured results from targets.

Performance or memory claims require a reproducible command, fixture
description, source hash, host description, raw measurement, and comparison
method. A lower internal cache size alone is not proof of lower whole-process
RAM use.

By contributing, the contributor agrees that the submitted work is licensed
under the repository's [MIT License](LICENSE).
