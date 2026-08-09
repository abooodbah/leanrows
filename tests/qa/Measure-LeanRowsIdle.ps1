[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [string]$DocumentPath,
    [ValidateRange(0, 60000)][int]$SettleMilliseconds = 1000,
    [ValidateRange(100, 60000)][int]$SampleDurationMilliseconds = 1000,
    [ValidateRange(10, 5000)][int]$SampleIntervalMilliseconds = 100,
    [ValidateRange(1, 120)][int]$StartupTimeoutSeconds = 10,
    [ValidateRange(1, 120)][int]$ShutdownTimeoutSeconds = 5,
    [string]$OutputPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

if ($env:OS -ne 'Windows_NT') {
    throw 'The native LeanRows idle-memory harness requires Windows.'
}

function ConvertTo-NativeQuotedArgument {
    param([Parameter(Mandatory = $true)][string]$Value)
    # Start the executable directly; never route a file path through a shell.
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Get-ProcessParentTable {
    @(
        Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId |
            ForEach-Object {
                [pscustomobject]@{
                    ProcessId = [int]$_.ProcessId
                    ParentProcessId = [int]$_.ParentProcessId
                }
            }
    )
}

function Update-KnownProcessTree {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.HashSet[int]]$KnownProcessIds
    )
    $table = @(Get-ProcessParentTable)
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($row in $table) {
            if ($KnownProcessIds.Contains([int]$row.ParentProcessId) -and
                -not $KnownProcessIds.Contains([int]$row.ProcessId)) {
                $null = $KnownProcessIds.Add([int]$row.ProcessId)
                $changed = $true
            }
        }
    }
}

function Get-MemoryPoint {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][datetime]$RootStartTime,
        [System.Diagnostics.Process]$ProcessObject
    )
    $target = $null
    $ownsTarget = $false
    try {
        if ($null -ne $ProcessObject) {
            $target = $ProcessObject
        }
        else {
            $target = Get-Process -Id $ProcessId -ErrorAction Stop
            $ownsTarget = $true
        }
        if ($target.HasExited) {
            return $null
        }
        $target.Refresh()
        if ($target.StartTime -lt $RootStartTime.AddSeconds(-1)) {
            return $null
        }
        [pscustomobject]@{
            process_id = $ProcessId
            private_bytes = [long]$target.PrivateMemorySize64
            working_set_bytes = [long]$target.WorkingSet64
        }
    }
    catch {
        return $null
    }
    finally {
        if ($ownsTarget -and $null -ne $target) {
            $target.Dispose()
        }
    }
}

function Get-TreeSample {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.HashSet[int]]$KnownProcessIds,
        [Parameter(Mandatory = $true)]
        [System.Diagnostics.Process]$RootProcess,
        [Parameter(Mandatory = $true)]
        [datetime]$RootStartTime,
        [Parameter(Mandatory = $true)]
        [long]$ElapsedMilliseconds
    )
    Update-KnownProcessTree -KnownProcessIds $KnownProcessIds
    $privateBytes = [long]0
    $workingSetBytes = [long]0
    $liveIds = New-Object 'System.Collections.Generic.List[int]'
    foreach ($knownId in @($KnownProcessIds)) {
        $processObject = if ($knownId -eq $RootProcess.Id) { $RootProcess } else { $null }
        $pointArguments = @{
            ProcessId = $knownId
            RootStartTime = $RootStartTime
            ProcessObject = $processObject
        }
        $point = Get-MemoryPoint @pointArguments
        if ($null -ne $point) {
            $privateBytes += [long]$point.private_bytes
            $workingSetBytes += [long]$point.working_set_bytes
            $liveIds.Add([int]$knownId)
        }
    }
    $observed = $liveIds.Count -gt 0 -and $privateBytes -gt 0 -and $workingSetBytes -gt 0
    [pscustomobject]@{
        elapsed_ms = $ElapsedMilliseconds
        process_count = $liveIds.Count
        memory_observed = $observed
        private_bytes = if ($observed) { $privateBytes } else { $null }
        working_set_bytes = if ($observed) { $workingSetBytes } else { $null }
        process_ids = $liveIds.ToArray()
    }
}

