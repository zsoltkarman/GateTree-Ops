param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath
)

$ErrorActionPreference = "Stop"
$MsiPath = (Resolve-Path $MsiPath).Path
$ExtractPath = Join-Path ([System.IO.Path]::GetTempPath()) ("gatetree-ops-msi-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $ExtractPath | Out-Null

try {
    $Arguments = @(
        "/a",
        ('"{0}"' -f $MsiPath),
        "/qn",
        ('TARGETDIR="{0}"' -f $ExtractPath)
    )
    $Process = Start-Process msiexec.exe -ArgumentList $Arguments -Wait -PassThru
    if ($Process.ExitCode -notin @(0, 3010)) {
        throw "MSI administrative extraction failed with exit code $($Process.ExitCode)."
    }
    $Agent = Get-ChildItem -Path $ExtractPath -Filter "gatetree-ops-agent.exe" -File -Recurse | Select-Object -First 1
    if (-not $Agent) {
        throw "The MSI does not contain gatetree-ops-agent.exe."
    }
    Write-Host "Verified bundled Agent: $($Agent.FullName)"
}
finally {
    Remove-Item -Path $ExtractPath -Recurse -Force -ErrorAction SilentlyContinue
}
