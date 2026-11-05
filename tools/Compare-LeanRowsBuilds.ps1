[CmdletBinding()]
param(
    # Each directory is a built checkout: target/release holds leanrows.exe
    # and leanrows-spike.exe.
    [Parameter(Mandatory = $true)][string]$BaseDirectory,
    [Parameter(Mandatory = $true)][string]$HeadDirectory,
    # A CSV written by the benchmark workflow; it is opened for the working
    # set and, when -Sort is given, sorted by its first column.
    [Parameter(Mandatory = $true)][string]$FixturePath,
    [ValidateRange(1, 20)][int]$StartRuns = 5,
    [switch]$Sort,
    [ValidateRange(60, 7200)][int]$SortTimeoutSeconds = 3600,
    [string]$OutputPath
)

# Measures main and a branch the same way on one machine: executable size,
# start time, working set with a large file open, and, when asked, a
# whole-file sort through the core engine and through the app's worker.
# Runs alternate between the two builds so that drift on the runner falls on
# both alike.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false)
$measureIdle = Join-Path $PSScriptRoot '../tests/qa/Measure-LeanRowsIdle.ps1'

function Get-Build {
    param([Parameter(Mandatory = $true)][string]$Name, [Parameter(Mandatory = $true)][string]$Directory)
    $root = (Resolve-Path -LiteralPath $Directory).Path
    $app = Join-Path $root 'target/release/leanrows.exe'
    $spike = Join-Path $root 'target/release/leanrows-spike.exe'
    foreach ($path in @($app, $spike)) {
        if (-not [System.IO.File]::Exists($path)) { throw "$Name build is missing $path" }
    }
    [pscustomobject]@{
        name = $Name
        root = $root
        app = $app
        spike = $spike
        commit = (& git -C $root rev-parse HEAD).Trim()
        exe_bytes = (Get-Item -LiteralPath $app).Length
        exe_sha256 = (Get-FileHash -LiteralPath $app -Algorithm SHA256).Hash
    }
}

function Measure-Start {
    param([Parameter(Mandatory = $true)]$Build, [string]$Document)
    $arguments = @{
        BinaryPath = $Build.app
        SettleMilliseconds = 1000
        SampleDurationMilliseconds = 3000
        SampleIntervalMilliseconds = 100
        StartupTimeoutSeconds = 30
    }
    if ($Document) { $arguments.DocumentPath = $Document }
    $report = & $measureIdle @arguments
    if ($report.residual_process_count -ne 0) { throw "$($Build.name) left a process running." }
    [pscustomobject]@{
        start_ms = [long]$report.startup_attested_elapsed_ms
        working_set_peak = [long]$report.process_tree_working_set_bytes_peak
        private_peak = [long]$report.process_tree_private_bytes_peak
    }
}

function Measure-CoreSort {
    param([Parameter(Mandatory = $true)]$Build)
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Build.spike
    $info.Arguments = '--sort "' + $FixturePath + '"'
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    try {
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $deadline = [DateTime]::UtcNow.AddSeconds($SortTimeoutSeconds)
        $privatePeak = [long]0
        $workingPeak = [long]0
        while (-not $process.WaitForExit(50)) {
            if ([DateTime]::UtcNow -gt $deadline) { $process.Kill(); throw 'Core sort timed out.' }
            $process.Refresh()
            $privatePeak = [Math]::Max($privatePeak, $process.PrivateMemorySize64)
            $workingPeak = [Math]::Max($workingPeak, $process.WorkingSet64)
        }
        $process.WaitForExit()
        $text = $stdout.GetAwaiter().GetResult()
        $errorText = $stderr.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "$($Build.name) core sort failed: $errorText" }
        $result = $text | ConvertFrom-Json
        [pscustomobject]@{
            elapsed_ms = [long]$result.elapsed_ms
            rows = [long]$result.record_count
            managed_peak = [long]$result.managed_peak_bytes
            scratch_peak = [long]$result.scratch_peak_bytes
            private_peak = $privatePeak
            working_set_peak = $workingPeak
            first_rows = ($result.sample_source_rows -join ',')
        }
    }
    finally {
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
}

