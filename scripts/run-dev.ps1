$ErrorActionPreference = "Stop"

$RepoDir = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $RepoDir "src-tauri")
try {
    cargo build -p gatetree-ops-agent
    if ($LASTEXITCODE -ne 0) { throw "The GateTree-Ops Agent build failed." }

    $HostTarget = (rustc -vV | Select-String '^host: ' | ForEach-Object { $_.Line.Substring(6) })
    New-Item -ItemType Directory -Force -Path "binaries" | Out-Null
    Copy-Item "target\debug\gatetree-ops-agent.exe" "binaries\gatetree-ops-agent-$HostTarget.exe" -Force

    cargo run --bin gatetree-ops
    if ($LASTEXITCODE -ne 0) { throw "GateTree-Ops exited with an error." }
}
finally {
    Pop-Location
}
