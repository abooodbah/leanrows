@{
    # Read by New-StorePackage.ps1, New-StoreImages.ps1 and Test-StorePackage.ps1.
    AppId = 'LeanRows'
    Executable = 'leanrows.exe'
    # README documents passing file paths on the command line, so the package
    # adds a command alias for it.
    Alias = 'leanrows.exe'
    IconSvg = 'branding\leanrows-icon.svg'
    # The extensions installer\Install-LeanRows.ps1 registers (.txt is left alone there too).
    Extensions = @('.csv', '.tsv', '.jsonl', '.ndjson', '.log')
    RequiredFiles = @('LICENSE.txt')
    # ZIP-only files. In the MSIX, Windows installs, associates and uninstalls.
    ExcludeFromPackage = @('install.ps1', 'uninstall.ps1', 'manifest.json', 'UNSIGNED.txt')
    ReleaseAssetPattern = 'LeanRows-v{0}-windows-x64*.zip'
    SampleFile = 'store\samples\weather-stations.csv'
    # Used until the repository variables hold the Partner Center values.
    PlaceholderIdentityName = 'Placeholder.LeanRows'
    PlaceholderPublisher = 'CN=PLACEHOLDER-SET-MSIX_PUBLISHER'
    DefaultPublisherDisplayName = 'Lean Apps Studio'
}
