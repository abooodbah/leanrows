# ADR 0002: Managed-memory accounting

- **Status:** Accepted
- **Date:** 2026-08-09

## Context

“RAM usage” is not one measurement. A cache limit does not include executable
pages, Windows controls, thread stacks, allocator overhead, the operating-system
file cache, or a helper process. Reporting a managed capacity as total
application RAM would therefore be misleading.

## Decision

Every file-size-dependent core operation declares and validates the capacities
it needs before allocating. The plan covers read buffers, checkpoint indexes,
viewport payload, scratch buffers, projected fields, decoded rows, previews,
and query pages. Reservations use checked arithmetic and fallible allocation;
an operation that cannot fit returns a structured error or a bounded partial
result.

The native document engine selects fixed 0.1 limits for these structures rather
than presenting unimplemented Small, Balanced, or Large profiles. These limits
include a 4,096-entry checkpoint index, 128-row viewport, 4 KiB raw record
preview, 64 projected delimited fields, and bounded per-field and display-cell
decoding.

Whole-process evidence is collected separately for the complete discovered
process tree. Release receipts distinguish at least private bytes and working
set and preserve missing observations as missing rather than zero.

## Consequences

- File-size-dependent allocations outside an explicit plan are architecture
  defects.
- Very wide or long values are truncated for display under documented bounds.
- Allocation failure remains an ordinary error path that must not panic or
  bypass a limit.
- Total private bytes can exceed managed data capacity without contradicting
  the internal ledger.
- Public RAM comparisons require a reproducible process-tree measurement, named
  hardware, and a defined workload.

## Validation

`CORE-01`, `FORMAT-01`, and `MEMORY-01` in
[Acceptance and assurance](../ACCEPTANCE.md) qualify the 0.1 release. Full scans
of larger non-sparse corpora and comparative tool benchmarks remain extended
assurance.
