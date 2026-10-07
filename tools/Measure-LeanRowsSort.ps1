[CmdletBinding()]
param(
    [string]$Executable = (Join-Path $PSScriptRoot '../target/release/leanrows-spike.exe'),
    [int[]]$Rows = @(100000, 1000000),
    [string]$OutputPath = (Join-Path $PSScriptRoot '../artifacts/qa/sort-benchmark.json'),
    [int]$TimeoutSeconds = 300
)
$ErrorActionPreference = 'Stop'
$binary = [System.IO.Path]::GetFullPath($Executable)
$base = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$root = Join-Path $base ('leanrows-sort-benchmark-' + [guid]::NewGuid().ToString('N'))
$null = [System.IO.Directory]::CreateDirectory($root)
$sentinel = Join-Path $root '.sort_benchmark_owner'
[System.IO.File]::WriteAllText($sentinel, 'synthetic_sort_benchmark')
$utf8 = New-Object System.Text.UTF8Encoding($false)
$observations = New-Object System.Collections.Generic.List[object]
try {
    foreach ($count in $Rows) {
        if ($count -lt 8 -or $count -gt 20000000 -or $count % 97 -eq 0) {
            throw 'Rows must be 8..20000000 and coprime with 97.'
        }
        $fixture = Join-Path $root ("shuffled-$count.csv")
        $expected = New-Object 'long[]' 8
        $writer = New-Object System.IO.StreamWriter($fixture, $false, $utf8, 65536)
        try {
            for ($row = 0L; $row -lt $count; $row++) {
                $value = ($row * 97L + 13L) % $count
                if ($value -lt 8) { $expected[$value] = $row }
                $writer.WriteLine($value.ToString('D9') + ',row-' + $row.ToString('D9'))
            }
        } finally { $writer.Dispose() }
        $hash = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash
        foreach ($numeric in @($false, $true)) {
            $info = New-Object System.Diagnostics.ProcessStartInfo
            $info.FileName = $binary
            $info.Arguments = '--sort "' + $fixture + '"' + $(if ($numeric) { ' --numeric' } else { '' })
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
                $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
                $privatePeak = [long]0; $workingPeak = [long]0; $samples = 0
                while (-not $process.WaitForExit(20)) {
                    if ([DateTime]::UtcNow -gt $deadline) { $process.Kill(); throw 'Sort benchmark timed out.' }
                    $process.Refresh()
                    $privatePeak = [Math]::Max($privatePeak, $process.PrivateMemorySize64)
                    $workingPeak = [Math]::Max($workingPeak, $process.WorkingSet64)
                    $samples++
                }
                $text = $stdout.GetAwaiter().GetResult()
                $errorText = $stderr.GetAwaiter().GetResult()
                if ($process.ExitCode -ne 0) { throw "Sort benchmark failed: $errorText" }
                $result = $text | ConvertFrom-Json
                if ($result.record_count -ne $count -or
                    ($result.sample_source_rows -join ',') -ne ($expected -join ',')) {
                    throw 'Large sort did not match its independent fixture oracle.'
                }
                if ($result.managed_peak_bytes -gt 16MB -or $result.scratch_peak_bytes -gt 8GB) {
                    throw 'Large sort exceeded a configured resource limit.'
                }
                $observations.Add([pscustomobject]@{
                    rows = $count
                    mode = $(if ($numeric) { 'numeric' } else { 'text' })
                    elapsed_ms = $result.elapsed_ms
                    managed_peak_bytes = $result.managed_peak_bytes
                    scratch_peak_bytes = $result.scratch_peak_bytes
                    private_peak_bytes = $(if ($samples -gt 0) { $privatePeak } else { $null })
                    working_set_peak_bytes = $(if ($samples -gt 0) { $workingPeak } else { $null })
                    samples = $samples
                })
            } finally {
                if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
                $process.Dispose()
            }
        }
        if ((Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash -ne $hash) {
            throw 'Benchmark changed its source.'
        }
    }
    $receipt = [ordered]@{
        schema = 'leanrows.sort-benchmark-qa'
        version = 1
        scope = 'core external sort probe; process observations do not measure the full GUI'
        nonsparse = $true
        observations = @($observations.ToArray())
    }
    $output = [System.IO.Path]::GetFullPath($OutputPath)
    $null = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($output))
    [System.IO.File]::WriteAllText($output, ($receipt | ConvertTo-Json -Depth 5), $utf8)
    $observations.ToArray()
} finally {
    $resolved = [System.IO.Path]::GetFullPath($root)
    if (-not $resolved.StartsWith($base.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $sentinel) -or
        [System.IO.File]::ReadAllText($sentinel) -ne 'synthetic_sort_benchmark') {
        throw 'Refusing cleanup outside the owned benchmark fixture.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
