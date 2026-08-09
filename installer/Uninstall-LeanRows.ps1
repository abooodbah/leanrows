[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param()

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$progId = 'LeanRows.AssocFile.v1'
$registeredApplicationName = 'LeanRows'
$supportedExtensions = @('.csv', '.tsv', '.jsonl', '.ndjson', '.log')
$allowedInstalledFiles = @(
    'leanrows.exe',
    'uninstall.ps1',
    'LICENSE.txt',
    'UNSIGNED.txt'
)

function Assert-SafeInstalledLeafName {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ([string]::IsNullOrWhiteSpace($Name) -or
        [System.IO.Path]::GetFileName($Name) -ne $Name -or
        $Name.IndexOfAny([System.IO.Path]::GetInvalidFileNameChars()) -ge 0 -or
        $allowedInstalledFiles -notcontains $Name) {
        throw "Installed manifest contains a file outside the cleanup allow-list: '$Name'."
    }
}

function Remove-OwnedRegistryTree {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (Test-Path -Path $Path) {
        Remove-Item -Path $Path -Recurse -Force
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

$localAppData = [System.Environment]::GetFolderPath(
    [System.Environment+SpecialFolder]::LocalApplicationData
)
$expectedInstallRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $localAppData 'Programs\LeanRows')
).TrimEnd('\')
$installRoot = $expectedInstallRoot

if ([System.IO.Directory]::Exists($installRoot)) {
    $attributes = [System.IO.File]::GetAttributes($installRoot)
    if (($attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'Refusing to uninstall through a reparse-point installation directory.'
    }
}

$manifestPath = Join-Path $installRoot 'leanrows-install-manifest.json'
if (-not [System.IO.File]::Exists($manifestPath)) {
    throw "The owned installation manifest is missing; no files or registry entries were removed: $manifestPath"
}

$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([int]$manifest.schema_version -ne 1 -or [string]$manifest.product -ne 'LeanRows') {
    throw 'The installed manifest identity is invalid; no cleanup was attempted.'
}

$filesToRemove = @()
foreach ($entry in @($manifest.files)) {
    if ([bool]$entry.install) {
        $name = [string]$entry.name
        Assert-SafeInstalledLeafName -Name $name
        if ($filesToRemove -contains $name) {
            throw "The installed manifest contains duplicate cleanup entry '$name'."
        }
        $filesToRemove += $name
    }
}

if ($PSCmdlet.ShouldProcess($installRoot, 'Remove LeanRows per-user registration and allow-listed files')) {
    $classesRoot = 'HKCU:\Software\Classes'
    foreach ($extension in $supportedExtensions) {
        $openWithPath = Join-Path $classesRoot "$extension\OpenWithProgids"
        if (Test-Path -Path $openWithPath) {
            Remove-ItemProperty -Path $openWithPath -Name $progId -Force -ErrorAction SilentlyContinue
        }
    }

    Remove-OwnedRegistryTree -Path (Join-Path $classesRoot $progId)
    Remove-OwnedRegistryTree -Path (Join-Path $classesRoot 'Applications\leanrows.exe')
    Remove-OwnedRegistryTree -Path 'HKCU:\Software\LeanRows\Capabilities'
    Remove-ItemProperty -Path 'HKCU:\Software\RegisteredApplications' `
        -Name $registeredApplicationName -Force -ErrorAction SilentlyContinue
    Remove-OwnedRegistryTree `
        -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LeanRows'

    $programs = [System.Environment]::GetFolderPath(
        [System.Environment+SpecialFolder]::Programs
    )
    $shortcutPath = [System.IO.Path]::GetFullPath((Join-Path $programs 'LeanRows.lnk'))
    $expectedShortcutParent = [System.IO.Path]::GetFullPath($programs).TrimEnd('\')
    if (-not [string]::Equals(
            [System.IO.Path]::GetDirectoryName($shortcutPath).TrimEnd('\'),
            $expectedShortcutParent,
            [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to remove a shortcut outside the current-user Programs directory.'
    }
    if ([System.IO.File]::Exists($shortcutPath)) {
        Remove-Item -LiteralPath $shortcutPath -Force
    }

    foreach ($name in $filesToRemove) {
        $target = [System.IO.Path]::GetFullPath((Join-Path $installRoot $name))
        $targetParent = [System.IO.Path]::GetDirectoryName($target).TrimEnd('\')
        if (-not [string]::Equals(
                $targetParent,
                $expectedInstallRoot,
                [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove a file outside the owned installation root: $target"
        }
        if ([System.IO.File]::Exists($target)) {
            Remove-Item -LiteralPath $target -Force
        }
    }

    Remove-Item -LiteralPath $manifestPath -Force
    if ([System.IO.Directory]::Exists($installRoot) -and
        @(Get-ChildItem -LiteralPath $installRoot -Force).Count -eq 0) {
        Remove-Item -LiteralPath $installRoot -Force
    }

    Notify-ShellAssociationChange
    Write-Output 'LeanRows per-user registration and allow-listed installed files were removed.'
}
