[CmdletBinding()]
param(
    # The app icon that every logo is drawn from. Read from store/msix/app.psd1
    # when not given.
    [string]$SvgPath,
    [string]$OutputDirectory,
    [string]$EdgePath
)

# Draws the MSIX visual assets (Start menu, taskbar, tile and Store logos) from
# the app's existing SVG icon, using Microsoft Edge in headless mode. No new
# artwork: every image is the same icon, scaled, on a transparent background.
# Run it again after the icon changes and commit the PNGs in store/msix/Images.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$StoreRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $StoreRoot '..'))
$Config = Import-PowerShellDataFile -LiteralPath (Join-Path $PSScriptRoot 'app.psd1')
if (-not $SvgPath) { $SvgPath = Join-Path $RepoRoot $Config.IconSvg }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $PSScriptRoot 'Images' }
if (-not (Test-Path -LiteralPath $SvgPath -PathType Leaf)) {
    throw ('Icon source is missing: {0}' -f $SvgPath)
}

if (-not $EdgePath) {
    $EdgePath = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
        (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (-not $EdgePath) { throw 'Microsoft Edge is required to draw the icon.' }

# Name, canvas width, canvas height, icon size. Scale variants follow the
# Windows scale factors 100, 125, 150, 200 and 400 percent. Tiles keep the
# icon at about two thirds of the tile height, as Windows' own tiles do.
$Images = [Collections.Generic.List[object]]::new()
function Add-Image([string]$Name, [int]$Width, [int]$Height, [int]$Icon) {
    $Images.Add([pscustomobject]@{ Name = $Name; Width = $Width; Height = $Height; Icon = $Icon })
}
foreach ($Scale in 100, 125, 150, 200, 400) {
    $f = $Scale / 100.0
    $s44 = [int][Math]::Round(44 * $f)
    Add-Image ('Square44x44Logo.scale-{0}.png' -f $Scale) $s44 $s44 $s44
    $s150 = [int][Math]::Round(150 * $f)
    Add-Image ('Square150x150Logo.scale-{0}.png' -f $Scale) $s150 $s150 ([int][Math]::Round($s150 * 0.66))
    $w = [int][Math]::Round(310 * $f); $h = [int][Math]::Round(150 * $f)
    Add-Image ('Wide310x150Logo.scale-{0}.png' -f $Scale) $w $h ([int][Math]::Round($h * 0.66))
    $s50 = [int][Math]::Round(50 * $f)
    Add-Image ('StoreLogo.scale-{0}.png' -f $Scale) $s50 $s50 $s50
}
foreach ($Target in 16, 24, 32, 48, 256) {
    Add-Image ('Square44x44Logo.targetsize-{0}.png' -f $Target) $Target $Target $Target
    Add-Image ('Square44x44Logo.altform-unplated_targetsize-{0}.png' -f $Target) $Target $Target $Target
}

$Svg = Get-Content -LiteralPath $SvgPath -Raw
$Temp = Join-Path ([IO.Path]::GetTempPath()) ('store-images-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Temp -Force | Out-Null
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
Add-Type -AssemblyName System.Drawing
try {
    foreach ($Image in $Images) {
        $Html = Join-Path $Temp 'page.html'
        $Page = @"
<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;padding:0;background:transparent;overflow:hidden}
body{width:$($Image.Width)px;height:$($Image.Height)px;display:flex;align-items:center;justify-content:center}
svg{width:$($Image.Icon)px;height:$($Image.Icon)px;display:block}
</style></head><body>$Svg</body></html>
"@
        [IO.File]::WriteAllText($Html, $Page, [Text.UTF8Encoding]::new($false))
        $Out = Join-Path $OutputDirectory $Image.Name
        if (Test-Path -LiteralPath $Out) { Remove-Item -LiteralPath $Out -Force }
        $EdgeProfile = Join-Path $Temp 'profile'
        $Arguments = @(
            '--headless=new', '--disable-gpu', '--hide-scrollbars', '--no-first-run',
            '--force-device-scale-factor=1', '--default-background-color=00000000',
            ('--user-data-dir="{0}"' -f $EdgeProfile),
            ('--window-size={0},{1}' -f $Image.Width, $Image.Height),
            ('--screenshot="{0}"' -f $Out),
            ('"file:///{0}"' -f $Html.Replace('\', '/'))
        )
        $Process = Start-Process -FilePath $EdgePath -ArgumentList $Arguments -PassThru -WindowStyle Hidden
        if (-not $Process.WaitForExit(60000)) { $Process.Kill(); throw 'Edge did not finish drawing.' }
        if (-not (Test-Path -LiteralPath $Out -PathType Leaf)) { throw ('Edge drew nothing for {0}' -f $Image.Name) }
        $Bitmap = [Drawing.Image]::FromFile($Out)
        try {
            if ($Bitmap.Width -ne $Image.Width -or $Bitmap.Height -ne $Image.Height) {
                throw ('{0} is {1}x{2}, expected {3}x{4}' -f $Image.Name, $Bitmap.Width, $Bitmap.Height, $Image.Width, $Image.Height)
            }
        } finally { $Bitmap.Dispose() }
        Write-Host ('{0} {1}x{2}' -f $Image.Name, $Image.Width, $Image.Height)
    }
} finally {
    Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue
}
