$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $ProjectRoot
try {
    $RustInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0) { throw 'Rust toolchain is unavailable.' }
    $TargetTriple = ($RustInfo | Where-Object { $_ -like 'host: *' }) -replace '^host: ', ''
    & cargo fetch --locked --target $TargetTriple
    if ($LASTEXITCODE -ne 0) { throw 'Cargo dependency download failed.' }
    Write-Host 'GPUI Kit 0.9.1 dependencies ready. Run cargo run --locked.'
} finally {
    Pop-Location
}
