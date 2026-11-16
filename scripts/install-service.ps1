# Installs the NetQuota service. Run in an elevated PowerShell.
# Usage:
#   .\install-service.ps1 -ExtensionId <32-char-chrome-extension-id>
# Build first: cargo build --release -p netquota-windows-service -p netquota-cli -p netquota-native-host
param(
    [string]$ExtensionId = "",
    [string]$InstallDir = "$env:ProgramFiles\NetQuota"
)

$ErrorActionPreference = "Stop"
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this script from an elevated PowerShell."
}

$root = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $root "target\release"
foreach ($exe in @("netquota-service.exe", "netquota-cli.exe", "netquota-native-host.exe")) {
    if (-not (Test-Path (Join-Path $bin $exe))) {
        throw "Missing $exe. Run: cargo build --release"
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item (Join-Path $bin "netquota-service.exe") $InstallDir -Force
Copy-Item (Join-Path $bin "netquota-cli.exe") $InstallDir -Force
Copy-Item (Join-Path $bin "netquota-native-host.exe") $InstallDir -Force

$dataDir = Join-Path $env:ProgramData "NetQuota"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null

$serviceExe = Join-Path $InstallDir "netquota-service.exe"
$existing = Get-Service -Name "NetQuota" -ErrorAction SilentlyContinue
if ($existing) {
    Write-Host "Service already installed; stopping and replacing."
    & sc.exe stop NetQuota | Out-Null
    Start-Sleep -Seconds 2
    & sc.exe delete NetQuota | Out-Null
    Start-Sleep -Seconds 1
}

# LocalSystem is required for ETW kernel sessions and firewall changes.
& sc.exe create NetQuota binPath= "`"$serviceExe`"" start= auto obj= LocalSystem DisplayName= "NetQuota Network Manager" | Out-Null
& sc.exe description NetQuota "Local-first per-application network monitoring and quota enforcement." | Out-Null
& sc.exe failure NetQuota reset= 86400 actions= restart/5000/restart/10000/restart/30000 | Out-Null
& sc.exe start NetQuota | Out-Null

if ($ExtensionId) {
    & (Join-Path $PSScriptRoot "register-native-host.ps1") -ExtensionId $ExtensionId -HostPath (Join-Path $InstallDir "netquota-native-host.exe")
}

Write-Host "NetQuota service installed. Verify with:"
Write-Host "  & `"$InstallDir\netquota-cli.exe`" doctor"
