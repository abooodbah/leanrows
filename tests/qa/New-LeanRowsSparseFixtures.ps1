[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateCount(1, 8)][int[]]$SizesGiB = @(1, 10),
    [ValidateRange(1, 4096)][int]$OrdinaryRows = 256,
    [switch]$Force
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$sentinelName = '.leanrows_qa_fixture_root'

if ($env:OS -ne 'Windows_NT') {
    throw 'Sparse scale fixtures currently require Windows.'
}
foreach ($size in $SizesGiB) {
    if ($size -lt 1 -or $size -gt 100) {
        throw "Each sparse fixture size must be between 1 and 100 GiB; received $size."
    }
}
if (@($SizesGiB | Sort-Object -Unique).Count -ne $SizesGiB.Count) {
    throw 'Sparse fixture sizes must be unique.'
}

if ($null -eq ('LeanRows.Qa.SparseFileNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace LeanRows.Qa
{
    public static class SparseFileNative
    {
        private const uint FsctlSetSparse = 0x000900C4;

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool DeviceIoControl(
            SafeFileHandle device,
            uint controlCode,
            IntPtr input,
            uint inputSize,
            IntPtr output,
            uint outputSize,
            out uint returned,
            IntPtr overlapped);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint GetCompressedFileSizeW(string fileName, out uint high);

        public static void MarkSparse(SafeFileHandle handle)
        {
            uint returned;
            if (!DeviceIoControl(
                    handle,
                    FsctlSetSparse,
                    IntPtr.Zero,
                    0,
                    IntPtr.Zero,
                    0,
                    out returned,
                    IntPtr.Zero))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
        }

        public static long GetAllocatedBytes(string path)
        {
            uint high;
            uint low = GetCompressedFileSizeW(path, out high);
            if (low == 0xffffffff && high == 0)
            {
                int error = Marshal.GetLastWin32Error();
                if (error != 0)
                {
                    throw new Win32Exception(error);
                }
            }
            return checked((long)(((ulong)high << 32) | low));
        }
    }
}
'@
}

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        [System.BitConverter]::ToString($algorithm.ComputeHash($Bytes)).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Read-FileSlice {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][long]$Offset,
        [Parameter(Mandatory = $true)][int]$Count
    )
    $stream = [System.IO.File]::Open(
        $Path,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    try {
        $stream.Position = $Offset
        $bytes = New-Object byte[] $Count
        $read = $stream.Read($bytes, 0, $Count)
        if ($read -ne $Count) {
            throw "Short fixture slice read at offset $Offset; expected $Count bytes and read $read."
        }
        return $bytes
    }
    finally {
        $stream.Dispose()
    }
}

