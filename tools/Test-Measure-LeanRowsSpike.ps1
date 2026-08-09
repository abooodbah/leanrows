[CmdletBinding()]
param(
    [int]$Runs = 3
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($env:OS -ne 'Windows_NT') {
    Write-Warning 'The deterministic short-lived-process test currently targets Windows PowerShell.'
    return
}
if ($Runs -lt 1 -or $Runs -gt 20) {
    throw 'Runs must be between 1 and 20.'
}

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testDirectory = Join-Path $tempRoot ('leanrows-short-process-' + [guid]::NewGuid().ToString('N'))
$resolvedTestDirectory = [System.IO.Path]::GetFullPath($testDirectory)

try {
    $null = [System.IO.Directory]::CreateDirectory($resolvedTestDirectory)
    $fixture = Join-Path $resolvedTestDirectory 'fixture.log'
    [System.IO.File]::WriteAllText($fixture, ('row=1' + [char]10), (New-Object System.Text.UTF8Encoding($false)))
    $reportPath = Join-Path $resolvedTestDirectory 'report.json'

    $currentProcess = [System.Diagnostics.Process]::GetCurrentProcess()
    try {
        $currentPowerShell = [string]$currentProcess.MainModule.FileName
    }
    catch {
        throw ('Unable to resolve the current PowerShell host executable from process {0}: {1}' -f `
            $PID, $_.Exception.Message)
    }
    finally {
        $currentProcess.Dispose()
    }
    if ([string]::IsNullOrWhiteSpace($currentPowerShell)) {
        throw "The current PowerShell process $PID did not expose a host executable path."
    }
    $currentPowerShell = [System.IO.Path]::GetFullPath($currentPowerShell)
    if (-not [System.IO.File]::Exists($currentPowerShell)) {
        throw "The current PowerShell host executable does not exist: $currentPowerShell"
    }
    $expectedHostLeaf = if ([string]$PSVersionTable.PSEdition -eq 'Core') {
        'pwsh.exe'
    }
    elseif ([string]$PSVersionTable.PSEdition -eq 'Desktop') {
        'powershell.exe'
    }
    else {
        throw ('Unsupported PowerShell edition for the short-process self-test: {0}' -f `
            [string]$PSVersionTable.PSEdition)
    }
    $currentHostLeaf = [System.IO.Path]::GetFileName($currentPowerShell)
    if (-not [string]::Equals(
            $currentHostLeaf,
            $expectedHostLeaf,
            [System.StringComparison]::OrdinalIgnoreCase)) {
        throw (("The current PowerShell edition '{0}' is hosted by unsupported executable '{1}'; " +
            "expected the current CLI host '{2}'. No fallback host will be used.") -f `
            [string]$PSVersionTable.PSEdition, $currentHostLeaf, $expectedHostLeaf)
    }
    & (Join-Path $PSScriptRoot 'Measure-LeanRowsSpike.ps1') `
        -Executable $currentPowerShell `
        -FixturePath $fixture `
        -ArgumentTemplate '-NoProfile -NonInteractive -Command Start-Sleep -Milliseconds 75;# {fixture}' `
        -Runs $Runs `
        -SampleIntervalMilliseconds 10 `
        -TimeoutSeconds 15 `
        -IncludeSamples `
        -OutputPath $reportPath | Out-Null

    $report = Get-Content -Raw -LiteralPath $reportPath | ConvertFrom-Json
    if (-not [string]::Equals(
            [System.IO.Path]::GetFullPath([string]$report.executable.path),
            $currentPowerShell,
            [System.StringComparison]::OrdinalIgnoreCase)) {
        throw ('The sampler exercised a different executable than the current PowerShell host: {0}' -f `
            [string]$report.executable.path)
    }
    if ($report.observations.Count -ne $Runs) {
        throw ('Expected {0} observations; received {1}.' -f $Runs, $report.observations.Count)
    }

    $observedCount = 0
    $missingCount = 0
    foreach ($observation in $report.observations) {
        if ($null -eq $observation.process_elapsed_seconds -or
            $observation.process_elapsed_seconds -lt 0.05 -or
            $observation.process_elapsed_seconds -gt 5.0) {
            throw ('Expected a plausible target process lifetime; received {0}.' -f $observation.process_elapsed_seconds)
        }
        if ($observation.harness_observation_seconds -lt $observation.process_elapsed_seconds) {
            throw 'Harness observation time cannot be shorter than target process lifetime.'
        }
        if ($null -eq $observation.input_bytes_divided_by_process_elapsed_bps -or
            $observation.input_bytes_divided_by_process_elapsed_bps -le 0) {
            throw 'The fixture/process-lifetime calculation must be positive when lifetime is observed.'
        }
        if ($null -ne $observation.PSObject.Properties['elapsed_seconds'] -or
            $null -ne $observation.PSObject.Properties['input_bytes_divided_by_elapsed_bps']) {
            throw 'Deprecated ambiguous elapsed fields must not be emitted.'
        }
        if ($observation.process_tree_private_bytes_peak -eq 0 -or
            $observation.process_tree_working_set_bytes_peak -eq 0) {
            throw 'Memory peaks must never silently encode a missing observation as zero.'
        }

        switch ($observation.memory_observation_status) {
            'observed' {
                $observedCount++
                if ($observation.memory_observed_sample_count -lt 1 -or
                    $null -eq $observation.process_tree_private_bytes_peak -or
                    $null -eq $observation.process_tree_working_set_bytes_peak -or
                    $observation.process_tree_private_bytes_peak -le 0 -or
                    $observation.process_tree_working_set_bytes_peak -le 0) {
                    throw 'An observed run must have positive private-byte and working-set peaks.'
                }
            }
            'missing' {
                $missingCount++
                if ($observation.memory_observed_sample_count -ne 0 -or
                    $null -ne $observation.process_tree_private_bytes_peak -or
                    $null -ne $observation.process_tree_working_set_bytes_peak) {
                    throw 'A missing run must have zero observed samples and null peaks.'
                }
            }
            default {
                throw ('Unexpected memory observation status: {0}' -f $observation.memory_observation_status)
            }
        }
    }

    if ($observedCount -lt 1) {
        throw 'The root-first sampler did not capture any 75 ms short-lived PowerShell process.'
    }

    [pscustomobject]@{
        Runs = $Runs
        Observed = $observedCount
        MissingButExplicit = $missingCount
        SilentZeroPeaks = 0
        ProcessLifetimeAccounting = 'PASS'
        HostExecutableName = $currentHostLeaf
        HostEdition = [string]$PSVersionTable.PSEdition
        HostExecutableMatchesCurrentProcess = $true
        HostDiscovery = 'current_process_main_module'
    }
} finally {
    $leaf = [System.IO.Path]::GetFileName($resolvedTestDirectory)
    if (-not $resolvedTestDirectory.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase) -or
        -not $leaf.StartsWith('leanrows-short-process-')) {
        throw ('Refusing to clean an unexpected path: {0}' -f $resolvedTestDirectory)
    }
    if (Test-Path -LiteralPath $resolvedTestDirectory) {
        Remove-Item -LiteralPath $resolvedTestDirectory -Recurse -Force
    }
}
