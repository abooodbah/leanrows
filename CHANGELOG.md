# Changelog

All notable changes to LeanRows are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-08-09

### Added

- Added a responsive native top bar that keeps file identity, inline literal
  search, and primary document actions available across narrow, default, and
  wide window layouts.
- Added System and Light appearance choices, with a system-color fallback when
  Windows high contrast is active.

### Changed

- Reworked the Windows shell around warm, opaque surfaces, restrained rules,
  deliberate spacing, and one blue action accent to align with the LeanMark
  visual family.
- Replaced the modal Find entry point with an inline search field, Match case
  toggle, and previous/next controls while retaining `F3` and `Shift+F3`
  navigation.
- Preserved the owner-data grid and bounded worker/cache model beneath the new
  shell, so the visual redesign does not introduce a whole-file UI model.

## [0.1.1] - 2026-08-09

### Fixed

- Fixed the per-user installer failure that occurred when PowerShell bound the
  empty name required for a registry default value.
- Fixed registry-key creation so repeated writes preserve earlier sibling
  values, and opened direct-default keys writable for exact uninstall
  restoration or removal.

### Changed

- The installer now records and sets a direct per-user default for supported
  extensions that have no protected Explorer `UserChoice`, while continuing to
  register all supported extensions in **Open with**.
- Reinstallation preserves the original recorded handler, and uninstallation
  restores or removes only direct defaults that still point to LeanRows.
- Protected `UserChoice` selections, later user changes, and `.txt`
  associations remain untouched.

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

[0.1.2]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.2
[0.1.1]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.1
[0.1.0]: https://github.com/abooodbah/leanrows/releases/tag/v0.1.0
