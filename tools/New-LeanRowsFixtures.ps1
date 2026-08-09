[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'tests/fixtures/generated'),
    [long]$CsvRows = 100000,
    [long]$CsvMinimumBytes = 0,
    [int]$CsvPayloadBytes = 32,
    [long]$JsonlRows = 100000,
    [long]$JsonlMinimumBytes = 0,
    [int]$JsonlPayloadBytes = 32,
    [long]$LogRows = 100000,
    [long]$LogMinimumBytes = 0,
    [int]$LogPayloadBytes = 32,
    [long]$GiantRecordPayloadBytes = 67108864,
    [int]$Seed = 1729,
    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:InvariantCulture = [System.Globalization.CultureInfo]::InvariantCulture
$script:WriterBufferChars = 65536

function Assert-Range {
    param(
        [string]$Name,
        [long]$Value,
        [long]$Minimum,
        [long]$Maximum
    )

    if ($Value -lt $Minimum -or $Value -gt $Maximum) {
        throw "$Name must be between $Minimum and $Maximum; received $Value."
    }
}

function New-HashedUtf8Writer {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Overwrite
    )

    $mode = if ($Overwrite) { [System.IO.FileMode]::Create } else { [System.IO.FileMode]::CreateNew }
    $stream = New-Object System.IO.FileStream(
        $Path,
        $mode,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::Read,
        131072,
        [System.IO.FileOptions]::SequentialScan
    )

    return [pscustomobject]@{
        Path = $Path
        Stream = $stream
        Hash = [System.Security.Cryptography.SHA256]::Create()
        Buffer = New-Object System.Text.StringBuilder($script:WriterBufferChars)
        WrittenBytes = [long]0
        AcceptedBytes = [long]0
        Completed = $false
    }
}

function Write-HashedBytes {
    param(
        [Parameter(Mandatory = $true)]$Writer,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )

    if ($Bytes.Length -eq 0) {
        return
    }

    $Writer.Stream.Write($Bytes, 0, $Bytes.Length)
    $null = $Writer.Hash.TransformBlock($Bytes, 0, $Bytes.Length, $Bytes, 0)
    $Writer.WrittenBytes += [long]$Bytes.Length
}

function Flush-HashedWriter {
    param([Parameter(Mandatory = $true)]$Writer)

    if ($Writer.Buffer.Length -eq 0) {
        return
    }

    $text = $Writer.Buffer.ToString()
    $null = $Writer.Buffer.Clear()
    $bytes = $script:Utf8NoBom.GetBytes($text)
    Write-HashedBytes -Writer $Writer -Bytes $bytes
}

function Add-HashedText {
    param(
        [Parameter(Mandatory = $true)]$Writer,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text
    )

    if ($Text.Length -eq 0) {
        return
    }

    $Writer.AcceptedBytes += [long]$script:Utf8NoBom.GetByteCount($Text)

    if ($Text.Length -ge $script:WriterBufferChars) {
        Flush-HashedWriter -Writer $Writer
        Write-HashedBytes -Writer $Writer -Bytes $script:Utf8NoBom.GetBytes($Text)
        return
    }

    $null = $Writer.Buffer.Append($Text)
    if ($Writer.Buffer.Length -ge $script:WriterBufferChars) {
        Flush-HashedWriter -Writer $Writer
    }
}

function Complete-HashedWriter {
    param([Parameter(Mandatory = $true)]$Writer)

    if ($Writer.Completed) {
        throw "Writer for '$($Writer.Path)' has already been completed."
    }

    Flush-HashedWriter -Writer $Writer
    $null = $Writer.Hash.TransformFinalBlock([byte[]]@(), 0, 0)
    $Writer.Stream.Flush()
    $Writer.Stream.Dispose()

    $sha256 = [System.BitConverter]::ToString($Writer.Hash.Hash).Replace('-', '').ToLowerInvariant()
    $Writer.Hash.Dispose()
    $Writer.Completed = $true

    if ($Writer.WrittenBytes -ne $Writer.AcceptedBytes) {
        throw "Writer byte accounting failed for '$($Writer.Path)': accepted $($Writer.AcceptedBytes), wrote $($Writer.WrittenBytes)."
    }

    return [pscustomobject]@{
        Bytes = [long]$Writer.WrittenBytes
        Sha256 = $sha256
    }
}

function Close-HashedWriterSilently {
    param($Writer)

    if ($null -eq $Writer -or $Writer.Completed) {
        return
    }

    try { $Writer.Stream.Dispose() } catch { }
    try { $Writer.Hash.Dispose() } catch { }
}

