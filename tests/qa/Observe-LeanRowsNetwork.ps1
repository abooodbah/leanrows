[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [Parameter(Mandatory = $true)][string]$FixturePath,
    [ValidateRange(10, 5000)][int]$SampleIntervalMilliseconds = 50,
    [ValidateRange(250, 10000)][int]$MaximumSampleStartGapMilliseconds = 2500,
    [ValidateRange(5, 120)][int]$WorkflowTimeoutSeconds = 30,
    [ValidateRange(1, 30)][int]$ShutdownTimeoutSeconds = 5,
    [ValidateRange(1, 1000000)][int]$PreferredSeekIndex = 1024,
    [ValidateRange(1, 268435456)][long]$MaximumFixtureBytes = 67108864,
    [string]$OutputPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

if ($env:OS -ne 'Windows_NT') {
    throw 'The LeanRows network-observation gate requires Windows.'
}

if (-not ('LeanRowsQa.NativeNetworkObservation' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace LeanRowsQa {
    public static class NativeNetworkObservation {
        private delegate bool EnumWindowProc(IntPtr window, IntPtr parameter);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool EnumChildWindows(
            IntPtr parent,
            EnumWindowProc callback,
            IntPtr parameter);

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int GetClassName(
            IntPtr window,
            StringBuilder className,
            int maximumCount);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool SendMessageTimeout(
            IntPtr window,
            uint message,
            UIntPtr wParam,
            IntPtr lParam,
            uint flags,
            uint timeoutMilliseconds,
            out UIntPtr result);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool PostMessage(
            IntPtr window,
            uint message,
            UIntPtr wParam,
            IntPtr lParam);

        [DllImport("user32.dll")]
        private static extern bool IsWindow(IntPtr window);

        public static IntPtr FindChildByClass(IntPtr parent, string desiredClass) {
            IntPtr found = IntPtr.Zero;
            EnumChildWindows(parent, delegate(IntPtr candidate, IntPtr ignored) {
                StringBuilder name = new StringBuilder(128);
                int length = GetClassName(candidate, name, name.Capacity);
                if (length > 0 && String.Equals(
                    name.ToString(),
                    desiredClass,
                    StringComparison.OrdinalIgnoreCase)) {
                    found = candidate;
                    return false;
                }
                return true;
            }, IntPtr.Zero);
            return found;
        }

        public static bool TrySend(
            IntPtr window,
            uint message,
            UInt64 wParam,
            Int64 lParam,
            uint timeoutMilliseconds,
            out UInt64 result) {
            UIntPtr nativeResult;
            bool delivered = SendMessageTimeout(
                window,
                message,
                new UIntPtr(wParam),
                new IntPtr(lParam),
                0x0002,
                timeoutMilliseconds,
                out nativeResult);
            result = nativeResult.ToUInt64();
            return delivered;
        }

        public static bool TryPost(IntPtr window, uint message, UInt64 wParam) {
            return PostMessage(window, message, new UIntPtr(wParam), IntPtr.Zero);
        }

        public static bool IsLiveWindow(IntPtr window) {
            return IsWindow(window);
        }
    }
}
'@
}

function ConvertTo-NativeQuotedArgument {
    param([Parameter(Mandatory = $true)][string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Get-ProcessParentTable {
    @(
        Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId -ErrorAction Stop |
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
            # Protected and exiting processes need not expose their image path.
        }
        finally {
            $candidate.Dispose()
        }
    }
    $ids.ToArray()
}

function Get-LiveKnownProcessCount {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.HashSet[int]]$KnownProcessIds,
        [Parameter(Mandatory = $true)][datetime]$RootStartTime
    )
    $count = 0
    foreach ($knownId in @($KnownProcessIds)) {
        $candidate = $null
        try {
            $candidate = Get-Process -Id $knownId -ErrorAction Stop
            if (-not $candidate.HasExited -and
                $candidate.StartTime -ge $RootStartTime.AddSeconds(-1)) {
                $count++
            }
        }
        catch { }
        finally {
            if ($null -ne $candidate) {
                $candidate.Dispose()
            }
        }
    }
    return $count
}

function Stop-KnownProcessTree {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.HashSet[int]]$KnownProcessIds
    )
    try {
        Update-KnownProcessTree -KnownProcessIds $KnownProcessIds
    }
    catch { }
    foreach ($knownId in @($KnownProcessIds | Sort-Object -Descending)) {
        try {
            Stop-Process -Id $knownId -Force -ErrorAction Stop
        }
        catch {
            # An already-exited process is the expected cleanup race.
        }
    }
}

