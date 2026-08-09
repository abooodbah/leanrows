# LeanRows engine-spike tooling

These dependency-free PowerShell scripts create deterministic hostile/large-file fixtures and record raw observations from a supplied LeanRows spike executable. They make no performance claims.

## Requirements

- Windows PowerShell 5.1 or PowerShell 7+.
- Enough free disk space for the requested fixtures and any spike output.
- The benchmarked executable must provide a non-interactive mode that opens/scans one file and exits.

The benchmark harness discovers complete live process trees through `Win32_Process` on Windows, `/proc` on Linux, and `/bin/ps` on macOS. Sampling can miss processes that start and exit entirely between two samples; the interval and sample count are recorded in every report.

Sampling overhead is not subtracted. The configured interval is a requested minimum delay; process discovery can make the effective cadence longer, and `-IncludeSamples` records the actual elapsed time of every sample.

## Parse-check the scripts

From the repository root:

```powershell
& ./tools/Test-PowerShellScripts.ps1
```

This invokes PowerShell's AST parser for every `tools/*.ps1` file without executing the fixture or benchmark workloads.

On Windows, exercise the root-first sampler with deterministic 75 ms child processes:

```powershell
& ./tools/Test-Measure-LeanRowsSpike.ps1
```

The test requires every run to either contain positive memory observations or explicitly report `missing` with null peaks. A silent zero peak fails the test.

## Generate fixtures

Small smoke fixtures:

```powershell
& ./tools/New-LeanRowsFixtures.ps1 `
  -OutputDirectory ./tests/fixtures/generated `
  -CsvRows 1000 `
  -JsonlRows 1000 `
  -LogRows 1000 `
  -GiantRecordPayloadBytes 1048576
```

At-least-size fixtures suitable for an engine spike:

```powershell
& ./tools/New-LeanRowsFixtures.ps1 `
  -OutputDirectory D:/leanrows-fixtures `
  -CsvRows 100000 -CsvMinimumBytes 1073741824 -CsvPayloadBytes 64 `
  -JsonlRows 100000 -JsonlMinimumBytes 1073741824 -JsonlPayloadBytes 64 `
  -LogRows 100000 -LogMinimumBytes 1073741824 -LogPayloadBytes 64 `
  -GiantRecordPayloadBytes 1073741824
```

`*Rows` and `*MinimumBytes` are both lower bounds. Generation continues until both are satisfied, so a byte target may be exceeded by one complete logical record. Set a minimum-byte value to zero when an exact minimum row count is the only requirement.

The generator creates:

- `large-multiline.csv`: quoted commas, escaped quotes, and quoted CRLF records.
- `large.jsonl`: deterministic, valid UTF-8 JSON Lines.
- `large.log`: deterministic LF-terminated log rows.
- `malformed.csv`: width mismatch, stray quote, valid multiline row, and an unterminated final quote.
- `giant-record.csv`: one giant quoted logical record emitted in 64 KiB pieces.
- `manifest.json`: requested parameters, actual sizes, logical record counts, features, and SHA-256 hashes.
- `manifest.sha256`: SHA-256 of the manifest itself.

Generation uses UTF-8 without a BOM and computes each fixture hash as bytes are streamed. It never holds a whole fixture or the giant record in memory. Existing generated files are rejected unless `-Force` is supplied; `-Force` replaces only the seven named outputs above.

## Measure an engine spike

The argument template must contain `{fixture}`. The harness substitutes an absolute, quoted fixture path and starts the executable directly without a command shell.

```powershell
& ./tools/Measure-LeanRowsSpike.ps1 `
  -Executable ./target/release/leanrows-spike.exe `
  -FixturePath ./tests/fixtures/generated/large-multiline.csv, `
               ./tests/fixtures/generated/large.jsonl `
  -ArgumentTemplate '--scan {fixture}' `
  -Runs 3 `
  -SampleIntervalMilliseconds 100 `
  -TimeoutSeconds 300 `
  -OutputPath ./benchmark-results/spike.json
```

Use `-IncludeSamples` to retain every time-series sample; otherwise only counts and peaks are emitted. Each observation includes:

- executable and fixture SHA-256 hashes;
- exact fixture bytes and invoked arguments;
- exit code, timeout status, target process lifetime, and separate harness observation time;
- peak sum of live process-tree private bytes;
- peak sum of live process-tree working set;
- `observed` or `missing` memory status and the number of valid memory samples;
- peak discovered process count and sample count;
- fixture bytes divided by target process lifetime.

The last value is deliberately named `input_bytes_divided_by_process_elapsed_bps`: a generic harness cannot prove that a supplied executable read every byte. `process_elapsed_seconds` is derived from the operating system's process start and exit timestamps; `harness_observation_seconds` includes sampling and process-tree discovery overhead. Treat the ratio as a raw calculation, not as a throughput claim. Compare results only under a separately documented protocol controlling hardware, power mode, storage state, cache state, build profile, fixture hashes, arguments, and run order.

The launched root process is sampled directly before the slower descendant-discovery pass. Previously discovered descendants are also sampled first, and an attempted-PID set prevents double counting. If the entire tree exits before any positive sample, both peak fields are JSON `null` and `memory_observation_status` is `missing`; missing data is never encoded as a zero-memory result.

The harness does not capture target stdout/stderr, avoiding an unbounded in-memory output buffer. Output remains attached to the calling console. A timed-out run causes the discovered process tree to be terminated.
