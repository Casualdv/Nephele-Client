# Install a staged Nephele bundle for the current user.
#
# No administrator rights are needed: the application goes under
# %LOCALAPPDATA%\Programs\Nephele, the Start Menu shortcut under the user's own
# Start Menu, and the uninstall entry under HKCU.
#
# Updates are clean: the previous install's manifest lists exactly the files it
# put down, so those are removed before the new copy lands. Nothing inside the
# user data directory (profiles.json, saves, auth.json, servers.json, prepared
# resources, logs, crash reports) is ever touched.
#
# Usage:
#   .\install.ps1
#   .\install.ps1 -Source C:\path\to\nephele-0.1.0-win64
#   .\install.ps1 -DryRun
[CmdletBinding()]
param(
    [string]$Source = "",
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\Nephele'),
    [string]$StartMenuDir = (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),
    [string]$DataDir = (Join-Path $env:APPDATA 'Nephele'),
    [switch]$NoShortcut,
    [switch]$NoRegistry,
    [switch]$Force,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$dataVersion = 2

if (-not $Source) { $Source = $PSScriptRoot }
if (-not (Test-Path -LiteralPath $Source)) { throw "source directory not found: $Source" }

$required = @('nephele.exe', 'nephele-launcher.exe', 'nephele-prepare.exe', 'research-lock.json')
foreach ($name in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $Source $name))) {
        throw "$Source does not look like a Nephele bundle: $name is missing"
    }
}
$version = if (Test-Path -LiteralPath (Join-Path $Source 'version.txt')) {
    (Get-Content -LiteralPath (Join-Path $Source 'version.txt') -Raw).Trim()
}
else { '0.1.0' }

function Write-Step([string]$message) {
    if ($DryRun) { Write-Host "  would: $message" } else { Write-Host "  $message" }
}

function Stop-IfRunning {
    if ($Force) { return }
    $running = Get-Process -Name 'nephele', 'nephele-launcher' -ErrorAction SilentlyContinue
    if ($running) {
        $names = ($running | Select-Object -ExpandProperty ProcessName -Unique) -join ', '
        throw "close Nephele before installing or uninstalling ($names is running); use -Force for a scripted install"
    }
}

Write-Host "Installing Nephele $version" -ForegroundColor Cyan
Write-Host "  source:  $Source"
Write-Host "  app:     $InstallDir"
Write-Host "  data:    $DataDir (left alone on update)"

Stop-IfRunning

# --- clean update: remove exactly the files the previous install owned -------
$manifestPath = Join-Path $InstallDir 'install.json'
if (Test-Path -LiteralPath $manifestPath) {
    try {
        $previous = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    }
    catch {
        throw "the previous install manifest at $manifestPath is unreadable; remove the folder and retry"
    }
    foreach ($relative in @($previous.files)) {
        if (-not $relative) { continue }
        $target = Join-Path $InstallDir $relative
        if (Test-Path -LiteralPath $target) {
            Write-Step "remove stale $relative"
            if (-not $DryRun) { Remove-Item -LiteralPath $target -Force }
        }
    }
}

# --- copy the new bundle ----------------------------------------------------
if (-not $DryRun) { New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null }
$installed = New-Object System.Collections.Generic.List[string]
foreach ($file in Get-ChildItem -LiteralPath $Source -File) {
    Write-Step "copy $($file.Name)"
    if (-not $DryRun) {
        Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $InstallDir $file.Name) -Force
    }
    $installed.Add($file.Name)
}

# --- start menu shortcut ----------------------------------------------------
$shortcutPath = Join-Path $StartMenuDir 'Nephele Client.lnk'
if (-not $NoShortcut) {
    Write-Step "create shortcut $shortcutPath"
    if (-not $DryRun) {
        New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($shortcutPath)
        $shortcut.TargetPath = Join-Path $InstallDir 'nephele-launcher.exe'
        $shortcut.WorkingDirectory = $InstallDir
        $shortcut.IconLocation = Join-Path $InstallDir 'nephele-launcher.exe'
        $shortcut.Description = 'Nephele Client'
        $shortcut.Save()
    }
}

# --- Apps & Features entry --------------------------------------------------
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nephele'
if (-not $NoRegistry) {
    Write-Step "register uninstall entry at $uninstallKey"
    if (-not $DryRun) {
        New-Item -Path $uninstallKey -Force | Out-Null
        $uninstall = Join-Path $InstallDir 'uninstall.ps1'
        Set-ItemProperty -Path $uninstallKey -Name DisplayName -Value 'Nephele Client'
        Set-ItemProperty -Path $uninstallKey -Name DisplayVersion -Value $version
        Set-ItemProperty -Path $uninstallKey -Name Publisher -Value 'Nephele'
        Set-ItemProperty -Path $uninstallKey -Name InstallLocation -Value $InstallDir
        Set-ItemProperty -Path $uninstallKey -Name DisplayIcon -Value (Join-Path $InstallDir 'nephele-launcher.exe')
        Set-ItemProperty -Path $uninstallKey -Name UninstallString -Value ('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $uninstall)
        Set-ItemProperty -Path $uninstallKey -Name QuietUninstallString -Value ('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{0}" -Quiet' -f $uninstall)
        Set-ItemProperty -Path $uninstallKey -Name NoModify -Value 1 -Type DWord
        Set-ItemProperty -Path $uninstallKey -Name NoRepair -Value 1 -Type DWord
    }
}

# --- data directory: create, migrate, never wipe ----------------------------
Write-Step "prepare data directory $DataDir"
if (-not $DryRun) {
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    foreach ($sub in @('logs', 'crashes', 'prepared', 'launcher')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $DataDir $sub) | Out-Null
    }
    $dataManifest = Join-Path $DataDir 'install.json'
    $previousData = $null
    if (Test-Path -LiteralPath $dataManifest) {
        try { $previousData = Get-Content -LiteralPath $dataManifest -Raw | ConvertFrom-Json } catch { $previousData = $null }
    }
    if ($null -eq $previousData -or $previousData.dataVersion -lt $dataVersion) {
        Write-Host '  migrating the data directory (user files are preserved)' -ForegroundColor Yellow
    }
    [pscustomobject]@{
        dataVersion       = $dataVersion
        lastClientVersion = $version
        updatedAt         = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    } | ConvertTo-Json | Set-Content -LiteralPath $dataManifest -Encoding UTF8
}

# --- install manifest -------------------------------------------------------
if (-not $DryRun) {
    [pscustomobject]@{
        version     = $version
        files       = @($installed)
        installedAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    } | ConvertTo-Json | Set-Content -LiteralPath $manifestPath -Encoding UTF8
}

Write-Host ''
if ($DryRun) {
    Write-Host 'Dry run complete; nothing was changed.' -ForegroundColor Yellow
}
else {
    Write-Host "Installed. Launch from the Start Menu, or run $(Join-Path $InstallDir 'nephele-launcher.exe')" -ForegroundColor Green
}