function New-AsciiFiller {
    param(
        [int]$Length,
        [long]$Row,
        [int]$FixtureSalt
    )

    if ($Length -eq 0) {
        return ''
    }

    $letterIndex = [int](($Row + [long]$Seed + $FixtureSalt) % 26)
    $letter = [char](97 + $letterIndex)
    return ([string]$letter) * $Length
}

function New-FixtureEntry {
    param(
        [string]$Name,
        [string]$Format,
        $Result,
        [long]$LogicalRecords,
        [long]$DataRecords,
        [string[]]$Features,
        [System.Collections.IDictionary]$Requested
    )

    return [ordered]@{
        name = $Name
        format = $Format
        bytes = [long]$Result.Bytes
        sha256 = $Result.Sha256
        logical_records = $LogicalRecords
        data_records = $DataRecords
        features = $Features
        requested = $Requested
    }
}

Assert-Range -Name 'CsvRows' -Value $CsvRows -Minimum 0 -Maximum 1000000000000
Assert-Range -Name 'CsvMinimumBytes' -Value $CsvMinimumBytes -Minimum 0 -Maximum ([long]::MaxValue)
Assert-Range -Name 'CsvPayloadBytes' -Value $CsvPayloadBytes -Minimum 0 -Maximum 1048576
Assert-Range -Name 'JsonlRows' -Value $JsonlRows -Minimum 0 -Maximum 1000000000000
Assert-Range -Name 'JsonlMinimumBytes' -Value $JsonlMinimumBytes -Minimum 0 -Maximum ([long]::MaxValue)
Assert-Range -Name 'JsonlPayloadBytes' -Value $JsonlPayloadBytes -Minimum 0 -Maximum 1048576
Assert-Range -Name 'LogRows' -Value $LogRows -Minimum 0 -Maximum 1000000000000
Assert-Range -Name 'LogMinimumBytes' -Value $LogMinimumBytes -Minimum 0 -Maximum ([long]::MaxValue)
Assert-Range -Name 'LogPayloadBytes' -Value $LogPayloadBytes -Minimum 0 -Maximum 1048576
Assert-Range -Name 'GiantRecordPayloadBytes' -Value $GiantRecordPayloadBytes -Minimum 1 -Maximum ([long]::MaxValue)
Assert-Range -Name 'Seed' -Value $Seed -Minimum 0 -Maximum ([int]::MaxValue)

$resolvedOutput = [System.IO.Path]::GetFullPath($OutputDirectory)
$null = [System.IO.Directory]::CreateDirectory($resolvedOutput)

$fixtureNames = @(
    'large-multiline.csv',
    'large.jsonl',
    'large.log',
    'malformed.csv',
    'giant-record.csv',
    'manifest.json',
    'manifest.sha256'
)

if (-not $Force) {
    $existing = @($fixtureNames | Where-Object { [System.IO.File]::Exists((Join-Path $resolvedOutput $_)) })
    if ($existing.Count -gt 0) {
        throw "Output files already exist. Use -Force to replace only these generated files: $($existing -join ', ')"
    }
}

$fixtures = New-Object 'System.Collections.Generic.List[object]'

