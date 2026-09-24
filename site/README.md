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
  `D0D24FCA9730BE6C7A0E1385AAB5CB6F08C8D495A595FC32E4066F583BC88A8D`
  and `artifacts/qa/v0.2.0-final-frozen-pass/qa-receipt.json`. Do not present
  this pre-commit evidence as tagged or publicly verified.
- The architecture visual is intentionally labeled as an explanatory diagram.
  The separate application figure is a real 1180x720 capture of the frozen
  v0.2.0 executable in Light appearance using the generated 120-row,
  six-field `operations-sample.csv` fixture, with a second synthetic file open
  in another tab. The image is 63,395 bytes with SHA-256
  `90DE75F43C8DD66AC4AD11A1EF10749CB78BCF5DFF45CBBDE6B96DF72A582650`.
  It records `Ready | 120 rows | 7600 / 7600 bytes | 100%`; the binary and
  fixtures remained unchanged, the saved Dark preference was restored, and
  the application closed cleanly. Never substitute a fabricated screenshot.
- Current interface copy must retain the v0.2.0 boundaries: tabs in one window, a responsive top
  bar, inline search, the owner-data grid, System/Light/Dark appearance, System
  resolution from the active Windows app theme, a flat two-DIP progress rule,
  and the Windows high-contrast system-color fallback.
- Companion-project copy must preserve the separate purposes of LeanRows and
  LeanMark, their shared read-only boundary, and the link to
  https://abooodbah.github.io/leanmark/.
- Local release-evidence copy may state the 25/25 final-frozen result, but it
  must distinguish that receipt from the tagged workflow's independent release
  commit, rebuild, quality-gate, checksum, and published-asset verification.
