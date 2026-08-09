[CmdletBinding()]
param()

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cargoManifest = Join-Path $repositoryRoot 'Cargo.toml'
$content = [System.IO.File]::ReadAllText($cargoManifest)
$workspacePackage = [regex]::Match(
    $content,
    '(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*"(?<version>[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)"'
)

if (-not $workspacePackage.Success) {
    throw 'Cargo.toml does not contain a supported [workspace.package] semantic version.'
}

$workspacePackage.Groups['version'].Value
