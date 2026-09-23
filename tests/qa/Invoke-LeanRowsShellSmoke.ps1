[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [ValidateRange(5, 120)][int]$TimeoutSeconds = 30,
    [ValidateRange(2, 30)][int]$DialogTimeoutSeconds = 10
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$usage = 'usage: leanrows [--smoke-test | --document-smoke-test] [--] [FILE...]'

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) {
        throw $Message
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

function Invoke-RedirectedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$Arguments,
        [Parameter(Mandatory = $true)][int]$Timeout
    )
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($Executable)
    $startInfo.Arguments = $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $target = New-Object System.Diagnostics.Process
    $target.StartInfo = $startInfo
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $started = $false
    try {
        if (-not $target.Start()) {
            throw "Failed to start $Executable."
        }
        $started = $true
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
        if ($script:Utf8NoBom.GetByteCount($stdout) -gt 65536 -or
            $script:Utf8NoBom.GetByteCount($stderr) -gt 65536) {
            throw 'Native shell smoke output exceeded the 64 KiB capture limit.'
        }
        [pscustomobject]@{
            exit_code = if ($target.HasExited) { [int]$target.ExitCode } else { $null }
            timed_out = $timedOut
            elapsed_ms = [long]$watch.ElapsedMilliseconds
            stdout = $stdout
            stderr = $stderr
        }
    }
    finally {
        if ($started -and -not $target.HasExited) {
            try { Stop-Process -Id $target.Id -Force -ErrorAction Stop } catch { }
        }
        $target.Dispose()
    }
}

function Test-OrdinaryUsageDialog {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][int]$Timeout
    )
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($Executable)
    $startInfo.Arguments = '--not-an-option'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $target = New-Object System.Diagnostics.Process
    $target.StartInfo = $startInfo
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $started = $false
    $dialogObserved = $false
    $dialogTitle = $null
    try {
        if (-not $target.Start()) {
            throw "Failed to start $Executable for ordinary-dialog verification."
        }
        $started = $true
        $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
        do {
            if ($target.HasExited) {
                break
            }
            $target.Refresh()
            if ($target.MainWindowHandle -ne [IntPtr]::Zero) {
                $dialogTitle = [string]$target.MainWindowTitle
                if ($dialogTitle -eq 'LeanRows') {
                    $dialogObserved = $true
                    break
                }
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $deadline)

        Assert-True -Condition $dialogObserved `
            -Message 'Ordinary usage failure did not expose a visible LeanRows dialog.'
        Assert-True -Condition $target.CloseMainWindow() `
            -Message 'Ordinary usage dialog did not accept WM_CLOSE.'
        Assert-True -Condition $target.WaitForExit(5000) `
            -Message 'LeanRows did not exit after its ordinary usage dialog closed.'
        Assert-True -Condition ([int]$target.ExitCode -eq 2) `
            -Message "Ordinary usage failure exited with code $($target.ExitCode), expected 2."
        $watch.Stop()
        [pscustomobject]@{
            dialog_observed = $dialogObserved
            dialog_title = $dialogTitle
            exit_code = [int]$target.ExitCode
            elapsed_ms = [long]$watch.ElapsedMilliseconds
        }
    }
    finally {
        if ($started -and -not $target.HasExited) {
            try { Stop-Process -Id $target.Id -Force -ErrorAction Stop } catch { }
            $null = $target.WaitForExit(5000)
        }
        $target.Dispose()
    }
}

