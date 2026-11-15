# Removes the per-user native messaging registration. Elevated PowerShell.
$ErrorActionPreference = "Stop"

$key = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.netquota.host"
if (Test-Path $key) { Remove-Item $key -Recurse -Force }

$manifestDir = Join-Path $env:LOCALAPPDATA "NetQuota"
$manifest = Join-Path $manifestDir "com.netquota.host.json"
if (Test-Path $manifest) { Remove-Item $manifest -Force }

$dataDir = Join-Path $env:ProgramData "NetQuota"
foreach ($dir in @($dataDir, (Join-Path $env:LOCALAPPDATA "NetQuota\data"))) {
    $allow = Join-Path $dir "extension-id.txt"
    if (Test-Path $allow) { Remove-Item $allow -Force }
}

Write-Host "Native messaging host unregistered."
