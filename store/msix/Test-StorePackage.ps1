[CmdletBinding(DefaultParameterSetName = 'Layout')]
param(
    # Register the unpacked layout from New-StorePackage.ps1. Needs Windows
    # developer mode to be on already; this script never turns it on.
    [Parameter(Mandatory = $true, ParameterSetName = 'Layout')]
    [string]$LayoutDirectory,

    # Install the test-signed copy from New-StorePackage.ps1 -SignTestCopy.
    [Parameter(Mandatory = $true, ParameterSetName = 'Signed')]
    [string]$PackagePath,
    [Parameter(Mandatory = $true, ParameterSetName = 'Signed')]
    [string]$CertificatePath,
    # Adds the throwaway certificate to LocalMachine\TrustedPeople for the
    # install and removes it afterwards. Meant for disposable CI machines only.
    [Parameter(ParameterSetName = 'Signed')]
    [switch]$TrustTestCertificateOnThisMachine,

    # A document to open through the execution alias.
    [string]$SampleFile,
    [int]$WindowTimeoutSeconds = 45
)

# Installs the package for the current user, starts the app from its Start
# menu entry and through its command alias with a sample document, checks the
# file associations Windows registered for it, then removes the package.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Config = Import-PowerShellDataFile -LiteralPath (Join-Path $PSScriptRoot 'app.psd1')
$ExeStem = [IO.Path]::GetFileNameWithoutExtension($Config.Executable)
$Results = [ordered]@{}

