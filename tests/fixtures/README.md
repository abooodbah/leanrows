# LeanRows fixtures

Large binary/text fixtures are generated locally rather than committed. See [`../../tools/README.md`](../../tools/README.md) and run:

```powershell
& ./tools/New-LeanRowsFixtures.ps1 -OutputDirectory ./tests/fixtures/generated
```

The generated `manifest.json` is the source of truth for fixture sizes, hashes, record counts, and adversarial features. The generator is deterministic for a fixed parameter set and seed.

The `.xlsx` fixtures in [`xlsx/`](xlsx/README.md) are committed, including one
4 MB workbook with 120,001 rows. They come from real producers and are listed
there with their producer, version and SHA-256.
