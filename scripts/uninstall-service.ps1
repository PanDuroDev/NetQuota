# Removes the service and any firewall rules NetQuota owns. Elevated PowerShell.
param(
    [string]$InstallDir = "$env:ProgramFiles\NetQuota",
    [switch]$RemoveData
)

$ErrorActionPreference = "Stop"
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this script from an elevated PowerShell."
}

# Remove owned rules before the binaries disappear, so a leftover block cannot
# survive the uninstall.
$cli = Join-Path $InstallDir "netquota-cli.exe"
if (Test-Path $cli) {
    & $cli recover
}

$service = Get-Service -Name "NetQuota" -ErrorAction SilentlyContinue
if ($service) {
    & sc.exe stop NetQuota | Out-Null
    Start-Sleep -Seconds 2
    & sc.exe delete NetQuota | Out-Null
}

& (Join-Path $PSScriptRoot "unregister-native-host.ps1")

if (Test-Path $InstallDir) {
    Remove-Item $InstallDir -Recurse -Force
}

if ($RemoveData) {
    $dataDir = Join-Path $env:ProgramData "NetQuota"
    if (Test-Path $dataDir) { Remove-Item $dataDir -Recurse -Force }
    Write-Host "Removed usage database and settings."
}

Write-Host "NetQuota uninstalled."
