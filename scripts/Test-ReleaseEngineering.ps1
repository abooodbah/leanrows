[CmdletBinding()]
param(
    [string]$BinaryPath,
    [string]$ArtifactDirectory
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$expectedExtensions = @('.csv', '.jsonl', '.log', '.ndjson', '.tsv')

function Assert-True {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-Matches {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text,

        [Parameter(Mandatory = $true)]
        [string]$Pattern,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    Assert-True -Condition ([regex]::IsMatch($Text, $Pattern)) -Message $Message
}

function Test-PowerShellParsing {
    $parseFailures = @()
    $scripts = @(
        Get-ChildItem -LiteralPath (Join-Path $repositoryRoot 'scripts') -Filter '*.ps1' -File
        Get-ChildItem -LiteralPath (Join-Path $repositoryRoot 'installer') -Filter '*.ps1' -File
        Get-ChildItem -LiteralPath (Join-Path $repositoryRoot 'tests/qa') -Filter '*.ps1' -File
    )
    foreach ($script in $scripts) {
        $tokens = $null
        $errors = $null
        [void][System.Management.Automation.Language.Parser]::ParseFile(
            $script.FullName,
            [ref]$tokens,
            [ref]$errors
        )
        foreach ($parseError in @($errors)) {
            $parseFailures += "$($script.FullName): $($parseError.Message)"
        }
    }
    Assert-True -Condition ($parseFailures.Count -eq 0) `
        -Message ("PowerShell parser failures:`n" + ($parseFailures -join "`n"))
}

function Get-DeclaredExtensions {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ScriptText
    )

    $block = [regex]::Match(
        $ScriptText,
        '(?ms)\$supportedExtensions\s*=\s*@\((?<body>.*?)\)'
    )
    Assert-True -Condition $block.Success -Message 'Association allow-list was not found.'
    @(
        [regex]::Matches($block.Groups['body'].Value, "'(?<extension>\.[^']+)'") |
            ForEach-Object { $_.Groups['extension'].Value } |
            Sort-Object
    )
}

function Test-InstallerContract {
    $installPath = Join-Path $repositoryRoot 'installer/Install-LeanRows.ps1'
    $uninstallPath = Join-Path $repositoryRoot 'installer/Uninstall-LeanRows.ps1'
    $installText = [System.IO.File]::ReadAllText($installPath)
    $uninstallText = [System.IO.File]::ReadAllText($uninstallPath)

    foreach ($text in @($installText, $uninstallText)) {
        $actualExtensions = @(Get-DeclaredExtensions -ScriptText $text)
        Assert-True -Condition (($actualExtensions -join ',') -eq ($expectedExtensions -join ',')) `
            -Message "Association set differs from the approved allow-list: $($actualExtensions -join ', ')"
    }

    Assert-True -Condition (-not $installText.Contains('UserChoice')) `
        -Message 'Installer must never read or write the protected UserChoice key.'
    Assert-True -Condition (-not $uninstallText.Contains('UserChoice')) `
        -Message 'Uninstaller must never read or write the protected UserChoice key.'
    Assert-True -Condition (-not [regex]::IsMatch(
            $installText,
            "(?m)^\s*'\.txt'\s*(?:,|$)")) `
        -Message 'Installer must not claim .txt.'
    $expectedOpenCommand = @'
$openCommand = '"' + $executablePath + '" "%1"'
'@.Trim()
    Assert-True -Condition $installText.Contains($expectedOpenCommand) `
        -Message 'Open command must quote both the executable and the file argument.'
    Assert-Matches -Text $installText -Pattern 'HKCU:\\Software\\RegisteredApplications' `
        -Message 'Per-user RegisteredApplications capability is missing.'
    Assert-Matches -Text $installText -Pattern 'OpenWithProgids' `
        -Message 'OpenWithProgids registration is missing.'
    Assert-Matches -Text $installText `
        -Pattern 'CurrentVersion\\Uninstall\\LeanRows' `
        -Message 'Per-user Installed Apps registration is missing.'
    Assert-Matches -Text $installText -Pattern 'LeanRows\.lnk' `
        -Message 'Current-user Start Menu shortcut creation is missing.'
    Assert-Matches -Text $uninstallText `
        -Pattern 'CurrentVersion\\Uninstall\\LeanRows' `
        -Message 'Uninstaller does not remove the owned Installed Apps registration.'
    Assert-Matches -Text $uninstallText -Pattern 'LeanRows\.lnk' `
        -Message 'Uninstaller does not remove the owned Start Menu shortcut.'
    Assert-Matches -Text $uninstallText -Pattern 'allowedInstalledFiles' `
        -Message 'Uninstaller file allow-list is missing.'
    Assert-True -Condition (-not [regex]::IsMatch(
            $uninstallText,
            '(?i)Remove-Item\s+-LiteralPath\s+\$installRoot\s+-Recurse')) `
        -Message 'Uninstaller must not recursively delete the installation directory.'
}

function Test-WindowsSourceContract {
    $mainText = [System.IO.File]::ReadAllText(
        (Join-Path $repositoryRoot 'crates/leanrows-app/src/main.rs')
    )
    $buildText = [System.IO.File]::ReadAllText(
        (Join-Path $repositoryRoot 'crates/leanrows-app/build_support.rs')
    )
    $resourceText = [System.IO.File]::ReadAllText(
        (Join-Path $repositoryRoot 'crates/leanrows-app/resources/leanrows.rc')
    )
    $manifestText = [System.IO.File]::ReadAllText(
        (Join-Path $repositoryRoot 'crates/leanrows-app/resources/leanrows.manifest')
    )
    Assert-Matches -Text $mainText `
        -Pattern '(?s)cfg_attr\(\s*all\(windows, not\(debug_assertions\)\),\s*windows_subsystem = "windows"' `
        -Message 'The packaged release is not configured as a release-only Windows GUI subsystem.'
    Assert-True -Condition $buildText.Contains('.env_remove("INCLUDE")') `
        -Message 'Resource compilation must not inherit an ambient Windows SDK INCLUDE path.'
    Assert-True -Condition $resourceText.Contains('#include "leanrows-version.rc"') `
        -Message 'The deterministic generated VERSIONINFO resource is not included.'
    Assert-Matches -Text $resourceText `
        -Pattern '(?s)#define IDR_ACCELERATORS 102.*?IDR_ACCELERATORS ACCELERATORS\s*BEGIN\s*"O",\s*ID_FILE_OPEN,\s*VIRTKEY,\s*CONTROL\s*0x74,\s*ID_FILE_RELOAD,\s*VIRTKEY\s*"C",\s*ID_EDIT_COPY,\s*VIRTKEY,\s*CONTROL\s*"F",\s*ID_EDIT_FIND,\s*VIRTKEY,\s*CONTROL\s*0x72,\s*ID_EDIT_FIND_NEXT,\s*VIRTKEY\s*0x72,\s*ID_EDIT_FIND_PREVIOUS,\s*VIRTKEY,\s*SHIFT\s*"G",\s*ID_EDIT_GOTO,\s*VIRTKEY,\s*CONTROL\s*END' `
        -Message 'The embedded accelerator table does not preserve the approved keyboard command map.'
    Assert-True -Condition (-not [regex]::IsMatch(
            $resourceText,
            '(?im)^\s*#include\s*[<"](?:windows|winuser)\.h[>"]')) `
        -Message 'Resource compilation must not depend on Windows SDK headers.'

    $nativeText = [System.IO.File]::ReadAllText(
        (Join-Path $repositoryRoot 'crates/leanrows-win32/src/native.rs')
    )
    Assert-Matches -Text $nativeText -Pattern '\bLoadAcceleratorsW\s*\(' `
        -Message 'Native startup does not load the embedded accelerator table.'
    Assert-True -Condition (-not [regex]::IsMatch(
            $nativeText,
            '\bCreateAcceleratorTable[AW]?\s*\(')) `
        -Message 'Native startup must not recreate the CI-fragile runtime accelerator table.'
    $acceleratorCommandIds = [ordered]@{
        ID_FILE_OPEN = 100; ID_FILE_RELOAD = 101; ID_EDIT_COPY = 110
        ID_EDIT_FIND = 111; ID_EDIT_FIND_NEXT = 112
        ID_EDIT_FIND_PREVIOUS = 113; ID_EDIT_GOTO = 114
    }
    foreach ($entry in $acceleratorCommandIds.GetEnumerator()) {
        $name = [regex]::Escape([string]$entry.Key)
        $value = [string]$entry.Value
        Assert-Matches -Text $resourceText `
            -Pattern "(?m)^\s*#define\s+$name\s+$value\s*$" `
            -Message "Resource command ID $($entry.Key) drifted."
        Assert-Matches -Text $nativeText `
            -Pattern "(?m)^\s*const\s+${name}:\s*u16\s*=\s*$value;\s*$" `
            -Message "Native command ID $($entry.Key) drifted."
    }
    Assert-Matches -Text $manifestText `
        -Pattern '(?s)name="LeanRows\.Desktop"\s+processorArchitecture="amd64"' `
        -Message 'The x64 application manifest definition identity is not explicitly amd64.'

    $forbiddenConsoleCalls = New-Object 'System.Collections.Generic.List[string]'
    foreach ($source in Get-ChildItem `
            -LiteralPath (Join-Path $repositoryRoot 'crates') `
            -Recurse -Filter '*.rs' -File) {
        $sourceText = [System.IO.File]::ReadAllText($source.FullName)
        if ($sourceText -match '(?i)\b(?:AttachConsole|AllocConsole)\s*\(') {
            $forbiddenConsoleCalls.Add($source.FullName)
        }
    }
    Assert-True -Condition ($forbiddenConsoleCalls.Count -eq 0) `
        -Message ("Release code must not attach or allocate a console:`n" +
            ($forbiddenConsoleCalls -join "`n"))
}

function Test-WorkflowContract {
    $workflowPath = Join-Path $repositoryRoot '.github/workflows/windows.yml'
    $workflow = [System.IO.File]::ReadAllText($workflowPath)
    Assert-True -Condition (-not $workflow.Contains("`t")) `
        -Message 'GitHub workflow must not contain tab indentation.'

    $requirements = [ordered]@{
        'Pinned Rust 1.97.1' = 'rustup toolchain install 1\.97\.1'
        'Locked fetch' = 'cargo \+1\.97\.1 fetch --locked'
        'Formatting gate' = 'cargo \+1\.97\.1 fmt --all -- --check'
        'Workspace tests' = 'cargo \+1\.97\.1 test --workspace --all-targets --locked'
        'Strict Clippy' = 'cargo \+1\.97\.1 clippy --workspace --all-targets --locked -- -D warnings'
        'Site validation' = 'node \./site/validate\.mjs'
        'Release app and spike build' = 'cargo \+1\.97\.1 build --release --locked -p leanrows-app -p leanrows-spike'
        'Redirected native smoke test' = 'Invoke-LeanRowsShellSmoke\.ps1'
        'Reduced CI QA harness' = 'Invoke-LeanRowsReleaseQa\.ps1'
        'Reduced CI QA scale disclosure' = '-SkipSparseScaleFixtures'
        'Reduced CI QA seek bound' = '-RandomSeekRuns 12'
        'Reduced CI QA evidence root' = '-EvidenceDirectory \./artifacts/ci-qa'
        'Separate CI QA artifact' = 'name: leanrows-ci-qa-evidence'
        'Packaging command' = 'Package-Windows\.ps1'
        'Tag-only release' = "if: startsWith\(github\.ref, 'refs/tags/v'\)"
        'Existing tag verification' = '--verify-tag'
        'GitHub release command' = 'gh release create'
        'Release notes file' = '--notes-file \./docs/releases/v0\.1\.0\.md'
        'Checksum upload' = 'artifacts/\*\.sha256'
        'Unsigned artifact disclosure' = 'Upload the unsigned portable package'
        'Pages waits for release' = '(?s)deploy-pages:.*needs: github-release'
        'Pages deployment' = 'actions/deploy-pages@cd2ce8fcbc39b97be8ca5fce6e763baed58fa128'
    }
    foreach ($requirement in $requirements.GetEnumerator()) {
        Assert-Matches -Text $workflow -Pattern $requirement.Value `
            -Message "Workflow contract missing: $($requirement.Key)"
    }

    $shellSmokeStep = [regex]::Match(
        $workflow,
        '(?ms)^[ ]{6}- name: Smoke-test native control creation\r?\n(?<body>.*?)(?=^[ ]{6}- name:|\z)'
    )
    Assert-True -Condition $shellSmokeStep.Success `
        -Message 'Redirected native shell smoke step was not found.'
    foreach ($shellSmokeContract in @(
        'Invoke-LeanRowsShellSmoke.ps1',
        '-BinaryPath ./target/release/leanrows.exe'
    )) {
        Assert-True -Condition $shellSmokeStep.Value.Contains($shellSmokeContract) `
            -Message "Native shell smoke step is missing: $shellSmokeContract"
    }
    Assert-True -Condition (-not $shellSmokeStep.Value.Contains('leanrows.exe --smoke-test')) `
        -Message 'A GUI-subsystem executable must not be invoked directly for CI smoke verification.'

    $workflowDirectory = Join-Path $repositoryRoot '.github/workflows'
    foreach ($workflowFile in Get-ChildItem -LiteralPath $workflowDirectory -Filter '*.yml' -File) {
        $workflowText = [System.IO.File]::ReadAllText($workflowFile.FullName)
        foreach ($action in [regex]::Matches(
                $workflowText,
                '(?m)^\s*uses:\s*(?<name>[^@\s]+)@(?<ref>[^\s#]+)')) {
            Assert-True -Condition ($action.Groups['ref'].Value -match '^[0-9a-f]{40}$') `
                -Message "Workflow action is not pinned in $($workflowFile.Name): $($action.Value.Trim())"
        }
    }

    $qaRunStep = [regex]::Match(
        $workflow,
        '(?ms)^[ ]{6}- name: Run reduced fail-closed CI QA\r?\n(?<body>.*?)(?=^[ ]{6}- name:|\z)'
    )
    Assert-True -Condition $qaRunStep.Success `
        -Message 'Reduced CI QA step was not found as a distinct workflow step.'
    foreach ($qaArgument in @(
        '-BinaryPath ./target/release/leanrows.exe',
        '-SpikeBinaryPath ./target/release/leanrows-spike.exe',
        '-EvidenceDirectory ./artifacts/ci-qa',
        '-RandomSeekRuns 12',
        '-SkipSparseScaleFixtures'
    )) {
        Assert-True -Condition $qaRunStep.Value.Contains($qaArgument) `
            -Message "Reduced CI QA step is missing the explicit argument: $qaArgument"
    }
    Assert-True -Condition (-not $qaRunStep.Value.Contains('continue-on-error')) `
        -Message 'Reduced CI QA must require a passing process exit code.'

    $qaReceiptStep = [regex]::Match(
        $workflow,
        '(?ms)^[ ]{6}- name: Require reduced CI QA evidence\r?\n(?<body>.*?)(?=^[ ]{6}- name:|\z)'
    )
    Assert-True -Condition $qaReceiptStep.Success `
        -Message 'Reduced CI QA must have a distinct fail-closed receipt check.'
    Assert-True -Condition $qaReceiptStep.Value.Contains(
        './artifacts/ci-qa/qa-receipt.json') `
        -Message 'Reduced CI QA receipt check does not require qa-receipt.json.'
    Assert-True -Condition (-not $qaReceiptStep.Value.Contains('continue-on-error')) `
        -Message 'Reduced CI QA receipt validation must fail closed.'

    $qaUploadStep = [regex]::Match(
        $workflow,
        '(?ms)^[ ]{6}- name: Upload reduced CI QA evidence\r?\n(?<body>.*?)(?=^[ ]{6}- name:|\z)'
    )
    Assert-True -Condition $qaUploadStep.Success `
        -Message 'Reduced CI QA evidence must use its own upload step.'
    foreach ($qaUploadContract in @(
        'if: ${{ always() && hashFiles(''artifacts/ci-qa/**'') != '''' }}',
        'uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a',
        'name: leanrows-ci-qa-evidence',
        'path: artifacts/ci-qa/',
        'if-no-files-found: error',
        'retention-days: 14'
    )) {
        Assert-True -Condition $qaUploadStep.Value.Contains($qaUploadContract) `
            -Message "Reduced CI QA upload is missing: $qaUploadContract"
    }

    $releaseUploadStep = [regex]::Match(
        $workflow,
        '(?ms)^[ ]{6}- name: Upload the unsigned portable package\r?\n(?<body>.*?)(?=^[ ]{6}- name:|\z)'
    )
    Assert-True -Condition $releaseUploadStep.Success `
        -Message 'Portable release upload step was not found.'
    Assert-True -Condition $releaseUploadStep.Value.Contains('artifacts/*.zip') `
        -Message 'Portable release upload must include the top-level ZIP.'
    Assert-True -Condition $releaseUploadStep.Value.Contains('artifacts/*.sha256') `
        -Message 'Portable release upload must include the top-level checksum.'
    Assert-True -Condition (-not $releaseUploadStep.Value.Contains('ci-qa')) `
        -Message 'CI QA evidence must not contaminate the exact portable release artifact set.'
}

