# Architecture validation history

LeanRows began with a focused spike that tested whether a large-file viewer
could preserve row correctness and useful random access without retaining data
in proportion to file size. The spike is complete; its successful decisions
now form the production 0.1 architecture.

## Questions evaluated

The spike concentrated on five risks:

1. maintaining CSV logical-record boundaries across arbitrary read blocks,
   including quoted CR, LF, and CRLF;
2. reconstructing rows from fixed-capacity adaptive checkpoints instead of an
   offset for every row;
3. previewing a giant record without materializing it;
4. cancelling stale work when source evidence changes; and
5. admitting file-size-dependent allocations through explicit operation
   budgets.

Those mechanisms are implemented in `leanrows-core` and integrated into the
Windows document engine. Unit, integration, adversarial, document-smoke, and QA
harnesses now exercise them.

## What the spike did not establish

An architecture spike is not a universal RAM benchmark, release signature,
screen-reader certification, long-duration fuzz campaign, or proof about every
storage device. Those evidence categories remain separate in
[Acceptance and assurance](ACCEPTANCE.md) and
[Validation status](VALIDATION_STATUS.md).

## Resulting decisions

- [ADR 0001: Read-only external-memory pipeline](adr/0001-read-only-external-memory-pipeline.md)
- [ADR 0002: Managed-memory accounting](adr/0002-managed-memory-accounting.md)
- [ADR 0003: Malformed-input policy](adr/0003-malformed-input-policy.md)

The current production structure is documented in [Architecture](ARCHITECTURE.md).
