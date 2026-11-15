# Registers the Chrome native messaging host for the current user.
# Usage: .\register-native-host.ps1 -ExtensionId <32-char-id> [-HostPath <exe>]
param(
    [Parameter(Mandatory = $true)][string]$ExtensionId,
    [string]$HostPath = ""
)

$ErrorActionPreference = "Stop"
if ($ExtensionId -notmatch '^[a-p]{32}$') {
    throw "ExtensionId must be 32 characters in the range a-p."
}

$root = Split-Path -Parent $PSScriptRoot
if (-not $HostPath) {
    $HostPath = Join-Path $root "target\release\netquota-native-host.exe"
}
if (-not (Test-Path $HostPath)) {
    throw "Native host not found at $HostPath. Build it or pass -HostPath."
}

$manifestDir = Join-Path $env:LOCALAPPDATA "NetQuota"
New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null
$manifestPath = Join-Path $manifestDir "com.netquota.host.json"

$manifest = [ordered]@{
    name = "com.netquota.host"
    description = "NetQuota native bridge"
    path = (Resolve-Path $HostPath).Path
    type = "stdio"
    allowed_origins = @("chrome-extension://$ExtensionId/")
}
$manifest | ConvertTo-Json -Depth 4 | Set-Content -Path $manifestPath -Encoding UTF8

$key = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.netquota.host"
New-Item -Path $key -Force | Out-Null
Set-ItemProperty -Path $key -Name "(default)" -Value $manifestPath

# The host also enforces this allow-list, so a copied manifest alone is not enough.
$dataDir = Join-Path $env:ProgramData "NetQuota"
if (-not (Test-Path $dataDir)) {
    # Users without an installed service keep their data next to their profile.
    $dataDir = Join-Path $env:LOCALAPPDATA "NetQuota\data"
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
}
Set-Content -Path (Join-Path $dataDir "extension-id.txt") -Value $ExtensionId -Encoding ASCII

Write-Host "Registered com.netquota.host for chrome-extension://$ExtensionId/"
Write-Host "Reload the extension in chrome://extensions to connect."
