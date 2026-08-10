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
- Do not add memory, startup, file-size, or seek measurements until the final
  packaged binary has a stable QA receipt that records its exact hash.
- The architecture visual is intentionally labeled as an explanatory diagram.
  The separate application figure is a real 1180x720 capture of the frozen
  release binary using only a generated synthetic six-column fixture; replace
  it only with another equivalently verified capture, never a fabricated
  application screenshot.
- Current interface copy must retain the verified v0.1.2 boundaries: warm
  opaque surfaces, a responsive top bar, inline search, the owner-data grid,
  System and Light appearance choices, and the Windows high-contrast
  system-color fallback. Do not imply a native dark theme.