function Get-NetworkCountSample {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.Generic.HashSet[int]]$KnownProcessIds,
        [Parameter(Mandatory = $true)][long]$ElapsedMilliseconds,
        [Parameter(Mandatory = $true)][string]$WorkflowState
    )
    Update-KnownProcessTree -KnownProcessIds $KnownProcessIds

    # Endpoint objects exist only for this in-memory aggregation. Addresses,
    # ports, and endpoint-shaped objects are never written to the receipt.
    $tcp = @(
        Get-NetTCPConnection -ErrorAction Stop |
            Where-Object { $KnownProcessIds.Contains([int]$_.OwningProcess) }
    )
    $udp = @(
        Get-NetUDPEndpoint -ErrorAction Stop |
            Where-Object { $KnownProcessIds.Contains([int]$_.OwningProcess) }
    )
    $stateCounts = New-Object 'System.Collections.Generic.List[object]'
    foreach ($group in @($tcp | Group-Object -Property State | Sort-Object -Property Name)) {
        $stateCounts.Add([ordered]@{
            protocol = 'tcp'
            state = [string]$group.Name
            count = [int]$group.Count
        })
    }
    if ($udp.Count -gt 0) {
        $stateCounts.Add([ordered]@{
            protocol = 'udp'
            state = 'bound'
            count = [int]$udp.Count
        })
    }
    [pscustomobject]@{
        elapsed_ms = $ElapsedMilliseconds
        workflow_state = $WorkflowState
        discovered_process_count = $KnownProcessIds.Count
        tcp_connection_count = $tcp.Count
        udp_endpoint_count = $udp.Count
        protocol_state_counts = $stateCounts.ToArray()
    }
}

function Invoke-NativeMessage {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Window,
        [Parameter(Mandatory = $true)][uint32]$Message,
        [uint64]$WParam = 0,
        [int64]$LParam = 0,
        [ValidateRange(50, 5000)][int]$TimeoutMilliseconds = 1000
    )
    [uint64]$result = 0
    $delivered = [LeanRowsQa.NativeNetworkObservation]::TrySend(
        $Window,
        $Message,
        $WParam,
        $LParam,
        [uint32]$TimeoutMilliseconds,
        [ref]$result
    )
    if (-not $delivered) {
        throw 'The native message timed out or could not be delivered.'
    }
    return $result
}

function Test-LocalSyntheticFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][long]$MaximumBytes
    )
    if ($Path.StartsWith('\\', [System.StringComparison]::Ordinal)) {
        throw 'The network-observation fixture must not use a UNC path.'
    }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($item.PSIsContainer) {
        throw 'The network-observation fixture must be a regular file.'
    }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The network-observation fixture must not be a reparse point.'
    }
    if ([long]$item.Length -le 0 -or [long]$item.Length -gt $MaximumBytes) {
        throw 'The network-observation fixture is empty or exceeds the bounded synthetic-fixture limit.'
    }
    $root = [System.IO.Path]::GetPathRoot($item.FullName)
    $drive = New-Object System.IO.DriveInfo($root)
    if ($drive.DriveType -ne [System.IO.DriveType]::Fixed) {
        throw 'The network-observation fixture must be on a fixed local drive.'
    }

    $tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $fixtureDirectory = [System.IO.Path]::GetDirectoryName($item.FullName)
    $candidateRoots = @(
        $fixtureDirectory
        [System.IO.Path]::GetDirectoryName($fixtureDirectory)
    )
    $attestedRoot = $null
    foreach ($candidateRoot in $candidateRoots) {
        if (-not [string]::IsNullOrWhiteSpace($candidateRoot)) {
            $fullCandidate = [System.IO.Path]::GetFullPath($candidateRoot)
            $sentinel = Join-Path $fullCandidate '.support_tool_fixture_root'
            if ($fullCandidate.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
                [System.IO.File]::Exists($sentinel) -and
                [string]::Equals(
                    [System.IO.File]::ReadAllText($sentinel).Trim(),
                    'synthetic_self_test',
                    [System.StringComparison]::Ordinal)) {
                $attestedRoot = $fullCandidate
                break
            }
        }
    }
    if ($null -eq $attestedRoot) {
        throw 'The fixture must be inside a sentinel-marked disposable system-temporary root.'
    }
    return $item
}

