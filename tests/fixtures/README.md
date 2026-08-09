# LeanRows fixtures

Large binary/text fixtures are generated locally rather than committed. See [`../../tools/README.md`](../../tools/README.md) and run:

```powershell
& ./tools/New-LeanRowsFixtures.ps1 -OutputDirectory ./tests/fixtures/generated
```

The generated `manifest.json` is the source of truth for fixture sizes, hashes, record counts, and adversarial features. The generator is deterministic for a fixed parameter set and seed.
