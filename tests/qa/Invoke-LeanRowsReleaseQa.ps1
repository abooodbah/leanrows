[CmdletBinding()]
param(
    [string]$BinaryPath = './target/release/leanrows.exe',
    [string]$SpikeBinaryPath = './target/release/leanrows-spike.exe',
    [string]$EvidenceDirectory,
    [ValidateRange(1, 1000)][int]$RandomSeekRuns = 24,
    [ValidateRange(5, 300)][int]$DocumentSmokeTimeoutSeconds = 30,
    [ValidateRange(5, 120)][int]$NetworkObservationTimeoutSeconds = 30,
    [switch]$SkipSparseScaleFixtures
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$checks = New-Object 'System.Collections.Generic.List[object]'
$hashes = [ordered]@{}
$harnessWatch = [System.Diagnostics.Stopwatch]::StartNew()
$fixtureRoot = $null
$cleanupAttempted = $false
$cleanupRemoved = $false

function Add-Check {
    param(
        [Parameter(Mandatory = $true)][string]$Id,
        [Parameter(Mandatory = $true)][string]$Status,
        [Parameter(Mandatory = $true)][string]$Summary,
        $Details
    )
    $script:checks.Add([ordered]@{
        id = $Id
        status = $Status
        summary = $Summary
        details = $Details
    })
}

function Get-TextSha256 {
    param([AllowEmptyString()][string]$Text)
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $script:Utf8NoBom.GetBytes($Text)
        [System.BitConverter]::ToString($algorithm.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Resolve-RepositoryPath {
    param([Parameter(Mandatory = $true)][string]$Value)
    if ([System.IO.Path]::IsPathRooted($Value)) {
        return [System.IO.Path]::GetFullPath($Value)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $Value))
}

function Test-QaScriptParsing {
    $failures = New-Object 'System.Collections.Generic.List[string]'
    $scripts = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File)
    foreach ($script in $scripts) {
        $tokens = $null
        $errors = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile(
            $script.FullName,
            [ref]$tokens,
            [ref]$errors
        )
        foreach ($parseError in @($errors)) {
            $failures.Add("$($script.Name): $($parseError.Message)")
        }
    }
    if ($failures.Count -gt 0) {
        throw ($failures -join '; ')
    }
    return $scripts.Count
}

function New-MarkdownReceipt {
    param([Parameter(Mandatory = $true)]$Receipt)
    $builder = New-Object System.Text.StringBuilder
    $null = $builder.AppendLine('# LeanRows local v0.1 QA receipt')
    $null = $builder.AppendLine()
    $null = $builder.AppendLine("Verdict: **$($Receipt.verdict)**")
    $null = $builder.AppendLine()
    $null = $builder.AppendLine($Receipt.scope_disclosure)
    $null = $builder.AppendLine()
    $null = $builder.AppendLine('| Check | Status | Summary |')
    $null = $builder.AppendLine('| --- | --- | --- |')
    foreach ($check in $Receipt.checks) {
        $summary = ([string]$check.summary).Replace('|', '/')
        $null = $builder.AppendLine("| $($check.id) | $($check.status) | $summary |")
    }
    $null = $builder.AppendLine()
    $null = $builder.AppendLine('## Explicitly not demonstrated')
    $null = $builder.AppendLine()
    foreach ($item in $Receipt.not_demonstrated) {
        $null = $builder.AppendLine("- $item")
    }
    $null = $builder.AppendLine()
    $null = $builder.AppendLine('## Cleanup')
    $null = $builder.AppendLine()
    $null = $builder.AppendLine(
        "Fixture cleanup attempted: $($Receipt.cleanup.attempted); removed: $($Receipt.cleanup.fixture_root_removed)."
    )
    return $builder.ToString()
}

if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
    $EvidenceDirectory = Join-Path $repositoryRoot "artifacts/qa/$stamp"
}
$evidenceRoot = [System.IO.Path]::GetFullPath($EvidenceDirectory)
$null = [System.IO.Directory]::CreateDirectory($evidenceRoot)
$binary = Resolve-RepositoryPath -Value $BinaryPath
$spikeBinary = Resolve-RepositoryPath -Value $SpikeBinaryPath
$fixtureRoot = [System.IO.Path]::GetFullPath(
    (Join-Path ([System.IO.Path]::GetTempPath()) ('leanrows-qa-' + [guid]::NewGuid().ToString('N')))
)
$null = [System.IO.Directory]::CreateDirectory($fixtureRoot)
$fixtureSentinel = Join-Path $fixtureRoot '.leanrows_qa_fixture_root'
[System.IO.File]::WriteAllText(
    $fixtureSentinel,
    ('Disposable synthetic LeanRows QA fixture root.' + [Environment]::NewLine),
    $script:Utf8NoBom
)
$supportToolSentinel = Join-Path $fixtureRoot '.support_tool_fixture_root'
[System.IO.File]::WriteAllText(
    $supportToolSentinel,
    ('synthetic_self_test' + [Environment]::NewLine),
    $script:Utf8NoBom
)

