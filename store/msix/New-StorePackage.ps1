[CmdletBinding()]
param(
    # Three-part app version, for example 0.4.2. The package gets x.y.z.0.
    [Parameter(Mandatory = $true)]
    [string]$Version,
    # The Windows release ZIP from GitHub Releases. Give this or PayloadDirectory.
    [string]$ReleaseZip,
    # A folder that already holds the release files (the extracted ZIP).
    [string]$PayloadDirectory,
    # Product identity from Partner Center. Empty values fall back to the
    # MSIX_* environment variables, then to the placeholders in app.psd1.
    [string]$IdentityName,
    [string]$Publisher,
    [string]$PublisherDisplayName,
    [string]$OutputDirectory,
    # Also make a copy signed with a throwaway self-signed certificate, for an
    # install test only. The certificate is written to the output folder and
    # never added to a certificate store. The Store copy stays unsigned.
    [switch]$SignTestCopy
)

# Builds the x64 MSIX that is uploaded to Partner Center. The Store signs it.
# Steps: stage the release files without the ZIP installer, add the logos and
# AppxManifest.xml, index the logos with MakePri, pack with MakeAppx (which
# also checks the manifest against the SDK schema), and unpack it again to
# confirm the contents.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Config = Import-PowerShellDataFile -LiteralPath (Join-Path $PSScriptRoot 'app.psd1')
$StoreRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $StoreRoot '..'))
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $RepoRoot 'release\msix' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)

function Select-Value([string[]]$Values) {
    foreach ($Value in $Values) {
        if (-not [string]::IsNullOrWhiteSpace($Value)) { return $Value.Trim() }
    }
    return $null
}
$IdentityName = Select-Value @($IdentityName, $env:MSIX_IDENTITY_NAME, $Config.PlaceholderIdentityName)
$Publisher = Select-Value @($Publisher, $env:MSIX_PUBLISHER, $Config.PlaceholderPublisher)
$PublisherDisplayName = Select-Value @($PublisherDisplayName, $env:MSIX_PUBLISHER_DISPLAY_NAME, $Config.DefaultPublisherDisplayName)
$UsesPlaceholders = ($IdentityName -eq $Config.PlaceholderIdentityName) -or ($Publisher -eq $Config.PlaceholderPublisher)

# Identity rules from the AppxManifest schema: Name is 3 to 50 letters, digits,
# periods and dashes; Publisher is a distinguished name such as CN=<GUID>.
if ($IdentityName -notmatch '^[A-Za-z0-9.\-]{3,50}$') {
    throw ('MSIX identity name is not valid: {0}' -f $IdentityName)
}
if ($Publisher -notmatch '^CN=' -or $Publisher.Length -gt 8192) {
    throw ('MSIX publisher must be a distinguished name starting with CN=: {0}' -f $Publisher)
}
if ($PublisherDisplayName.Length -gt 256) { throw 'Publisher display name is too long.' }

$Version = $Version.Trim().TrimStart('v')
if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
    throw ('Version must be x.y.z: {0}' -f $Version)
}
foreach ($Part in $Matches[1], $Matches[2], $Matches[3]) {
    if ([int64]$Part -gt 65535) { throw ('Version part is above 65535: {0}' -f $Version) }
}
$PackageVersion = $Version + '.0'

function Find-SdkTool([string]$Name) {
    $Root = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $Found = Get-ChildItem -LiteralPath $Root -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^10\.0\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName ('x64\' + $Name) } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
    if (-not $Found) { throw ('{0} was not found in the Windows SDK under {1}.' -f $Name, $Root) }
    return $Found
}
function Invoke-Tool([string]$Path, [string[]]$Arguments) {
    $Output = & $Path @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        $Output | ForEach-Object { Write-Host $_ }
        throw ('{0} failed with exit code {1}.' -f [IO.Path]::GetFileName($Path), $LASTEXITCODE)
    }
    $Output | ForEach-Object { Write-Verbose $_ }
}
$MakeAppx = Find-SdkTool 'makeappx.exe'
$MakePri = Find-SdkTool 'makepri.exe'

