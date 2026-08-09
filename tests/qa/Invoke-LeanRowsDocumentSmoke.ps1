[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [Parameter(Mandatory = $true)][string]$FixturePath,
    [ValidateRange(5, 300)][int]$TimeoutSeconds = 30,
    [switch]$ExpectFailure,
    [switch]$DescriptorOnlyIntegrity,
    [string]$OutputPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

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

function ConvertTo-NativeQuotedArgument {
    param([Parameter(Mandatory = $true)][string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
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

function Invoke-BoundedSmoke {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$Fixture,
        [Parameter(Mandatory = $true)][int]$Timeout
    )
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($Executable)
    $startInfo.Arguments = '--document-smoke-test ' + (ConvertTo-NativeQuotedArgument -Value $Fixture)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $target = New-Object System.Diagnostics.Process
    $target.StartInfo = $startInfo
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    if (-not $target.Start()) {
        throw "Failed to start $Executable."
    }
    $stdoutTask = $target.StandardOutput.ReadToEndAsync()
    $stderrTask = $target.StandardError.ReadToEndAsync()
    $timedOut = -not $target.WaitForExit($Timeout * 1000)
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
    $watch.Stop()
    $stdout = [string]$stdoutTask.Result
    $stderr = [string]$stderrTask.Result
    $exitCode = if ($target.HasExited) { [int]$target.ExitCode } else { $null }
    $target.Dispose()
    if ($script:Utf8NoBom.GetByteCount($stdout) -gt 65536 -or
        $script:Utf8NoBom.GetByteCount($stderr) -gt 65536) {
        throw 'Document smoke output exceeded the 64 KiB capture limit.'
    }
    [pscustomobject]@{
        exit_code = $exitCode
        timed_out = $timedOut
        elapsed_ms = [long]$watch.ElapsedMilliseconds
        stdout = $stdout
        stderr = $stderr
    }
}

$binary = (Resolve-Path -LiteralPath $BinaryPath).Path
if (-not [System.IO.File]::Exists($binary)) {
    throw "LeanRows binary does not exist: $binary"
}
$fixture = [System.IO.Path]::GetFullPath($FixturePath)
$fixtureExists = [System.IO.File]::Exists($fixture)
if (-not $ExpectFailure -and -not $fixtureExists) {
    throw "Document smoke fixture does not exist: $fixture"
}

$beforeIds = @(Get-MatchingExecutableIds -Executable $binary)
$beforeLength = $null
$beforeWriteUtc = $null
$beforeHash = $null
if ($fixtureExists) {
    $before = [System.IO.FileInfo]::new($fixture)
    $beforeLength = [long]$before.Length
    $beforeWriteUtc = $before.LastWriteTimeUtc
    if (-not $DescriptorOnlyIntegrity) {
        $beforeHash = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

$result = Invoke-BoundedSmoke -Executable $binary -Fixture $fixture -Timeout $TimeoutSeconds
if ($result.timed_out) {
    throw 'Document smoke timed out.'
}
if ($ExpectFailure) {
    if ($null -eq $result.exit_code -or $result.exit_code -eq 0) {
        throw 'Negative document smoke did not fail closed with a nonzero exit code.'
    }
    $schema = $null
    $version = $null
    $sourceBytes = $beforeLength
    $cacheFirst = $null
    $cacheRows = $null
    $rowNumber = $null
    $previewHash = $null
    $scanPhase = $null
    $scanComplete = $null
    $scanScannedBytes = $null
    $scanIndexedRows = $null
    $scanAvailableRows = $null
    $shellEvidence = $null
}
else {
    if ($result.exit_code -ne 0) {
        throw "Document smoke exited with code $($result.exit_code)."
    }
    $trimmed = $result.stdout.Trim()
    if ([string]::IsNullOrWhiteSpace($trimmed) -or
        $trimmed.Contains([Environment]::NewLine)) {
        throw 'Document smoke must emit exactly one non-empty JSON line.'
    }
    try {
        $json = $trimmed | ConvertFrom-Json
    }
    catch {
        throw 'Document smoke stdout is not valid JSON.'
    }
    if ([string]$json.schema -ne 'leanrows.document-smoke' -or [int]$json.version -ne 1) {
        throw 'Document smoke emitted an unsupported schema or version.'
    }
    if ([long]$json.source_bytes -ne $beforeLength) {
        throw 'Document smoke source byte count differs from the fixture.'
    }
    if ([long]$json.cache.first_row -ne 0 -or
        [long]$json.cache.row_count -lt 1 -or
        [long]$json.cache.row_count -gt 128) {
        throw 'Document smoke cache bounds are invalid.'
    }
    if ([string]::IsNullOrWhiteSpace([string]$json.row.number) -or
        [string]::IsNullOrWhiteSpace([string]$json.row.preview)) {
        throw 'Document smoke did not attest a real row number and preview.'
    }
    if ([long]$json.scan.scanned_bytes -gt $beforeLength -or
        [long]$json.scan.indexed_rows -gt [long]$json.scan.available_rows -or
        [long]$json.scan.available_rows -lt [long]$json.cache.row_count -or
        [string]::IsNullOrWhiteSpace([string]$json.scan.phase)) {
        throw 'Document smoke scan progress is internally inconsistent.'
    }
    if (-not [bool]$json.shell.controls -or
        -not [bool]$json.shell.accessibility -or
        -not [bool]$json.shell.keyboard_focus -or
        -not [bool]$json.shell.system_colors) {
        throw 'Document smoke did not satisfy every native shell assertion.'
    }
    $schema = [string]$json.schema
    $version = [int]$json.version
    $sourceBytes = [long]$json.source_bytes
    $cacheFirst = [long]$json.cache.first_row
    $cacheRows = [long]$json.cache.row_count
    $rowNumber = [string]$json.row.number
    $previewHash = Get-TextSha256 -Text ([string]$json.row.preview)
    $scanPhase = [string]$json.scan.phase
    $scanComplete = [bool]$json.scan.complete
    $scanScannedBytes = [long]$json.scan.scanned_bytes
    $scanIndexedRows = [long]$json.scan.indexed_rows
    $scanAvailableRows = [long]$json.scan.available_rows
    $shellEvidence = [ordered]@{
        controls = [bool]$json.shell.controls
        accessibility = [bool]$json.shell.accessibility
        keyboard_focus = [bool]$json.shell.keyboard_focus
        system_colors = [bool]$json.shell.system_colors
    }
}

if ($fixtureExists) {
    $after = [System.IO.FileInfo]::new($fixture)
    if ($after.Length -ne $beforeLength -or $after.LastWriteTimeUtc -ne $beforeWriteUtc) {
        throw 'Document smoke changed fixture length or last-write timestamp.'
    }
    if (-not $DescriptorOnlyIntegrity) {
        $afterHash = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($afterHash -ne $beforeHash) {
            throw 'Document smoke changed fixture content.'
        }
    }
}
Start-Sleep -Milliseconds 100
$newIds = @(
    Get-MatchingExecutableIds -Executable $binary |
        Where-Object { $beforeIds -notcontains $_ }
)
if ($newIds.Count -ne 0) {
    throw "Document smoke left $($newIds.Count) LeanRows process(es) running."
}

$report = [ordered]@{
    schema_version = 'leanrows_document_smoke_evidence.v1'
    measured_at_utc = [DateTime]::UtcNow.ToString('o')
    expected_failure = [bool]$ExpectFailure
    exit_code = $result.exit_code
    timed_out = $result.timed_out
    elapsed_ms = $result.elapsed_ms
    executable_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
    fixture_path = $fixture
    source_bytes = $sourceBytes
    source_sha256 = $beforeHash
    source_integrity = if (-not $fixtureExists) {
        'fixture_absent_expected_failure'
    }
    elseif ($DescriptorOnlyIntegrity) {
        'length_and_timestamp_unchanged; descriptor verified separately'
    }
    else {
        'length_timestamp_and_sha256_unchanged'
    }
    document_schema = $schema
    document_schema_version = $version
    cache_first_row = $cacheFirst
    cached_rows = $cacheRows
    row_number = $rowNumber
    preview_sha256 = $previewHash
    scan_phase = $scanPhase
    scan_complete = $scanComplete
    scan_scanned_bytes = $scanScannedBytes
    scan_indexed_rows = $scanIndexedRows
    scan_available_rows = $scanAvailableRows
    shell = $shellEvidence
    stdout_bytes = $script:Utf8NoBom.GetByteCount($result.stdout)
    stderr_bytes = $script:Utf8NoBom.GetByteCount($result.stderr)
    stdout_sha256 = Get-TextSha256 -Text $result.stdout
    stderr_sha256 = Get-TextSha256 -Text $result.stderr
    residual_process_count = 0
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