$smallManifest = $null
$smallManifestPath = $null
$smallFixtureDirectory = Join-Path $fixtureRoot 'small'
try {
    try {
        $parsedCount = Test-QaScriptParsing
        Add-Check -Id 'QA-PARSE' -Status 'pass' -Summary 'All isolated QA scripts parse.' -Details ([ordered]@{
            parsed_scripts = $parsedCount
        })
    }
    catch {
        Add-Check -Id 'QA-PARSE' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
    }

    try {
        $toolOutput = & (Join-Path $repositoryRoot 'tools/Test-PowerShellScripts.ps1') | Out-String
        Add-Check -Id 'TOOLS-PARSE' -Status 'pass' -Summary 'Existing fixture and measurement tools parse.' -Details ([ordered]@{
            output_sha256 = Get-TextSha256 -Text $toolOutput
        })
    }
    catch {
        Add-Check -Id 'TOOLS-PARSE' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
    }

    try {
        $memorySelfTest = & (Join-Path $repositoryRoot 'tools/Test-Measure-LeanRowsSpike.ps1') -Runs 3
        if ([int]$memorySelfTest.SilentZeroPeaks -ne 0 -or
            [string]$memorySelfTest.ProcessLifetimeAccounting -ne 'PASS') {
            throw 'The existing process sampler self-test did not preserve explicit missing observations.'
        }
        Add-Check -Id 'MEMORY-MISSING-STATE' -Status 'pass' -Summary 'Short-lived process sampling never encoded missing memory as zero.' -Details ([ordered]@{
            runs = [int]$memorySelfTest.Runs
            observed = [int]$memorySelfTest.Observed
            missing_but_explicit = [int]$memorySelfTest.MissingButExplicit
            silent_zero_peaks = [int]$memorySelfTest.SilentZeroPeaks
        })
    }
    catch {
        Add-Check -Id 'MEMORY-MISSING-STATE' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
    }

    if ([System.IO.File]::Exists($binary)) {
        $hashes.application_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
        Add-Check -Id 'BINARY' -Status 'pass' -Summary 'Release binary exists and was hashed.' -Details ([ordered]@{
            path = $binary
            bytes = [System.IO.FileInfo]::new($binary).Length
            sha256 = $hashes.application_sha256
        })
    }
    else {
        Add-Check -Id 'BINARY' -Status 'fail_closed' -Summary "Release binary is missing: $binary" -Details $null
    }

    try {
        $fixtureResult = & (Join-Path $repositoryRoot 'tools/New-LeanRowsFixtures.ps1') -OutputDirectory $smallFixtureDirectory -CsvRows 5000 -JsonlRows 5000 -LogRows 5000 -GiantRecordPayloadBytes 4194304
        $smallManifestPath = [string]$fixtureResult.Manifest
        $smallManifest = Get-Content -Raw -LiteralPath $smallManifestPath | ConvertFrom-Json
        $manifestHash = (Get-FileHash -LiteralPath $smallManifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($manifestHash -ne [string]$fixtureResult.ManifestSha256) {
            throw 'Fixture manifest hash differs from the generator result.'
        }
        $hashes.small_fixture_manifest_sha256 = $manifestHash
        Copy-Item -LiteralPath $smallManifestPath -Destination (Join-Path $evidenceRoot 'small-fixture-manifest.json')
        Add-Check -Id 'FIXTURE-SMALL' -Status 'pass' -Summary 'Deterministic small and adversarial fixtures were generated and verified.' -Details ([ordered]@{
            fixture_count = [int]$fixtureResult.FixtureCount
            total_bytes = [long]$fixtureResult.TotalFixtureBytes
            manifest_sha256 = $manifestHash
        })
    }
    catch {
        Add-Check -Id 'FIXTURE-SMALL' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
    }

    if ($null -ne $smallManifest -and [System.IO.File]::Exists($binary)) {
        $tab = [char]9
        $lineFeed = [char]10
        [System.IO.File]::WriteAllText(
            (Join-Path $smallFixtureDirectory 'small.tsv'),
            [string]::Concat('alpha', $tab, 'bravo', $lineFeed, 'charlie', $tab, 'delta', $lineFeed),
            $script:Utf8NoBom
        )
        [System.IO.File]::WriteAllText(
            (Join-Path $smallFixtureDirectory 'small.ndjson'),
            [string]::Concat(
                '{"kind":"alpha","value":1}',
                $lineFeed,
                '{"kind":"beta","value":2}',
                $lineFeed
            ),
            $script:Utf8NoBom
        )
        [System.IO.File]::WriteAllText(
            (Join-Path $smallFixtureDirectory 'small.txt'),
            [string]::Concat('first line', $lineFeed, 'second line', $lineFeed),
            $script:Utf8NoBom
        )

        foreach ($name in @(
            'large-multiline.csv',
            'large.jsonl',
            'large.log',
            'malformed.csv',
            'giant-record.csv',
            'small.tsv',
            'small.ndjson',
            'small.txt'
        )) {
            $id = 'DOC-' + $name.ToUpperInvariant().Replace('.', '-')
            try {
                $outputFile = Join-Path $evidenceRoot ($id.ToLowerInvariant() + '.json')
                $details = & (Join-Path $PSScriptRoot 'Invoke-LeanRowsDocumentSmoke.ps1') -BinaryPath $binary -FixturePath (Join-Path $smallFixtureDirectory $name) -TimeoutSeconds $DocumentSmokeTimeoutSeconds -OutputPath $outputFile
                Add-Check -Id $id -Status 'pass' -Summary "Real document smoke passed for $name." -Details $details
            }
            catch {
                Add-Check -Id $id -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
            }
        }

        $negativeRoot = Join-Path $fixtureRoot 'negative'
        $null = [System.IO.Directory]::CreateDirectory($negativeRoot)
        $emptyPath = Join-Path $negativeRoot 'empty.log'
        $unsupportedPath = Join-Path $negativeRoot 'unsupported.bin'
        $missingPath = Join-Path $negativeRoot 'missing.csv'
        [System.IO.File]::WriteAllBytes($emptyPath, [byte[]]@())
        [System.IO.File]::WriteAllBytes($unsupportedPath, [byte[]](1, 2, 3, 4))
        foreach ($negative in @(
            [pscustomobject]@{ Id = 'DOC-NEGATIVE-EMPTY'; Path = $emptyPath },
            [pscustomobject]@{ Id = 'DOC-NEGATIVE-UNSUPPORTED'; Path = $unsupportedPath },
            [pscustomobject]@{ Id = 'DOC-NEGATIVE-MISSING'; Path = $missingPath }
        )) {
            try {
                $details = & (Join-Path $PSScriptRoot 'Invoke-LeanRowsDocumentSmoke.ps1') -BinaryPath $binary -FixturePath $negative.Path -TimeoutSeconds $DocumentSmokeTimeoutSeconds -ExpectFailure
                Add-Check -Id $negative.Id -Status 'pass' -Summary 'Invalid document smoke failed closed.' -Details $details
            }
            catch {
                Add-Check -Id $negative.Id -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
            }
        }

        try {
            $networkOutput = Join-Path $evidenceRoot 'network-observation.json'
            $network = & (Join-Path $PSScriptRoot 'Observe-LeanRowsNetwork.ps1') `
                -BinaryPath $binary `
                -FixturePath (Join-Path $smallFixtureDirectory 'large.log') `
                -WorkflowTimeoutSeconds $NetworkObservationTimeoutSeconds `
                -OutputPath $networkOutput
            Add-Check -Id 'NET-01' -Status 'pass' -Summary 'Local TCP/UDP polling observed no endpoint during native open, view, seek, reload, and clean close.' -Details ([ordered]@{
                method = [string]$network.method.mode
                sample_count = [int]$network.sampling.sample_count
                requested_sample_interval_ms = [int]$network.settings.requested_sample_interval_ms
                maximum_sample_start_gap_ms = [long]$network.sampling.maximum_sample_start_gap_ms
                maximum_discovered_process_count = [int]$network.sampling.maximum_discovered_process_count
                observed_protocol_counts = @($network.sampling.observed_protocol_counts)
                open_view_observed = [bool]$network.actions.open_view_observed
                native_seek_completed = [bool]$network.actions.native_seek_completed
                reload_completed = [bool]$network.actions.reload_completed
                clean_close_observed = [bool]$network.actions.clean_close_observed
                synthetic_disposable_fixture_attested = [bool]$network.source.synthetic_disposable_fixture_attested
                source_unchanged = [bool]$network.source.unchanged
                residual_process_count = [int]$network.process_tree.residual_process_count
                raw_endpoints_persisted = [bool]$network.method.raw_endpoints_persisted
                disclosure = [string]$network.method.disclosure
            })
        }
        catch {
            Add-Check -Id 'NET-01' -Status 'fail_closed' -Summary 'The local network-observation workflow failed closed.' -Details $null
        }
    }
    elseif (-not [System.IO.File]::Exists($binary) -or $null -eq $smallManifest) {
        Add-Check -Id 'NET-01' -Status 'fail_closed' -Summary 'The binary or synthetic fixture required by network observation is unavailable.' -Details $null
    }

    if ($SkipSparseScaleFixtures) {
        Add-Check -Id 'FIXTURE-SPARSE-SCALE' -Status 'not_run' -Summary 'Sparse scale fixtures were explicitly skipped.' -Details $null
    }
    else {
        $sparseDirectory = Join-Path $fixtureRoot 'leanrows-qa-sparse'
        try {
            $sparseResult = & (Join-Path $PSScriptRoot 'New-LeanRowsSparseFixtures.ps1') -OutputDirectory $sparseDirectory -SizesGiB 1, 10
            $sparseManifest = Get-Content -Raw -LiteralPath $sparseResult.ManifestPath | ConvertFrom-Json
            $hashes.sparse_fixture_manifest_sha256 = [string]$sparseResult.ManifestSha256
            Copy-Item -LiteralPath $sparseResult.ManifestPath -Destination (Join-Path $evidenceRoot 'sparse-fixture-manifest.json')
            Add-Check -Id 'FIXTURE-SPARSE-SCALE' -Status 'pass' -Summary '1 and 10 GiB logical sparse fixtures were structurally verified.' -Details ([ordered]@{
                manifest_sha256 = [string]$sparseResult.ManifestSha256
                fixtures = @($sparseManifest.fixtures | ForEach-Object {
                    [ordered]@{
                        name = $_.name
                        logical_size_bytes = [long]$_.logical_size_bytes
                        allocated_bytes_observed = [long]$_.allocated_bytes_observed
                        descriptor_sha256 = [string]$_.descriptor_sha256
                        full_sha256_status = [string]$_.full_sha256_status
                    }
                })
            })
            if ([System.IO.File]::Exists($binary)) {
                foreach ($sparse in @($sparseManifest.fixtures)) {
                    $id = 'DOC-SPARSE-' + ([string]$sparse.name).ToUpperInvariant().Replace('.', '-')
                    try {
                        $details = & (Join-Path $PSScriptRoot 'Invoke-LeanRowsDocumentSmoke.ps1') -BinaryPath $binary -FixturePath ([string]$sparse.path) -TimeoutSeconds $DocumentSmokeTimeoutSeconds -DescriptorOnlyIntegrity
                        Add-Check -Id $id -Status 'pass' -Summary 'First viewport passed on a sparse logical-size fixture.' -Details $details
                    }
                    catch {
                        Add-Check -Id $id -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
                    }
                }
            }
        }
        catch {
            Add-Check -Id 'FIXTURE-SPARSE-SCALE' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
        }
    }

    if ($null -ne $smallManifest -and [System.IO.File]::Exists($spikeBinary)) {
        try {
            $seekOutput = Join-Path $evidenceRoot 'random-seek.json'
            $details = & (Join-Path $PSScriptRoot 'Test-LeanRowsRandomSeek.ps1') -SpikeBinaryPath $spikeBinary -ManifestPath $smallManifestPath -FixtureDirectory $smallFixtureDirectory -Runs $RandomSeekRuns -OutputPath $seekOutput
            $hashes.spike_sha256 = [string]$details.executable_sha256
            Add-Check -Id 'SEEK-RANDOM-ENTRYPOINT' -Status 'pass' -Summary "$RandomSeekRuns deterministic architecture-spike viewport seeks matched requested rows." -Details $details
        }
        catch {
            Add-Check -Id 'SEEK-RANDOM-ENTRYPOINT' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
        }
    }
    else {
        Add-Check -Id 'SEEK-RANDOM-ENTRYPOINT' -Status 'not_exposed' -Summary 'The spike binary was unavailable; native UI random-seek automation is not exposed.' -Details $null
    }

    if ([System.IO.File]::Exists($binary)) {
        foreach ($memoryCase in @(
            [pscustomobject]@{ Id = 'MEMORY-COLD-IDLE'; Document = $null; File = 'memory-cold.json' },
            [pscustomobject]@{ Id = 'MEMORY-DOCUMENT-IDLE'; Document = (Join-Path $smallFixtureDirectory 'large.log'); File = 'memory-document.json' }
        )) {
            if ($null -ne $memoryCase.Document -and -not [System.IO.File]::Exists($memoryCase.Document)) {
                Add-Check -Id $memoryCase.Id -Status 'fail_closed' -Summary 'Required memory fixture is unavailable.' -Details $null
                continue
            }
            try {
                $memoryArguments = @{
                    BinaryPath = $binary
                    OutputPath = (Join-Path $evidenceRoot $memoryCase.File)
                    SettleMilliseconds = 750
                    SampleDurationMilliseconds = 1250
                    SampleIntervalMilliseconds = 100
                }
                if ($null -ne $memoryCase.Document) {
                    $memoryArguments.DocumentPath = $memoryCase.Document
                }
                $memory = & (Join-Path $PSScriptRoot 'Measure-LeanRowsIdle.ps1') @memoryArguments
                if ($memory.memory_observation_status -ne 'observed' -or
                    $null -eq $memory.process_tree_private_bytes_peak -or
                    $null -eq $memory.process_tree_working_set_bytes_peak -or
                    [long]$memory.process_tree_private_bytes_peak -le 0 -or
                    [long]$memory.process_tree_working_set_bytes_peak -le 0 -or
                    [int]$memory.residual_process_count -ne 0) {
                    throw 'Idle memory was missing, non-positive, or left a residual process.'
                }
                Add-Check -Id $memoryCase.Id -Status 'pass' -Summary 'Full process-tree idle memory was observed with no residual process.' -Details ([ordered]@{
                    private_bytes_peak = [long]$memory.process_tree_private_bytes_peak
                    working_set_bytes_peak = [long]$memory.process_tree_working_set_bytes_peak
                    commit_bytes_status = [string]$memory.process_tree_commit_bytes_status
                    sample_count = [int]$memory.sample_count
                    startup_attested_elapsed_ms = [long]$memory.startup_attested_elapsed_ms
                    startup_attestation_method = [string]$memory.input_idle_method
                    shutdown_method = [string]$memory.shutdown_method
                    residual_process_count = [int]$memory.residual_process_count
                })
            }
            catch {
                Add-Check -Id $memoryCase.Id -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
            }
        }
    }

    if ([System.IO.File]::Exists($binary)) {
        try {
            $releaseOutput = & (Join-Path $repositoryRoot 'scripts/Test-ReleaseEngineering.ps1') -BinaryPath $binary | Out-String
            $version = & (Join-Path $repositoryRoot 'scripts/Get-WorkspaceVersion.ps1')
            $packageDirectory = Join-Path $evidenceRoot 'package'
            $package = & (Join-Path $repositoryRoot 'scripts/Package-Windows.ps1') -BinaryPath $binary -Version $version -OutputDirectory $packageDirectory
            $artifactOutput = & (Join-Path $repositoryRoot 'scripts/Test-ReleaseEngineering.ps1') -ArtifactDirectory $packageDirectory | Out-String
            $hashes.windows_package_sha256 = [string]$package.Sha256
            Add-Check -Id 'PACKAGE-WINDOWS' -Status 'pass' -Summary 'Installer static, ValidateOnly, package, and checksum checks passed.' -Details ([ordered]@{
                zip = [string]$package.ZipPath
                checksum = [string]$package.ChecksumPath
                sha256 = [string]$package.Sha256
                signing_status = [string]$package.SigningStatus
                release_output_sha256 = Get-TextSha256 -Text $releaseOutput
                artifact_output_sha256 = Get-TextSha256 -Text $artifactOutput
            })
        }
        catch {
            Add-Check -Id 'PACKAGE-WINDOWS' -Status 'fail_closed' -Summary $_.Exception.Message -Details $null
        }
    }
}
finally {
    $cleanupAttempted = $true
    $tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $leaf = if ($null -eq $fixtureRoot) { '' } else { [System.IO.Path]::GetFileName($fixtureRoot) }
    if ($null -ne $fixtureRoot -and
        $fixtureRoot.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
        $leaf.StartsWith('leanrows-qa-', [System.StringComparison]::OrdinalIgnoreCase) -and
        [System.IO.File]::Exists((Join-Path $fixtureRoot '.leanrows_qa_fixture_root'))) {
        try {
            Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
        }
        catch { }
    }
    $cleanupRemoved = $null -eq $fixtureRoot -or -not [System.IO.Directory]::Exists($fixtureRoot)
}

if ($cleanupRemoved) {
    Add-Check -Id 'CLEANUP' -Status 'pass' -Summary 'Disposable fixture root was removed.' -Details ([ordered]@{
        attempted = $cleanupAttempted
        fixture_root_removed = $cleanupRemoved
    })
}
else {
    Add-Check -Id 'CLEANUP' -Status 'fail_closed' -Summary "Disposable fixture cleanup failed: $fixtureRoot" -Details ([ordered]@{
        attempted = $cleanupAttempted
        fixture_root_removed = $cleanupRemoved
        manual_rollback = "Delete only the sentinel-marked fixture root: $fixtureRoot"
    })
}
$harnessWatch.Stop()

$failedChecks = @($checks | Where-Object { $_.status -eq 'fail_closed' })
$verdict = if ($failedChecks.Count -eq 0) { 'pass' } else { 'fail_closed' }
$commit = 'not_available'
try {
    $commitOutput = & git -C $repositoryRoot rev-parse HEAD 2>$null
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace([string]$commitOutput)) {
        $commit = ([string]$commitOutput).Trim()
    }
}
catch { }

