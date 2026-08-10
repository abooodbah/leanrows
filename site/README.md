# LeanRows project site

This directory is a dependency-free static site intended for GitHub Pages.

## Local preview

Serve `site/` as the web root with any static file server. For example:

```powershell
python -m http.server 8080 --directory site
```

Then open `http://localhost:8080/`.

## Validation

The validator uses only Node.js built-ins:

```powershell
node site/validate.mjs
```

It checks the primary semantic landmarks, the single release CTA, same-page
anchors, local assets, allowed external hosts, visible focus CSS, and the
mobile/tablet media-query contracts. It does not replace browser-based visual
or accessibility testing.

## Release-freeze checklist

- The canonical URL assumes the public project remains
  `abooodbah/leanrows`. Update the canonical and Open Graph URLs if the owner,
  repository name, or Pages domain changes.
- The checksum example must name the tagged workspace version. Update it when
  the workspace version changes.
- The structured-data version and visible release copy must name the same
  tagged workspace version.
- The four numeric values on the page are implementation capacity constants,
  not performance results. Reconcile them with `document_engine.rs` if those
  limits change.
- Runtime measurements must identify the exact executable and stable QA
  receipt. The current local evidence is tied to executable SHA-256
  `F8394AC84BFDD1E4857AF087B509088B5586172048A499E761DF0D5701F8A7A1`
  and `artifacts/qa/v0.1.3-final-frozen-pass/qa-receipt.json`. Do not present
  this pre-commit evidence as tagged or publicly verified.
- The architecture visual is intentionally labeled as an explanatory diagram.
  The separate application figure is a real 1180x720 capture of the frozen
  v0.1.3 executable in Light appearance using the generated 120-row,
  six-field `operations-sample.csv` fixture. The image is 67,555 bytes with
  SHA-256
  `40B5C9B9358FAC0A0882005333876255F551997D8EBB84C8A050F3DE8284C011`.
  It records `Ready | 120 rows | 7600 / 7600 bytes | 100%`; the binary and
  fixture remained unchanged, the saved System preference was restored, and
  the application closed cleanly. Never substitute a fabricated screenshot.
- Current interface copy must retain the v0.1.3 boundaries: a responsive top
  bar, inline search, the owner-data grid, System/Light/Dark appearance, System
  resolution from the active Windows app theme, a flat two-DIP progress rule,
  and the Windows high-contrast system-color fallback.
- Local release-evidence copy may state the 25/25 final-frozen result, but it
  must distinguish that receipt from the tagged workflow's independent release
  commit, rebuild, quality-gate, checksum, and published-asset verification.
