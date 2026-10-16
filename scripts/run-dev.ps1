$ErrorActionPreference = "Stop"

$RepoDir = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $RepoDir "src-tauri")
try {
    cargo build -p pharos-agent
    if ($LASTEXITCODE -ne 0) { throw "The PHAROS Agent build failed." }

    $HostTarget = (rustc -vV | Select-String '^host: ' | ForEach-Object { $_.Line.Substring(6) })
    New-Item -ItemType Directory -Force -Path "binaries" | Out-Null
    Copy-Item "target\debug\pharos-agent.exe" "binaries\pharos-agent-$HostTarget.exe" -Force

    cargo run --bin pharos
    if ($LASTEXITCODE -ne 0) { throw "PHAROS exited with an error." }
}
finally {
    Pop-Location
}