$hostInfo = [ordered]@{
    os_description = [System.Environment]::OSVersion.VersionString
    os_architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    process_architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    logical_processor_count = [System.Environment]::ProcessorCount
    powershell_version = $PSVersionTable.PSVersion.ToString()
}
try {
    $computer = Get-CimInstance -ClassName Win32_ComputerSystem -Property TotalPhysicalMemory, Manufacturer, Model
    $hostInfo.total_physical_memory_bytes = [long]$computer.TotalPhysicalMemory
    $hostInfo.manufacturer = [string]$computer.Manufacturer
    $hostInfo.model = [string]$computer.Model
}
catch {
    $hostInfo.hardware_status = 'not_observed'
}

$notDemonstrated = @(
    'Rust workspace tests, formatting, and Clippy; CI may run them as separate gates, but this receipt does not ingest their results.'
    'A full Narrator, Accessibility Insights, or manual keyboard audit.'
    'Append, truncate, replace, or rotation races after open.'
    'Automated random-seek interaction through the native owner-data UI; the repeated seek matrix exercises the architecture-spike CLI.'
    'Complete scans of real non-sparse 1, 10, or 100 GiB corpora.'
    if ($SkipSparseScaleFixtures) {
        'The 1 and 10 GiB logical sparse-fixture construction and first-viewport checks; they were explicitly skipped for this reduced run.'
    }
    else {
        'Full SHA-256 hashes for sparse logical-size fixtures.'
    }
    'Twenty-four aggregate CPU-hours of fuzzing.'
    'The acceptance threshold for total-process memory or first-present latency.'
    'Signed Windows artifacts, macOS artifacts, or Linux artifacts.'
)