function Test-ChecksumArtifact {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Directory
    )

    $artifactRoot = [System.IO.Path]::GetFullPath($Directory)
    $zipFiles = @(Get-ChildItem -LiteralPath $artifactRoot -Filter '*.zip' -File)
    $checksumFiles = @(Get-ChildItem -LiteralPath $artifactRoot -Filter '*.sha256' -File)
    Assert-True -Condition ($zipFiles.Count -eq 1) `
        -Message 'Artifact directory must contain exactly one ZIP.'
    Assert-True -Condition ($checksumFiles.Count -eq 1) `
        -Message 'Artifact directory must contain exactly one checksum file.'
    Assert-Matches -Text $zipFiles[0].Name `
        -Pattern '^LeanRows-v.+-windows-x64-unsigned\.zip$' `
        -Message 'Portable ZIP name must disclose unsigned status.'

    $line = [System.IO.File]::ReadAllText($checksumFiles[0].FullName).Trim()
    $match = [regex]::Match($line, '^(?<hash>[0-9a-f]{64})  (?<file>[^\\/]+\.zip)$')
    Assert-True -Condition $match.Success -Message 'Checksum file format is invalid.'
    Assert-True -Condition ($match.Groups['file'].Value -eq $zipFiles[0].Name) `
        -Message 'Checksum file names a different ZIP.'
    $actualHash = (Get-FileHash -LiteralPath $zipFiles[0].FullName -Algorithm SHA256).Hash
    Assert-True -Condition ($actualHash -eq $match.Groups['hash'].Value) `
        -Message 'Portable ZIP does not match its SHA-256 file.'
}