function Wait-MainWindowTitle {
    param(
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Target,
        [Parameter(Mandatory = $true)][string]$Title,
        [Parameter(Mandatory = $true)][int]$Timeout
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
    do {
        if ($Target.HasExited) {
            return $false
        }
        $Target.Refresh()
        if ([string]$Target.MainWindowTitle -eq $Title) {
            return $true
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    $false
}

function Invoke-ForwardingLaunch {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$Document,
        [Parameter(Mandatory = $true)][int]$Timeout
    )
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($Executable)
    $startInfo.Arguments = '"' + $Document + '"'
    $startInfo.UseShellExecute = $false
    $launch = New-Object System.Diagnostics.Process
    $launch.StartInfo = $startInfo
    try {
        if (-not $launch.Start()) {
            throw "Failed to start $Executable for single-window verification."
        }
        if (-not $launch.WaitForExit($Timeout * 1000)) {
            try { Stop-Process -Id $launch.Id -Force -ErrorAction Stop } catch { }
            throw 'A second launch kept running instead of handing its file to the open window.'
        }
        [int]$launch.ExitCode
    }
    finally {
        $launch.Dispose()
    }
}

# A second ordinary launch hands its file to the running window and exits, so
# the file opens as a tab. Opening a file that is already open selects its tab.
function Test-SingleWindowForwarding {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][int]$Timeout
    )
    Assert-True -Condition (@(Get-MatchingExecutableIds -Executable $Executable).Count -eq 0) `
        -Message 'Close LeanRows windows started from this binary before the single-window check.'
    $directory = Join-Path ([System.IO.Path]::GetTempPath()) ('leanrows-forwarding-' + [guid]::NewGuid().ToString('N'))
    [System.IO.Directory]::CreateDirectory($directory) | Out-Null
    $first = Join-Path $directory 'first.csv'
    $second = Join-Path $directory 'second.csv'
    [System.IO.File]::WriteAllText($first, "name,value`nalpha,1`n", $script:Utf8NoBom)
    [System.IO.File]::WriteAllText($second, "name,value`nbeta,2`n", $script:Utf8NoBom)
    $dash = [string][char]0x2014
    $firstTitle = "first.csv $dash LeanRows"
    $secondTitle = "second.csv $dash LeanRows"

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = [System.IO.Path]::GetDirectoryName($Executable)
    $startInfo.Arguments = '"' + $first + '"'
    $startInfo.UseShellExecute = $false
    $primary = New-Object System.Diagnostics.Process
    $primary.StartInfo = $startInfo
    $started = $false
    try {
        if (-not $primary.Start()) {
            throw "Failed to start $Executable for single-window verification."
        }
        $started = $true
        Assert-True -Condition (Wait-MainWindowTitle -Target $primary -Title $firstTitle -Timeout $Timeout) `
            -Message 'The first launch did not show first.csv.'

        $secondExit = Invoke-ForwardingLaunch -Executable $Executable -Document $second -Timeout $Timeout
        Assert-True -Condition ($secondExit -eq 0) `
            -Message "The forwarding launch exited with code $secondExit, expected 0."
        Assert-True -Condition (Wait-MainWindowTitle -Target $primary -Title $secondTitle -Timeout $Timeout) `
            -Message 'The running window did not open the forwarded file in a new tab.'

        $repeatExit = Invoke-ForwardingLaunch -Executable $Executable -Document $first -Timeout $Timeout
        Assert-True -Condition ($repeatExit -eq 0) `
            -Message "The repeat launch exited with code $repeatExit, expected 0."
        Assert-True -Condition (Wait-MainWindowTitle -Target $primary -Title $firstTitle -Timeout $Timeout) `
            -Message 'Opening an already open file did not select its tab.'

        $running = @(Get-MatchingExecutableIds -Executable $Executable)
        Assert-True -Condition ($running.Count -eq 1 -and $running[0] -eq $primary.Id) `
            -Message "Expected one LeanRows process after forwarding, found $($running.Count)."

        Assert-True -Condition $primary.CloseMainWindow() `
            -Message 'The tabbed window did not accept WM_CLOSE.'
        Assert-True -Condition $primary.WaitForExit($Timeout * 1000) `
            -Message 'LeanRows did not exit after closing a window with two tabs.'
        Assert-True -Condition ([int]$primary.ExitCode -eq 0) `
            -Message "The tabbed window exited with code $($primary.ExitCode), expected 0."
        [pscustomobject]@{
            forwarded_exit_code = $secondExit
            repeat_exit_code = $repeatExit
            process_count = $running.Count
            exit_code = [int]$primary.ExitCode
        }
    }
    finally {
        if ($started -and -not $primary.HasExited) {
            try { Stop-Process -Id $primary.Id -Force -ErrorAction Stop } catch { }
            $null = $primary.WaitForExit(5000)
        }
        $primary.Dispose()
        Remove-Item -LiteralPath $directory -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$binary = (Resolve-Path -LiteralPath $BinaryPath).Path
Assert-True -Condition ([System.IO.File]::Exists($binary)) `
    -Message "LeanRows binary does not exist: $binary"
$beforeIds = @(Get-MatchingExecutableIds -Executable $binary)

$success = Invoke-RedirectedProcess `
    -Executable $binary -Arguments '--smoke-test' -Timeout $TimeoutSeconds
Assert-True -Condition (-not $success.timed_out) -Message 'Native shell smoke timed out.'
$successDiagnostic = if ([string]::IsNullOrWhiteSpace($success.stderr)) {
    '<empty>'
}
else {
    $success.stderr.Trim()
}
Assert-True -Condition ($success.exit_code -eq 0) `
    -Message "Native shell smoke exited with code $($success.exit_code); stderr: $successDiagnostic"
Assert-True -Condition ([string]::IsNullOrWhiteSpace($success.stdout)) `
    -Message 'Successful native shell smoke wrote unexpected stdout.'
Assert-True -Condition ([string]::IsNullOrWhiteSpace($success.stderr)) `
    -Message 'Successful native shell smoke wrote unexpected stderr.'

$conflict = Invoke-RedirectedProcess `
    -Executable $binary `
    -Arguments '--smoke-test --document-smoke-test' `
    -Timeout $TimeoutSeconds
Assert-True -Condition (-not $conflict.timed_out) `
    -Message 'Conflicting automation flags opened a dialog or timed out.'
Assert-True -Condition ($conflict.exit_code -eq 2) `
    -Message "Conflicting automation flags exited with code $($conflict.exit_code), expected 2."
Assert-True -Condition ([string]::IsNullOrWhiteSpace($conflict.stdout)) `
    -Message 'Conflicting automation flags wrote unexpected stdout.'
Assert-True -Condition ($conflict.stderr.Trim() -eq $usage) `
    -Message 'Conflicting automation flags did not preserve the usage diagnostic on stderr.'

$ordered = Invoke-RedirectedProcess `
    -Executable $binary `
    -Arguments 'first.csv second.csv --smoke-test' `
    -Timeout $TimeoutSeconds
Assert-True -Condition (-not $ordered.timed_out) `
    -Message 'Late automation flag opened a usage dialog or timed out.'
Assert-True -Condition ($ordered.exit_code -eq 2) `
    -Message "Late automation flag exited with code $($ordered.exit_code), expected 2."
Assert-True -Condition ([string]::IsNullOrWhiteSpace($ordered.stdout)) `
    -Message 'Late automation flag wrote unexpected stdout.'
Assert-True -Condition ($ordered.stderr.Trim() -eq $usage) `
    -Message 'Late automation flag did not preserve the usage diagnostic on stderr.'

$ordinary = Test-OrdinaryUsageDialog `
    -Executable $binary -Timeout $DialogTimeoutSeconds

$forwarding = Test-SingleWindowForwarding `
    -Executable $binary -Timeout $DialogTimeoutSeconds

Start-Sleep -Milliseconds 100
$newIds = @(
    Get-MatchingExecutableIds -Executable $binary |
        Where-Object { $beforeIds -notcontains $_ }
)
Assert-True -Condition ($newIds.Count -eq 0) `
    -Message "Native shell smoke left $($newIds.Count) LeanRows process(es) running."

[pscustomobject]@{
    schema_version = 'leanrows_shell_smoke_evidence.v1'
    executable_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
    shell_exit_code = $success.exit_code
    shell_elapsed_ms = $success.elapsed_ms
    conflict_exit_code = $conflict.exit_code
    conflict_stderr = $conflict.stderr.Trim()
    late_flag_exit_code = $ordered.exit_code
    late_flag_stderr = $ordered.stderr.Trim()
    ordinary_dialog_observed = $ordinary.dialog_observed
    ordinary_dialog_title = $ordinary.dialog_title
    ordinary_exit_code = $ordinary.exit_code
    forwarding_exit_code = $forwarding.forwarded_exit_code
    forwarding_repeat_exit_code = $forwarding.repeat_exit_code
    forwarding_process_count = $forwarding.process_count
    forwarding_window_exit_code = $forwarding.exit_code
    residual_process_count = $newIds.Count
}
