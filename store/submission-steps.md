# LeanRows: Microsoft Store submission steps

These are the steps that need the owner's own account. Everything else (the
MSIX package, the listing text, the age rating answers, and the screenshots)
is in this folder.

What is here:

- `listing.md`: every listing field, ready to paste.
- `age-rating.md`: answers for the IARC age rating questionnaire.
- `screenshots/`: Store screenshots at 1920x1080.
- `msix/`: the package manifest template, logos, and the scripts the
  "Microsoft Store package" workflow runs.

## 1. Developer account (once, for all three apps)

1. Open https://storedeveloper.microsoft.com and choose to sign up as an
   individual developer. Sign in with the Microsoft account you want to own
   the apps. Individual accounts are free.
2. Complete identity verification (photo ID and a selfie).
3. Publisher display name: **Lean Apps Studio**. Skip this if the account already
   exists with that name.

## 2. Reserve the name

1. In Partner Center, open **Apps and games** > **New product** >
   **MSIX or PWA app**.
2. Enter **LeanRows** and select **Reserve product name**.

## 3. Copy the product identity into GitHub

1. In Partner Center, open LeanRows > **Product management** >
   **Product identity**.
2. In GitHub, open https://github.com/abooodbah/leanrows/settings/variables/actions and create three
   repository variables (Variables tab, not Secrets):

   | Variable | Value from Product identity |
   | --- | --- |
   | `MSIX_IDENTITY_NAME` | Package/Identity/Name |
   | `MSIX_PUBLISHER` | Package/Identity/Publisher (starts with `CN=`) |
   | `MSIX_PUBLISHER_DISPLAY_NAME` | Package/Properties/PublisherDisplayName (Lean Apps Studio) |

   Copy them exactly. If they do not match, Partner Center rejects the
   upload with an identity error.

## 4. Build the package

1. Open https://github.com/abooodbah/leanrows/actions/workflows/store-msix.yml and select **Run workflow**.
   Leave the tag empty to package the current release (v0.3.0), or
   enter another release tag such as `v0.3.0`.
2. When the run finishes, check that its summary has no warning about
   placeholder identity values. If it has one, the variables in step 3 are
   missing or misspelled.
3. Download the artifact `LeanRows-v0.3.0-store-msix` and unzip it.
   Upload `LeanRows-v0.3.0-x64.msix`. Do not upload anything from a
   `test-signing` folder; the workflow does not publish those.

Later releases: pushing a `v*` tag runs the same workflow, which waits for the
GitHub release and then builds the package. Each Store update is a new
submission with the new `.msix`.

## 5. Create the submission

Open LeanRows > **Start your submission** and fill in each section:

1. **Pricing and availability**: Markets: all markets. Visibility: public.
   Pricing: **Free**. No free trial.
2. **Properties**: Category **Productivity**, secondary category **Utilities &
   tools**. Privacy policy URL: https://github.com/abooodbah/leanrows#privacy-policy. Website: https://abooodbah.github.io/leanrows/.
   Support contact: https://github.com/abooodbah/leanrows/issues. "This product accesses, collects, or
   transmits personal information": No. System requirements: see
   `listing.md`.
3. **Age ratings**: complete the questionnaire with the answers in
   `age-rating.md`.
4. **Packages**: upload the `.msix` from step 4. Leave the device family as
   Windows 10/11 Desktop.
5. **Store listings** > **English (United States)**: paste the description,
   short description, features, search terms, what's new, copyright line, and
   short title from `listing.md`. Upload the screenshots from `screenshots/`
   in their numbered order, with the captions from `listing.md`.
6. **Submission options** > Restricted capabilities: the package declares
   `runFullTrust`. Paste this justification:

   > LeanRows is a desktop (Win32) application packaged with MSIX. runFullTrust
   > is required for any packaged desktop application. The app only reads
   > files the user opens and does not change system settings.

7. Select **Submit to the Store**. Certification usually takes up to three
   business days. Partner Center emails the result.

## Notes

- The Store signs the package. The repository never holds a signing
  certificate for it, and the workflow's test-signed copy uses a throwaway
  certificate made for that run only.
- The Store version installs next to a ZIP or winget installation without
  conflict. They are separate copies; Windows lets the user pick which one
  opens each file type.
- Settings live under `HKCU\Software\LeanRows` and app data under
  `%LOCALAPPDATA%`. In the packaged app Windows keeps these in the package's
  own storage and removes them on uninstall. Nothing is written to the install
  folder.
