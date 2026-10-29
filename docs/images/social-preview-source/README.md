# Social preview source

`social-preview.html` is the source of `site/assets/social-preview.png`, the
1280x640 image that link previews and the GitHub social preview use. It
follows the layout of the LeanDoc and LeanMark previews.

The product image is the real capture in `docs/images/leanrows-20m-rows.png`
(20,000,001 rows, 1,532,454,643 bytes, fully indexed). The numbers in the text
come from the README.

## Render

The page uses IBM Plex Sans 500 and 600 and IBM Plex Serif 600 (SIL Open Font
License 1.1). The font files are not kept in this repository. Copy
`ibm-plex-sans-latin-500-normal.woff2`, `ibm-plex-sans-latin-600-normal.woff2`,
and `ibm-plex-serif-latin-600-normal.woff2` from the `site/assets/fonts/`
folder of LeanDoc or LeanMark into a `fonts/` folder here, then render with
Microsoft Edge:

```powershell
& "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" --headless=new --disable-gpu `
  --allow-file-access-from-files --hide-scrollbars --force-device-scale-factor=1 `
  --window-size=1280,640 --virtual-time-budget=3000 `
  --screenshot="$PWD\site\assets\social-preview.png" `
  "file:///$($PWD -replace '\\','/')/docs/images/social-preview-source/social-preview.html"
```

Run it from the repository root, then delete the `fonts/` folder.
