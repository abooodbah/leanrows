# Changelog

All notable changes to LeanRows are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-09

### Added

- Native Windows x64 viewer for CSV, TSV, JSONL, NDJSON, log, and text files.
- Progressive first-viewport loading with background indexing.
- Quote-aware CSV and TSV record scanning across fixed-size read blocks.
- Independent native columns for bounded CSV and TSV fields.
- Fixed-capacity adaptive row checkpoints and bounded viewport previews.
- Demand-driven literal Find with next/previous navigation, exact or ASCII-only
  case-insensitive byte matching, bounded result pages, and a 64 MiB scratch
  quota.
- Checked absolute-row navigation, bounded tab-delimited clipboard copy, and
  supported-file drag-and-drop.
- Read-only snapshot handles with source identity and revision checks.
- Visible escaping of invalid UTF-8 bytes and explicit preview truncation.
- Keyboard navigation, native accessibility names, system colors, and
  per-monitor DPI support.
- Deterministic unsigned portable ZIP, package manifest, SHA-256 checksum, and
  optional per-user installer with Start menu and Installed Apps registration.
- Automated core, adversarial-input, native-shell, document, packaging, and QA
  harnesses.

### Security

- Remote and UNC paths, device paths, reparse points, symbolic links, and
  non-regular files are rejected.
- In-place source writes are not shared while a snapshot is open; atomic
  replacement is detected before stale work can continue.
- File content is displayed as inert native text and no network or telemetry
  path is included.

[0.1.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.0