$Work = Join-Path $OutputDirectory 'work'
if (Test-Path -LiteralPath $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
New-Item -ItemType Directory -Path $Work -Force | Out-Null

# Find the release files.
if ($ReleaseZip) {
    $Extracted = Join-Path $Work 'zip'
    Expand-Archive -LiteralPath $ReleaseZip -DestinationPath $Extracted
    $PayloadDirectory = $Extracted
}
if (-not $PayloadDirectory) { throw 'Give -ReleaseZip or -PayloadDirectory.' }
$Candidates = @((Get-Item -LiteralPath $PayloadDirectory)) + @(Get-ChildItem -LiteralPath $PayloadDirectory -Directory)
$Payload = $Candidates | Where-Object {
    Test-Path -LiteralPath (Join-Path $_.FullName $Config.Executable) -PathType Leaf
} | Select-Object -First 1
if (-not $Payload) { throw ('{0} was not found in {1}.' -f $Config.Executable, $PayloadDirectory) }
$Payload = $Payload.FullName.TrimEnd('\')

# Stage the package layout. The ZIP's own installer scripts and notes are left
# out: in the MSIX, Windows does the install, file associations and uninstall.
$Layout = Join-Path $Work 'layout'
New-Item -ItemType Directory -Path $Layout -Force | Out-Null
$Prefix = $Payload + '\'
foreach ($Item in Get-ChildItem -LiteralPath $Payload -Recurse -File -Force) {
    if ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw ('Release files must not contain links: {0}' -f $Item.FullName)
    }
    $Relative = $Item.FullName.Substring($Prefix.Length)
    $Top = $Relative.Split('\')[0]
    if ($Config.ExcludeFromPackage -contains $Top) { continue }
    $Destination = Join-Path $Layout $Relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force | Out-Null
    Copy-Item -LiteralPath $Item.FullName -Destination $Destination
}
foreach ($Required in @($Config.Executable) + @($Config.RequiredFiles)) {
    if (-not (Test-Path -LiteralPath (Join-Path $Layout $Required) -PathType Leaf)) {
        throw ('Required package file is missing: {0}' -f $Required)
    }
}
$ImagesTarget = Join-Path $Layout 'Images'
if (Test-Path -LiteralPath $ImagesTarget) { throw 'The release files already have an Images folder.' }
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Images') -Destination $ImagesTarget -Recurse
foreach ($Logo in 'Square44x44Logo.scale-100.png', 'Square150x150Logo.scale-100.png',
                  'Wide310x150Logo.scale-100.png', 'StoreLogo.scale-100.png',
                  'Square44x44Logo.targetsize-256.png') {
    if (-not (Test-Path -LiteralPath (Join-Path $ImagesTarget $Logo) -PathType Leaf)) {
        throw ('Logo is missing: Images\{0}. Run New-StoreImages.ps1.' -f $Logo)
    }
}

# Fill the manifest template.
function ConvertTo-XmlText([string]$Text) { [Security.SecurityElement]::Escape($Text) }
$Manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'AppxManifest.template.xml') -Raw
$Manifest = $Manifest.Replace('{{IDENTITY_NAME}}', (ConvertTo-XmlText $IdentityName))
$Manifest = $Manifest.Replace('{{PUBLISHER}}', (ConvertTo-XmlText $Publisher))
$Manifest = $Manifest.Replace('{{PUBLISHER_DISPLAY_NAME}}', (ConvertTo-XmlText $PublisherDisplayName))
$Manifest = $Manifest.Replace('{{VERSION}}', $PackageVersion)
if ($Manifest -match '\{\{[A-Z_]+\}\}') { throw ('Unfilled manifest token: {0}' -f $Matches[0]) }
$ManifestPath = Join-Path $Layout 'AppxManifest.xml'
[IO.File]::WriteAllText($ManifestPath, $Manifest, [Text.UTF8Encoding]::new($false))
[void][xml]$Manifest

# Index the scale and target-size logo variants so Windows picks the right one.
$PriConfig = Join-Path $Work 'priconfig.xml'
Invoke-Tool $MakePri @('createconfig', '/cf', $PriConfig, '/dq', 'en-US', '/pv', '10.0.0', '/o')
Invoke-Tool $MakePri @('new', '/pr', $Layout, '/cf', $PriConfig, '/mn', $ManifestPath,
    '/of', (Join-Path $Layout 'resources.pri'), '/o')

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$PackageName = '{0}-v{1}-x64.msix' -f $Config.AppId, $Version
$PackagePath = Join-Path $OutputDirectory $PackageName
Invoke-Tool $MakeAppx @('pack', '/d', $Layout, '/p', $PackagePath, '/o')

# Unpack the result and check it holds what Windows needs to run the app.
$Unpacked = Join-Path $Work 'unpacked'
Invoke-Tool $MakeAppx @('unpack', '/p', $PackagePath, '/d', $Unpacked, '/o')
foreach ($Required in 'AppxManifest.xml', 'AppxBlockMap.xml', 'resources.pri', $Config.Executable) {
    if (-not (Test-Path -LiteralPath (Join-Path $Unpacked $Required) -PathType Leaf)) {
        throw ('The packed MSIX is missing {0}.' -f $Required)
    }
}
[xml]$Packed = Get-Content -LiteralPath (Join-Path $Unpacked 'AppxManifest.xml') -Raw
if ($Packed.Package.Identity.Version -ne $PackageVersion) { throw 'Packed version does not match.' }

$Hash = (Get-FileHash -LiteralPath $PackagePath -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText($PackagePath + '.sha256', ('{0}  {1}{2}' -f $Hash, $PackageName, [char]10),
    [Text.Encoding]::ASCII)

$Result = [ordered]@{
    Package = $PackagePath
    Version = $PackageVersion
    IdentityName = $IdentityName
    Publisher = $Publisher
    PublisherDisplayName = $PublisherDisplayName
    UsesPlaceholders = $UsesPlaceholders
    SHA256 = $Hash
    Layout = $Layout
    TestPackage = $null
    TestCertificate = $null
}

if ($SignTestCopy) {
    # A throwaway certificate whose subject equals the manifest Publisher, made
    # in memory and saved only to the output folder. It is not added to any
    # certificate store here.
    $SignTool = Find-SdkTool 'signtool.exe'
    $TestDir = Join-Path $OutputDirectory 'test-signing'
    New-Item -ItemType Directory -Path $TestDir -Force | Out-Null
    $Rsa = [Security.Cryptography.RSA]::Create(2048)
    try {
        $Request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
            [Security.Cryptography.X509Certificates.X500DistinguishedName]::new($Publisher),
            $Rsa, [Security.Cryptography.HashAlgorithmName]::SHA256,
            [Security.Cryptography.RSASignaturePadding]::Pkcs1)
        $Request.CertificateExtensions.Add(
            [Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $true))
        $Request.CertificateExtensions.Add(
            [Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
                [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature, $true))
        $Usages = [Security.Cryptography.OidCollection]::new()
        [void]$Usages.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3'))
        $Request.CertificateExtensions.Add(
            [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($Usages, $false))
        $Now = [DateTimeOffset]::UtcNow
        $Certificate = $Request.CreateSelfSigned($Now.AddMinutes(-5), $Now.AddDays(2))
        $Password = [Guid]::NewGuid().ToString('N')
        $PfxPath = Join-Path $TestDir 'test-only.pfx'
        $CerPath = Join-Path $TestDir 'test-only.cer'
        [IO.File]::WriteAllBytes($PfxPath, $Certificate.Export(
            [Security.Cryptography.X509Certificates.X509ContentType]::Pfx, $Password))
        [IO.File]::WriteAllBytes($CerPath, $Certificate.Export(
            [Security.Cryptography.X509Certificates.X509ContentType]::Cert))
        $Certificate.Dispose()
    } finally { $Rsa.Dispose() }
    $TestPackage = Join-Path $TestDir ('{0}-v{1}-x64-TEST-SIGNED-do-not-upload.msix' -f $Config.AppId, $Version)
    Copy-Item -LiteralPath $PackagePath -Destination $TestPackage -Force
    Invoke-Tool $SignTool @('sign', '/fd', 'SHA256', '/f', $PfxPath, '/p', $Password, $TestPackage)
    Remove-Item -LiteralPath $PfxPath -Force
    $Result.TestPackage = $TestPackage
    $Result.TestCertificate = $CerPath
}

Write-Host ('Created {0}' -f $PackagePath)
Write-Host ('Identity {0}, publisher {1}, version {2}' -f $IdentityName, $Publisher, $PackageVersion)
if ($UsesPlaceholders) {
    Write-Host 'This package uses placeholder identity values. Set the MSIX_* repository variables from Partner Center before uploading.'
}
[pscustomobject]$Result
