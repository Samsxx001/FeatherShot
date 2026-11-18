param([switch]$Test, [switch]$Check, [switch]$Clippy)
$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    $featherToolchain = 'stable-x86_64-pc-windows-gnu'
    $featherCompiler = (& rustup which rustc --toolchain $featherToolchain).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Install the GNU toolchain: rustup toolchain install stable-x86_64-pc-windows-gnu' }
    $featherRustRoot = Split-Path (Split-Path $featherCompiler -Parent) -Parent
    $featherLinkTools = Join-Path $featherRustRoot 'lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained'
    $env:PATH = $featherLinkTools + ';' + $env:PATH
    $featherMingw = if ($env:FEATHERSHOT_MINGW_BIN) { $env:FEATHERSHOT_MINGW_BIN } else { Join-Path $PSScriptRoot '../savehaven-rust/.tools/w64devkit/bin' }
    if (Test-Path -LiteralPath $featherMingw) {
        $env:PATH = $featherMingw + ';' + $env:PATH
        $env:CARGO_ENCODED_RUSTFLAGS = '-C' + [char]31 + ('dlltool=' + (Join-Path $featherMingw 'dlltool.exe'))
    }
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $featherLinkTools 'x86_64-w64-mingw32-gcc.exe'
    if ($Check) {
        & cargo +$featherToolchain check --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Check failed.' }
        return
    }
    if ($Clippy) {
        & cargo +$featherToolchain clippy --locked --offline --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
        return
    }
    if ($Test) {
        & cargo +$featherToolchain test --locked --offline -- --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    }
    & cargo +$featherToolchain build --release --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    Copy-Item -LiteralPath target/release/feathershot.exe -Destination dist/FeatherShot.exe
    foreach ($featherFile in @('README-TH.md', 'LICENSE', 'VALIDATION-TH.md')) {
        if (Test-Path -LiteralPath $featherFile) { Copy-Item -LiteralPath $featherFile -Destination dist }
    }
    Write-Output (Join-Path $PSScriptRoot 'dist/FeatherShot.exe')
} finally { Pop-Location }
