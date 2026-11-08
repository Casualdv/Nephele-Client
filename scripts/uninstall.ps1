# Remove a per-user Nephele install.
#
# User data (profiles, saves, sign-in token, server list, prepared resources,
# logs and crash reports) is KEPT by default: uninstalling a game should not
# delete somebody's worlds by surprise. Pass -RemoveData to delete it too.
#
# Usage:
#   .\uninstall.ps1
#   .\uninstall.ps1 -RemoveData
#   .\uninstall.ps1 -DryRun
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\Nephele'),
    [string]$StartMenuDir = (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),
    [string]$DataDir = (Join-Path $env:APPDATA 'Nephele'),
    [switch]$RemoveData,
    [switch]$Quiet,
    [switch]$Force,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

function Write-Step([string]$message) {
    if ($DryRun) { Write-Host "  would: $message" } else { Write-Host "  $message" }
}

function Stop-IfRunning {
    if ($Force) { return }
    $running = Get-Process -Name 'nephele', 'nephele-launcher' -ErrorAction SilentlyContinue
    if ($running) {
        $names = ($running | Select-Object -ExpandProperty ProcessName -Unique) -join ', '
        throw "close Nephele before uninstalling ($names is running); use -Force for a scripted uninstall"
    }
}

Write-Host "Uninstalling Nephele" -ForegroundColor Cyan
Write-Host "  app:  $InstallDir"
Write-Host "  data: $DataDir"

Stop-IfRunning

# Remove only the files the installer owns, then prune empty directories.
$manifestPath = Join-Path $InstallDir 'install.json'
if (Test-Path -LiteralPath $manifestPath) {
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    }
    catch {
        $manifest = $null
    }
    foreach ($relative in @($manifest.files)) {
        if (-not $relative) { continue }
        $target = Join-Path $InstallDir $relative
        if (Test-Path -LiteralPath $target) {
            Write-Step "remove $relative"
            if (-not $DryRun) { Remove-Item -LiteralPath $target -Force }
        }
    }
}
else {
    Write-Host '  no install manifest; removing known binaries only' -ForegroundColor Yellow
    foreach ($name in @('nephele.exe', 'nephele-launcher.exe', 'nephele-prepare.exe')) {
        $target = Join-Path $InstallDir $name
        if (Test-Path -LiteralPath $target) {
            Write-Step "remove $name"
            if (-not $DryRun) { Remove-Item -LiteralPath $target -Force }
        }
    }
}

$manifestLeft = Join-Path $InstallDir 'install.json'
if (Test-Path -LiteralPath $manifestLeft) {
    Write-Step 'remove install.json'
    if (-not $DryRun) { Remove-Item -LiteralPath $manifestLeft -Force }
}
if ((Test-Path -LiteralPath $InstallDir) -and -not $DryRun) {
    $remaining = @(Get-ChildItem -LiteralPath $InstallDir -Force)
    if ($remaining.Count -eq 0) {
        Remove-Item -LiteralPath $InstallDir -Force
    }
    else {
        Write-Host "  kept $InstallDir because it still contains other files" -ForegroundColor Yellow
    }
}

# Start Menu shortcut.
$shortcut = Join-Path $StartMenuDir 'Nephele Client.lnk'
if (Test-Path -LiteralPath $shortcut) {
    Write-Step "remove shortcut $shortcut"
    if (-not $DryRun) { Remove-Item -LiteralPath $shortcut -Force }
}

# Apps & Features entry.
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nephele'
if (Test-Path -LiteralPath $uninstallKey) {
    Write-Step "remove registry key $uninstallKey"
    if (-not $DryRun) { Remove-Item -LiteralPath $uninstallKey -Recurse -Force }
}

# Data directory: opt-in deletion, guarded against a bad -DataDir.
if ($RemoveData) {
    $resolved = [System.IO.Path]::GetFullPath($DataDir)
    $leaf = Split-Path -Leaf $resolved
    $marker = Join-Path $resolved 'install.json'
    # Accept either the conventional folder name or a custom folder the
    # installer has marked. Never delete a drive root or an unmarked folder.
    $looksRight = ($leaf -eq 'Nephele') -or (Test-Path -LiteralPath $marker)
    if (-not $looksRight -or $resolved -eq [System.IO.Path]::GetPathRoot($resolved)) {
        throw "refusing to delete '$resolved': it is not a Nephele data directory"
    }
    if (-not $Quiet) {
        $answer = Read-Host "Delete all Nephele user data at $resolved? Type DELETE to confirm"
        if ($answer -ne 'DELETE') {
            Write-Host '  data kept.' -ForegroundColor Yellow
            return
        }
    }
    Write-Step "delete data directory $resolved"
    if (-not $DryRun) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
else {
    Write-Host ''
    Write-Host "User data kept at $DataDir" -ForegroundColor Green
    Write-Host '  (profiles, saves, sign-in, servers and prepared resources)'
    Write-Host '  re-run with -RemoveData to delete it as well.'
}

Write-Host ''
if ($DryRun) {
    Write-Host 'Dry run complete; nothing was changed.' -ForegroundColor Yellow
}
else {
    Write-Host 'Uninstalled.' -ForegroundColor Green
}
