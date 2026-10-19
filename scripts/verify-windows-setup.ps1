param([Parameter(Mandatory = $true)][string]$SetupPath)
$ErrorActionPreference = "Stop"
$SetupPath = (Resolve-Path $SetupPath).Path
$SevenZip = Get-Command 7z -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1
if (-not $SevenZip) { $SevenZip = Join-Path $env:ProgramFiles "7-Zip\7z.exe" }
if (-not (Test-Path $SevenZip)) { throw "7-Zip is required to verify the NSIS package." }
$ExtractPath = Join-Path ([System.IO.Path]::GetTempPath()) ("gatetree-ops-nsis-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $ExtractPath | Out-Null
try {
    & $SevenZip x $SetupPath "-o$ExtractPath" -y *> $null
    if ($LASTEXITCODE -ne 0) { throw "Could not extract the NSIS package for verification." }
    $Agent = Get-ChildItem $ExtractPath -Recurse -Filter "gatetree-ops-agent.exe" | Select-Object -First 1
    $Gui = Get-ChildItem $ExtractPath -Recurse -Filter "gatetree-ops.exe" | Select-Object -First 1
    if (-not $Agent -or -not $Gui) { throw "The per-user installer must contain both GUI and Agent." }
    Write-Host "Verified GUI and Agent in per-user Setup EXE."
}
finally { Remove-Item $ExtractPath -Recurse -Force }
