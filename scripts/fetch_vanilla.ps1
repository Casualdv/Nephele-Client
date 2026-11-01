<#
.SYNOPSIS
    Downloads a vanilla Minecraft Java client jar and its asset objects from
    Mojang's public metadata endpoints.

.DESCRIPTION
    Nephele never ships Mojang assets. This script fetches them locally into
    .\assets\vanilla\<version>\ so the renderer and UI layers have textures,
    fonts, sounds and language files to load at development time.

    The whole assets folder is gitignored.

.PARAMETER Version
    Minecraft version to fetch. Defaults to 1.8.9 (what Nephele targets).

.PARAMETER OutDir
    Destination root. Defaults to .\assets\vanilla.

.PARAMETER All
    Download every asset object (textures, sounds, lang, ...). Without this
    switch only the subsets listed in -Include are fetched.

.PARAMETER Include
    Virtual asset path prefixes to fetch when -All is not given.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\fetch_vanilla.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\fetch_vanilla.ps1 -All
#>
[CmdletBinding()]
param(
    [string]$Version = "1.8.9",
    [string]$OutDir = "",
    [switch]$All,
    [string[]]$Include = @(
        "minecraft/blockstates/",
        "minecraft/models/",
        "minecraft/textures/blocks/",
        "minecraft/textures/items/",
        "minecraft/textures/gui/",
        "minecraft/textures/font/",
        "minecraft/textures/misc/",
        "minecraft/font/",
        "minecraft/lang/en_US.lang",
        "minecraft/sounds.json"
    )
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $scriptRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    $OutDir = Join-Path (Split-Path -Parent $scriptRoot) "assets\vanilla"
}

$manifestUrl = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"

function Get-Json([string]$Url) {
    Write-Host "GET $Url"
    (Invoke-WebRequest -Uri $Url -UseBasicParsing).Content | ConvertFrom-Json
}

function Save-Url([string]$Url, [string]$Path) {
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    if (Test-Path -LiteralPath $Path) {
        $existing = (Get-Item -LiteralPath $Path).Length
        if ($existing -gt 0) { return }
    }
    Invoke-WebRequest -Uri $Url -UseBasicParsing -OutFile $Path
}

$root = [System.IO.Path]::GetFullPath($OutDir)
$versionDir = Join-Path $root $Version
New-Item -ItemType Directory -Force -Path $versionDir | Out-Null

Write-Host "Resolving $Version from Mojang version manifest..."
$manifest = Get-Json $manifestUrl
$entry = $manifest.versions | Where-Object { $_.id -eq $Version } | Select-Object -First 1
if (-not $entry) {
    throw "Version $Version was not found in the Mojang version manifest."
}

$versionJsonPath = Join-Path $versionDir "$Version.json"
Save-Url $entry.url $versionJsonPath
$versionJson = Get-Content -LiteralPath $versionJsonPath -Raw | ConvertFrom-Json

$clientUrl = $versionJson.downloads.client.url
$expectedClientSha1 = $versionJson.downloads.client.sha1.ToLowerInvariant()
$clientJar = Join-Path $versionDir "client.jar"
if (Test-Path -LiteralPath $clientJar) {
    Write-Host "client.jar already present, verifying"
} else {
    Write-Host "Downloading client jar ($([math]::Round($versionJson.downloads.client.size / 1MB, 1)) MiB)"
    Save-Url $clientUrl $clientJar
}

# Provenance. The jar is what every other file is derived from, so it is checked
# against the hash Mojang publishes for this exact version. A mismatch means the
# download is corrupt or was tampered with, and we refuse to build on it.
$actualClientSha1 = (Get-FileHash -LiteralPath $clientJar -Algorithm SHA1).Hash.ToLowerInvariant()
if ($actualClientSha1 -ne $expectedClientSha1) {
    throw ("client.jar hash mismatch: expected {0}, got {1}. Delete the file and re-run." -f $expectedClientSha1, $actualClientSha1)
}
Write-Host ("client.jar verified ({0})" -f $actualClientSha1)

