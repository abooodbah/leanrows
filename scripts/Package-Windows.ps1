[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$')]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$binary = [System.IO.Path]::GetFullPath($BinaryPath)
$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
$workspaceVersion = & (Join-Path $PSScriptRoot 'Get-WorkspaceVersion.ps1')

if ($Version -ne $workspaceVersion) {
    throw "Package version '$Version' does not match workspace version '$workspaceVersion'."
}

if (-not [System.IO.File]::Exists($binary)) {
    throw "Release binary was not found: $binary"
}

$signature = Get-AuthenticodeSignature -LiteralPath $binary
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
    throw "The v0.1 unsigned package contract expected NotSigned, but found '$($signature.Status)'."
}

[System.IO.Directory]::CreateDirectory($outputRoot) | Out-Null

$artifactBase = "LeanRows-v$Version-windows-x64-unsigned"
$zipPath = Join-Path $outputRoot "$artifactBase.zip"
$checksumPath = Join-Path $outputRoot "$artifactBase.sha256"
$archiveRoot = "$artifactBase/"

$sourceFiles = @(
    [pscustomobject]@{
        Name = 'leanrows.exe'
        Source = $binary
        Install = $true
    },
    [pscustomobject]@{
        Name = 'install.ps1'
        Source = (Join-Path $repositoryRoot 'installer/Install-LeanRows.ps1')
        Install = $false
    },
    [pscustomobject]@{
        Name = 'uninstall.ps1'
        Source = (Join-Path $repositoryRoot 'installer/Uninstall-LeanRows.ps1')
        Install = $true
    },
    [pscustomobject]@{
        Name = 'LICENSE.txt'
        Source = (Join-Path $repositoryRoot 'LICENSE')
        Install = $true
    },
    [pscustomobject]@{
        Name = 'UNSIGNED.txt'
        Source = (Join-Path $repositoryRoot 'installer/UNSIGNED.txt')
        Install = $true
    }
)

$manifestFiles = @()
foreach ($item in $sourceFiles) {
    $resolvedSource = [System.IO.Path]::GetFullPath($item.Source)
    if (-not [System.IO.File]::Exists($resolvedSource)) {
        throw "Required package input was not found: $resolvedSource"
    }
    $fileInfo = [System.IO.FileInfo]::new($resolvedSource)
    $manifestFiles += [ordered]@{
        name = $item.Name
        install = [bool]$item.Install
        size_bytes = $fileInfo.Length
        sha256 = (Get-FileHash -LiteralPath $resolvedSource -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

$manifest = [ordered]@{
    schema_version = 1
    product = 'LeanRows'
    version = $Version
    platform = 'windows-x64'
    signing_status = 'unsigned'
    default_install_scope = 'current-user'
    files = $manifestFiles
    file_associations = @('.csv', '.tsv', '.jsonl', '.ndjson', '.log')
}
$manifestText = ($manifest | ConvertTo-Json -Depth 5) -replace "`r`n", "`n"
$manifestBytes = [System.Text.UTF8Encoding]::new($false).GetBytes($manifestText + "`n")

$epoch = [System.DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [System.TimeSpan]::Zero)
$archiveStream = [System.IO.File]::Open(
    $zipPath,
    [System.IO.FileMode]::Create,
    [System.IO.FileAccess]::ReadWrite,
    [System.IO.FileShare]::None
)
try {
    $archive = [System.IO.Compression.ZipArchive]::new(
        $archiveStream,
        [System.IO.Compression.ZipArchiveMode]::Create,
        $false,
        [System.Text.Encoding]::UTF8
    )
    try {
        foreach ($item in @($sourceFiles | Sort-Object Name)) {
            $entry = $archive.CreateEntry(
                $archiveRoot + $item.Name,
                [System.IO.Compression.CompressionLevel]::Optimal
            )
            $entry.LastWriteTime = $epoch
            $entryStream = $entry.Open()
            try {
                $input = [System.IO.File]::OpenRead([System.IO.Path]::GetFullPath($item.Source))
                try {
                    $input.CopyTo($entryStream)
                }
                finally {
                    $input.Dispose()
                }
            }
            finally {
                $entryStream.Dispose()
            }
        }

        $manifestEntry = $archive.CreateEntry(
            $archiveRoot + 'manifest.json',
            [System.IO.Compression.CompressionLevel]::Optimal
        )
        $manifestEntry.LastWriteTime = $epoch
        $manifestStream = $manifestEntry.Open()
        try {
            $manifestStream.Write($manifestBytes, 0, $manifestBytes.Length)
        }
        finally {
            $manifestStream.Dispose()
        }
    }
    finally {
        $archive.Dispose()
    }
}
finally {
    $archiveStream.Dispose()
}

$archiveHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
$checksumLine = "$archiveHash  $([System.IO.Path]::GetFileName($zipPath))`n"
[System.IO.File]::WriteAllText(
    $checksumPath,
    $checksumLine,
    [System.Text.UTF8Encoding]::new($false)
)

[pscustomobject]@{
    ZipPath = $zipPath
    ChecksumPath = $checksumPath
    Sha256 = $archiveHash
    SigningStatus = 'unsigned'
}
