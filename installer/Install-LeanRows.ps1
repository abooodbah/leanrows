[CmdletBinding()]
param(
    [switch]$ValidateOnly,
    [switch]$OpenDefaultApps
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$productName = 'LeanRows'
$progId = 'LeanRows.AssocFile.v1'
$registeredApplicationName = 'LeanRows'
$supportedExtensions = @('.csv', '.tsv', '.jsonl', '.ndjson', '.log')
$allowedPackageFiles = @(
    'leanrows.exe',
    'install.ps1',
    'uninstall.ps1',
    'LICENSE.txt',
    'UNSIGNED.txt'
)

function Assert-SafeLeafName {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ([string]::IsNullOrWhiteSpace($Name) -or
        [System.IO.Path]::GetFileName($Name) -ne $Name -or
        $Name.IndexOfAny([System.IO.Path]::GetInvalidFileNameChars()) -ge 0 -or
        $allowedPackageFiles -notcontains $Name) {
        throw "Package manifest contains a file outside the allow-list: '$Name'."
    }
}

function Get-ValidatedPackageManifest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$PackageRoot
    )

    $manifestPath = Join-Path $PackageRoot 'manifest.json'
    if (-not [System.IO.File]::Exists($manifestPath)) {
        throw "Package manifest was not found: $manifestPath"
    }

    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ([int]$manifest.schema_version -ne 1 -or [string]$manifest.product -ne $productName) {
        throw 'Package manifest schema or product identity is not supported.'
    }
    if ([string]$manifest.signing_status -ne 'unsigned') {
        throw 'This installer supports only the explicitly unsigned v0.1 package contract.'
    }

    $extensions = @($manifest.file_associations | ForEach-Object { [string]$_ })
    if ($extensions.Count -ne $supportedExtensions.Count) {
        throw 'Package manifest association set is incomplete.'
    }
    foreach ($extension in $supportedExtensions) {
        if ($extensions -notcontains $extension) {
            throw "Package manifest is missing association '$extension'."
        }
    }

    $seen = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $files = @($manifest.files)
    if ($files.Count -ne $allowedPackageFiles.Count) {
        throw 'Package manifest does not contain the exact expected file set.'
    }

    foreach ($entry in $files) {
        $name = [string]$entry.name
        Assert-SafeLeafName -Name $name
        if (-not $seen.Add($name)) {
            throw "Package manifest contains duplicate file '$name'."
        }

        $sourcePath = [System.IO.Path]::GetFullPath((Join-Path $PackageRoot $name))
        if (-not [System.IO.File]::Exists($sourcePath)) {
            throw "Package file was not found: $name"
        }
        $actualSize = [System.IO.FileInfo]::new($sourcePath).Length
        if ($actualSize -ne [long]$entry.size_bytes) {
            throw "Package file size does not match the manifest: $name"
        }
        $actualHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash
        if ($actualHash -ne [string]$entry.sha256) {
            throw "Package file hash does not match the manifest: $name"
        }
    }

    return $manifest
}

function Assert-SafeInstallRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InstallRoot,

        [Parameter(Mandatory = $true)]
        [string]$LocalAppData
    )

    $expected = [System.IO.Path]::GetFullPath(
        (Join-Path $LocalAppData 'Programs\LeanRows')
    ).TrimEnd('\')
    $actual = [System.IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
    if (-not [string]::Equals($actual, $expected, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to install outside the fixed per-user directory: $expected"
    }
    if ([System.IO.Directory]::Exists($actual)) {
        $attributes = [System.IO.File]::GetAttributes($actual)
        if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'Refusing to install through a reparse-point installation directory.'
        }
    }
}

function Set-RegistryString {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Value
    )

    New-Item -Path $Path -Force | Out-Null
    if ($Name.Length -eq 0) {
        Set-Item -Path $Path -Value $Value
    }
    else {
        New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType String -Force |
            Out-Null
    }
}

function Set-RegistryDword {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [int]$Value
    )
    New-Item -Path $Path -Force | Out-Null
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType DWord -Force |
        Out-Null
}