function Stop-KnownProcessTree {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.HashSet[int]]$KnownProcessIds
    )
    Update-KnownProcessTree -KnownProcessIds $KnownProcessIds
    foreach ($knownId in @($KnownProcessIds | Sort-Object -Descending)) {
        try {
            Stop-Process -Id $knownId -Force -ErrorAction Stop
        }
        catch {
            # An already-exited process is the expected shutdown race.
        }
    }
}

function Get-MatchingExecutableIds {
    param([Parameter(Mandatory = $true)][string]$ExecutablePath)
    $ids = New-Object 'System.Collections.Generic.List[int]'
    foreach ($candidate in @(Get-Process -ErrorAction SilentlyContinue)) {
        try {
            if ([string]::Equals(
                    [System.IO.Path]::GetFullPath($candidate.Path),
                    $ExecutablePath,
                    [System.StringComparison]::OrdinalIgnoreCase)) {
                $ids.Add([int]$candidate.Id)
            }
        }
        catch {
            # Protected or exiting processes may not expose Path.
        }
        finally {
            $candidate.Dispose()
        }
    }
    $ids.ToArray()
}

$executablePath = (Resolve-Path -LiteralPath $BinaryPath).Path
if (-not [System.IO.File]::Exists($executablePath)) {
    throw "LeanRows executable does not exist: $executablePath"
}
$resolvedDocument = $null
if (-not [string]::IsNullOrWhiteSpace($DocumentPath)) {
    $resolvedDocument = (Resolve-Path -LiteralPath $DocumentPath).Path
    if (-not [System.IO.File]::Exists($resolvedDocument)) {
        throw "Document does not exist: $resolvedDocument"
    }
}

$preexistingIds = @(Get-MatchingExecutableIds -ExecutablePath $executablePath)
$startInfo = New-Object System.Diagnostics.ProcessStartInfo
$startInfo.FileName = $executablePath
$startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($executablePath)
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $false
if ($null -ne $resolvedDocument) {
    $startInfo.Arguments = ConvertTo-NativeQuotedArgument -Value $resolvedDocument
}

$rootProcess = New-Object System.Diagnostics.Process
$rootProcess.StartInfo = $startInfo
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$knownIds = New-Object 'System.Collections.Generic.HashSet[int]'
$samples = New-Object 'System.Collections.Generic.List[object]'
$started = $false
$inputIdle = $false
$inputIdleMethod = 'not_reached'
$startupAttestedElapsedMs = $null
$shutdownMethod = 'not_started'
$residualIds = @()
$rootStartTime = $null