if ($PSCmdlet.ParameterSetName -eq 'Layout') {
    $ManifestPath = Join-Path $LayoutDirectory 'AppxManifest.xml'
    $Unlock = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' -ErrorAction SilentlyContinue
    $DevMode = $null -ne $Unlock -and $Unlock.PSObject.Properties['AllowDevelopmentWithoutDevLicense'] -and
        [int]$Unlock.AllowDevelopmentWithoutDevLicense -eq 1
    if (-not $DevMode) {
        throw 'Registering a loose layout needs Windows developer mode, which is off. Use the signed test copy on a disposable machine instead.'
    }
} else {
    $Temp = Join-Path ([IO.Path]::GetTempPath()) ('store-test-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $Temp | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Zip = [IO.Compression.ZipFile]::OpenRead($PackagePath)
    try {
        $Entry = $Zip.GetEntry('AppxManifest.xml')
        $ManifestPath = Join-Path $Temp 'AppxManifest.xml'
        [IO.Compression.ZipFileExtensions]::ExtractToFile($Entry, $ManifestPath)
    } finally { $Zip.Dispose() }
}
[xml]$Manifest = Get-Content -LiteralPath $ManifestPath -Raw
$Name = $Manifest.Package.Identity.Name
$AppId = $Manifest.Package.Applications.Application.Id

if (Get-AppxPackage -Name $Name) {
    throw ('A package named {0} is already installed for this user. Remove it first.' -f $Name)
}

$Thumbprint = $null
$Installed = $null

# Process paths can come back in 8.3 short form (C:\Users\ABCDEF~1), so both
# sides are compared in long form.
if (-not ('StoreTest.Paths' -as [type])) {
    Add-Type -TypeDefinition @"
using System.Runtime.InteropServices;
using System.Text;
namespace StoreTest {
  public static class Paths {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern uint GetLongPathNameW(string shortPath, StringBuilder longPath, uint size);
    public static string Long(string path) {
      var buffer = new StringBuilder(32768);
      uint length = GetLongPathNameW(path, buffer, (uint)buffer.Capacity);
      return length == 0 || length >= buffer.Capacity ? path : buffer.ToString();
    }
  }
}
"@
}
# Opens a file through a given ProgID, the way Explorer does once the user has
# picked the app for that file type.
if (-not ('StoreTest.Shell' -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
namespace StoreTest {
  public static class Shell {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SHELLEXECUTEINFO {
      public int cbSize; public uint fMask; public IntPtr hwnd;
      public string lpVerb; public string lpFile; public string lpParameters; public string lpDirectory;
      public int nShow; public IntPtr hInstApp; public IntPtr lpIDList; public string lpClass;
      public IntPtr hkeyClass; public uint dwHotKey; public IntPtr hIcon; public IntPtr hProcess;
    }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool ShellExecuteExW(ref SHELLEXECUTEINFO info);
    public static void OpenWithClass(string path, string progId) {
      var info = new SHELLEXECUTEINFO();
      info.cbSize = Marshal.SizeOf(typeof(SHELLEXECUTEINFO));
      info.fMask = 0x00000001 | 0x00000100; // SEE_MASK_CLASSNAME | SEE_MASK_NOASYNC
      info.lpVerb = "open"; info.lpFile = path; info.lpClass = progId; info.nShow = 1;
      if (!ShellExecuteExW(ref info)) { throw new System.ComponentModel.Win32Exception(); }
    }
  }
}
"@
}function Get-PackageProcess {
    $Root = [StoreTest.Paths]::Long($Installed.InstallLocation).TrimEnd('\') + '\'
    @(Get-Process -Name $ExeStem -ErrorAction SilentlyContinue | Where-Object {
        $Path = $null; try { $Path = $_.Path } catch { }
        $Path -and [StoreTest.Paths]::Long($Path).StartsWith($Root, [StringComparison]::OrdinalIgnoreCase)
    })
}try {
    if ($PSCmdlet.ParameterSetName -eq 'Layout') {
        Add-AppxPackage -Register $ManifestPath
    } else {
        if ($TrustTestCertificateOnThisMachine) {
            $Certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($CertificatePath)
            $Thumbprint = $Certificate.Thumbprint
            $Store = [Security.Cryptography.X509Certificates.X509Store]::new('TrustedPeople', 'LocalMachine')
            $Store.Open('ReadWrite'); $Store.Add($Certificate); $Store.Close()
        }
        Add-AppxPackage -Path $PackagePath
    }
    $Installed = Get-AppxPackage -Name $Name
    if (-not $Installed) { throw 'The package did not install.' }
    $Results.PackageFullName = $Installed.PackageFullName
    $Results.InstallLocation = $Installed.InstallLocation
    $Aumid = '{0}!{1}' -f $Installed.PackageFamilyName, $AppId

    function Wait-AppWindow {
        $Deadline = [DateTime]::UtcNow.AddSeconds($WindowTimeoutSeconds)
        while ([DateTime]::UtcNow -lt $Deadline) {
            $Process = Get-PackageProcess |
                Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero } | Select-Object -First 1
            if ($Process) { return $Process }
            Start-Sleep -Milliseconds 250
        }
        throw ('No {0} window from the package appeared within {1} s.' -f $Config.Executable, $WindowTimeoutSeconds)
    }
    function Stop-App {
        foreach ($Process in Get-PackageProcess) {
            [void]$Process.CloseMainWindow()
            if (-not $Process.WaitForExit(10000)) { Stop-Process -Id $Process.Id -Force }
        }
        Start-Sleep -Seconds 1
    }

    # 1. Start menu entry.
    Start-Process -FilePath 'explorer.exe' -ArgumentList ('shell:AppsFolder\' + $Aumid)
    $Window = Wait-AppWindow
    $Results.StartMenuLaunch = 'window "{0}"' -f $Window.MainWindowTitle
    Stop-App

    # 2. Command alias with a document.
    if ($Config.Alias) {
        $AliasPath = Join-Path $env:LOCALAPPDATA ('Microsoft\WindowsApps\' + $Config.Alias)
        if (-not (Test-Path -LiteralPath $AliasPath)) { throw ('The alias {0} was not created.' -f $AliasPath) }
        $Arguments = @()
        if ($SampleFile) { $Arguments = @(('"{0}"' -f [IO.Path]::GetFullPath($SampleFile))) }
        if ($Arguments.Count -gt 0) {
            Start-Process -FilePath $AliasPath -ArgumentList $Arguments
        } else {
            Start-Process -FilePath $AliasPath
        }
        $Window = Wait-AppWindow
        Start-Sleep -Seconds 2
        $Window.Refresh()
        $Results.AliasLaunch = 'window "{0}"' -f $Window.MainWindowTitle
        if ($SampleFile) {
            $Leaf = [IO.Path]::GetFileName($SampleFile)
            $Results.AliasOpenedSample = $Window.MainWindowTitle -like ('*' + $Leaf + '*')
        }
        Stop-App
    }

    # 3. File associations registered by Windows for this package.
    $Associations = [ordered]@{}
    foreach ($Extension in $Config.Extensions) {
        $OpenWith = 'HKCU:\Software\Classes\' + $Extension + '\OpenWithProgids'
        $Match = $null
        if (Test-Path -LiteralPath $OpenWith) {
            foreach ($ProgId in (Get-Item -LiteralPath $OpenWith).GetValueNames()) {
                $AppKey = 'HKCU:\Software\Classes\' + $ProgId + '\Application'
                if ($ProgId -like 'AppX*' -and (Test-Path -LiteralPath $AppKey)) {
                    $Model = (Get-ItemProperty -LiteralPath $AppKey).PSObject.Properties['AppUserModelID']
                    if ($Model -and [string]$Model.Value -eq $Aumid) { $Match = $ProgId }
                }
            }
        }
        if (-not $Match) { throw ('Windows did not register {0} for the package.' -f $Extension) }
        $Associations[$Extension] = $Match
    }
    $Results.FileAssociations = ($Associations.Keys -join ' ')

    # 4. The sample document opened through the association Windows registered.
    if ($SampleFile) {
        $SampleProgId = $Associations[[IO.Path]::GetExtension($SampleFile).ToLowerInvariant()]
        [StoreTest.Shell]::OpenWithClass([IO.Path]::GetFullPath($SampleFile), $SampleProgId)
        $Window = Wait-AppWindow
        Start-Sleep -Seconds 2
        $Window.Refresh()
        $Results.FileAssociationLaunch = 'window "{0}"' -f $Window.MainWindowTitle
        $Results.FileAssociationOpenedSample = $Window.MainWindowTitle -like ('*' + [IO.Path]::GetFileName($SampleFile) + '*')
        Stop-App
    }

    foreach ($Key in 'AliasOpenedSample', 'FileAssociationOpenedSample') {
        if ($Results.Contains($Key) -and -not $Results[$Key]) {
            throw ('{0}: the window title does not name the sample file.' -f $Key)
        }
    }

    # 5. Where the app's own data went. Packaged apps write AppData into a
    # per-package folder that Windows removes on uninstall.
    $LocalCache = Join-Path $env:LOCALAPPDATA ('Packages\' + $Installed.PackageFamilyName + '\LocalCache')
    if (Test-Path -LiteralPath $LocalCache) {
        $Results.RedirectedAppData = @(Get-ChildItem -LiteralPath $LocalCache -Recurse -Directory -Depth 2 |
            ForEach-Object { $_.FullName.Substring($LocalCache.Length + 1) }) -join ', '
    } else {
        $Results.RedirectedAppData = '(none)'
    }
} finally {
    if ($Installed) {
        Get-PackageProcess | Stop-Process -Force -ErrorAction SilentlyContinue
        Remove-AppxPackage -Package $Installed.PackageFullName
        $Results.Removed = -not [bool](Get-AppxPackage -Name $Name)
    }
    if ($Thumbprint) {
        $Store = [Security.Cryptography.X509Certificates.X509Store]::new('TrustedPeople', 'LocalMachine')
        $Store.Open('ReadWrite')
        foreach ($Item in @($Store.Certificates.Find('FindByThumbprint', $Thumbprint, $false))) { $Store.Remove($Item) }
        $Store.Close()
    }
    if ((Get-Variable -Name Temp -ErrorAction SilentlyContinue) -and $Temp) {
        Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
[pscustomobject]$Results
