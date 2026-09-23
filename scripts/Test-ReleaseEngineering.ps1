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

    $tokens = $null
    $parseErrors = $null
    $installAst = [System.Management.Automation.Language.Parser]::ParseFile(
        $installPath, [ref]$tokens, [ref]$parseErrors
    )
    Assert-True -Condition (@($parseErrors).Count -eq 0) `
        -Message 'Installer could not be parsed for its registry binding contract.'
    $setRegistryString = $installAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Set-RegistryString'
        }, $true)
    Assert-True -Condition ($null -ne $setRegistryString) `
        -Message 'Set-RegistryString was not found in the installer.'
    $installGetNamedState = $installAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Get-RegistryNamedValueState'
        }, $true)
    Assert-True -Condition ($null -ne $installGetNamedState) `
        -Message 'Installer named registry-state reader was not found.'
    $installGetDefaultState = $installAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Get-RegistryDefaultState'
        }, $true)
    Assert-True -Condition ($null -ne $installGetDefaultState) `
        -Message 'Installer default registry-state reader was not found.'
    $nameParameter = @($setRegistryString.Body.ParamBlock.Parameters |
            Where-Object { $_.Name.VariablePath.UserPath -eq 'Name' })
    Assert-True -Condition ($nameParameter.Count -eq 1 -and
        @($nameParameter[0].Attributes.TypeName.FullName) -contains 'AllowEmptyString') `
        -Message 'Set-RegistryString Name must allow an empty registry default-value name.'

    $ensureRegistryKey = $installAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Ensure-RegistryKey'
        }, $true)
    $setRegistryDword = $installAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Set-RegistryDword'
        }, $true)
    Assert-True -Condition ($null -ne $ensureRegistryKey -and
        $null -ne $setRegistryDword) `
        -Message 'Safe registry-key creation or DWORD writing helper is missing.'

    $registryProbe = @{
        Values = @{}
        NewItems = 0
        DestructiveRecreates = 0
    }
    & {
        param($ensureText, $stringText, $dwordText, $state)

        function Test-Path {
            param([string]$LiteralPath)
            return $state.Values.ContainsKey($LiteralPath)
        }

        function New-Item {
            param([string]$Path, [switch]$Force)
            if ($state.Values.ContainsKey($Path)) {
                # Model the registry provider behavior that triggered the live
                # defect: force-creating an existing key clears its values.
                $state.DestructiveRecreates++
                $state.Values[$Path] = @{}
            }
            else {
                $state.NewItems++
                $state.Values[$Path] = @{}
            }
        }
        function Set-Item {
            param([string]$Path, [AllowEmptyString()][string]$Value)
            if (-not $state.Values.ContainsKey($Path)) {
                throw 'Mocked default-value write targeted a missing key.'
            }
            $state.Values[$Path][''] = $Value
        }
        function New-ItemProperty {
            param(
                [string]$Path,
                [string]$Name,
                [AllowEmptyString()][string]$Value,
                [string]$PropertyType,
                [switch]$Force
            )
            if (-not $state.Values.ContainsKey($Path)) {
                throw 'Mocked named-value write targeted a missing key.'
            }
            $state.Values[$Path][$Name] = $Value
        }

        . ([scriptblock]::Create($ensureText))
        . ([scriptblock]::Create($stringText))
        . ([scriptblock]::Create($dwordText))
        Set-RegistryString -Path 'HKCU:\LeanRowsBindingTest\DefaultIcon' `
            -Name '' -Value 'leanrows.exe,0'
        Set-RegistryString -Path 'HKCU:\LeanRowsBindingTest\SupportedTypes' `
            -Name '.csv' -Value ''
        Set-RegistryString -Path 'HKCU:\LeanRowsBindingTest\SupportedTypes' `
            -Name '.tsv' -Value ''
        Set-RegistryString -Path 'HKCU:\LeanRowsBindingTest\Uninstall' `
            -Name 'DisplayName' -Value 'LeanRows'
        Set-RegistryDword -Path 'HKCU:\LeanRowsBindingTest\Uninstall' `
            -Name 'NoModify' -Value 1
    } $ensureRegistryKey.Extent.Text $setRegistryString.Extent.Text `
        $setRegistryDword.Extent.Text $registryProbe
    Assert-True -Condition ($registryProbe.NewItems -eq 3 -and
        $registryProbe.DestructiveRecreates -eq 0 -and
        $registryProbe.Values['HKCU:\LeanRowsBindingTest\DefaultIcon'][''] -eq
            'leanrows.exe,0' -and
        $registryProbe.Values['HKCU:\LeanRowsBindingTest\SupportedTypes'].ContainsKey('.csv') -and
        $registryProbe.Values['HKCU:\LeanRowsBindingTest\SupportedTypes'].ContainsKey('.tsv') -and
        $registryProbe.Values['HKCU:\LeanRowsBindingTest\Uninstall']['DisplayName'] -eq
            'LeanRows' -and
        $registryProbe.Values['HKCU:\LeanRowsBindingTest\Uninstall']['NoModify'] -eq 1) `
        -Message 'Repeated registry writes recreated a key or cleared a prior value.'

    $setExtensionDefault = $installAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Set-ExtensionDefault'
        }, $true)
    Assert-True -Condition ($null -ne $setExtensionDefault) `
        -Message 'Set-ExtensionDefault was not found in the installer.'

    $uninstallTokens = $null
    $uninstallParseErrors = $null
    $uninstallAst = [System.Management.Automation.Language.Parser]::ParseFile(
        $uninstallPath, [ref]$uninstallTokens, [ref]$uninstallParseErrors
    )
    Assert-True -Condition (@($uninstallParseErrors).Count -eq 0) `
        -Message 'Uninstaller could not be parsed for its default-restore contract.'
    $restoreExtensionDefault = $uninstallAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Restore-ExtensionDefault'
        }, $true)
    Assert-True -Condition ($null -ne $restoreExtensionDefault) `
        -Message 'Restore-ExtensionDefault was not found in the uninstaller.'
    $uninstallGetNamedState = $uninstallAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Get-RegistryNamedValueState'
        }, $true)
    Assert-True -Condition ($null -ne $uninstallGetNamedState) `
        -Message 'Uninstaller named registry-state reader was not found.'
    $removeRegistryKeyIfEmpty = $uninstallAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Remove-RegistryKeyIfEmpty'
        }, $true)
    Assert-True -Condition ($null -ne $removeRegistryKeyIfEmpty) `
        -Message 'Conditional application-root cleanup helper was not found.'
    $setRegistryDefaultValue = $uninstallAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Set-RegistryDefaultValue'
        }, $true)
    $removeRegistryDefaultValue = $uninstallAst.Find({
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq 'Remove-RegistryDefaultValue'
        }, $true)
    Assert-True -Condition ($null -ne $setRegistryDefaultValue -and
        $null -ne $removeRegistryDefaultValue) `
        -Message 'Writable default-value restoration helpers are missing.'

    # Exercise the install/restore algorithms against an in-memory model. The
    # probe intentionally never opens or writes the real current-user registry.
    $associationProbe = @{
        Defaults = @{}
        Named = @{}
        UserChoices = @{}
        StateWrites = 0
    }
    & {
        param($installFunctionText, $restoreFunctionText, $state)

        $classesRoot = 'classes'
        $classesSubKeyRoot = 'Software\Classes'
        $fileExtsRoot = 'file-exts'
        $installStateRoot = 'state'
        $probeProgId = 'LeanRows.AssocFile.v1'
        $supportedExtensions = @('.csv', '.tsv', '.jsonl', '.ndjson', '.log')

        function Test-Path {
            param([string]$LiteralPath)
            return $state.UserChoices.ContainsKey($LiteralPath)
        }
        function Get-RegistryDefaultState {
            param([string]$Path, [string]$SubKey)
            $modelPath = $Path
            if ($PSBoundParameters.ContainsKey('SubKey')) {
                if (-not $SubKey.StartsWith($classesSubKeyRoot + '\')) {
                    throw 'Mocked restore escaped the approved Classes subkey.'
                }
                $modelPath = $classesRoot + $SubKey.Substring($classesSubKeyRoot.Length)
            }
            if ($state.Defaults.ContainsKey($modelPath)) {
                return [pscustomobject]@{
                    Present = $true
                    Value = $state.Defaults[$modelPath]
                }
            }
            return [pscustomobject]@{ Present = $false; Value = $null }
        }
        function Get-RegistryNamedValueState {
            param([string]$Path, [string]$Name)
            if ($Name -eq 'ProgId' -and $state.UserChoices.ContainsKey($Path)) {
                return [pscustomobject]@{
                    Present = $true
                    Value = $state.UserChoices[$Path]
                    Kind = [Microsoft.Win32.RegistryValueKind]::String
                }
            }
            $key = $Path + '|' + $Name
            if ($state.Named.ContainsKey($key)) {
                $entry = $state.Named[$key]
                return [pscustomobject]@{
                    Present = $true
                    Value = $entry.Value
                    Kind = $entry.Kind
                }
            }
            return [pscustomobject]@{
                Present = $false
                Value = $null
                Kind = $null
            }
        }
        function Set-RegistryString {
            param(
                [string]$Path,
                [AllowEmptyString()][string]$Name,
                [AllowEmptyString()][string]$Value
            )
            if ($Name.Length -eq 0) {
                $state.Defaults[$Path] = $Value
            }
            else {
                $state.Named[$Path + '|' + $Name] = [pscustomobject]@{
                    Value = $Value
                    Kind = [Microsoft.Win32.RegistryValueKind]::String
                }
                $state.StateWrites++
            }
        }
        function Set-RegistryDword {
            param([string]$Path, [string]$Name, [int]$Value)
            $state.Named[$Path + '|' + $Name] = [pscustomobject]@{
                Value = $Value
                Kind = [Microsoft.Win32.RegistryValueKind]::DWord
            }
            $state.StateWrites++
        }
        function Set-RegistryDefaultValue {
            param([string]$SubKey, [AllowEmptyString()][string]$Value)
            if (-not $SubKey.StartsWith($classesSubKeyRoot + '\')) {
                throw 'Mocked default restore escaped the approved Classes subkey.'
            }
            $modelPath = $classesRoot + $SubKey.Substring($classesSubKeyRoot.Length)
            $state.Defaults[$modelPath] = $Value
        }
        function Remove-RegistryDefaultValue {
            param([string]$SubKey)
            if (-not $SubKey.StartsWith($classesSubKeyRoot + '\')) {
                throw 'Mocked default removal escaped the approved Classes subkey.'
            }
            $modelPath = $classesRoot + $SubKey.Substring($classesSubKeyRoot.Length)
            [void]$state.Defaults.Remove($modelPath)
        }

        . ([scriptblock]::Create($installFunctionText))
        . ([scriptblock]::Create($restoreFunctionText))

        $csvPath = Join-Path $classesRoot '.csv'
        $state.Named['state|csv_HadPrevious'] = [pscustomobject]@{
            Value = 1
            Kind = [Microsoft.Win32.RegistryValueKind]::DWord
        }
        $state.Named['state|csv_Previous'] = [pscustomobject]@{
            Value = 7
            Kind = [Microsoft.Win32.RegistryValueKind]::DWord
        }
        $csvResult = Set-ExtensionDefault -Extension '.csv' `
            -ClassesRoot $classesRoot -FileExtsRoot $fileExtsRoot `
            -InstallStateRoot $installStateRoot -ProgId $probeProgId
        Assert-True -Condition ($csvResult -eq 'DefaultSet' -and
            $state.Defaults[$csvPath] -eq $probeProgId -and
            $state.Named['state|csv_HadPrevious'].Value -eq 0 -and
            $state.Named['state|csv_HadPrevious'].Kind -eq
                [Microsoft.Win32.RegistryValueKind]::DWord -and
            $state.Named['state|csv_Previous'].Value -eq '' -and
            $state.Named['state|csv_Previous'].Kind -eq
                [Microsoft.Win32.RegistryValueKind]::String) `
            -Message 'Empty direct-default state was not captured safely.'

        $writesAfterFirstInstall = $state.StateWrites
        $csvReinstallResult = Set-ExtensionDefault -Extension '.csv' `
            -ClassesRoot $classesRoot -FileExtsRoot $fileExtsRoot `
            -InstallStateRoot $installStateRoot -ProgId $probeProgId
        Assert-True -Condition ($csvReinstallResult -eq 'DefaultSet' -and
            $state.StateWrites -eq $writesAfterFirstInstall -and
            $state.Named['state|csv_HadPrevious'].Value -eq 0 -and
            $state.Named['state|csv_Previous'].Value -ne $probeProgId) `
            -Message 'Reinstall recorded LeanRows as its own previous handler.'

        $tsvPath = Join-Path $classesRoot '.tsv'
        $state.Defaults[$tsvPath] = 'Excel.CSV'
        $state.Named['state|tsv_HadPrevious'] = [pscustomobject]@{
            Value = 1
            Kind = [Microsoft.Win32.RegistryValueKind]::DWord
        }
        $state.Named['state|tsv_Previous'] = [pscustomobject]@{
            Value = $probeProgId
            Kind = [Microsoft.Win32.RegistryValueKind]::String
        }
        $tsvResult = Set-ExtensionDefault -Extension '.tsv' `
            -ClassesRoot $classesRoot -FileExtsRoot $fileExtsRoot `
            -InstallStateRoot $installStateRoot -ProgId $probeProgId
        Assert-True -Condition ($tsvResult -eq 'DefaultSet' -and
            $state.Defaults[$tsvPath] -eq $probeProgId -and
            $state.Named['state|tsv_HadPrevious'].Value -eq 1 -and
            $state.Named['state|tsv_HadPrevious'].Kind -eq
                [Microsoft.Win32.RegistryValueKind]::DWord -and
            $state.Named['state|tsv_Previous'].Value -eq 'Excel.CSV' -and
            $state.Named['state|tsv_Previous'].Kind -eq
                [Microsoft.Win32.RegistryValueKind]::String) `
            -Message 'A legacy self-predecessor was not replaced by the current handler.'

        $logPath = Join-Path $classesRoot '.log'
        $logChoicePath = Join-Path (Join-Path $fileExtsRoot '.log') 'UserChoice'
        $state.UserChoices[$logChoicePath] = 'Applications\notepad.exe'
        $logResult = Set-ExtensionDefault -Extension '.log' `
            -ClassesRoot $classesRoot -FileExtsRoot $fileExtsRoot `
            -InstallStateRoot $installStateRoot -ProgId $probeProgId
        Assert-True -Condition ($logResult -eq 'UserChoicePreserved' -and
            -not $state.Defaults.ContainsKey($logPath) -and
            -not $state.Named.ContainsKey('state|log_HadPrevious') -and
            $state.UserChoices[$logChoicePath] -eq 'Applications\notepad.exe') `
            -Message 'A protected UserChoice was not preserved without direct-default writes.'

        $jsonlPath = Join-Path $classesRoot '.jsonl'
        $jsonlChoicePath = Join-Path (Join-Path $fileExtsRoot '.jsonl') 'UserChoice'
        $state.UserChoices[$jsonlChoicePath] = $probeProgId
        $jsonlResult = Set-ExtensionDefault -Extension '.jsonl' `
            -ClassesRoot $classesRoot -FileExtsRoot $fileExtsRoot `
            -InstallStateRoot $installStateRoot -ProgId $probeProgId
        Assert-True -Condition ($jsonlResult -eq 'AlreadySelected' -and
            -not $state.Defaults.ContainsKey($jsonlPath)) `
            -Message 'An existing protected LeanRows selection was not recognized.'

        $ndjsonPath = Join-Path $classesRoot '.ndjson'
        $state.Named['state|ndjson_HadPrevious'] = [pscustomobject]@{
            Value = '1'
            Kind = [Microsoft.Win32.RegistryValueKind]::String
        }
        $state.Named['state|ndjson_Previous'] = [pscustomobject]@{
            Value = $probeProgId
            Kind = [Microsoft.Win32.RegistryValueKind]::String
        }
        [void](Set-ExtensionDefault -Extension '.ndjson' `
                -ClassesRoot $classesRoot -FileExtsRoot $fileExtsRoot `
                -InstallStateRoot $installStateRoot -ProgId $probeProgId)
        Assert-True -Condition (
            $state.Named['state|ndjson_HadPrevious'].Value -eq 0 -and
            $state.Named['state|ndjson_HadPrevious'].Kind -eq
                [Microsoft.Win32.RegistryValueKind]::DWord -and
            $state.Named['state|ndjson_Previous'].Value -eq '' -and
            $state.Named['state|ndjson_Previous'].Kind -eq
                [Microsoft.Win32.RegistryValueKind]::String) `
            -Message 'Installer trusted a REG_SZ HadPrevious flag during repair.'
        $state.Defaults[$ndjsonPath] = 'User.NewHandler'

        $state.Named['state|csv_HadPrevious'] = [pscustomobject]@{
            Value = '1'
            Kind = [Microsoft.Win32.RegistryValueKind]::String
        }
        $state.Named['state|csv_Previous'] = [pscustomobject]@{
            Value = 'Hijacked.Handler'
            Kind = [Microsoft.Win32.RegistryValueKind]::String
        }

        $tsvRestore = Restore-ExtensionDefault -Extension '.tsv' `
            -ClassesSubKeyRoot $classesSubKeyRoot `
            -InstallStateRoot $installStateRoot `
            -ProgId $probeProgId
        $csvRestore = Restore-ExtensionDefault -Extension '.csv' `
            -ClassesSubKeyRoot $classesSubKeyRoot `
            -InstallStateRoot $installStateRoot `
            -ProgId $probeProgId
        $ndjsonRestore = Restore-ExtensionDefault -Extension '.ndjson' `
            -ClassesSubKeyRoot $classesSubKeyRoot `
            -InstallStateRoot $installStateRoot `
            -ProgId $probeProgId
        Assert-True -Condition ($tsvRestore -eq 'Restored' -and
            $state.Defaults[$tsvPath] -eq 'Excel.CSV') `
            -Message 'Uninstall did not restore the recorded direct default.'
        Assert-True -Condition ($csvRestore -eq 'Removed' -and
            -not $state.Defaults.ContainsKey($csvPath)) `
            -Message 'Uninstall did not remove a LeanRows default with no predecessor.'
        Assert-True -Condition ($ndjsonRestore -eq 'Preserved' -and
            $state.Defaults[$ndjsonPath] -eq 'User.NewHandler') `
            -Message 'Uninstall overwrote a later user default selection.'
    } $setExtensionDefault.Extent.Text $restoreExtensionDefault.Extent.Text $associationProbe

    foreach ($valueWriter in @(
            $setRegistryString,
            $setRegistryDword,
            $setRegistryDefaultValue
        )) {
        Assert-True -Condition (-not [regex]::IsMatch(
                $valueWriter.Extent.Text,
                '(?im)^\s*New-Item\s+[^\r\n]*-Force')) `
            -Message "$($valueWriter.Name) must not force-create an existing registry key."
    }
    Assert-Matches -Text $ensureRegistryKey.Extent.Text `
        -Pattern '(?s)if\s*\(-not\s*\(Test-Path\s+-LiteralPath\s+\$Path\)\).*?New-Item\s+-Path\s+\$Path\s+-Force' `
        -Message 'Registry key creation is not gated on exact key absence.'
    $installerForceCreates = @([regex]::Matches(
            $installText,
            '(?im)^\s*New-Item\s+[^\r\n]*-Force'))
    Assert-True -Condition ($installerForceCreates.Count -eq 1) `
        -Message 'Installer has an unaudited force-create outside Ensure-RegistryKey.'
    Assert-True -Condition (-not [regex]::IsMatch(
            $uninstallText,
            '(?im)^\s*New-Item\s+[^\r\n]*-Force')) `
        -Message 'Uninstaller must not force-create registry keys during restoration.'

    Assert-Matches -Text $setRegistryDefaultValue.Extent.Text `
        -Pattern 'CurrentUser\.CreateSubKey\(\$SubKey\)' `
        -Message 'Default restoration does not open the exact HKCU subkey safely.'
    Assert-Matches -Text $removeRegistryDefaultValue.Extent.Text `
        -Pattern 'CurrentUser\.OpenSubKey\(\$SubKey,\s*\$true\)' `
        -Message 'Default removal does not open the exact HKCU subkey writable.'
    Assert-Matches -Text $removeRegistryDefaultValue.Extent.Text `
        -Pattern '\.DeleteValue\('''',\s*\$false\)' `
        -Message 'Default removal does not delete only the unnamed registry value.'
    foreach ($writableHelper in @(
            $setRegistryDefaultValue,
            $removeRegistryDefaultValue
        )) {
        Assert-True -Condition (-not $writableHelper.Extent.Text.Contains('Get-Item')) `
            -Message "$($writableHelper.Name) must not use a read-only provider key."
        Assert-Matches -Text $writableHelper.Extent.Text `
            -Pattern '(?s)finally\s*\{\s*\$key\.Dispose\(\)' `
            -Message "$($writableHelper.Name) does not dispose its writable registry key."
    }
    Assert-Matches -Text $restoreExtensionDefault.Extent.Text `
        -Pattern '\$ClassesSubKeyRoot\s+-ne\s+''Software\\Classes''' `
        -Message 'Default restore is not pinned to the per-user Classes subkey.'
    Assert-Matches -Text $restoreExtensionDefault.Extent.Text `
        -Pattern '\$supportedExtensions\s+-notcontains\s+\$Extension' `
        -Message 'Default restore does not enforce the supported-extension allow-list.'
    foreach ($namedStateReader in @(
            $installGetNamedState,
            $uninstallGetNamedState
        )) {
        Assert-Matches -Text $namedStateReader.Extent.Text `
            -Pattern '\.GetValueKind\(\$Name\)' `
            -Message "$($namedStateReader.Name) does not preserve registry value kinds."
        Assert-Matches -Text $namedStateReader.Extent.Text `
            -Pattern '(?s)finally\s*\{\s*\$key\.Dispose\(\)' `
            -Message "$($namedStateReader.Name) does not dispose its registry key."
    }
    Assert-Matches -Text $installGetDefaultState.Extent.Text `
        -Pattern '(?s)finally\s*\{\s*\$key\.Dispose\(\)' `
        -Message 'Installer default-state reader does not dispose its registry key.'
    Assert-Matches -Text $removeRegistryKeyIfEmpty.Extent.Text `
        -Pattern '\.GetSubKeyNames\(\)' `
        -Message 'Application-root cleanup does not inspect children through its key.'
    Assert-Matches -Text $removeRegistryKeyIfEmpty.Extent.Text `
        -Pattern '(?s)finally\s*\{\s*\$key\.Dispose\(\).*?Remove-Item' `
        -Message 'Application-root cleanup does not dispose its key before removal.'
    Assert-Matches -Text $setExtensionDefault.Extent.Text `
        -Pattern 'elseif\s*\(-not\s+\$stateValid\s+-or\s+\$recordedSelfAsPrevious\)' `
        -Message 'Installer does not repair a self-predecessor for a current other handler.'
    Assert-Matches -Text $setExtensionDefault.Extent.Text `
        -Pattern 'RegistryValueKind\]::DWord' `
        -Message 'Installer does not require a DWORD HadPrevious flag.'
    Assert-Matches -Text $setExtensionDefault.Extent.Text `
        -Pattern 'RegistryValueKind\]::String' `
        -Message 'Installer does not require a string Previous value.'
    Assert-Matches -Text $restoreExtensionDefault.Extent.Text `
        -Pattern 'RegistryValueKind\]::DWord' `
        -Message 'Uninstaller does not require a DWORD HadPrevious flag.'
    Assert-Matches -Text $restoreExtensionDefault.Extent.Text `
        -Pattern 'RegistryValueKind\]::String' `
        -Message 'Uninstaller does not require a string Previous value.'

    foreach ($text in @($installText, $uninstallText)) {
        $actualExtensions = @(Get-DeclaredExtensions -ScriptText $text)
        Assert-True -Condition (($actualExtensions -join ',') -eq ($expectedExtensions -join ',')) `
            -Message "Association set differs from the approved allow-list: $($actualExtensions -join ', ')"
    }

    Assert-Matches -Text $installText `
        -Pattern 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts' `
        -Message 'Installer does not inspect the fixed per-user FileExts root.'
    Assert-Matches -Text $setExtensionDefault.Extent.Text `
        -Pattern 'Test-Path\s+-LiteralPath\s+\$userChoicePath' `
        -Message 'Installer does not gate direct defaults on protected UserChoice presence.'
    Assert-True -Condition (-not [regex]::IsMatch(
            $setExtensionDefault.Extent.Text,
            '(?im)^\s*(?:New-Item(?:Property)?|Set-Item|Set-Registry(?:String|Dword)|Remove-Item(?:Property)?)\b[^\r\n]*\$userChoicePath')) `
        -Message 'Installer must never mutate or delete the protected UserChoice key.'
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
        -Pattern '(?s)foreach\s*\(\$extension\s+in\s+\$supportedExtensions\).*?OpenWithProgids.*?Set-ExtensionDefault' `
        -Message 'Every supported extension must receive Open With registration before default handling.'
    Assert-Matches -Text $installText -Pattern 'Software\\LeanRows\\InstallState' `
        -Message 'Installer does not use an owned direct-default state key.'
    Assert-Matches -Text $installText `
        -Pattern 'if\s*\(\$OpenDefaultApps\s+-and\s+\$protectedSelections\.Count\s+-gt\s+0\)' `
        -Message 'Default Apps must open only when protected selections need consent.'
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
    Assert-Matches -Text $uninstallText -Pattern 'Restore-ExtensionDefault' `
        -Message 'Uninstaller does not restore safe direct-default state.'
    Assert-Matches -Text $uninstallText `
        -Pattern 'Remove-OwnedRegistryTree\s+-Path\s+\$installStateRoot' `
        -Message 'Uninstaller does not remove its owned direct-default state.'
    Assert-Matches -Text $uninstallText `
        -Pattern 'Remove-RegistryKeyIfEmpty\s+-Path\s+\$appRoot' `
        -Message 'Uninstaller does not conditionally clean its empty application root.'
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
        -Pattern '(?s)#define IDR_ACCELERATORS 102.*?IDR_ACCELERATORS ACCELERATORS\s*BEGIN\s*"O",\s*ID_FILE_OPEN,\s*VIRTKEY,\s*CONTROL\s*0x74,\s*ID_FILE_RELOAD,\s*VIRTKEY\s*"C",\s*ID_EDIT_COPY,\s*VIRTKEY,\s*CONTROL\s*"F",\s*ID_EDIT_FIND,\s*VIRTKEY,\s*CONTROL\s*0x72,\s*ID_EDIT_FIND_NEXT,\s*VIRTKEY\s*0x72,\s*ID_EDIT_FIND_PREVIOUS,\s*VIRTKEY,\s*SHIFT\s*"G",\s*ID_EDIT_GOTO,\s*VIRTKEY,\s*CONTROL\s*"W",\s*ID_TAB_CLOSE,\s*VIRTKEY,\s*CONTROL\s*0x73,\s*ID_TAB_CLOSE,\s*VIRTKEY,\s*CONTROL\s*0x09,\s*ID_TAB_NEXT,\s*VIRTKEY,\s*CONTROL\s*0x09,\s*ID_TAB_PREVIOUS,\s*VIRTKEY,\s*CONTROL,\s*SHIFT\s*0x22,\s*ID_TAB_NEXT,\s*VIRTKEY,\s*CONTROL\s*0x21,\s*ID_TAB_PREVIOUS,\s*VIRTKEY,\s*CONTROL\s*"1",\s*ID_TAB_SELECT_FIRST,\s*VIRTKEY,\s*CONTROL\s*"2",\s*142,\s*VIRTKEY,\s*CONTROL\s*"3",\s*143,\s*VIRTKEY,\s*CONTROL\s*"4",\s*144,\s*VIRTKEY,\s*CONTROL\s*"5",\s*145,\s*VIRTKEY,\s*CONTROL\s*"6",\s*146,\s*VIRTKEY,\s*CONTROL\s*"7",\s*147,\s*VIRTKEY,\s*CONTROL\s*"8",\s*148,\s*VIRTKEY,\s*CONTROL\s*"9",\s*ID_TAB_SELECT_LAST,\s*VIRTKEY,\s*CONTROL\s*END' `
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
        ID_TAB_CLOSE = 130; ID_TAB_NEXT = 131; ID_TAB_PREVIOUS = 132
        ID_TAB_SELECT_FIRST = 141; ID_TAB_SELECT_LAST = 149
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
    $workspaceVersion = [string](& (Join-Path $PSScriptRoot 'Get-WorkspaceVersion.ps1'))
    $releaseNotesPattern = '--notes-file \./docs/releases/v' +
        [regex]::Escape($workspaceVersion) + '\.md'
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
        'Release notes file' = $releaseNotesPattern
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
        Assert-Matches -Text $manifestText `
            -Pattern ('version="' + [regex]::Escape($expectedVersion) + '"') `
            -Message "Embedded manifest identity version is not $expectedVersion."
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