# CSV: deterministic time-series rows. Every 97th row contains a quoted CRLF.
$csvName = 'large-multiline.csv'
$csvPath = Join-Path $resolvedOutput $csvName
$writer = $null
try {
    $writer = New-HashedUtf8Writer -Path $csvPath -Overwrite ([bool]$Force)
    Add-HashedText -Writer $writer -Text "row_id,timestamp_ms,value_milli,phase,notes`r`n"
    $csvDataRows = [long]0
    $csvMultilineRows = [long]0

    while ($csvDataRows -lt $CsvRows -or $writer.AcceptedBytes -lt $CsvMinimumBytes) {
        $timestamp = 1704067200000L + ($csvDataRows * 1000L)
        $value = ((($csvDataRows % 1000000L) * 1103515245L) + [long]$Seed) % 1000000L
        $phase = $csvDataRows % 7L
        $filler = New-AsciiFiller -Length $CsvPayloadBytes -Row $csvDataRows -FixtureSalt 11

        if (($csvDataRows % 97L) -eq 0L) {
            $notes = "window $csvDataRows, `"stable`"`r`ncontinuation $csvDataRows $filler"
            $csvMultilineRows++
        } else {
            $notes = "window $csvDataRows, `"stable`" $filler"
        }

        $escapedNotes = $notes.Replace('"', '""')
        $rowText = [string]::Format(
            $script:InvariantCulture,
            '{0},{1},{2},phase-{3},"{4}"' + "`r`n",
            $csvDataRows,
            $timestamp,
            $value,
            $phase,
            $escapedNotes
        )
        Add-HashedText -Writer $writer -Text $rowText
        $csvDataRows++
    }

    $csvResult = Complete-HashedWriter -Writer $writer
    $fixtures.Add((New-FixtureEntry -Name $csvName -Format 'csv' -Result $csvResult `
        -LogicalRecords ($csvDataRows + 1L) -DataRecords $csvDataRows `
        -Features @('header', 'quoted-commas', 'escaped-quotes', 'quoted-crlf-records') `
        -Requested ([ordered]@{ minimum_data_rows = $CsvRows; minimum_bytes = $CsvMinimumBytes; payload_bytes_per_row = $CsvPayloadBytes; multiline_data_rows = $csvMultilineRows })))
} finally {
    Close-HashedWriterSilently -Writer $writer
}

# JSONL: each physical LF-terminated line is one valid top-level object.
$jsonlName = 'large.jsonl'
$jsonlPath = Join-Path $resolvedOutput $jsonlName
$writer = $null
try {
    $writer = New-HashedUtf8Writer -Path $jsonlPath -Overwrite ([bool]$Force)
    $jsonlDataRows = [long]0
    while ($jsonlDataRows -lt $JsonlRows -or $writer.AcceptedBytes -lt $JsonlMinimumBytes) {
        $timestamp = 1704067200000L + ($jsonlDataRows * 1000L)
        $value = ((($jsonlDataRows % 1000000L) * 1664525L) + [long]$Seed + 1013904223L) % 1000000L
        $phase = $jsonlDataRows % 7L
        $filler = New-AsciiFiller -Length $JsonlPayloadBytes -Row $jsonlDataRows -FixtureSalt 17
        $rowText = [string]::Format(
            $script:InvariantCulture,
            '{{"row_id":{0},"timestamp_ms":{1},"value_milli":{2},"phase":"phase-{3}","payload":"{4}"}}' + "`n",
            $jsonlDataRows,
            $timestamp,
            $value,
            $phase,
            $filler
        )
        Add-HashedText -Writer $writer -Text $rowText
        $jsonlDataRows++
    }

    $jsonlResult = Complete-HashedWriter -Writer $writer
    $fixtures.Add((New-FixtureEntry -Name $jsonlName -Format 'jsonl' -Result $jsonlResult `
        -LogicalRecords $jsonlDataRows -DataRecords $jsonlDataRows `
        -Features @('utf-8', 'lf-terminated', 'top-level-objects', 'stable-key-order') `
        -Requested ([ordered]@{ minimum_rows = $JsonlRows; minimum_bytes = $JsonlMinimumBytes; payload_bytes_per_row = $JsonlPayloadBytes })))
} finally {
    Close-HashedWriterSilently -Writer $writer
}

# Log: fixed-format ASCII rows with LF terminators.
$logName = 'large.log'
$logPath = Join-Path $resolvedOutput $logName
$writer = $null
try {
    $writer = New-HashedUtf8Writer -Path $logPath -Overwrite ([bool]$Force)
    $logDataRows = [long]0
    while ($logDataRows -lt $LogRows -or $writer.AcceptedBytes -lt $LogMinimumBytes) {
        $timestamp = 1704067200000L + ($logDataRows * 1000L)
        $value = ((($logDataRows % 1000000L) * 22695477L) + [long]$Seed + 1L) % 1000000L
        $level = @('TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR')[[int]($logDataRows % 5L)]
        $filler = New-AsciiFiller -Length $LogPayloadBytes -Row $logDataRows -FixtureSalt 23
        $rowText = [string]::Format(
            $script:InvariantCulture,
            'timestamp_ms={0} level={1} row_id={2} sensor=sensor-{3} value_milli={4} payload={5}' + "`n",
            $timestamp,
            $level,
            $logDataRows,
            ($logDataRows % 32L),
            $value,
            $filler
        )
        Add-HashedText -Writer $writer -Text $rowText
        $logDataRows++
    }

    $logResult = Complete-HashedWriter -Writer $writer
    $fixtures.Add((New-FixtureEntry -Name $logName -Format 'log' -Result $logResult `
        -LogicalRecords $logDataRows -DataRecords $logDataRows `
        -Features @('utf-8', 'lf-terminated', 'five-log-levels') `
        -Requested ([ordered]@{ minimum_rows = $LogRows; minimum_bytes = $LogMinimumBytes; payload_bytes_per_row = $LogPayloadBytes })))
} finally {
    Close-HashedWriterSilently -Writer $writer
}

# Malformed CSV: valid rows followed by width mismatch, stray quote, valid multiline, then an unterminated quote.
$malformedName = 'malformed.csv'
$malformedPath = Join-Path $resolvedOutput $malformedName
$writer = $null
try {
    $writer = New-HashedUtf8Writer -Path $malformedPath -Overwrite ([bool]$Force)
    $malformedText = @(
        "id,name,notes`r`n",
        "1,alpha,valid`r`n",
        "2,beta,extra,column`r`n",
        "3,ga`"mma,unescaped-quote`r`n",
        "4,delta,`"quoted`r`nmultiline`"`r`n",
        "5,epsilon,`"unterminated"
    )
    foreach ($piece in $malformedText) {
        Add-HashedText -Writer $writer -Text $piece
    }
    $malformedResult = Complete-HashedWriter -Writer $writer
    $fixtures.Add((New-FixtureEntry -Name $malformedName -Format 'csv' -Result $malformedResult `
        -LogicalRecords 6L -DataRecords 5L `
        -Features @('field-count-mismatch', 'unescaped-quote', 'valid-multiline-record', 'unterminated-final-quote') `
        -Requested ([ordered]@{ fixed_fixture = $true; recovery_policy = 'must-not-silently-invent-record-boundaries' })))
} finally {
    Close-HashedWriterSilently -Writer $writer
}

# Giant CSV: one valid logical row whose quoted payload is streamed in fixed chunks.
$giantName = 'giant-record.csv'
$giantPath = Join-Path $resolvedOutput $giantName
$writer = $null
try {
    $writer = New-HashedUtf8Writer -Path $giantPath -Overwrite ([bool]$Force)
    Add-HashedText -Writer $writer -Text "id,payload`r`n1,`""

    $remaining = [long]$GiantRecordPayloadBytes
    $lineInterval = [long]1048576
    $untilEmbeddedCrLf = $lineInterval
    $embeddedCrLfCount = [long]0

    while ($remaining -gt 0L) {
        if ($untilEmbeddedCrLf -eq 0L -and $remaining -ge 2L) {
            Add-HashedText -Writer $writer -Text "`r`n"
            $remaining -= 2L
            $untilEmbeddedCrLf = $lineInterval
            $embeddedCrLfCount++
            continue
        }

        $take = [Math]::Min([long]65536, $remaining)
        if ($untilEmbeddedCrLf -gt 0L) {
            $take = [Math]::Min($take, $untilEmbeddedCrLf)
        }
        if ($take -le 0L) {
            $take = [Math]::Min([long]1, $remaining)
        }

        Add-HashedText -Writer $writer -Text (('G') * [int]$take)
        $remaining -= $take
        if ($untilEmbeddedCrLf -gt 0L) {
            $untilEmbeddedCrLf -= $take
        }
    }

    Add-HashedText -Writer $writer -Text "`"`r`n"
    $giantResult = Complete-HashedWriter -Writer $writer
    $fixtures.Add((New-FixtureEntry -Name $giantName -Format 'csv' -Result $giantResult `
        -LogicalRecords 2L -DataRecords 1L `
        -Features @('single-giant-logical-record', 'quoted-payload', 'embedded-crlf', 'stream-generated') `
        -Requested ([ordered]@{ payload_bytes = $GiantRecordPayloadBytes; embedded_crlf_count = $embeddedCrLfCount; write_chunk_bytes = 65536 })))
} finally {
    Close-HashedWriterSilently -Writer $writer
}

$manifest = [ordered]@{
    schema_version = 1
    generator = 'tools/New-LeanRowsFixtures.ps1'
    deterministic_seed = $Seed
    encoding = 'UTF-8 without BOM'
    note = 'minimum byte targets may be exceeded by one complete logical record; actual byte counts and SHA-256 hashes are authoritative'
    fixtures = $fixtures.ToArray()
}

$manifestPath = Join-Path $resolvedOutput 'manifest.json'
$manifestJson = ($manifest | ConvertTo-Json -Depth 10) + "`n"
[System.IO.File]::WriteAllText($manifestPath, $manifestJson, $script:Utf8NoBom)
$manifestHash = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText(
    (Join-Path $resolvedOutput 'manifest.sha256'),
    "$manifestHash  manifest.json`n",
    $script:Utf8NoBom
)

$totalFixtureBytes = [long]0
foreach ($fixtureEntry in $fixtures) {
    $totalFixtureBytes += [long]$fixtureEntry['bytes']
}

[pscustomobject]@{
    OutputDirectory = $resolvedOutput
    Manifest = $manifestPath
    ManifestSha256 = $manifestHash
    FixtureCount = $fixtures.Count
    TotalFixtureBytes = $totalFixtureBytes
}
