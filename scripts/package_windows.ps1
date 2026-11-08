# Build the Windows release bundle that install.ps1 understands.
#
# The repository never commits binaries: this script builds them, stages them
# next to the legal text and the installer, then writes hashes. Run it before a
# release, and run scripts/reproducible_build.ps1 to compare two builds.
#
# Usage:
#   .\scripts\package_windows.ps1
#   .\scripts\package_windows.ps1 -SkipBuild
#   .\scripts\package_windows.ps1 -Version 0.2.0
[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutDir = "",
    [switch]$SkipBuild,
    [switch]$SkipZip
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if (-not $Version) {
    $manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    $match = [regex]::Match($manifest, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) { throw 'could not read the workspace version from Cargo.toml' }
    $Version = $match.Groups[1].Value
}
if (-not $OutDir) { $OutDir = Join-Path $root 'dist' }
$stage = Join-Path $OutDir "nephele-$Version-win64"

Write-Host "Packaging Nephele $Version for win64" -ForegroundColor Cyan
Write-Host "  stage: $stage"

if (-not $SkipBuild) {
    # --locked is the first half of a reproducible build: the dependency graph
    # is exactly Cargo.lock, not whatever the registry serves today.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo build --release --locked -p nephele -p nephele-launcher -p nephele-prepare
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($code -ne 0) { throw "cargo build failed with exit code $code" }
}

$release = Join-Path $root 'target\release'
$required = @('nephele.exe', 'nephele-launcher.exe', 'nephele-prepare.exe')
foreach ($name in $required) {
    $path = Join-Path $release $name
    if (-not (Test-Path -LiteralPath $path)) {
        throw "$name is missing from $release; run without -SkipBuild"
    }
}

if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

foreach ($name in $required) {
    Copy-Item -LiteralPath (Join-Path $release $name) -Destination $stage
}
foreach ($relative in @('research-lock.json', 'README.md', 'scripts\install.ps1', 'scripts\uninstall.ps1')) {
    $source = Join-Path $root $relative
    if (-not (Test-Path -LiteralPath $source)) { throw "$relative is missing" }
    Copy-Item -LiteralPath $source -Destination $stage
}

# Double-clickable entry points. They are generated rather than committed so
# the repository stays free of binaries and shell wrappers that drift.
@'
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
'@ | Set-Content -LiteralPath (Join-Path $stage 'install.cmd') -Encoding ASCII
@'
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" %*
'@ | Set-Content -LiteralPath (Join-Path $stage 'uninstall.cmd') -Encoding ASCII
Set-Content -LiteralPath (Join-Path $stage 'version.txt') -Value $Version -Encoding ASCII

$commit = (& git -C $root rev-parse --short HEAD 2>$null)
if (-not $commit) { $commit = 'unknown' }
$toolchain = (& rustc --version)
$updateKey = if ($env:NEPHELE_UPDATE_PUBLIC_KEY) { 'configured' } else { 'EMPTY (unsigned build)' }
$updateUrl = if ($env:NEPHELE_UPDATE_URL) { $env:NEPHELE_UPDATE_URL } else { 'not configured' }
$crashUrl = if ($env:NEPHELE_CRASH_URL) { $env:NEPHELE_CRASH_URL } else { 'not configured' }

$hashes = foreach ($name in $required) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $stage $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    "  $hash  $name"
}

$buildInfo = @"
Nephele Client $Version (win64)
commit:        $commit
built:         $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))
toolchain:     $toolchain
update key:    $updateKey
update url:    $updateUrl
crash url:     $crashUrl

SHA-256:
$($hashes -join "`n")

Nephele is not an official Minecraft product and is not affiliated with
Mojang or Microsoft. It ships no Minecraft code or assets; resources are built
on the user's machine from their own 1.8.9 jar.
"@
Set-Content -LiteralPath (Join-Path $stage 'BUILD-INFO.txt') -Value $buildInfo -Encoding UTF8

if (-not $SkipZip) {
    $zip = Join-Path $OutDir "nephele-$Version-win64.zip"
    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
    $zipHash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    "$zipHash  $(Split-Path -Leaf $zip)" |
        Set-Content -LiteralPath (Join-Path $OutDir 'SHA256SUMS.txt') -Encoding ASCII
    Write-Host "  zip:   $zip"
    Write-Host "  sha256 $zipHash"
}

Write-Host ''
Write-Host 'Bundle ready. Install with:' -ForegroundColor Green
Write-Host "  $(Join-Path $stage 'install.cmd')"
if (-not $env:NEPHELE_UPDATE_PUBLIC_KEY) {
    Write-Host ''
    Write-Host 'This build is UNSIGNED (NEPHELE_UPDATE_PUBLIC_KEY is empty).' -ForegroundColor Yellow
    Write-Host 'Update checks are disabled in the UI and the log. Code signing is still the blocker.'
}
