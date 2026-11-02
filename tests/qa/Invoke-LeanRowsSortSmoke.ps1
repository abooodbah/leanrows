[CmdletBinding()]
param(
    [string]$Executable = (Join-Path $PSScriptRoot '../../target/release/leanrows.exe'),
    [string]$OutputPath = (Join-Path $PSScriptRoot '../../artifacts/qa/sort-smoke.json'),
    [int]$TimeoutSeconds = 60
)
$ErrorActionPreference = 'Stop'
$binary = [System.IO.Path]::GetFullPath($Executable)
if (-not [System.IO.File]::Exists($binary)) { throw "Executable missing: $binary" }
$temporaryBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$fixtureRoot = Join-Path $temporaryBase ('leanrows-sort-qa-' + [guid]::NewGuid().ToString('N'))
$null = [System.IO.Directory]::CreateDirectory($fixtureRoot)
$sentinel = Join-Path $fixtureRoot '.leanrows_sort_qa'
[System.IO.File]::WriteAllText($sentinel, 'synthetic_sort_fixture')
$fixture = Join-Path $fixtureRoot 'sort.csv'
$secondary = Join-Path $fixtureRoot 'sort.sort-tab.log'
$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($fixture, "z,10`na,2`nm,9007199254740993`nb,9007199254740992`n", $utf8)
[System.IO.File]::WriteAllText($secondary, "other tab`n", $utf8)
$beforeHash = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash
$secondaryHash = (Get-FileHash -LiteralPath $secondary -Algorithm SHA256).Hash
$process = $null
try {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $binary
    $info.Arguments = '--sort-smoke "' + $fixture + '"'
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    $null = $process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $privatePeak = [long]0
    $workingPeak = [long]0
    $samples = 0
    while (-not $process.WaitForExit(20)) {
        if ([DateTime]::UtcNow -gt $deadline) {
            $process.Kill()
            throw 'Native sort smoke exceeded its deadline.'
        }
        $process.Refresh()
        $privatePeak = [Math]::Max($privatePeak, $process.PrivateMemorySize64)
        $workingPeak = [Math]::Max($workingPeak, $process.WorkingSet64)
        $samples++
    }
    $text = $stdout.GetAwaiter().GetResult().Trim()
    $diagnostic = $stderr.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) { throw "Native sort smoke failed ($($process.ExitCode)): $diagnostic" }
    $receipt = $text | ConvertFrom-Json
    if ($receipt.schema -ne 'leanrows.sort-smoke' -or $receipt.version -ne 1 -or
        -not $receipt.native_workflow) { throw 'Native sort smoke receipt is invalid.' }
    if ((Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash -ne $beforeHash -or
        (Get-FileHash -LiteralPath $secondary -Algorithm SHA256).Hash -ne $secondaryHash) {
        throw 'Sort smoke changed its source file.'
    }
    $scratch = Join-Path $env:LOCALAPPDATA 'LeanRows/sort-cache'
    $residual = @(Get-ChildItem -LiteralPath $scratch -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name.StartsWith("leanrows-sort-v1-$($process.Id)-") })
    if ($residual.Count -ne 0) { throw 'Native sort smoke left owned scratch directories.' }
    $report = [ordered]@{
        schema = 'leanrows.sort-qa'
        version = 1
        passed = $true
        native_workflow = $true
        text_source_rows = @(1, 3, 2, 0)
        numeric_source_rows = @(1, 0, 3, 2)
        checks = @('real_header_clicks', 'text_both_directions', 'numeric_both_directions',
            'exact_large_integers', 'source_row_navigation', 'find_reveal', 'copy',
            'tab_restore', 'background_selection_reset', 'reject_invalid_source_row',
            'restore_source_order', 'cancel', 'source_hash', 'scratch_cleanup')
        private_bytes_peak = $(if ($samples -gt 0) { $privatePeak } else { $null })
        working_set_peak = $(if ($samples -gt 0) { $workingPeak } else { $null })
        samples = $samples
        process_id = $process.Id
    }
    $output = [System.IO.Path]::GetFullPath($OutputPath)
    $null = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($output))
    [System.IO.File]::WriteAllText($output, ($report | ConvertTo-Json -Depth 4), $utf8)
    [pscustomobject]$report
} finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
    $resolved = [System.IO.Path]::GetFullPath($fixtureRoot)
    if (-not $resolved.StartsWith($temporaryBase.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $sentinel) -or
        [System.IO.File]::ReadAllText($sentinel) -ne 'synthetic_sort_fixture') {
        throw 'Refusing cleanup outside the owned synthetic sort fixture.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