function Test-LocalExecutable {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ($Path.StartsWith('\\', [System.StringComparison]::Ordinal)) {
        throw 'The supplied executable must not use a UNC path.'
    }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($item.PSIsContainer -or
        -not [string]::Equals($item.Extension, '.exe', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'The supplied executable must be a regular .exe file.'
    }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'The supplied executable must not be a reparse point.'
    }
    $root = [System.IO.Path]::GetPathRoot($item.FullName)
    $drive = New-Object System.IO.DriveInfo($root)
    if ($drive.DriveType -ne [System.IO.DriveType]::Fixed) {
        throw 'The supplied executable must be on a fixed local drive.'
    }
    return $item
}

$failures = New-Object 'System.Collections.Generic.List[string]'
$samples = New-Object 'System.Collections.Generic.List[object]'
$workflowState = 'preflight'
$binary = $null
$binaryHash = $null
$fixture = $null
$beforeHash = $null
$beforeLength = $null
$beforeWriteTicks = $null
$afterHash = $null
$afterLength = $null
$afterWriteTicks = $null
$rootProcess = $null
$rootStartTime = $null
$knownIds = New-Object 'System.Collections.Generic.HashSet[int]'
$preexistingIds = @()
$started = $false
$cleanClose = $false
$forcedCleanup = $false
$residualCount = $null
$mainWindow = [IntPtr]::Zero
$listWindow = [IntPtr]::Zero
$viewItemCount = $null
$seekTarget = $null
$seekTopBefore = $null
$seekTopAfter = $null
$seekPageCount = $null
$seekCompleted = $false
$reloadCommandDelivered = $false
$reloadEmptyViewObserved = $false
$reloadItemCount = $null
$reloadCompleted = $false
$maxDiscoveredProcessCount = 0
$networkObserved = $false
$samplingAvailable = $true
$syntheticFixtureAttested = $false
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$lastSampleStartedMs = $null
$maximumSampleStartGapMs = $null

