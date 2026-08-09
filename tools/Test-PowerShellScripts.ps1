[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scripts = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File | Sort-Object Name)
$failureCount = 0

foreach ($script in $scripts) {
    $tokens = $null
    $errors = $null
    $null = [System.Management.Automation.Language.Parser]::ParseFile(
        $script.FullName,
        [ref]$tokens,
        [ref]$errors
    )

    if ($errors.Count -eq 0) {
        Write-Host "PASS $($script.Name)"
        continue
    }

    foreach ($parseError in $errors) {
        Write-Error ("{0}:{1}:{2}: {3}" -f `
            $script.FullName,
            $parseError.Extent.StartLineNumber,
            $parseError.Extent.StartColumnNumber,
            $parseError.Message)
    }
    $failureCount += $errors.Count
}

if ($failureCount -gt 0) {
    throw "$failureCount PowerShell parse error(s) found."
}

Write-Host "Parsed $($scripts.Count) PowerShell script(s) successfully."
