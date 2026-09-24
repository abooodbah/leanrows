# Windows x64 release contract

LeanRows v0.1 uses a portable, explicitly unsigned ZIP. Packaging creates:

```text
LeanRows-v0.2.0-windows-x64-unsigned.zip
LeanRows-v0.2.0-windows-x64-unsigned.sha256
```

The version is read from `[workspace.package]` in `Cargo.toml`; a mismatched
version or tag fails the build. ZIP entries have a fixed timestamp, fixed
ordering, UTF-8 names, and deterministic content. CI packages the same binary
twice and requires identical SHA-256 digests.

## Portable package contents

```text
LeanRows-v0.2.0-windows-x64-unsigned/
  leanrows.exe
  install.ps1
  uninstall.ps1
  LICENSE.txt
  UNSIGNED.txt
  manifest.json
```

`manifest.json` records the version, unsigned status, exact association set,
and SHA-256 and size of every payload file. `install.ps1 -ValidateOnly`
verifies this contract without writing files or registry entries.

## Per-user installation

From an extracted package:

```powershell
.\install.ps1
```

The application is copied to `%LOCALAPPDATA%\Programs\LeanRows`. Installation
requires no elevation. LeanRows is added to the current user's Start Menu and
Installed Apps list. The optional command below opens Windows Default Apps
after registration:

```powershell
.\install.ps1 -OpenDefaultApps
```

The installer always registers LeanRows as an available **Open with** handler
for `.csv`, `.tsv`, `.jsonl`, `.ndjson`, and `.log`. It does not claim `.txt`
and never writes or deletes the protected Explorer `UserChoice` key.

For each supported extension without a protected `UserChoice`, installation
records whether a direct per-user default already existed under
`HKCU\Software\Classes`, saves its value in the LeanRows-owned
`HKCU\Software\LeanRows\InstallState` key when present, and makes
`LeanRows.AssocFile.v1` the direct per-user default. When a protected choice
exists, installation preserves it and does not write a direct default. Use
`-OpenDefaultApps` to open Windows Default Apps for manual confirmation.

Reinstallation is idempotent: it does not replace the saved original handler
with LeanRows itself.

## Uninstallation

Run the installed script:

```powershell
& "$env:LOCALAPPDATA\Programs\LeanRows\uninstall.ps1"
```

Before removing registration, uninstallation inspects each direct per-user
default. If it still points to LeanRows, the uninstaller restores the recorded
prior value or removes the value when none existed. If the user selected
another direct handler after installation, that newer choice is preserved.
Owned install state is then removed. The uninstaller never writes or deletes
`UserChoice`.

Uninstallation also removes only LeanRows-owned
ProgID/application/capability and Installed Apps keys, only LeanRows values
from shared extension and `RegisteredApplications` keys, the exact current-user
Start Menu shortcut, and only allow-listed files found in the installed
manifest. It never recursively deletes the installation directory; the
directory is removed only when empty.

## GitHub workflow

`.github/workflows/windows.yml` runs on pull requests, `main`, manual dispatch,
and `v*` tags. It installs Rust 1.97.1, fetches locked dependencies, checks
formatting, runs all workspace tests and strict Clippy, builds the release app,
executes the native-control smoke test, validates deterministic packaging, and
uploads the ZIP and checksum.

Only an existing matching `v<workspace-version>` tag can enter the GitHub
Release job. The workflow does not create or push tags. The release text and
artifact name state that the build is unsigned.
