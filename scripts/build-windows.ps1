$ErrorActionPreference = "Stop"

cargo tauri --version *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'Install the Tauri CLI first: cargo install tauri-cli --version "^2" --locked'
}

$RepoDir = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $RepoDir "src-tauri")
try {
    rustup target add x86_64-pc-windows-msvc
    cargo build --release -p pharos-agent --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {
        throw "The PHAROS Agent build failed."
    }
    New-Item -ItemType Directory -Force -Path "binaries" | Out-Null
    Copy-Item `
        "target\x86_64-pc-windows-msvc\release\pharos-agent.exe" `
        "binaries\pharos-agent-x86_64-pc-windows-msvc.exe" `
        -Force
    cargo tauri build --bundles msi --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {
        throw "The MSI build failed."
    }
    $Msi = Get-ChildItem "target\x86_64-pc-windows-msvc\release\bundle\msi\*.msi" | Select-Object -First 1
    & (Join-Path $RepoDir "scripts\verify-windows-installer.ps1") -MsiPath $Msi.FullName
}
finally {
    Pop-Location
}

Write-Host "MSI created under src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/"