function Measure-WorkerSort {
    param([Parameter(Mandatory = $true)]$Build)
    $env:LEANROWS_SORT_BENCHMARK_FILE = $FixturePath
    try {
        Push-Location -LiteralPath $Build.root
        # Cargo reports progress on standard error; that is not a failure.
        $ErrorActionPreference = 'Continue'
        try {
            $lines = @(& cargo +1.97.1 test --release --locked -p leanrows-win32 --lib -- `
                    --ignored --exact worker::tests::sort_benchmark_from_environment --nocapture 2>&1)
            $exit = $LASTEXITCODE
        }
        finally {
            Pop-Location
            $ErrorActionPreference = 'Stop'
        }
    }
    finally { Remove-Item Env:LEANROWS_SORT_BENCHMARK_FILE -ErrorAction SilentlyContinue }
    $json = @($lines | ForEach-Object { "$_" } | Where-Object { $_ -match '^\{"schema":"leanrows.worker-sort-benchmark"' })
    if ($exit -ne 0) { throw "$($Build.name) worker sort failed: $($lines -join [Environment]::NewLine)" }
    if ($json.Count -ne 1) {
        # Builds from before the benchmark test existed report nothing.
        return $null
    }
    $result = $json[0] | ConvertFrom-Json
    [pscustomobject]@{
        elapsed_ms = [long]$result.elapsed_ms
        rows = [long]$result.rows
        managed_peak = [long]$result.managed_peak_bytes
        scratch_peak = [long]$result.scratch_peak_bytes
    }
}

function Get-Median {
    param([long[]]$Values)
    $sorted = @($Values | Sort-Object)
    if ($sorted.Count -eq 0) { return $null }
    $sorted[[int][Math]::Floor(($sorted.Count - 1) / 2)]
}

function Format-Range {
    param([long[]]$Values, [double]$Divisor = 1, [string]$Unit = '', [int]$Digits = 0)
    if (@($Values).Count -eq 0) { return 'n/a' }
    $format = '{0:N' + $Digits + '}'
    $median = $format -f ((Get-Median $Values) / $Divisor)
    $low = $format -f ((($Values | Measure-Object -Minimum).Minimum) / $Divisor)
    $high = $format -f ((($Values | Measure-Object -Maximum).Maximum) / $Divisor)
    "$median$Unit ($low to $high)"
}

$fixture = (Resolve-Path -LiteralPath $FixturePath).Path
$FixturePath = $fixture
$builds = @((Get-Build -Name 'main' -Directory $BaseDirectory), (Get-Build -Name 'branch' -Directory $HeadDirectory))
$results = @{}
foreach ($build in $builds) {
    $results[$build.name] = [ordered]@{
        cold = New-Object System.Collections.Generic.List[object]
        open = New-Object System.Collections.Generic.List[object]
        core = $null
        worker = $null
    }
}

for ($run = 0; $run -lt $StartRuns; $run++) {
    foreach ($build in $builds) {
        $results[$build.name].cold.Add((Measure-Start -Build $build))
        $results[$build.name].open.Add((Measure-Start -Build $build -Document $fixture))
    }
}

if ($Sort) {
    foreach ($build in $builds) {
        $results[$build.name].core = Measure-CoreSort -Build $build
    }
    $orders = @($builds | ForEach-Object { $results[$_.name].core.first_rows } | Sort-Object -Unique)
    if ($orders.Count -ne 1) { throw "The two builds sorted the fixture differently: $($orders -join ' / ')" }
    foreach ($build in $builds) {
        $results[$build.name].worker = Measure-WorkerSort -Build $build
    }
}

$fixtureInfo = Get-Item -LiteralPath $fixture
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('## Main and branch on one runner')
$lines.Add('')
$lines.Add(('Fixture: {0:N0} bytes, SHA-256 `{1}`. Start runs per build: {2}. Times are medians with the range in brackets.' -f `
            $fixtureInfo.Length, (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant(), $StartRuns))
$lines.Add('')
$lines.Add('| Measure | main | branch |')
$lines.Add('| --- | --- | --- |')
$lines.Add(('| Commit | `{0}` | `{1}` |' -f $builds[0].commit.Substring(0, 7), $builds[1].commit.Substring(0, 7)))
$lines.Add(('| Executable size | {0:N0} bytes | {1:N0} bytes |' -f $builds[0].exe_bytes, $builds[1].exe_bytes))
$row = {
    param([string]$Label, [scriptblock]$Pick, [double]$Divisor, [string]$Unit, [int]$Digits)
    $cells = foreach ($build in $builds) {
        $values = @(& $Pick $results[$build.name] | Where-Object { $null -ne $_ })
        Format-Range -Values ([long[]]$values) -Divisor $Divisor -Unit $Unit -Digits $Digits
    }
    $lines.Add("| $Label | $($cells[0]) | $($cells[1]) |")
}
& $row 'Start to input idle, no file' { param($r) $r.cold | ForEach-Object { $_.start_ms } } 1 ' ms' 0
& $row 'Peak working set, no file' { param($r) $r.cold | ForEach-Object { $_.working_set_peak } } 1MB ' MiB' 1
& $row 'Start to input idle, large file' { param($r) $r.open | ForEach-Object { $_.start_ms } } 1 ' ms' 0
& $row 'Peak working set, large file open' { param($r) $r.open | ForEach-Object { $_.working_set_peak } } 1MB ' MiB' 1
& $row 'Peak private bytes, large file open' { param($r) $r.open | ForEach-Object { $_.private_peak } } 1MB ' MiB' 1
if ($Sort) {
    & $row 'Sort, core engine' { param($r) $r.core.elapsed_ms } 1000 ' s' 1
    & $row 'Sort, core engine, managed peak' { param($r) $r.core.managed_peak } 1MB ' MiB' 2
    & $row 'Sort, core engine, scratch peak' { param($r) $r.core.scratch_peak } 1MB ' MiB' 0
    & $row 'Sort, core engine, process working set peak' { param($r) $r.core.working_set_peak } 1MB ' MiB' 1
    & $row 'Sort, app worker' { param($r) if ($r.worker) { $r.worker.elapsed_ms } } 1000 ' s' 1
    & $row 'Sort, app worker, managed peak' { param($r) if ($r.worker) { $r.worker.managed_peak } } 1MB ' MiB' 2
    $lines.Add(('| Rows sorted | {0:N0} | {1:N0} |' -f $results['main'].core.rows, $results['branch'].core.rows))
}
$markdown = $lines -join [Environment]::NewLine
Write-Host $markdown
if ($env:GITHUB_STEP_SUMMARY) {
    [System.IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $markdown + [Environment]::NewLine, $utf8)
}
if ($OutputPath) {
    $report = [ordered]@{
        schema = 'leanrows.build-comparison'
        version = 1
        fixture_bytes = $fixtureInfo.Length
        builds = $builds
        results = $results
    }
    [System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($OutputPath), ($report | ConvertTo-Json -Depth 8), $utf8)
}
