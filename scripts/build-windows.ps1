$ErrorActionPreference = "Stop"
$RepoDir = Split-Path -Parent $PSScriptRoot
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    $env:TAURI_SIGNING_PRIVATE_KEY = Join-Path $HOME ".config\gatetree-ops\updater.key"
}
Push-Location (Join-Path $RepoDir "src-tauri")
try {
    cargo tauri --version
    if ($LASTEXITCODE -ne 0) { throw "Install tauri-cli v2 first." }
    rustup target add x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "Could not install the Windows target." }
    cargo build --release --locked -p gatetree-ops-agent --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "Agent build failed." }
    New-Item -ItemType Directory -Force -Path binaries | Out-Null
    Copy-Item "target\x86_64-pc-windows-msvc\release\gatetree-ops-agent.exe" "binaries\gatetree-ops-agent-x86_64-pc-windows-msvc.exe" -Force
    cargo tauri build --bundles nsis,msi --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "Installer build failed." }
    $Bundle = "target\x86_64-pc-windows-msvc\release\bundle"
    $Msi = Get-ChildItem "$Bundle\msi\*.msi" | Select-Object -First 1
    if (-not $Msi) { throw "MSI was not created." }
    & (Join-Path $RepoDir "scripts\verify-windows-installer.ps1") -MsiPath $Msi.FullName
    $Setup = Get-ChildItem "$Bundle\nsis\*-setup.exe" | Select-Object -First 1
    if (-not $Setup -or -not (Test-Path ($Setup.FullName + ".sig"))) { throw "Signed per-user updater installer was not created." }
    & (Join-Path $RepoDir "scripts\verify-windows-setup.ps1") -SetupPath $Setup.FullName
    Write-Host "Per-user Setup EXE (recommended) and MSI: $Bundle"
}
finally { Pop-Location }