try {
    if (-not $rootProcess.Start()) {
        throw "Failed to start '$executablePath'."
    }
    $started = $true
    $rootStartTime = $rootProcess.StartTime
    $null = $knownIds.Add([int]$rootProcess.Id)
    try {
        $inputIdle = $rootProcess.WaitForInputIdle($StartupTimeoutSeconds * 1000)
        if ($inputIdle) {
            $inputIdleMethod = 'wait_for_input_idle'
        }
    }
    catch {
        $inputIdle = $false
    }
    if (-not $inputIdle) {
        $startupDeadline = [DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
        do {
            if ($rootProcess.HasExited) {
                break
            }
            $rootProcess.Refresh()
            if ($rootProcess.MainWindowHandle -ne [IntPtr]::Zero -and $rootProcess.Responding) {
                $inputIdle = $true
                $inputIdleMethod = 'responsive_main_window'
                break
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $startupDeadline)
    }
    if (-not $inputIdle -or $rootProcess.HasExited) {
        throw 'LeanRows did not reach a live Windows input-idle state before the startup timeout.'
    }
    $startupAttestedElapsedMs = [long]$stopwatch.ElapsedMilliseconds
    if ($SettleMilliseconds -gt 0) {
        Start-Sleep -Milliseconds $SettleMilliseconds
    }
    if ($rootProcess.HasExited) {
        throw 'LeanRows exited before the idle-memory observation window.'
    }
    $sampleStart = $stopwatch.ElapsedMilliseconds
    do {
        $sampleArguments = @{
            KnownProcessIds = $knownIds
            RootProcess = $rootProcess
            RootStartTime = $rootStartTime
            ElapsedMilliseconds = $stopwatch.ElapsedMilliseconds
        }
        $samples.Add((Get-TreeSample @sampleArguments))
        Start-Sleep -Milliseconds $SampleIntervalMilliseconds
    } while (($stopwatch.ElapsedMilliseconds - $sampleStart) -lt $SampleDurationMilliseconds)
}
finally {
    if ($started) {
        try {
            Update-KnownProcessTree -KnownProcessIds $knownIds
        }
        catch { }
        try {
            if (-not $rootProcess.HasExited -and $rootProcess.CloseMainWindow()) {
                if ($rootProcess.WaitForExit($ShutdownTimeoutSeconds * 1000)) {
                    $shutdownMethod = 'wm_close'
                }
            }
        }
        catch { }
        if ($shutdownMethod -ne 'wm_close') {
            try {
                Stop-KnownProcessTree -KnownProcessIds $knownIds
                $shutdownMethod = 'forced_known_tree'
            }
            catch {
                $shutdownMethod = 'forced_known_tree_failed'
            }
        }
        Start-Sleep -Milliseconds 100
        $residual = New-Object 'System.Collections.Generic.HashSet[int]'
        foreach ($knownId in @($knownIds)) {
            try {
                $candidate = Get-Process -Id $knownId -ErrorAction Stop
                if ($candidate.StartTime -ge $rootStartTime.AddSeconds(-1)) {
                    $null = $residual.Add([int]$knownId)
                }
                $candidate.Dispose()
            }
            catch { }
        }
        foreach ($candidateId in @(Get-MatchingExecutableIds -ExecutablePath $executablePath)) {
            if ($preexistingIds -notcontains $candidateId) {
                $null = $residual.Add([int]$candidateId)
            }
        }
        $residualIds = @($residual | Sort-Object)
    }
    $stopwatch.Stop()
    $rootProcess.Dispose()
}

$observedSamples = @($samples | Where-Object { $_.memory_observed })
$peakPrivate = $null
$peakWorkingSet = $null
if ($observedSamples.Count -gt 0) {
    $peakPrivate = [long](($observedSamples | Measure-Object -Property private_bytes -Maximum).Maximum)
    $peakWorkingSet = [long](($observedSamples | Measure-Object -Property working_set_bytes -Maximum).Maximum)
}
$memoryStatus = if ($observedSamples.Count -gt 0) { 'observed' } else { 'missing' }
$report = [ordered]@{
    schema_version = 'leanrows_idle_memory.v1'
    measured_at_utc = [DateTime]::UtcNow.ToString('o')
    executable_path = $executablePath
    executable_sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    document_path = $resolvedDocument
    input_idle_reached = $inputIdle
    input_idle_method = $inputIdleMethod
    startup_attested_elapsed_ms = $startupAttestedElapsedMs
    settings = [ordered]@{
        settle_ms = $SettleMilliseconds
        sample_duration_ms = $SampleDurationMilliseconds
        sample_interval_ms = $SampleIntervalMilliseconds
        startup_timeout_seconds = $StartupTimeoutSeconds
        shutdown_timeout_seconds = $ShutdownTimeoutSeconds
    }
    memory_observation_status = $memoryStatus
    memory_observation_note = if ($memoryStatus -eq 'observed') {
        'At least one complete live process-tree sample had positive private-byte and working-set totals.'
    }
    else {
        'No positive complete-tree sample was captured. Peaks are null and must not be interpreted as zero.'
    }
    sample_count = $samples.Count
    observed_sample_count = $observedSamples.Count
    process_tree_private_bytes_peak = $peakPrivate
    process_tree_working_set_bytes_peak = $peakWorkingSet
    process_tree_commit_bytes_status = 'not_observed'
    shutdown_method = $shutdownMethod
    residual_process_count = $residualIds.Count
    residual_process_ids = $residualIds
    samples = $samples.ToArray()
}

if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
    $parentDirectory = [System.IO.Path]::GetDirectoryName($resolvedOutput)
    if (-not [string]::IsNullOrWhiteSpace($parentDirectory)) {
        $null = [System.IO.Directory]::CreateDirectory($parentDirectory)
    }
    [System.IO.File]::WriteAllText(
        $resolvedOutput,
        (($report | ConvertTo-Json -Depth 10) + [Environment]::NewLine),
        $script:Utf8NoBom
    )
}
[pscustomobject]$report