$receipt = [ordered]@{
    schema_version = 'support_tool_fixture_receipt.v1'
    generated_at_utc = [DateTime]::UtcNow.ToString('o')
    fixture_id = 'leanrows-v0.1-local-qa'
    candidate_id = 'leanrows-v0.1'
    approval_state = 'user_requested_local_release_qa'
    spec_path = 'tests/qa/README.md'
    output_dir = $evidenceRoot
    verdict = $verdict
    failure_reasons = @($failedChecks | ForEach-Object { "$($_.id): $($_.summary)" })
    scope_disclosure = 'Local synthetic QA evidence only. A pass is not a claim that every docs/ACCEPTANCE.md release gate passed.'
    network = [ordered]@{
        mode = 'offline_local_observation'
        destinations = @()
        data_classes = @(
            'discovered_process_count'
            'tcp_connection_count_and_state'
            'udp_endpoint_count'
        )
        disclosure = 'Only local executables, files, and Windows endpoint tables are intentionally used. NET-01 uses high-frequency Get-NetTCPConnection/Get-NetUDPEndpoint polling of the discovered LeanRows process tree; it persists counts/protocol/state only, not endpoints. It is not ETW capture and cannot prove absence between samples.'
    }
    command = [ordered]@{
        allowlist_id = 'leanrows-local-qa-v1'
        version_pin = 'LeanRows workspace 0.1.0; Rust 1.97.1'
        argv_summary = 'Local document smoke, native network observation, spike viewport, memory, and release-engineering entry points with synthetic paths.'
        timeout_seconds = [Math]::Max($DocumentSmokeTimeoutSeconds, $NetworkObservationTimeoutSeconds)
        expected_exit_codes = @(0)
        environment_policy = 'Inherited local environment; environment values are not persisted.'
    }
    timing = [ordered]@{ duration_ms = [long]$harnessWatch.ElapsedMilliseconds }
    exit_code = if ($verdict -eq 'pass') { 0 } else { 1 }
    redaction = [ordered]@{
        stdout_redaction_count = 0
        stderr_redaction_count = 0
        pattern_count_classes = @()
    }
    stdout_sanitized = 'Raw child output is not retained in the aggregate receipt; bounded hashes and parsed assertions are recorded.'
    stderr_sanitized = 'Raw child output is not retained in the aggregate receipt; bounded hashes and parsed assertions are recorded.'
    hashes = $hashes
    host = $hostInfo
    repository_commit = $commit
    checks = $checks.ToArray()
    cleanup = [ordered]@{
        attempted = $cleanupAttempted
        fixture_root_removed = $cleanupRemoved
        root_exists_after = -not $cleanupRemoved
    }
    rollback = [ordered]@{
        persistent_install_changed = $false
        delete_paths = @($evidenceRoot)
        instructions = 'Delete this evidence directory to remove receipts and the unsigned package. Installer checks used ValidateOnly.'
    }
    not_demonstrated = $notDemonstrated
}

$receiptPath = Join-Path $evidenceRoot 'qa-receipt.json'
$markdownPath = Join-Path $evidenceRoot 'qa-receipt.md'
[System.IO.File]::WriteAllText(
    $receiptPath,
    (($receipt | ConvertTo-Json -Depth 15) + [Environment]::NewLine),
    $script:Utf8NoBom
)
[System.IO.File]::WriteAllText(
    $markdownPath,
    (New-MarkdownReceipt -Receipt ([pscustomobject]$receipt)),
    $script:Utf8NoBom
)

[pscustomobject]@{
    Verdict = $verdict
    ReceiptJson = $receiptPath
    ReceiptMarkdown = $markdownPath
    CheckCount = $checks.Count
    FailedCheckCount = $failedChecks.Count
}
if ($verdict -ne 'pass') {
    exit 1
}
