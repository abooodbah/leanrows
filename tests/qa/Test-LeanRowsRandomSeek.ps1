[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SpikeBinaryPath,
    [Parameter(Mandatory = $true)][string]$ManifestPath,
    [Parameter(Mandatory = $true)][string]$FixtureDirectory,
    [ValidateRange(1, 1000)][int]$Runs = 24,
    [int]$Seed = 20260809,
    [string]$OutputPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function ConvertTo-NativeQuotedArgument {
    param([Parameter(Mandatory = $true)][string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Invoke-Seek {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$Fixture,
        [Parameter(Mandatory = $true)][long]$Row
    )
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($Executable)
    $startInfo.Arguments = (
        '--scan ' +
        (ConvertTo-NativeQuotedArgument -Value $Fixture) +
        ' --row ' +
        [string]$Row +
        ' --count 1'
    )
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $target = New-Object System.Diagnostics.Process
    $target.StartInfo = $startInfo
    if (-not $target.Start()) {
        throw "Failed to start $Executable."
    }
    $stdoutTask = $target.StandardOutput.ReadToEndAsync()
    $stderrTask = $target.StandardError.ReadToEndAsync()
    $timedOut = -not $target.WaitForExit(30000)
    if ($timedOut) {
        try {
            Stop-Process -Id $target.Id -Force -ErrorAction Stop
        }
        catch { }
        $null = $target.WaitForExit(5000)
    }
    else {
        $target.WaitForExit()
    }
    $stdout = [string]$stdoutTask.Result
    $stderr = [string]$stderrTask.Result
    $exitCode = if ($target.HasExited) { [int]$target.ExitCode } else { $null }
    $target.Dispose()
    if ($script:Utf8NoBom.GetByteCount($stdout) -gt 65536 -or
        $script:Utf8NoBom.GetByteCount($stderr) -gt 65536) {
        throw 'Seek output exceeded the 64 KiB capture limit.'
    }
    if ($timedOut -or $exitCode -ne 0) {
        throw "Seek process timed out or exited with code $exitCode."
    }
    try {
        return $stdout.Trim() | ConvertFrom-Json
    }
    catch {
        throw 'Seek process stdout was not valid JSON.'
    }
}

function Get-MatchingExecutableIds {
    param([Parameter(Mandatory = $true)][string]$Executable)
    $ids = New-Object 'System.Collections.Generic.List[int]'
    foreach ($candidate in @(Get-Process -ErrorAction SilentlyContinue)) {
        try {
            if ([string]::Equals(
                    [System.IO.Path]::GetFullPath($candidate.Path),
                    $Executable,
                    [System.StringComparison]::OrdinalIgnoreCase)) {
                $ids.Add([int]$candidate.Id)
            }
        }
        catch { }
        finally {
            $candidate.Dispose()
        }
    }
    $ids.ToArray()
}

$spike = (Resolve-Path -LiteralPath $SpikeBinaryPath).Path
$manifestFile = (Resolve-Path -LiteralPath $ManifestPath).Path
$fixtureRoot = (Resolve-Path -LiteralPath $FixtureDirectory).Path
$manifest = Get-Content -Raw -LiteralPath $manifestFile | ConvertFrom-Json
$candidates = @(
    $manifest.fixtures |
        Where-Object { $_.name -in @('large-multiline.csv', 'large.jsonl', 'large.log') }
)
if ($candidates.Count -ne 3) {
    throw 'Random-seek fixture manifest is incomplete.'
}

$sourceBefore = [ordered]@{}
foreach ($fixtureInfo in $candidates) {
    $path = Join-Path $fixtureRoot ([string]$fixtureInfo.name)
    $sourceBefore[[string]$fixtureInfo.name] = [ordered]@{
        bytes = [System.IO.FileInfo]::new($path).Length
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        last_write_utc = [System.IO.FileInfo]::new($path).LastWriteTimeUtc.ToString('o')
    }
}

$beforeIds = @(Get-MatchingExecutableIds -Executable $spike)
$random = New-Object System.Random($Seed)
$cases = New-Object 'System.Collections.Generic.List[object]'
$mismatchCount = 0
for ($run = 0; $run -lt $Runs; $run++) {
    $fixtureInfo = $candidates[$run % $candidates.Count]
    $recordCount = [long]$fixtureInfo.logical_records
    $requested = [long]($random.NextDouble() * [double]$recordCount)
    if ($requested -ge $recordCount) {
        $requested = $recordCount - 1
    }
    $path = Join-Path $fixtureRoot ([string]$fixtureInfo.name)
    $json = Invoke-Seek -Executable $spike -Fixture $path -Row $requested
    $returned = $null
    if ([long]$json.viewport.returned_count -eq 1) {
        $returned = [long]$json.viewport.records[0].row
    }
    $match = $null -ne $returned -and $returned -eq $requested
    if (-not $match) {
        $mismatchCount++
    }
    $cases.Add([ordered]@{
        run = $run
        fixture = [string]$fixtureInfo.name
        requested_row = $requested
        returned_row = $returned
        match = $match
    })
}

foreach ($fixtureInfo in $candidates) {
    $name = [string]$fixtureInfo.name
    $path = Join-Path $fixtureRoot $name
    $after = [System.IO.FileInfo]::new($path)
    $before = $sourceBefore[$name]
    if ($after.Length -ne [long]$before.bytes -or
        $after.LastWriteTimeUtc.ToString('o') -ne [string]$before.last_write_utc -or
        (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$before.sha256) {
        throw "Random-seek entry point changed source fixture $name."
    }
}
Start-Sleep -Milliseconds 100
$newIds = @(
    Get-MatchingExecutableIds -Executable $spike |
        Where-Object { $beforeIds -notcontains $_ }
)
if ($newIds.Count -ne 0) {
    throw "Random-seek matrix left $($newIds.Count) spike process(es) running."
}
if ($mismatchCount -ne 0) {
    throw "Random-seek matrix found $mismatchCount mismatch(es)."
}

$report = [ordered]@{
    schema_version = 'leanrows_random_seek_evidence.v1'
    measured_at_utc = [DateTime]::UtcNow.ToString('o')
    executable_sha256 = (Get-FileHash -LiteralPath $spike -Algorithm SHA256).Hash.ToLowerInvariant()
    fixture_manifest_sha256 = (Get-FileHash -LiteralPath $manifestFile -Algorithm SHA256).Hash.ToLowerInvariant()
    seed = $Seed
    runs = $Runs
    fixture_count = $candidates.Count
    mismatches = $mismatchCount
    source_integrity = 'length_timestamp_and_sha256_unchanged'
    residual_process_count = 0
    cases = $cases.ToArray()
}
if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
    $null = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($resolvedOutput))
    [System.IO.File]::WriteAllText(
        $resolvedOutput,
        (($report | ConvertTo-Json -Depth 8) + [Environment]::NewLine),
        $script:Utf8NoBom
    )
}
[pscustomobject]$report
