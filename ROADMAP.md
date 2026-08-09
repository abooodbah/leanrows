# Roadmap

LeanRows follows a narrow rule: features should make large local row files
easier to inspect without turning the application into a spreadsheet, database,
or general-purpose editor. Priorities are guided by reproducible user problems
and evidence from memory-constrained systems.

This roadmap describes direction, not release commitments or dates.

## 0.1 foundation

- Native Windows x64 interface.
- Progressive, read-only viewing for CSV, TSV, JSONL, NDJSON, log, and text
  files.
- Bounded scanners, adaptive indexes, viewport caches, and invalid-byte
  representation.
- Literal find-next/find-previous, absolute-row navigation, bounded clipboard
  copy, and native file drag-and-drop.
- Snapshot integrity checks, accessibility foundations, deterministic
  packaging, and release QA.

## Near-term candidates

- Provide explicit format and encoding overrides without silent detection.
- Improve CSV column naming and schema inspection while preserving bounded
  field projection.
- Add native automation coverage for find, go-to-row, and clipboard workflows
  without weakening their UI-thread and memory boundaries.
- Publish repeatable cross-tool memory and first-viewport benchmarks on named
  hardware.

## Extended assurance

- Longer coverage-guided fuzzing campaigns and a maintained regression corpus.
- Larger real, non-sparse fixture runs where storage capacity permits.
- Broader accessibility testing with assistive technologies.
- Reproducible network-observation and process-tree resource traces for each
  release family.

## Deliberate non-goals

Editing, spreadsheet formulas, arbitrary JSON trees, SQL, charts, pivots,
global in-memory sort, remote retrieval, plug-ins, scripts, telemetry, and AI
features are not planned for the 0.1 product line. Cross-platform ports require
an independent native UI and release-quality evidence; they will not be claimed
from core-library portability alone.