function Install-StartMenuShortcut {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ExecutablePath,

        [Parameter(Mandatory = $true)]
        [string]$InstallRoot
    )

    $programs = [System.Environment]::GetFolderPath(
        [System.Environment+SpecialFolder]::Programs
    )
    $shortcutPath = [System.IO.Path]::GetFullPath((Join-Path $programs 'LeanRows.lnk'))
    $expectedParent = [System.IO.Path]::GetFullPath($programs).TrimEnd('\')
    if (-not [string]::Equals(
            [System.IO.Path]::GetDirectoryName($shortcutPath).TrimEnd('\'),
            $expectedParent,
            [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to create the Start Menu shortcut outside the current-user Programs directory.'
    }

    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($shortcutPath)
        try {
            $shortcut.TargetPath = $ExecutablePath
            $shortcut.WorkingDirectory = $InstallRoot
            $shortcut.IconLocation = $ExecutablePath + ',0'
            $shortcut.Description = 'Open LeanRows'
            $shortcut.Save()
        }
        finally {
            if ($null -ne $shortcut -and [System.Runtime.InteropServices.Marshal]::IsComObject($shortcut)) {
                [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
            }
        }
    }
    finally {
        if ([System.Runtime.InteropServices.Marshal]::IsComObject($shell)) {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
        }
    }
}

function Notify-ShellAssociationChange {
    if ($null -eq ('LeanRows.Release.ShellNotification' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace LeanRows.Release
{
    public static class ShellNotification
    {
        [DllImport("shell32.dll")]
        public static extern void SHChangeNotify(
            uint eventId,
            uint flags,
            IntPtr item1,
            IntPtr item2);
    }
}
'@
    }

    [LeanRows.Release.ShellNotification]::SHChangeNotify(
        0x08000000,
        0x00001000,
        [System.IntPtr]::Zero,
        [System.IntPtr]::Zero
    )
}

$packageRoot = [System.IO.Path]::GetFullPath($PSScriptRoot)
$manifest = Get-ValidatedPackageManifest -PackageRoot $packageRoot
if ($ValidateOnly) {
    Write-Output "Validated LeanRows package v$($manifest.version) ($($manifest.signing_status))."
    exit 0
}

$localAppData = [System.Environment]::GetFolderPath(
    [System.Environment+SpecialFolder]::LocalApplicationData
)
$installRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $localAppData 'Programs\LeanRows')
)
Assert-SafeInstallRoot -InstallRoot $installRoot -LocalAppData $localAppData
[System.IO.Directory]::CreateDirectory($installRoot) | Out-Null

foreach ($entry in @($manifest.files)) {
    if ([bool]$entry.install) {
        $name = [string]$entry.name
        Assert-SafeLeafName -Name $name
        Copy-Item -LiteralPath (Join-Path $packageRoot $name) `
            -Destination (Join-Path $installRoot $name) -Force
    }
}
Copy-Item -LiteralPath (Join-Path $packageRoot 'manifest.json') `
    -Destination (Join-Path $installRoot 'leanrows-install-manifest.json') -Force

$executablePath = Join-Path $installRoot 'leanrows.exe'
$openCommand = '"' + $executablePath + '" "%1"'
$classesRoot = 'HKCU:\Software\Classes'
$progIdRoot = Join-Path $classesRoot $progId

New-Item -Path $progIdRoot -Force | Out-Null
Set-Item -Path $progIdRoot -Value 'LeanRows data file'
Set-RegistryString -Path (Join-Path $progIdRoot 'DefaultIcon') -Name '' `
    -Value ($executablePath + ',0')
Set-RegistryString -Path (Join-Path $progIdRoot 'shell\open\command') -Name '' `
    -Value $openCommand

$applicationRoot = Join-Path $classesRoot 'Applications\leanrows.exe'
Set-RegistryString -Path $applicationRoot -Name 'FriendlyAppName' -Value $productName
Set-RegistryString -Path (Join-Path $applicationRoot 'shell\open\command') `
    -Name '' -Value $openCommand
$supportedTypesRoot = Join-Path $applicationRoot 'SupportedTypes'
foreach ($extension in $supportedExtensions) {
    Set-RegistryString -Path $supportedTypesRoot -Name $extension -Value ''
    Set-RegistryString -Path (Join-Path $classesRoot "$extension\OpenWithProgids") `
        -Name $progId -Value ''
}

$capabilitiesRoot = 'HKCU:\Software\LeanRows\Capabilities'
Set-RegistryString -Path $capabilitiesRoot -Name 'ApplicationName' -Value $productName
Set-RegistryString -Path $capabilitiesRoot -Name 'ApplicationDescription' `
    -Value 'A read-only, memory-conscious viewer for large row-oriented local files.'
$fileAssociationsRoot = Join-Path $capabilitiesRoot 'FileAssociations'
foreach ($extension in $supportedExtensions) {
    Set-RegistryString -Path $fileAssociationsRoot -Name $extension -Value $progId
}
Set-RegistryString -Path 'HKCU:\Software\RegisteredApplications' `
    -Name $registeredApplicationName -Value 'Software\LeanRows\Capabilities'

$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LeanRows'
$uninstallScript = Join-Path $installRoot 'uninstall.ps1'
$uninstallCommand = 'powershell.exe -NoProfile -File "' + $uninstallScript + '"'
$installedBytes = @(
    $manifest.files |
        Where-Object { [bool]$_.install } |
        Measure-Object -Property size_bytes -Sum
).Sum
$installedBytes += [System.IO.FileInfo]::new(
    (Join-Path $installRoot 'leanrows-install-manifest.json')
).Length
$estimatedSizeKb = [int][Math]::Ceiling($installedBytes / 1KB)
Set-RegistryString -Path $uninstallRoot -Name 'DisplayName' -Value $productName
Set-RegistryString -Path $uninstallRoot -Name 'DisplayVersion' -Value ([string]$manifest.version)
Set-RegistryString -Path $uninstallRoot -Name 'Publisher' -Value 'Abdulfatah Bahbouh'
Set-RegistryString -Path $uninstallRoot -Name 'DisplayIcon' -Value ($executablePath + ',0')
Set-RegistryString -Path $uninstallRoot -Name 'InstallLocation' -Value $installRoot
Set-RegistryString -Path $uninstallRoot -Name 'UninstallString' -Value $uninstallCommand
Set-RegistryString -Path $uninstallRoot -Name 'QuietUninstallString' -Value $uninstallCommand
Set-RegistryString -Path $uninstallRoot -Name 'URLInfoAbout' -Value 'https://github.com/abooodbah/leanrows'
Set-RegistryDword -Path $uninstallRoot -Name 'NoModify' -Value 1
Set-RegistryDword -Path $uninstallRoot -Name 'NoRepair' -Value 1
Set-RegistryDword -Path $uninstallRoot -Name 'EstimatedSize' -Value $estimatedSizeKb

Install-StartMenuShortcut -ExecutablePath $executablePath -InstallRoot $installRoot

Notify-ShellAssociationChange

Write-Output "Installed LeanRows v$($manifest.version) for the current user at: $installRoot"
Write-Output 'This build is unsigned. Its package checksum should be verified before installation.'
Write-Output 'LeanRows is registered as an available handler; Windows defaults were not changed.'

if ($OpenDefaultApps) {
    Start-Process 'ms-settings:defaultapps?registeredAppUser=LeanRows'
}
