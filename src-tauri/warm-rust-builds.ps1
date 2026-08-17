$ErrorActionPreference = 'Stop'

Set-Location $PSScriptRoot
$Host.UI.RawUI.WindowTitle = 'Rust build warm-up — kastor'

$msys2UcrtBin = 'C:\Users\user\AppData\Local\msys64\ucrt64\bin'
if (Test-Path $msys2UcrtBin) {
    $env:PATH = "$msys2UcrtBin;$env:PATH"
}

Write-Host '=== Release build ===' -ForegroundColor Cyan
cargo build --release

Write-Host "`n=== Debug build ===" -ForegroundColor Cyan
cargo build

Write-Host "`nBuild warm-up completed successfully." -ForegroundColor Green
Read-Host 'Press Enter to close this window'
