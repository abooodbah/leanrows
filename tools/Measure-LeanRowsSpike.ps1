[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string[]]$FixturePath,
    [string]$ArgumentTemplate = '--scan {fixture}',
    [string]$WorkingDirectory,
    [string]$OutputPath = (Join-Path (Get-Location) 'leanrows-spike-benchmark.json'),
    [int]$Runs = 1,
    [int]$SampleIntervalMilliseconds = 100,
    [int]$TimeoutSeconds = 300,
    [switch]$IncludeSamples
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

if ($ArgumentTemplate.IndexOf('{fixture}', [System.StringComparison]::Ordinal) -lt 0) {
    throw 'ArgumentTemplate must contain the literal placeholder {fixture}.'
}
if ($Runs -lt 1 -or $Runs -gt 1000) {
    throw 'Runs must be between 1 and 1000.'
}
if ($SampleIntervalMilliseconds -lt 10 -or $SampleIntervalMilliseconds -gt 60000) {
    throw 'SampleIntervalMilliseconds must be between 10 and 60000.'
}
if ($TimeoutSeconds -lt 1 -or $TimeoutSeconds -gt 86400) {
    throw 'TimeoutSeconds must be between 1 and 86400.'
}

function ConvertTo-NativeQuotedArgument {
    param([Parameter(Mandatory = $true)][string]$Value)

    # Fixture paths are files rather than trailing directory separators. This is
    # sufficient for ProcessStartInfo.Arguments without invoking a command shell.
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Get-ProcessParentTable {
    if ($env:OS -eq 'Windows_NT') {
        return @(Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId |
            ForEach-Object {
                [pscustomobject]@{
                    ProcessId = [int]$_.ProcessId
                    ParentProcessId = [int]$_.ParentProcessId
                }
            })
    }

    if (Test-Path -LiteralPath '/proc') {
        $rows = New-Object 'System.Collections.Generic.List[object]'
        foreach ($directory in Get-ChildItem -LiteralPath '/proc' -Directory -ErrorAction SilentlyContinue) {
            if ($directory.Name -notmatch '^\d+$') {
                continue
            }

            $statPath = Join-Path $directory.FullName 'stat'
            try {
                $line = [System.IO.File]::ReadAllText($statPath)
                $closeParen = $line.LastIndexOf(')')
                if ($closeParen -lt 0 -or ($closeParen + 2) -ge $line.Length) {
                    continue
                }
                $pid = [int]$directory.Name
                $rest = $line.Substring($closeParen + 2).Split(@(' '), [System.StringSplitOptions]::RemoveEmptyEntries)
                if ($rest.Length -lt 2) {
                    continue
                }
                $rows.Add([pscustomobject]@{ ProcessId = $pid; ParentProcessId = [int]$rest[1] })
            } catch {
                # Processes may disappear while /proc is being enumerated.
            }
        }
        return $rows.ToArray()
    }

    if (Test-Path -LiteralPath '/bin/ps') {
        $rows = New-Object 'System.Collections.Generic.List[object]'
        foreach ($line in & /bin/ps -axo 'pid=,ppid=') {
            if ($line -match '^\s*(\d+)\s+(\d+)\s*$') {
                $rows.Add([pscustomobject]@{ ProcessId = [int]$Matches[1]; ParentProcessId = [int]$Matches[2] })
            }
        }
        return $rows.ToArray()
    }

    throw 'No dependency-free process-parent discovery route is available on this platform.'
}

function Update-KnownProcessTree {
    param(
        [Parameter(Mandatory = $true)][System.Collections.Generic.HashSet[int]]$KnownProcessIds
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

function Get-ProcessMemoryPoint {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][datetime]$RootStartTime,
        [System.Diagnostics.Process]$ProcessObject
    )

    $process = $null
    $ownsProcess = $false
    try {
        if ($null -ne $ProcessObject) {
            $process = $ProcessObject
        } else {
            $process = Get-Process -Id $ProcessId -ErrorAction Stop
            $ownsProcess = $true
        }

        if ($process.HasExited) {
            return $null
        }
        $process.Refresh()
        if ($process.StartTime -lt $RootStartTime.AddSeconds(-1)) {
            return $null
        }

        return [pscustomobject]@{
            process_id = $ProcessId
            private_bytes = [long]$process.PrivateMemorySize64
            working_set_bytes = [long]$process.WorkingSet64
        }
    } catch {
        return $null
    } finally {
        if ($ownsProcess -and $null -ne $process) {
            $process.Dispose()
        }
    }
}

function Get-TreeMemorySample {
    param(
        [Parameter(Mandatory = $true)][System.Collections.Generic.HashSet[int]]$KnownProcessIds,
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$RootProcess,
        [Parameter(Mandatory = $true)][datetime]$RootStartTime,
        [Parameter(Mandatory = $true)][long]$ElapsedMilliseconds
    )

    $privateBytes = [long]0
    $workingSetBytes = [long]0
    $liveIds = New-Object 'System.Collections.Generic.List[int]'
    $attemptedIds = New-Object 'System.Collections.Generic.HashSet[int]'

    # Sample the root from the Process object returned by Start() before the
    # slower process-table walk. This preserves a measurement for short-lived
    # commands that may exit during Win32_Process/CIM enumeration.
    $rootId = [int]$RootProcess.Id
    $null = $attemptedIds.Add($rootId)
    $rootPoint = Get-ProcessMemoryPoint -ProcessId $rootId -RootStartTime $RootStartTime -ProcessObject $RootProcess
    if ($null -ne $rootPoint) {
        $privateBytes += [long]$rootPoint.private_bytes
        $workingSetBytes += [long]$rootPoint.working_set_bytes
        $liveIds.Add($rootId)
    }

    # Previously discovered descendants are also sampled before rediscovery.
    foreach ($processId in $KnownProcessIds) {
        if (-not $attemptedIds.Add([int]$processId)) {
            continue
        }
        $point = Get-ProcessMemoryPoint -ProcessId ([int]$processId) -RootStartTime $RootStartTime
        if ($null -ne $point) {
            $privateBytes += [long]$point.private_bytes
            $workingSetBytes += [long]$point.working_set_bytes
            $liveIds.Add([int]$processId)
        }
    }

    Update-KnownProcessTree -KnownProcessIds $KnownProcessIds

    # Sample only newly discovered descendants so the root and known children
    # are never counted twice in one process-tree total.
    foreach ($processId in $KnownProcessIds) {
        if (-not $attemptedIds.Add([int]$processId)) {
            continue
        }
        $point = Get-ProcessMemoryPoint -ProcessId ([int]$processId) -RootStartTime $RootStartTime
        if ($null -ne $point) {
            $privateBytes += [long]$point.private_bytes
            $workingSetBytes += [long]$point.working_set_bytes
            $liveIds.Add([int]$processId)
        }
    }

    $memoryObserved = $liveIds.Count -gt 0 -and $privateBytes -gt 0L -and $workingSetBytes -gt 0L
    return [pscustomobject]@{
        elapsed_ms = $ElapsedMilliseconds
        process_count = $liveIds.Count
        memory_observed = $memoryObserved
        private_bytes = if ($memoryObserved) { $privateBytes } else { $null }
        working_set_bytes = if ($memoryObserved) { $workingSetBytes } else { $null }
        process_ids = $liveIds.ToArray()
    }
}

function Stop-KnownProcessTree {
    param([Parameter(Mandatory = $true)][System.Collections.Generic.HashSet[int]]$KnownProcessIds)

    # Reverse PID order is not a dependency rule, but descendants are also
    # terminated independently, so a parent exiting first cannot orphan work.
    foreach ($processId in @($KnownProcessIds | Sort-Object -Descending)) {
        try {
            Stop-Process -Id $processId -Force -ErrorAction Stop
        } catch {
            # Already-exited processes are expected here.
        }
    }
}

function Invoke-SpikeRun {
    param(
        [string]$ExecutablePath,
        [string]$Fixture,
        [string]$Arguments,
        [string]$RunWorkingDirectory,
        [int]$RunNumber
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $ExecutablePath
    $startInfo.Arguments = $Arguments
    $startInfo.WorkingDirectory = $RunWorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $false
    $startInfo.RedirectStandardError = $false

    $rootProcess = New-Object System.Diagnostics.Process
    $rootProcess.StartInfo = $startInfo
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    if (-not $rootProcess.Start()) {
        throw "Failed to start '$ExecutablePath'."
    }

    $rootStartTime = $rootProcess.StartTime
    $knownIds = New-Object 'System.Collections.Generic.HashSet[int]'
    $null = $knownIds.Add([int]$rootProcess.Id)
    $samples = New-Object 'System.Collections.Generic.List[object]'
    $peakPrivateBytes = [long]0
    $peakWorkingSetBytes = [long]0
    $peakProcessCount = 0
    $sampleCount = 0
    $memoryObservedSampleCount = 0
    $emptySamplesAfterRootExit = 0
    $timedOut = $false
    $processElapsedSeconds = $null

    try {
        while ($true) {
            $sample = Get-TreeMemorySample -KnownProcessIds $knownIds -RootProcess $rootProcess `
                -RootStartTime $rootStartTime -ElapsedMilliseconds $stopwatch.ElapsedMilliseconds
            $sampleCount++
            if ($sample.memory_observed) {
                $memoryObservedSampleCount++
                if ($sample.private_bytes -gt $peakPrivateBytes) { $peakPrivateBytes = [long]$sample.private_bytes }
                if ($sample.working_set_bytes -gt $peakWorkingSetBytes) { $peakWorkingSetBytes = [long]$sample.working_set_bytes }
            }
            if ($sample.process_count -gt $peakProcessCount) { $peakProcessCount = [int]$sample.process_count }
            if ($IncludeSamples) { $samples.Add($sample) }

            $rootExited = $rootProcess.HasExited
            if ($rootExited -and $sample.process_count -eq 0) {
                $emptySamplesAfterRootExit++
            } else {
                $emptySamplesAfterRootExit = 0
            }
            if ($rootExited -and $emptySamplesAfterRootExit -ge 2) {
                break
            }

            if ($stopwatch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                $timedOut = $true
                Stop-KnownProcessTree -KnownProcessIds $knownIds
                break
            }

            Start-Sleep -Milliseconds $SampleIntervalMilliseconds
        }
    } finally {
        $stopwatch.Stop()
    }

    $exitCode = $null
    try {
        if (-not $rootProcess.HasExited) {
            $null = $rootProcess.WaitForExit(5000)
        }
        if ($rootProcess.HasExited) {
            $exitCode = [int]$rootProcess.ExitCode
            $candidateProcessSeconds = [double](
                $rootProcess.ExitTime.ToUniversalTime() - $rootStartTime.ToUniversalTime()
            ).TotalSeconds
            if ($candidateProcessSeconds -ge 0.0) {
                $processElapsedSeconds = $candidateProcessSeconds
            }
        }
    } catch { }

    $rootProcess.Dispose()
    $fixtureBytes = [long](Get-Item -LiteralPath $Fixture).Length
    $harnessObservationSeconds = [double]$stopwatch.Elapsed.TotalSeconds
    $inputBytesDividedByElapsed = if ($null -ne $processElapsedSeconds -and $processElapsedSeconds -gt 0.0) {
        [double]$fixtureBytes / $processElapsedSeconds
    } else {
        $null
    }
    $memoryObservationStatus = if ($memoryObservedSampleCount -gt 0) { 'observed' } else { 'missing' }

    return [ordered]@{
        fixture = $Fixture
        fixture_bytes = $fixtureBytes
        fixture_sha256 = (Get-FileHash -LiteralPath $Fixture -Algorithm SHA256).Hash.ToLowerInvariant()
        run = $RunNumber
        arguments = $Arguments
        exit_code = $exitCode
        timed_out = $timedOut
        process_elapsed_seconds = if ($null -eq $processElapsedSeconds) { $null } else { [Math]::Round($processElapsedSeconds, 6) }
        harness_observation_seconds = [Math]::Round($harnessObservationSeconds, 6)
        input_bytes_divided_by_process_elapsed_bps = if ($null -eq $inputBytesDividedByElapsed) { $null } else { [Math]::Round($inputBytesDividedByElapsed, 3) }
        sample_interval_ms = $SampleIntervalMilliseconds
        sample_count = $sampleCount
        memory_observed_sample_count = $memoryObservedSampleCount
        memory_observation_status = $memoryObservationStatus
        memory_observation_note = if ($memoryObservationStatus -eq 'observed') {
            'At least one instantaneous complete-tree sample returned positive private-byte and working-set totals.'
        } else {
            'The process tree exited before a positive memory sample was captured. Peaks are null and must not be interpreted as zero.'
        }
        discovered_process_count_peak = $peakProcessCount
        process_tree_private_bytes_peak = if ($memoryObservedSampleCount -gt 0) { $peakPrivateBytes } else { $null }
        process_tree_working_set_bytes_peak = if ($memoryObservedSampleCount -gt 0) { $peakWorkingSetBytes } else { $null }
        samples = if ($IncludeSamples) { $samples.ToArray() } else { $null }
    }
}

$executablePath = (Resolve-Path -LiteralPath $Executable).Path
if (-not [System.IO.File]::Exists($executablePath)) {
    throw "Executable does not exist: $executablePath"
}

if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) {
    $WorkingDirectory = Split-Path -Parent $executablePath
}
$resolvedWorkingDirectory = (Resolve-Path -LiteralPath $WorkingDirectory).Path

$resolvedFixtures = New-Object 'System.Collections.Generic.List[string]'
foreach ($fixture in $FixturePath) {
    $resolved = (Resolve-Path -LiteralPath $fixture).Path
    if (-not [System.IO.File]::Exists($resolved)) {
        throw "Fixture is not a file: $resolved"
    }
    $resolvedFixtures.Add($resolved)
}

$resultRows = New-Object 'System.Collections.Generic.List[object]'
foreach ($fixture in $resolvedFixtures) {
    $quotedFixture = ConvertTo-NativeQuotedArgument -Value $fixture
    $arguments = $ArgumentTemplate.Replace('{fixture}', $quotedFixture)
    for ($run = 1; $run -le $Runs; $run++) {
        Write-Verbose "Running fixture '$fixture' ($run of $Runs)."
        $resultRows.Add((Invoke-SpikeRun -ExecutablePath $executablePath -Fixture $fixture `
            -Arguments $arguments -RunWorkingDirectory $resolvedWorkingDirectory -RunNumber $run))
    }
}

$hostInfo = [ordered]@{
    os_description = [System.Environment]::OSVersion.VersionString
    os_architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    process_architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    logical_processor_count = [System.Environment]::ProcessorCount
    powershell_version = $PSVersionTable.PSVersion.ToString()
    tree_discovery = if ($env:OS -eq 'Windows_NT') { 'Win32_Process via CIM' } elseif (Test-Path '/proc') { '/proc/<pid>/stat' } else { '/bin/ps' }
}

$report = [ordered]@{
    schema_version = 2
    harness = 'tools/Measure-LeanRowsSpike.ps1'
    measured_at_utc = [DateTime]::UtcNow.ToString('o')
    disclaimer = 'Raw observations only. input_bytes_divided_by_process_elapsed_bps is fixture size divided by target process lifetime; it does not prove that every byte was read and is not a performance claim. Harness observation time is reported separately.'
    executable = [ordered]@{
        path = $executablePath
        sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    settings = [ordered]@{
        argument_template = $ArgumentTemplate
        working_directory = $resolvedWorkingDirectory
        runs_per_fixture = $Runs
        sample_interval_ms = $SampleIntervalMilliseconds
        timeout_seconds = $TimeoutSeconds
        includes_sample_series = [bool]$IncludeSamples
    }
    host = $hostInfo
    observations = $resultRows.ToArray()
}

$resolvedOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$outputDirectory = [System.IO.Path]::GetDirectoryName($resolvedOutputPath)
if (-not [string]::IsNullOrWhiteSpace($outputDirectory)) {
    $null = [System.IO.Directory]::CreateDirectory($outputDirectory)
}
[System.IO.File]::WriteAllText(
    $resolvedOutputPath,
    (($report | ConvertTo-Json -Depth 10) + "`n"),
    $script:Utf8NoBom
)

[pscustomobject]@{
    OutputPath = $resolvedOutputPath
    ObservationCount = $resultRows.Count
}
