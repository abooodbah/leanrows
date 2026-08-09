# Security policy

## Supported versions

| Version | Security updates |
| --- | --- |
| 0.1.x | Supported |
| Earlier versions | Not supported |

Security fixes are applied to the latest 0.1 patch release and the `main`
branch as appropriate.

## Report a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/abooodbah/leanrows/security/advisories/new)
for vulnerabilities that could affect LeanRows users. Reports should include
the affected version, reproduction steps or a proof of concept, impact, and any
known mitigations.

Do not disclose a suspected vulnerability in a public issue, discussion, or
pull request before coordinated disclosure. The project aims to acknowledge a
complete report within seven days and provide a status update within fourteen
days. These are response targets rather than a service-level agreement.

## Security boundary

LeanRows displays explicitly selected, regular files from a local Windows
volume. File bytes, paths, metadata, record lengths, delimiters, encodings, and
row counts are untrusted.

The 0.1 implementation applies the following controls:

- source handles are opened without write access;
- remote and UNC paths, device paths, reparse points, symbolic links, and
  non-regular files are rejected;
- the retained Windows handle does not share in-place write access, preventing
  bytes from changing under indexed offsets;
- file identity, size, and revision evidence are revalidated while work is in
  progress and stale operations fail closed;
- checked 64-bit offsets and bounded scanners, indexes, caches, fields, and
  previews limit data-dependent allocation;
- literal Find uses bounded input, read, and result pages plus a 64 MiB scratch
  quota; scratch ownership and stale-file recovery are confined to the
  application query-cache directory and coordinated across live processes;
- invalid UTF-8 bytes are escaped as inert text instead of being silently
  replaced;
- content is rendered by native text controls and is never executed as HTML,
  ANSI commands, spreadsheet formulas, scripts, shell input, or URLs; and
- the application has no telemetry, updater, network retrieval, account, or
  cloud synchronization path.

On Windows, the open handle shares deletion so applications that save through
atomic replacement can continue. LeanRows detects that replacement and
requires a controlled reload. Applications that append, truncate, or rewrite
the same file in place may receive a sharing violation until the document is
closed.

## Hostile-file considerations

LeanRows is designed to remain bounded when a file contains extremely long
records or fields, malformed CSV quoting, invalid bytes, or very large row
counts. Bounded previews may omit part of a value, but truncation remains
visible. CSV row boundaries are quote-aware. JSONL and NDJSON are line-oriented
in 0.1 and are not schema-validated or expanded into JSON trees.

Availability still depends on the operating system, storage device, and
available process memory. A managed allocation bound is not a whole-process
memory guarantee because executable pages, thread stacks, controls, allocator
overhead, and the operating-system file cache are outside that ledger.

## Release and supply-chain notes

The Windows x64 0.1 package is not Authenticode-signed. A release includes a
separate SHA-256 file and a manifest containing the digest and size of every
payload. Verify the ZIP against the checksum from the same GitHub release.

The optional installer operates per user without elevation. It registers
LeanRows as an available file handler but does not write the protected Windows
`UserChoice` key or silently replace an existing default application.

## Out of scope for 0.1

LeanRows does not accept remote URLs, UNC paths, archives, plug-ins, macros,
scripts, SQL, active content, or file edits. Security reports for future or
third-party features remain useful, but they may be handled as feature-design
feedback rather than as vulnerabilities in the supported release.

The reproducible security and assurance checks are defined in
[docs/ACCEPTANCE.md](docs/ACCEPTANCE.md).
