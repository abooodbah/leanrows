# ADR 0001: Read-only external-memory pipeline

- **Status:** Accepted
- **Date:** 2026-08-09

## Context

Loading or decoding a complete source makes memory grow with file size. CSV
random access also cannot rely on physical newline offsets because quoted fields
can contain CR, LF, or CRLF. A whole-file memory map would weaken live-change
safety and make resident-memory results harder to interpret.

## Decision

LeanRows keeps the local regular file as the primary backing store and retains a
read-only snapshot handle. Positioned fixed-size reads feed one incremental
boundary scanner. Fixed-capacity adaptive checkpoints store sparse row and byte
positions; when capacity is reached, the stride grows and entries are thinned.

Viewport reconstruction starts at a trustworthy checkpoint, scans forward, and
retains only bounded rows and previews. The background worker coalesces obsolete
requests and carries a document generation so stale work can be cancelled.

On Windows, the handle shares reads and deletion but not writes. This blocks
in-place source mutation while allowing atomic replacement. Repeated handle and
path evidence checks stop work when the opened snapshot is no longer
trustworthy.

The core literal-query engine follows the same positioned, cancellable design.
Its result pages and application-owned scratch storage have explicit capacity
and quota contracts. The native Find workflow exposes exact and ASCII-only
case-insensitive literal matching through demand-driven next/previous commands.
Query scratch is quota-limited, stored outside the source directory, and
coordinated across concurrent LeanRows processes.

## Consequences

- Data-dependent index and viewport storage does not grow directly with source
  size.
- A larger checkpoint stride can require more rescanning for a distant row.
- CSV boundary logic is shared critical infrastructure.
- An application that writes a file in place may receive a Windows sharing
  violation while LeanRows holds the snapshot.
- Atomic replacement invalidates the active path and requires reload.
- Editing, saving, global sorting, and source-side scratch files remain outside
  this architecture.

## Validation

The release requirements are `CORE-01`, `DOC-01`, `SOURCE-01`, `MEMORY-01`, and
`SCALE-01` in [Acceptance and assurance](../ACCEPTANCE.md). Larger non-sparse
corpora and the 100,000-seek oracle are extended campaigns, not assumed passes.
