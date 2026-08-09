# Architecture decision records

These records define the constraints adopted by the LeanRows 0.1
implementation:

- [0001 - Read-only external-memory pipeline](0001-read-only-external-memory-pipeline.md)
- [0002 - Managed-memory accounting](0002-managed-memory-accounting.md)
- [0003 - Malformed-input policy](0003-malformed-input-policy.md)

`Accepted` means that the decision governs the current implementation. It does
not turn an unrun benchmark or extended assurance campaign into passing
evidence. Runtime qualification remains in
[Acceptance and assurance](../ACCEPTANCE.md).

New decisions should use the same context, decision, consequences, and
validation structure. An accepted decision is superseded by a new record rather
than silently rewritten when the architectural constraint changes.