$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
$outputLeaf = [System.IO.Path]::GetFileName($outputRoot.TrimEnd('\'))
$sentinelPath = Join-Path $outputRoot $sentinelName
if ([System.IO.Directory]::Exists($outputRoot)) {
    $entries = @(Get-ChildItem -LiteralPath $outputRoot -Force)
    if ($entries.Count -gt 0 -and -not [System.IO.File]::Exists($sentinelPath)) {
        throw "Refusing a non-empty directory without $($sentinelName): $outputRoot"
    }
}
elseif (-not $outputLeaf.StartsWith('leanrows-qa-', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "A new sparse fixture directory must use the dedicated leanrows-qa-* prefix: $outputRoot"
}
$null = [System.IO.Directory]::CreateDirectory($outputRoot)
if (-not [System.IO.File]::Exists($sentinelPath)) {
    [System.IO.File]::WriteAllText(
        $sentinelPath,
        ('Disposable synthetic LeanRows QA fixture root.' + [Environment]::NewLine),
        $script:Utf8NoBom
    )
}

$prefixBuilder = New-Object System.Text.StringBuilder
for ($row = 0; $row -lt $OrdinaryRows; $row++) {
    $line = [string]::Format(
        [System.Globalization.CultureInfo]::InvariantCulture,
        ('row={0:D6} sensor={1:D2} value={2:D8} payload=leanrows-qa' + [Environment]::NewLine),
        $row,
        ($row % 32),
        (($row * 1103515245L + 1729L) % 100000000L)
    )
    $null = $prefixBuilder.Append($line)
}
$prefixBytes = $script:Utf8NoBom.GetBytes($prefixBuilder.ToString())
$prefixSha256 = Get-BytesSha256 -Bytes $prefixBytes
$entries = New-Object 'System.Collections.Generic.List[object]'

foreach ($sizeGiB in $SizesGiB) {
    $length = [long]$sizeGiB * 1024L * 1024L * 1024L
    if ($length -le ($prefixBytes.Length + 1)) {
        throw "Sparse fixture size $sizeGiB GiB is too small for its deterministic prefix."
    }
    $name = "sparse-$($sizeGiB)g.log"
    $path = Join-Path $outputRoot $name
    if ([System.IO.File]::Exists($path)) {
        if (-not $Force) {
            throw "Fixture already exists; use -Force to replace this named output: $path"
        }
        [System.IO.File]::Delete($path)
    }

    $stream = New-Object System.IO.FileStream(
        $path,
        [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::Read,
        65536,
        [System.IO.FileOptions]::RandomAccess
    )
    try {
        [LeanRows.Qa.SparseFileNative]::MarkSparse($stream.SafeFileHandle)
        $stream.Write($prefixBytes, 0, $prefixBytes.Length)
        $stream.SetLength($length)
        $stream.Position = $length - 1
        $stream.WriteByte(10)
        $stream.Flush($true)
    }
    finally {
        $stream.Dispose()
    }

    $file = [System.IO.FileInfo]::new($path)
    $attributes = [System.IO.File]::GetAttributes($path)
    if ($file.Length -ne $length) {
        throw "Sparse fixture length mismatch for $path."
    }
    if (($attributes -band [System.IO.FileAttributes]::SparseFile) -eq 0) {
        throw "The filesystem did not preserve the sparse-file attribute for $path."
    }
    $prefixCheck = Read-FileSlice -Path $path -Offset 0 -Count $prefixBytes.Length
    if ((Get-BytesSha256 -Bytes $prefixCheck) -ne $prefixSha256) {
        throw "Sparse fixture prefix verification failed for $path."
    }
    $suffixCheck = Read-FileSlice -Path $path -Offset ($length - 4096) -Count 4096
    if ($suffixCheck[$suffixCheck.Length - 1] -ne 10) {
        throw "Sparse fixture terminator verification failed for $path."
    }
    for ($index = 0; $index -lt ($suffixCheck.Length - 1); $index++) {
        if ($suffixCheck[$index] -ne 0) {
            throw "Sparse fixture zero-tail verification failed for $path."
        }
    }

    $zeroSpanBytes = $length - $prefixBytes.Length - 1L
    $descriptor = [ordered]@{
        schema_version = 'leanrows_sparse_descriptor.v1'
        logical_size_bytes = $length
        prefix_bytes = $prefixBytes.Length
        prefix_sha256 = $prefixSha256
        zero_span_bytes = $zeroSpanBytes
        final_byte_hex = '0a'
    }
    $descriptorBytes = $script:Utf8NoBom.GetBytes(
        (($descriptor | ConvertTo-Json -Compress) + [Environment]::NewLine)
    )
    $allocatedBytes = [LeanRows.Qa.SparseFileNative]::GetAllocatedBytes($path)
    $entries.Add([ordered]@{
        name = $name
        path = $path
        fixture_kind = 'sparse_synthetic_first_viewport'
        logical_size_bytes = $length
        allocated_bytes_observed = $allocatedBytes
        sparse_attribute = $true
        ordinary_prefix_rows = $OrdinaryRows
        logical_records_expected = ([long]$OrdinaryRows + 1L)
        pathological_tail_record_bytes = ($zeroSpanBytes + 1L)
        prefix_sha256 = $prefixSha256
        suffix_4096_sha256 = (Get-BytesSha256 -Bytes $suffixCheck)
        descriptor_sha256 = (Get-BytesSha256 -Bytes $descriptorBytes)
        full_sha256 = $null
        full_sha256_status = 'not_computed_by_design'
        limitation = 'Sparse logical-size fixture; it is not evidence for a real non-sparse corpus or a complete 1/10 GiB scan.'
    })
}

$manifest = [ordered]@{
    schema_version = 'leanrows_sparse_fixture_manifest.v1'
    generated_at_utc = [DateTime]::UtcNow.ToString('o')
    generator = 'tests/qa/New-LeanRowsSparseFixtures.ps1'
    deterministic_seed = 1729
    network_mode = 'offline'
    fixture_root = $outputRoot
    sentinel = $sentinelName
    fixtures = $entries.ToArray()
}
$manifestPath = Join-Path $outputRoot 'sparse-manifest.json'
[System.IO.File]::WriteAllText(
    $manifestPath,
    (($manifest | ConvertTo-Json -Depth 8) + [Environment]::NewLine),
    $script:Utf8NoBom
)
[pscustomobject]@{
    OutputDirectory = $outputRoot
    ManifestPath = $manifestPath
    ManifestSha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    FixtureCount = $entries.Count
}