try {
    foreach ($requiredCommand in @('Get-NetTCPConnection', 'Get-NetUDPEndpoint', 'Get-CimInstance')) {
        if ($null -eq (Get-Command $requiredCommand -ErrorAction SilentlyContinue)) {
            $failures.Add('required_sampling_cmdlet_unavailable')
            $samplingAvailable = $false
            break
        }
    }

    try {
        $binaryPathResolved = (Resolve-Path -LiteralPath $BinaryPath -ErrorAction Stop).Path
        $binaryItem = Test-LocalExecutable -Path $binaryPathResolved
        $binary = $binaryItem.FullName
        $binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
        $fixturePathResolved = (Resolve-Path -LiteralPath $FixturePath -ErrorAction Stop).Path
        $fixture = Test-LocalSyntheticFixture -Path $fixturePathResolved -MaximumBytes $MaximumFixtureBytes
        $syntheticFixtureAttested = $true
        $beforeLength = [long]$fixture.Length
        $beforeWriteTicks = [long]$fixture.LastWriteTimeUtc.Ticks
        $beforeHash = (Get-FileHash -LiteralPath $fixture.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    catch {
        $failures.Add('local_fixture_or_binary_preflight_failed')
    }

    if ($samplingAvailable -and $null -ne $binary -and $null -ne $fixture) {
        try {
            # A successful preflight enumeration proves both local cmdlets can
            # execute before the application is launched. No endpoint fields
            # from this host-wide preflight are retained.
            $null = @(Get-NetTCPConnection -ErrorAction Stop).Count
            $null = @(Get-NetUDPEndpoint -ErrorAction Stop).Count
            $null = @(Get-ProcessParentTable).Count
        }
        catch {
            $failures.Add('network_sampling_preflight_failed')
            $samplingAvailable = $false
        }
    }

    if ($samplingAvailable -and $null -ne $binary -and $null -ne $fixture) {
        $preexistingIds = @(Get-MatchingExecutableIds -ExecutablePath $binary)
        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = $binary
        $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($binary)
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.Arguments = ConvertTo-NativeQuotedArgument -Value $fixture.FullName

        $rootProcess = New-Object System.Diagnostics.Process
        $rootProcess.StartInfo = $startInfo
        if (-not $rootProcess.Start()) {
            throw 'The LeanRows process could not be started.'
        }
        $started = $true
        $rootStartTime = $rootProcess.StartTime
        $null = $knownIds.Add([int]$rootProcess.Id)
        $workflowState = 'opening'

        $deadline = [DateTime]::UtcNow.AddSeconds($WorkflowTimeoutSeconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            $sampleStartedMs = [long]$stopwatch.ElapsedMilliseconds
            if ($null -ne $lastSampleStartedMs) {
                $gap = [long]($sampleStartedMs - $lastSampleStartedMs)
                if ($null -eq $maximumSampleStartGapMs -or $gap -gt $maximumSampleStartGapMs) {
                    $maximumSampleStartGapMs = $gap
                }
                if ($gap -gt $MaximumSampleStartGapMilliseconds) {
                    $samplingAvailable = $false
                    $failures.Add('sampling_gap_exceeded_bound')
                    break
                }
            }
            $lastSampleStartedMs = $sampleStartedMs
            try {
                $sample = Get-NetworkCountSample -KnownProcessIds $knownIds -ElapsedMilliseconds $sampleStartedMs -WorkflowState $workflowState
                $samples.Add($sample)
                if ([int]$sample.discovered_process_count -gt $maxDiscoveredProcessCount) {
                    $maxDiscoveredProcessCount = [int]$sample.discovered_process_count
                }
                if ([int]$sample.tcp_connection_count -gt 0 -or [int]$sample.udp_endpoint_count -gt 0) {
                    $networkObserved = $true
                    $failures.Add('tcp_or_udp_activity_observed')
                    break
                }
            }
            catch {
                $samplingAvailable = $false
                $failures.Add('network_or_process_tree_sample_failed')
                break
            }

            if ($rootProcess.HasExited) {
                if ($workflowState -eq 'closing') {
                    try {
                        Update-KnownProcessTree -KnownProcessIds $knownIds
                    }
                    catch {
                        $samplingAvailable = $false
                        $failures.Add('final_process_tree_discovery_failed')
                        break
                    }
                    if ((Get-LiveKnownProcessCount -KnownProcessIds $knownIds -RootStartTime $rootStartTime) -eq 0) {
                        $cleanClose = $true
                    }
                    else {
                        $failures.Add('descendant_process_remained_after_clean_close')
                    }
                }
                else {
                    $failures.Add('application_exited_before_workflow_completed')
                }
                break
            }
            $rootProcess.Refresh()
            if ($mainWindow -eq [IntPtr]::Zero) {
                $mainWindow = $rootProcess.MainWindowHandle
                if ($mainWindow -ne [IntPtr]::Zero) {
                    $workflowState = 'viewing'
                }
            }
            if ($mainWindow -ne [IntPtr]::Zero -and $listWindow -eq [IntPtr]::Zero) {
                $listWindow = [LeanRowsQa.NativeNetworkObservation]::FindChildByClass(
                    $mainWindow,
                    'SysListView32'
                )
            }

            if ($listWindow -ne [IntPtr]::Zero -and $null -eq $viewItemCount) {
                $itemCount = [long](Invoke-NativeMessage -Window $listWindow -Message 0x1004)
                if ($itemCount -gt 1) {
                    $pageCount = [long](Invoke-NativeMessage -Window $listWindow -Message 0x1028)
                    if ($pageCount -gt 0 -and $itemCount -gt ($pageCount + 1)) {
                        $viewItemCount = $itemCount
                        $seekPageCount = $pageCount
                        $workflowState = 'view_observed'
                        continue
                    }
                }
            }

            if ($workflowState -eq 'view_observed' -and -not $seekCompleted) {
                $seekTarget = [long][Math]::Min([long]$PreferredSeekIndex, [long]$viewItemCount - 1)
                if ($seekTarget -le [long]$seekPageCount) {
                    $failures.Add('fixture_did_not_expose_a_scrollable_native_seek')
                    break
                }
                $seekTopBefore = [long](Invoke-NativeMessage -Window $listWindow -Message 0x1027)
                $ensureResult = Invoke-NativeMessage -Window $listWindow -Message 0x1013 -WParam ([uint64]$seekTarget)
                if ($ensureResult -eq 0) {
                    $failures.Add('native_list_seek_was_rejected')
                    break
                }
                $seekTopAfter = [long](Invoke-NativeMessage -Window $listWindow -Message 0x1027)
                if ($seekTopAfter -le $seekTopBefore -or
                    $seekTarget -lt $seekTopAfter -or
                    $seekTarget -ge ($seekTopAfter + [long]$seekPageCount + 1)) {
                    $failures.Add('native_list_seek_was_not_observed')
                    break
                }
                $seekCompleted = $true
                $workflowState = 'seek_observed'
                continue
            }

            if ($workflowState -eq 'seek_observed') {
                try {
                    # WM_COMMAND / File > Reload (ID 101) is delivered
                    # synchronously. The handler clears the virtual list before
                    # queuing fresh worker work, making the action observable.
                    $null = Invoke-NativeMessage -Window $mainWindow -Message 0x0111 -WParam 101
                    $reloadCommandDelivered = $true
                    $clearedCount = [long](Invoke-NativeMessage -Window $listWindow -Message 0x1004)
                    $reloadEmptyViewObserved = $clearedCount -eq 0
                    if (-not $reloadEmptyViewObserved) {
                        $failures.Add('reload_did_not_clear_the_native_view')
                        break
                    }
                    $workflowState = 'reloading'
                    continue
                }
                catch {
                    $failures.Add('reload_command_was_unavailable')
                    break
                }
            }

            if ($workflowState -eq 'reloading') {
                $reloadedCount = [long](Invoke-NativeMessage -Window $listWindow -Message 0x1004)
                if ($reloadedCount -gt 0) {
                    $reloadItemCount = $reloadedCount
                    $reloadCompleted = $true
                    $workflowState = 'reload_observed'
                }
            }
            elseif ($workflowState -eq 'reload_observed') {
                # Require a complete post-reload network sample before closing.
                if (-not [LeanRowsQa.NativeNetworkObservation]::TryPost($mainWindow, 0x0010, 0)) {
                    $failures.Add('clean_close_message_was_unavailable')
                    break
                }
                $workflowState = 'closing'
            }
            elseif ($workflowState -eq 'closing') {
                if ($rootProcess.WaitForExit(0)) {
                    try {
                        Update-KnownProcessTree -KnownProcessIds $knownIds
                    }
                    catch {
                        $samplingAvailable = $false
                        $failures.Add('final_process_tree_discovery_failed')
                        break
                    }
                    if ((Get-LiveKnownProcessCount -KnownProcessIds $knownIds -RootStartTime $rootStartTime) -eq 0) {
                        $cleanClose = $true
                        break
                    }
                }
            }

            $sampleElapsed = [long]($stopwatch.ElapsedMilliseconds - $sampleStartedMs)
            $remainingDelay = [long]$SampleIntervalMilliseconds - $sampleElapsed
            if ($remainingDelay -gt 0) {
                Start-Sleep -Milliseconds ([int]$remainingDelay)
            }
        }

        if (-not $seekCompleted) {
            $failures.Add('native_seek_workflow_incomplete')
        }
        if (-not $reloadCompleted) {
            $failures.Add('reload_workflow_incomplete')
        }
        if (-not $cleanClose) {
            $failures.Add('clean_shutdown_not_observed')
        }
    }
}
catch {
    $failures.Add('unexpected_local_runtime_failure')
}
finally {
    if ($started -and $null -ne $rootProcess) {
        try {
            Update-KnownProcessTree -KnownProcessIds $knownIds
        }
        catch {
            if (-not $failures.Contains('final_process_tree_discovery_failed')) {
                $failures.Add('final_process_tree_discovery_failed')
            }
        }
        if (-not $rootProcess.HasExited) {
            if ($mainWindow -ne [IntPtr]::Zero -and
                [LeanRowsQa.NativeNetworkObservation]::IsLiveWindow($mainWindow)) {
                $null = [LeanRowsQa.NativeNetworkObservation]::TryPost($mainWindow, 0x0010, 0)
                try {
                    $null = $rootProcess.WaitForExit($ShutdownTimeoutSeconds * 1000)
                }
                catch { }
            }
        }
        if (-not $rootProcess.HasExited -or
            (Get-LiveKnownProcessCount -KnownProcessIds $knownIds -RootStartTime $rootStartTime) -gt 0) {
            Stop-KnownProcessTree -KnownProcessIds $knownIds
            $forcedCleanup = $true
            Start-Sleep -Milliseconds 100
        }
        $residual = New-Object 'System.Collections.Generic.HashSet[int]'
        foreach ($knownId in @($knownIds)) {
            $candidate = $null
            try {
                $candidate = Get-Process -Id $knownId -ErrorAction Stop
                if ($candidate.StartTime -ge $rootStartTime.AddSeconds(-1)) {
                    $null = $residual.Add([int]$knownId)
                }
            }
            catch { }
            finally {
                if ($null -ne $candidate) {
                    $candidate.Dispose()
                }
            }
        }
        foreach ($candidateId in @(Get-MatchingExecutableIds -ExecutablePath $binary)) {
            if ($preexistingIds -notcontains $candidateId) {
                $null = $residual.Add([int]$candidateId)
            }
        }
        $residualCount = $residual.Count
        if ($residualCount -ne 0) {
            $failures.Add('residual_leanrows_process_observed')
        }
        if ($forcedCleanup) {
            $failures.Add('forced_process_cleanup_was_required')
        }
        $rootProcess.Dispose()
    }

    if ($null -ne $fixture) {
        try {
            $afterItem = Get-Item -LiteralPath $fixture.FullName -Force -ErrorAction Stop
            $afterLength = [long]$afterItem.Length
            $afterWriteTicks = [long]$afterItem.LastWriteTimeUtc.Ticks
            $afterHash = (Get-FileHash -LiteralPath $fixture.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($afterLength -ne $beforeLength -or
                $afterWriteTicks -ne $beforeWriteTicks -or
                $afterHash -ne $beforeHash) {
                $failures.Add('source_fixture_changed')
            }
        }
        catch {
            $failures.Add('source_fixture_postcheck_failed')
        }
    }
    $stopwatch.Stop()
}

$tcpTotal = [long]0
$udpTotal = [long]0
foreach ($sample in $samples.ToArray()) {
    $tcpTotal += [long]$sample.tcp_connection_count
    $udpTotal += [long]$sample.udp_endpoint_count
}
if ($samples.Count -lt 5) {
    $failures.Add('insufficient_complete_network_samples')
}
$uniqueFailures = @($failures | Select-Object -Unique)
$verdict = if ($uniqueFailures.Count -eq 0 -and
    $samplingAvailable -and
    $samples.Count -ge 5 -and
    -not $networkObserved -and
    $seekCompleted -and
    $reloadCompleted -and
    $cleanClose -and
    $residualCount -eq 0) { 'pass' } else { 'fail_closed' }

$report = [ordered]@{
    schema_version = 'leanrows_network_observation.v1'
    generated_at_utc = [DateTime]::UtcNow.ToString('o')
    verdict = $verdict
    failure_reasons = $uniqueFailures
    method = [ordered]@{
        mode = 'local_polling'
        cmdlets = @('Get-NetTCPConnection', 'Get-NetUDPEndpoint')
        disclosure = 'High-frequency local polling of the complete discovered LeanRows process tree. This is not ETW packet or event capture and cannot prove the absence of activity between samples.'
        raw_endpoints_persisted = $false
        endpoint_addresses_persisted = $false
        endpoint_ports_persisted = $false
    }
    settings = [ordered]@{
        requested_sample_interval_ms = $SampleIntervalMilliseconds
        maximum_sample_start_gap_ms = $MaximumSampleStartGapMilliseconds
        workflow_timeout_seconds = $WorkflowTimeoutSeconds
        shutdown_timeout_seconds = $ShutdownTimeoutSeconds
    }
    sampling = [ordered]@{
        available = $samplingAvailable
        sample_count = $samples.Count
        maximum_sample_start_gap_ms = $maximumSampleStartGapMs
        maximum_discovered_process_count = $maxDiscoveredProcessCount
        observed_protocol_counts = @(
            [ordered]@{ protocol = 'tcp'; count = $tcpTotal }
            [ordered]@{ protocol = 'udp'; count = $udpTotal }
        )
        samples = $samples.ToArray()
    }
    actions = [ordered]@{
        open_view_observed = $null -ne $viewItemCount
        native_item_count_observed = $viewItemCount
        native_seek_completed = $seekCompleted
        native_seek_target = $seekTarget
        native_seek_top_before = $seekTopBefore
        native_seek_top_after = $seekTopAfter
        reload_command_delivered = $reloadCommandDelivered
        reload_empty_view_observed = $reloadEmptyViewObserved
        reload_completed = $reloadCompleted
        reloaded_native_item_count = $reloadItemCount
        clean_close_observed = $cleanClose
    }
    source = [ordered]@{
        synthetic_disposable_fixture_attested = $syntheticFixtureAttested
        bytes_before = $beforeLength
        bytes_after = $afterLength
        sha256_before = $beforeHash
        sha256_after = $afterHash
        last_write_ticks_before = $beforeWriteTicks
        last_write_ticks_after = $afterWriteTicks
        unchanged = ($null -ne $beforeHash -and
            $beforeHash -eq $afterHash -and
            $beforeLength -eq $afterLength -and
            $beforeWriteTicks -eq $afterWriteTicks)
    }
    process_tree = [ordered]@{
        executable_sha256 = $binaryHash
        launch_use_shell_execute = $false
        launch_create_no_window = $true
        forced_cleanup_required = $forcedCleanup
        residual_process_count = $residualCount
    }
    duration_ms = [long]$stopwatch.ElapsedMilliseconds
}

if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
    $parentDirectory = [System.IO.Path]::GetDirectoryName($resolvedOutput)
    if (-not [string]::IsNullOrWhiteSpace($parentDirectory)) {
        $null = [System.IO.Directory]::CreateDirectory($parentDirectory)
    }
    [System.IO.File]::WriteAllText(
        $resolvedOutput,
        (($report | ConvertTo-Json -Depth 15) + [Environment]::NewLine),
        $script:Utf8NoBom
    )
}

if ($verdict -ne 'pass') {
    throw 'LeanRows network observation failed closed; inspect the status-only receipt for reason classes.'
}
[pscustomobject]$report
