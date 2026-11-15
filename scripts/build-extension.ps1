# Builds the Chrome extension into apps/chrome-extension/dist.
$ErrorActionPreference = "Stop"
$dir = Join-Path (Split-Path -Parent $PSScriptRoot) "apps\chrome-extension"
Push-Location $dir
try {
    if (Test-Path "package-lock.json") {
        npm ci --no-audit --no-fund
    } else {
        npm install --no-audit --no-fund
    }
    npx tsc -p tsconfig.json
    Write-Host "Extension built. Load unpacked from: $dir"
} finally {
    Pop-Location
}
