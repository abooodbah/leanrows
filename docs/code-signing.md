# Code signing with SignPath Foundation

## Status: signing is off

LeanRows applied to [SignPath Foundation](https://signpath.org/) for free code
signing. The application was declined in October 2026, so signing is off.

- The Windows ZIP on GitHub Releases is unsigned and keeps the `-unsigned`
  suffix. Windows SmartScreen may show a warning for it.
- The Microsoft Store build is signed by Microsoft when the Store publishes it.

The SignPath steps in [.github/workflows/windows.yml](../.github/workflows/windows.yml)
and [.signpath/artifact-configuration.xml](../.signpath/artifact-configuration.xml)
are kept, but they do nothing: they run only when the repository variable
`SIGNPATH_ORGANIZATION_ID` is set, and it is not set. The README and the product
site no longer carry a code signing policy or a SignPath credit; both keep the
privacy policy and the uninstallation notes. If a later application is
accepted, the policy section has to be added back before applying, and the
steps below apply.

## Before applying

1. Turn on two-factor authentication for the `abooodbah` GitHub account
   (GitHub Settings, Password and authentication). SignPath Foundation requires
   it for everyone with commit or approval rights.
2. Check that the latest release is public, built by GitHub Actions, and that
   the README and the product site show the "Code signing policy" section.

## Apply

3. Apply at <https://signpath.org/apply> with the repository
   <https://github.com/abooodbah/leanrows>. Use the MIT license and describe
   LeanRows as a read-only CSV and table viewer for Windows.

## After SignPath accepts the project

4. In the SignPath web app, create a project with the slug `leanrows` and link it
   to the GitHub repository as a trusted build system.
5. In that project, create a signing policy with the slug `release-signing` and
   the SignPath Foundation certificate. Turn on manual approval and make
   `abooodbah` the only approver.
6. Open the project's artifact configuration, paste the contents of
   `.signpath/artifact-configuration.xml`, and save it as the default
   configuration. If the file changes later, paste it again.
7. Create an API token for a CI user that may submit signing requests to the
   `leanrows` project.
8. In the GitHub repository settings (Secrets and variables, Actions):
   - add the repository variable `SIGNPATH_ORGANIZATION_ID` with the
     organization ID shown in SignPath
   - add the repository secret `SIGNPATH_API_TOKEN` with the token from step 7

## What each release does after that

When a `v*` tag is pushed, the Windows workflow builds and tests `leanrows.exe`
as before. Because `SIGNPATH_ORGANIZATION_ID` is set, it then uploads the
executable, sends a signing request to SignPath, and waits up to one hour.

9. Approve the request in SignPath (you get an email). Each release needs this
   approval; nothing is signed without it.

After the approval the workflow checks that the signature is valid and that the
executable reports `LeanRows` and the release version, then packages it as
`LeanRows-vX.Y.Z-windows-x64.zip` (no `-unsigned` suffix, no `UNSIGNED.txt`, and
`"signing_status": "signed"` in `manifest.json`). The GitHub release gets that
ZIP and its `.sha256` file.

If the request is denied or not approved within the hour, the workflow fails and
no release is published. Rerun the failed workflow run to send a new request.

Pull requests and pushes to `main` are never signed. They keep producing the
unsigned ZIP, so CI works the same without SignPath.

## Things to update when signing goes live

The first signed release still ships text written for unsigned builds. Before
tagging it, update these to match:

- the "Download" section in README.md and the "Before installing" band on the
  product site, which name the `-unsigned` ZIP and describe the SmartScreen
  warning
- the release notes file for that version in `docs/releases/`
- `installer/README.md`, which describes the unsigned package layout

To turn signing off again, delete the `SIGNPATH_ORGANIZATION_ID` variable. The
workflow then goes back to the unsigned package.
