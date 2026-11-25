[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [ValidateRange(5, 120)][int]$TimeoutSeconds = 30
)

# Opens LeanRows in English, Simplified Chinese and Japanese and checks that
# the window shows each language's own strings, as the string tables in
# crates/leanrows-win32/src/localization give them. For each language it saves
# the choice where the settings panel saves it (HKCU\Software\LeanRows,
# Language), runs the native shell smoke in that language, then starts the
# window normally and reads its controls' text. The reader's own choice is put
# back afterwards.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) {
        throw $Message
    }
}

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$tableDirectory = Join-Path $root 'crates\leanrows-win32\src\localization'
$tableFiles = [ordered]@{
    'en' = 'english.rs'
    'zh-CN' = 'chinese.rs'
    'ja' = 'japanese.rs'
}

function Read-StringTable {
    param([Parameter(Mandatory = $true)][string]$Path)
    Assert-True -Condition (Test-Path -LiteralPath $Path -PathType Leaf) `
        -Message "The string table $Path does not exist."
    $table = @{}
    foreach ($line in [System.IO.File]::ReadAllLines($Path, [System.Text.Encoding]::UTF8)) {
        if ($line -match '^\s+([a-z0-9_]+): "(.*)",$') {
            $table[$Matches[1]] = $Matches[2].Replace('\"', '"').Replace('\\', '\')
        }
    }
    Assert-True -Condition ($table.Count -gt 200) `
        -Message "The string table $Path has only $($table.Count) strings."
    $table
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class LeanRowsWindowText
{
    private delegate bool EnumProc(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowTextW(IntPtr window, StringBuilder text, int count);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassNameW(IntPtr window, StringBuilder text, int count);

    public static string[] ChildTexts(IntPtr parent)
    {
        var texts = new List<string>();
        EnumChildWindows(parent, (window, parameter) =>
        {
            var className = new StringBuilder(64);
            GetClassNameW(window, className, className.Capacity);
            var text = new StringBuilder(1024);
            GetWindowTextW(window, text, text.Capacity);
            texts.Add(className.ToString() + "\t" + text.ToString());
            return true;
        }, IntPtr.Zero);
        return texts.ToArray();
    }
}
'@

function Get-SavedLanguage {
    $key = Get-Item -LiteralPath 'HKCU:\Software\LeanRows' -ErrorAction SilentlyContinue
    if ($null -eq $key) {
        return $null
    }
    $key.GetValue('Language', $null)
}

function Set-SavedLanguage {
    param([AllowNull()][string]$Value)
    if ($null -eq $Value) {
        Remove-ItemProperty -LiteralPath 'HKCU:\Software\LeanRows' -Name 'Language' `
            -ErrorAction SilentlyContinue
        return
    }
    $null = New-Item -Path 'HKCU:\Software\LeanRows' -Force -ErrorAction SilentlyContinue
    Set-ItemProperty -LiteralPath 'HKCU:\Software\LeanRows' -Name 'Language' -Value $Value -Type String
}

function Invoke-ShellSmoke {
    param([Parameter(Mandatory = $true)][string]$Executable)
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.Arguments = '--smoke-test'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($startInfo)
    try {
        $stderr = $process.StandardError.ReadToEndAsync()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            throw 'The native shell smoke timed out.'
        }
        [pscustomobject]@{
            exit_code = $process.ExitCode
            stdout = [string]$stdout.Result
            stderr = [string]$stderr.Result
        }
    }
    finally {
        $process.Dispose()
    }
}

function Get-WindowTexts {
    param([Parameter(Mandatory = $true)][string]$Executable)
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $process = [System.Diagnostics.Process]::Start($startInfo)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        $texts = @()
        do {
            Start-Sleep -Milliseconds 100
            $process.Refresh()
            if ($process.MainWindowHandle -ne [IntPtr]::Zero -and $process.MainWindowTitle -eq 'LeanRows') {
                $texts = [LeanRowsWindowText]::ChildTexts($process.MainWindowHandle)
                if ($texts.Count -gt 10) {
                    break
                }
            }
        } while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited)
        Assert-True -Condition ($texts.Count -gt 10) -Message 'The LeanRows window did not appear.'
        Assert-True -Condition $process.CloseMainWindow() -Message 'The window did not accept WM_CLOSE.'
        Assert-True -Condition $process.WaitForExit($TimeoutSeconds * 1000) `
            -Message 'LeanRows did not exit after WM_CLOSE.'
        Assert-True -Condition ($process.ExitCode -eq 0) `
            -Message "LeanRows exited with code $($process.ExitCode)."
        $texts
    }
    finally {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
        $process.Dispose()
    }
}

# The same rule as system_language in the localization module.
function Resolve-SystemLanguage {
    param([string]$Tag)
    $lower = ([string]$Tag).Split('.', '@')[0].Replace('_', '-').ToLowerInvariant()
    if ($lower -eq 'ja' -or $lower.StartsWith('ja-')) { return 'ja' }
    if ($lower -eq 'zh' -or $lower.StartsWith('zh-')) { return 'zh-CN' }
    'en'
}

$binary = (Resolve-Path -LiteralPath $BinaryPath).Path
$tables = @{}
foreach ($language in $tableFiles.Keys) {
    $tables[$language] = Read-StringTable -Path (Join-Path $tableDirectory $tableFiles[$language])
}

$original = Get-SavedLanguage
$evidence = New-Object 'System.Collections.Generic.List[object]'
try {
    $systemLanguage = Resolve-SystemLanguage -Tag ([System.Globalization.CultureInfo]::CurrentUICulture.Name)
    foreach ($choice in @('ja', 'zh-CN', 'en', 'system')) {
        Set-SavedLanguage -Value $choice
        $language = if ($choice -eq 'system') { $systemLanguage } else { $choice }
        $strings = $tables[$language]

        $smoke = Invoke-ShellSmoke -Executable $binary
        Assert-True -Condition ($smoke.exit_code -eq 0) `
            -Message "The shell smoke in $choice exited with $($smoke.exit_code): $($smoke.stderr.Trim())"
        Assert-True -Condition ([string]::IsNullOrWhiteSpace($smoke.stdout + $smoke.stderr)) `
            -Message "The shell smoke in $choice wrote output: $($smoke.stderr.Trim())"

        $texts = @(Get-WindowTexts -Executable $binary)
        $shown = @($texts | ForEach-Object { $_.Split("`t", 2)[1] })
        foreach ($key in @('open_button', 'empty_open_button', 'welcome_caption', 'welcome_body', 'idle_file_meta')) {
            Assert-True -Condition ($shown -contains $strings[$key]) `
                -Message "In $choice the window does not show $key as '$($strings[$key])'. It shows: $($shown -join ' | ')"
        }
        if ($language -ne 'en') {
            Assert-True -Condition (-not ($shown -contains $tables['en']['open_button'])) `
                -Message "In $choice the Open button is still in English."
        }
        $evidence.Add([pscustomobject]@{
            choice = $choice
            language = $language
            open_button = $strings['open_button']
            child_windows = $texts.Count
        })
    }
}
finally {
    Set-SavedLanguage -Value $original
}

[pscustomobject]@{
    schema_version = 'leanrows_language_smoke_evidence.v1'
    system_ui_culture = [System.Globalization.CultureInfo]::CurrentUICulture.Name
    runs = $evidence
}