function Test-Include([string]$VirtualPath) {
    if ($All) { return $true }
    foreach ($prefix in $Include) {
        if ($VirtualPath.StartsWith($prefix)) { return $true }
    }
    return $false
}

# 1.8.9 ships most assets (blockstates, models, textures, lang, font) inside
# client.jar; only sounds live in the asset index.
Write-Host "Extracting assets/ from client.jar..."
Add-Type -AssemblyName System.IO.Compression.FileSystem | Out-Null
$jarAssets = Join-Path $versionDir "assets"
$zip = [System.IO.Compression.ZipFile]::OpenRead($clientJar)
$extracted = 0
try {
    foreach ($entry in $zip.Entries) {
        if (-not $entry.FullName.StartsWith("assets/")) { continue }
        if ($entry.FullName.EndsWith("/")) { continue }
        $virtual = $entry.FullName.Substring("assets/".Length)
        if (-not (Test-Include $virtual)) { continue }
        $target = Join-Path $jarAssets $entry.FullName.Substring("assets/".Length)
        $parent = Split-Path -Parent $target
        if (-not (Test-Path -LiteralPath $parent)) {
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
        }
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $true)
        $extracted++
    }
}
finally {
    $zip.Dispose()
}
Write-Host ("Extracted {0} asset file(s) from client.jar" -f $extracted)

$assetIndex = $versionJson.assetIndex
$indexPath = Join-Path $root "indexes\$($assetIndex.id).json"
Save-Url $assetIndex.url $indexPath
$index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json

$objects = $index.objects.PSObject.Properties
$selected = @()
foreach ($object in $objects) {
    if (Test-Include $object.Name) { $selected += $object }
}

Write-Host ("Downloading {0} index object(s) of {1} total..." -f $selected.Count, $objects.Count)

$map = @{}
$downloaded = 0
$skipped = 0
foreach ($object in $selected) {
    $hash = $object.Value.hash
    $size = [int64]$object.Value.size
    $target = Join-Path $root ("objects\{0}\{1}" -f $hash.Substring(0, 2), $hash)

    if ((Test-Path -LiteralPath $target) -and ((Get-Item -LiteralPath $target).Length -eq $size)) {
        $skipped++
    } else {
        $url = "https://resources.download.minecraft.net/{0}/{1}" -f $hash.Substring(0, 2), $hash
        Save-Url $url $target
        $downloaded++
        if (($downloaded % 100) -eq 0) {
            Write-Host ("  {0}/{1}" -f $downloaded, $selected.Count)
        }
    }

    # The asset index names the content hash of every object, so each one is
    # verified whether it was just downloaded or already on disk.
    $actual = (Get-FileHash -LiteralPath $target -Algorithm SHA1).Hash.ToLowerInvariant()
    if ($actual -ne $hash) {
        throw ("asset object {0} hash mismatch: expected {1}, got {2}" -f $object.Name, $hash, $actual)
    }
    $map[$object.Name] = @{ hash = $hash; size = $size }
}

$outManifest = [ordered]@{
    version               = $Version
    assetIndex            = $assetIndex.id
    clientJar             = "client.jar"
    clientJarSha1         = $actualClientSha1
    clientJarSize         = (Get-Item -LiteralPath $clientJar).Length
    assetIndexSha1        = $assetIndex.sha1
    assetIndexSize        = $assetIndex.size
    verifiedAgainstMojang = $true
    extractedRoot         = "assets"
    objectsRoot           = "objects"
    fetchedAtUtc          = (Get-Date).ToUniversalTime().ToString("o")
    filter                = if ($All) { "all" } else { $Include }
    extractedFiles        = $extracted
    indexObjects          = $map
}
$manifestOut = Join-Path $versionDir "nephele_assets.json"
$outManifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestOut -Encoding UTF8

Write-Host ""
Write-Host ("Done. {0} downloaded, {1} already cached." -f $downloaded, $skipped)
Write-Host "Assets root : $root"
Write-Host "Manifest    : $manifestOut"
