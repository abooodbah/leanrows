# ADR 0003: Malformed-input policy

- **Status:** Accepted
- **Date:** 2026-08-09

## Context

Silent repair can merge rows, hide bytes, change identifiers, or make a viewer
disagree with the process that produced the file. Hostile or accidental input
can also contain huge fields, invalid UTF-8, an unterminated CSV quote, or a
single record large enough to exhaust memory if it is materialized.

## Decision

LeanRows preserves trustworthy record boundaries, uses bounded previews, and
makes loss of display information visible.

- A CSV or TSV quote opens only at field start; doubled quotes escape a quote.
- CR, LF, and CRLF delimit records only outside quoted fields.
- EOF inside a quoted field produces the available bounded final record and an
  unterminated-quote diagnostic; no closing quote is invented.
- Oversized records and fields continue scanning without being allocated as one
  value. Cache metadata records source and display truncation.
- Invalid UTF-8 bytes are shown as `\xNN` escapes, preserving their byte value
  in an inert representation rather than silently substituting a character.
- JSONL and NDJSON remain physical-line views in 0.1. LeanRows does not claim
  that each line is valid JSON or attempt recovery as a JSON document.

The UI must not execute displayed content or infer an action from a formula,
ANSI escape, HTML fragment, path, or URL.

## Consequences

- A preview can be incomplete even though the scanner continues to the true
  record boundary.
- Invalid bytes expand in the display because each escaped byte uses four ASCII
  characters.
- Some malformed CSV tails cannot support a later trustworthy structured row;
  the diagnostic boundary is preferable to invented data.
- JSON validation, schema inference, and raw whole-record export are not part of
  the 0.1 viewer.
- Copy, search, and future detail views must preserve the same byte and boundary
  rules rather than adding a second parser.

## Validation

`CORE-01`, `DOC-01`, and `FORMAT-01` in
[Acceptance and assurance](../ACCEPTANCE.md) are required for the 0.1 tag. The
24 CPU-hour fuzzing campaign remains extended assurance and is not described as
completed release evidence.