function Find-WindowsSdkTool {
    param([Parameter(Mandatory = $true)][string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $command -and -not [string]::IsNullOrWhiteSpace([string]$command.Source)) {
        return [string]$command.Source
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
    if ([string]::IsNullOrWhiteSpace($programFilesX86)) {
        return $null
    }
    $binRoot = Join-Path $programFilesX86 'Windows Kits\10\bin'
    if (-not [System.IO.Directory]::Exists($binRoot)) {
        return $null
    }
    @(
        Get-ChildItem -LiteralPath $binRoot -Directory |
            Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName "x64\$Name" } |
            Where-Object { [System.IO.File]::Exists($_) }
    ) | Select-Object -First 1
}

function Test-WindowsBinaryMetadata {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ApplicationBinary
    )

    $binary = [System.IO.Path]::GetFullPath($ApplicationBinary)
    Assert-True -Condition ([System.IO.File]::Exists($binary)) `
        -Message "Windows metadata test binary does not exist: $binary"
    $bytes = [System.IO.File]::ReadAllBytes($binary)
    Assert-True -Condition ($bytes.Length -ge 256) `
        -Message 'Windows application is too small to contain a valid PE header.'
    Assert-True -Condition ([BitConverter]::ToUInt16($bytes, 0) -eq 0x5A4D) `
        -Message 'Windows application is missing the MZ signature.'
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
    Assert-True -Condition ($peOffset -ge 0 -and $peOffset -le ($bytes.Length - 96)) `
        -Message 'Windows application contains an invalid PE header offset.'
    Assert-True -Condition ([BitConverter]::ToUInt32($bytes, $peOffset) -eq 0x00004550) `
        -Message 'Windows application is missing the PE signature.'
    Assert-True -Condition ([BitConverter]::ToUInt16($bytes, $peOffset + 4) -eq 0x8664) `
        -Message 'Windows application is not an x64 PE image.'
    $optionalOffset = $peOffset + 24
    Assert-True -Condition ([BitConverter]::ToUInt16($bytes, $optionalOffset) -eq 0x020B) `
        -Message 'Windows application is not a PE32+ image.'
    $subsystem = [BitConverter]::ToUInt16($bytes, $optionalOffset + 68)
    Assert-True -Condition ($subsystem -eq 2) `
        -Message "Windows application subsystem is $subsystem, expected GUI subsystem 2."

    $workspaceVersion = & (Join-Path $PSScriptRoot 'Get-WorkspaceVersion.ps1')
    $versionMatch = [regex]::Match(
        [string]$workspaceVersion,
        '^(?<major>[0-9]+)\.(?<minor>[0-9]+)\.(?<patch>[0-9]+)(?:[-+].*)?$'
    )
    Assert-True -Condition $versionMatch.Success `
        -Message "Workspace version is not supported for Windows VERSIONINFO: $workspaceVersion"
    $major = [int]$versionMatch.Groups['major'].Value
    $minor = [int]$versionMatch.Groups['minor'].Value
    $patch = [int]$versionMatch.Groups['patch'].Value
    $expectedVersion = "$major.$minor.$patch.0"
    $version = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($binary)
    foreach ($field in @(
        [pscustomobject]@{ Name = 'FileVersion'; Actual = $version.FileVersion; Expected = $expectedVersion },
        [pscustomobject]@{ Name = 'ProductVersion'; Actual = $version.ProductVersion; Expected = $expectedVersion },
        [pscustomobject]@{ Name = 'ProductName'; Actual = $version.ProductName; Expected = 'LeanRows' },
        [pscustomobject]@{ Name = 'FileDescription'; Actual = $version.FileDescription; Expected = 'LeanRows large row-file viewer' },
        [pscustomobject]@{ Name = 'OriginalFilename'; Actual = $version.OriginalFilename; Expected = 'leanrows.exe' },
        [pscustomobject]@{ Name = 'InternalName'; Actual = $version.InternalName; Expected = 'leanrows' },
        [pscustomobject]@{ Name = 'CompanyName'; Actual = $version.CompanyName; Expected = 'Abdulfatah Bahbouh' },
        [pscustomobject]@{ Name = 'LegalCopyright'; Actual = $version.LegalCopyright; Expected = 'Copyright (c) 2026 Abdulfatah Bahbouh' }
    )) {
        Assert-True -Condition ([string]$field.Actual -eq [string]$field.Expected) `
            -Message "$($field.Name) is '$($field.Actual)', expected '$($field.Expected)'."
    }
    Assert-True -Condition (
        $version.FileMajorPart -eq $major -and
        $version.FileMinorPart -eq $minor -and
        $version.FileBuildPart -eq $patch -and
        $version.FilePrivatePart -eq 0 -and
        $version.ProductMajorPart -eq $major -and
        $version.ProductMinorPart -eq $minor -and
        $version.ProductBuildPart -eq $patch -and
        $version.ProductPrivatePart -eq 0
    ) -Message 'Binary file/product numeric version parts do not match the workspace version.'
    Assert-True -Condition (-not $version.IsDebug) `
        -Message 'Release VERSIONINFO unexpectedly carries the debug flag.'

    $manifestTool = Find-WindowsSdkTool -Name 'mt.exe'
    Assert-True -Condition (-not [string]::IsNullOrWhiteSpace($manifestTool)) `
        -Message 'Windows SDK mt.exe was not found for embedded-manifest validation.'
    $manifestPath = Join-Path ([System.IO.Path]::GetTempPath()) `
        ("LeanRows-manifest-$PID-" + [guid]::NewGuid().ToString('N') + '.xml')
    try {
        & $manifestTool '-nologo' "-inputresource:$binary;#1" "-out:$manifestPath"
        Assert-True -Condition ($LASTEXITCODE -eq 0 -and [System.IO.File]::Exists($manifestPath)) `
            -Message 'mt.exe could not extract the embedded application manifest.'
        & $manifestTool '-nologo' '-manifest' $manifestPath '-validate_manifest'
        Assert-True -Condition ($LASTEXITCODE -eq 0) `
            -Message 'mt.exe rejected the embedded application manifest.'
        $manifestText = [System.IO.File]::ReadAllText($manifestPath)
        Assert-Matches -Text $manifestText -Pattern 'version="0\.1\.0\.0"' `
            -Message 'Embedded manifest identity version is not 0.1.0.0.'
        Assert-Matches -Text $manifestText -Pattern '>PerMonitorV2<' `
            -Message 'Embedded manifest lost PerMonitorV2 DPI awareness.'
    }
    finally {
        if ([System.IO.File]::Exists($manifestPath)) {
            Remove-Item -LiteralPath $manifestPath -Force
        }
    }
}

function Test-DeterministicPackage {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ApplicationBinary
    )

    $binary = [System.IO.Path]::GetFullPath($ApplicationBinary)
    Assert-True -Condition ([System.IO.File]::Exists($binary)) `
        -Message "Package test binary does not exist: $binary"

    $temporaryBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
    $temporaryRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $temporaryBase ("LeanRows-release-test-$PID-" + [guid]::NewGuid().ToString('N')))
    )
    Assert-True -Condition ($temporaryRoot.StartsWith(
            $temporaryBase + '\',
            [System.StringComparison]::OrdinalIgnoreCase)) `
        -Message 'Temporary package test directory escaped the system temporary directory.'

    [System.IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null
    try {
        $version = & (Join-Path $PSScriptRoot 'Get-WorkspaceVersion.ps1')
        $firstDirectory = Join-Path $temporaryRoot 'first'
        $secondDirectory = Join-Path $temporaryRoot 'second'
        $first = & (Join-Path $PSScriptRoot 'Package-Windows.ps1') `
            -BinaryPath $binary -Version $version -OutputDirectory $firstDirectory
        $second = & (Join-Path $PSScriptRoot 'Package-Windows.ps1') `
            -BinaryPath $binary -Version $version -OutputDirectory $secondDirectory

        Assert-True -Condition ($first.Sha256 -eq $second.Sha256) `
            -Message 'Two packages from identical inputs produced different SHA-256 digests.'
        Test-ChecksumArtifact -Directory $firstDirectory
        Test-ChecksumArtifact -Directory $secondDirectory

        $extractDirectory = Join-Path $temporaryRoot 'extracted'
        [System.IO.Compression.ZipFile]::ExtractToDirectory($first.ZipPath, $extractDirectory)
        $packageRoot = @(Get-ChildItem -LiteralPath $extractDirectory -Directory)
        Assert-True -Condition ($packageRoot.Count -eq 1) `
            -Message 'Portable ZIP must contain one top-level directory.'

        $expectedNames = @(
            'install.ps1',
            'leanrows.exe',
            'LICENSE.txt',
            'manifest.json',
            'uninstall.ps1',
            'UNSIGNED.txt'
        )
        $actualNames = @(
            Get-ChildItem -LiteralPath $packageRoot[0].FullName -File |
                Select-Object -ExpandProperty Name |
                Sort-Object
        )
        Assert-True -Condition (($actualNames -join ',') -eq (($expectedNames | Sort-Object) -join ',')) `
            -Message "Portable package contains an unexpected file set: $($actualNames -join ', ')"

        $manifest = Get-Content -LiteralPath (Join-Path $packageRoot[0].FullName 'manifest.json') `
            -Raw | ConvertFrom-Json
        Assert-True -Condition ([string]$manifest.signing_status -eq 'unsigned') `
            -Message 'Package manifest must disclose unsigned status.'
        $manifestExtensions = @($manifest.file_associations | Sort-Object)
        Assert-True -Condition (($manifestExtensions -join ',') -eq ($expectedExtensions -join ',')) `
            -Message 'Package manifest association set is not the approved set.'

        & (Join-Path $packageRoot[0].FullName 'install.ps1') -ValidateOnly | Out-Host
    }
    finally {
        $resolvedForCleanup = [System.IO.Path]::GetFullPath($temporaryRoot)
        if ($resolvedForCleanup.StartsWith(
                $temporaryBase + '\LeanRows-release-test-',
                [System.StringComparison]::OrdinalIgnoreCase) -and
            [System.IO.Directory]::Exists($resolvedForCleanup)) {
            Remove-Item -LiteralPath $resolvedForCleanup -Recurse -Force
        }
    }
}

Test-PowerShellParsing
Test-InstallerContract
Test-WindowsSourceContract
Test-WorkflowContract

if (-not [string]::IsNullOrWhiteSpace($BinaryPath)) {
    Test-WindowsBinaryMetadata -ApplicationBinary $BinaryPath
    Test-DeterministicPackage -ApplicationBinary $BinaryPath
}

if (-not [string]::IsNullOrWhiteSpace($ArtifactDirectory)) {
    Test-ChecksumArtifact -Directory $ArtifactDirectory
}

Write-Output 'LeanRows release-engineering gates passed.'
