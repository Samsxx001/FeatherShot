$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    if (-not (Test-Path -LiteralPath 'dist/FeatherShot.exe')) { throw 'Run build.ps1 first.' }
    Copy-Item -LiteralPath 'README-TH.md','VALIDATION-TH.md','LICENSE' -Destination dist
    $featherMeta = (& cargo metadata --format-version 1 --locked --offline | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) { throw 'Could not read dependency metadata.' }
    $featherVersion = ($featherMeta.packages | Where-Object { $_.name -eq 'feathershot' }).version
    $featherReleaseZip = 'FeatherShot-{0}-win64.zip' -f $featherVersion
    $featherSourceZip = 'FeatherShot-{0}-source.zip' -f $featherVersion
    $featherNotices = [Collections.Generic.List[string]]::new()
    $featherNotices.Add('FeatherShot third-party dependencies (Cargo.lock)')
    foreach ($featherPackage in $featherMeta.packages) {
        if ($featherPackage.name -eq 'feathershot') { continue }
        $featherNotices.Add(('{0} {1}: {2} | {3}' -f $featherPackage.name,$featherPackage.version,$featherPackage.license,$featherPackage.repository))
        $featherLicenseDir = Join-Path 'dist/licenses' ('{0}-{1}' -f $featherPackage.name,$featherPackage.version)
        New-Item -ItemType Directory -Path $featherLicenseDir -Force | Out-Null
        $featherSource = Split-Path $featherPackage.manifest_path -Parent
        Get-ChildItem -LiteralPath $featherSource -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)' } | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination $featherLicenseDir
        }
    }
    $featherNotices | Set-Content -LiteralPath dist/THIRD-PARTY.txt -Encoding utf8
    $featherRelease = @('dist/FeatherShot.exe','dist/README-TH.md','dist/VALIDATION-TH.md','dist/LICENSE','dist/THIRD-PARTY.txt','dist/licenses')
    Compress-Archive -LiteralPath $featherRelease -DestinationPath $featherReleaseZip -Force
    $featherSourceFiles = @('Cargo.toml','Cargo.lock','build.rs','build.ps1','package.ps1','src','README-TH.md','VALIDATION-TH.md','LICENSE','.gitignore')
    Compress-Archive -LiteralPath $featherSourceFiles -DestinationPath $featherSourceZip -Force
    Get-Item -LiteralPath 'dist/FeatherShot.exe',$featherReleaseZip,$featherSourceZip | Select-Object Name,Length
} finally { Pop-Location }
