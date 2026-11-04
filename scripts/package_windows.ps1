[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$')]
    [string]$Version,
    [string]$Tag = "",
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$distRoot = Join-Path $repoRoot "dist"
$stageRoot = Join-Path $distRoot "stage-$Version"
$versionRoot = Join-Path $stageRoot "versions\$Version"
$pfxPath = Join-Path $stageRoot "codesign.pfx"
$privateKeyPath = Join-Path $stageRoot "update-signing-key.pem"
$certificate = $null
trap {
    if (Test-Path -LiteralPath $pfxPath) { Remove-Item -LiteralPath $pfxPath -Force -ErrorAction SilentlyContinue }
    if (Test-Path -LiteralPath $privateKeyPath) { Remove-Item -LiteralPath $privateKeyPath -Force -ErrorAction SilentlyContinue }
    if ($certificate) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -Force -ErrorAction SilentlyContinue
    }
    throw
}
$updateSource = Get-Content -LiteralPath (Join-Path $repoRoot "crates\nephele-launcher\src\updates.rs") -Raw
$publicKeyMatch = [regex]::Match($updateSource, 'PINNED_UPDATE_PUBLIC_MODULUS_HEX: &str = "([0-9a-f]+)"')
if (-not $publicKeyMatch.Success) {
    throw "Could not read the launcher's pinned update key."
}
$env:NEPHELE_UPDATE_PUBLIC_MODULUS_HEX = $publicKeyMatch.Groups[1].Value

if (-not (Test-Path -LiteralPath $repoRoot -PathType Container)) {
    throw "Repository root is missing: $repoRoot"
}
if ($Release) {
    foreach ($name in @("NEPHELE_CLIENT_ID", "NEPHELE_CRASH_REPORT_URL", "NEPHELE_UPDATE_SIGNING_KEY_B64", "NEPHELE_CODESIGN_PFX_B64", "NEPHELE_CODESIGN_PASSWORD")) {
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
            throw "Release packaging requires the $name environment variable."
        }
    }
}
if (-not [string]::IsNullOrWhiteSpace($env:NEPHELE_CRASH_REPORT_URL) -and $env:NEPHELE_CRASH_REPORT_URL -notmatch '^https://') {
    throw "NEPHELE_CRASH_REPORT_URL must use HTTPS."
}

if (Test-Path -LiteralPath $stageRoot) {
    if (-not (Test-Path -LiteralPath (Join-Path $stageRoot ".nephele-package-stage"))) {
        throw "Refusing to replace an unmarked staging folder: $stageRoot"
    }
    Remove-Item -LiteralPath $stageRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $versionRoot -Force | Out-Null
New-Item -ItemType File -Path (Join-Path $stageRoot ".nephele-package-stage") -Force | Out-Null

$env:NEPHELE_APP_VERSION = $Version

Write-Host "Building Nephele $Version for Windows x64..."
cargo build --release -p nephele -p nephele-prepare -p nephele-launcher --bins
if ($LASTEXITCODE -ne 0) {
    throw "Cargo release build failed with exit code $LASTEXITCODE"
}

$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot "target" }
$releaseRoot = Join-Path $targetRoot "release"
foreach ($binary in @("nephele.exe", "nephele-launcher.exe", "nephele-prepare.exe")) {
    $source = Join-Path $releaseRoot $binary
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Cargo did not produce $source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $versionRoot $binary)
}
Copy-Item -LiteralPath (Join-Path $repoRoot "research-lock.json") -Destination (Join-Path $versionRoot "research-lock.json")
[System.IO.File]::WriteAllText((Join-Path $stageRoot "current-version.txt"), $Version)
[System.IO.File]::WriteAllText((Join-Path $stageRoot ".nephele-install"), "Nephele managed install root")

$iconPath = Join-Path $stageRoot "nephele.ico"
& (Join-Path $PSScriptRoot "generate_brand_icon.ps1") -OutputPath $iconPath
if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) {
    throw "Brand icon generation failed"
}

function Set-AuthenticodeSignatureForRelease {
    param([string]$Path, $Certificate)
    $result = Set-AuthenticodeSignature -FilePath $Path -Certificate $Certificate -HashAlgorithm SHA256 -TimestampServer "http://timestamp.digicert.com"
    if ($result.Status -ne "Valid") {
        throw "Authenticode signing failed for ${Path}: $($result.StatusMessage)"
    }
}

if (-not [string]::IsNullOrWhiteSpace($env:NEPHELE_CODESIGN_PFX_B64)) {
    [System.IO.File]::WriteAllBytes($pfxPath, [Convert]::FromBase64String($env:NEPHELE_CODESIGN_PFX_B64))
    $password = ConvertTo-SecureString $env:NEPHELE_CODESIGN_PASSWORD -AsPlainText -Force
    $importedCertificates = Import-PfxCertificate -FilePath $pfxPath -CertStoreLocation Cert:\CurrentUser\My -Password $password
    $certificate = $importedCertificates | Where-Object { $_.HasPrivateKey } | Select-Object -First 1
    if ($null -eq $certificate) {
        throw "Could not import the Authenticode certificate."
    }
    foreach ($binary in @("nephele.exe", "nephele-launcher.exe", "nephele-prepare.exe")) {
        Set-AuthenticodeSignatureForRelease -Path (Join-Path $versionRoot $binary) -Certificate $certificate
    }
}

$env:NEPHELE_APP_VERSION = $Version
$env:NEPHELE_STAGE_DIR = $stageRoot
$iscc = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
if ($null -eq $iscc) {
    $candidate = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    if (Test-Path -LiteralPath $candidate) {
        $iscc = [PSCustomObject]@{ Source = $candidate }
    } else {
        throw "Inno Setup 6 (ISCC.exe) is required. Install it from https://jrsoftware.org/isinfo.php"
    }
}
& $iscc.Source (Join-Path $repoRoot "installer\Nephele.iss")
if ($LASTEXITCODE -ne 0) {
    throw "Inno Setup failed with exit code $LASTEXITCODE"
}

$installerPath = Join-Path $distRoot "NepheleSetup-$Version-win-x64.exe"
if ($certificate) {
    Set-AuthenticodeSignatureForRelease -Path $installerPath -Certificate $certificate
}

if ($Release) {
    $tag = if ($Tag) { $Tag } elseif ($env:GITHUB_REF_NAME -match '^v') { $env:GITHUB_REF_NAME } else { "v$Version" }
    $channel = if ($tag -match '(?i)(alpha|beta|preview|rc)') { "preview" } else { "stable" }
    $files = [ordered]@{}
    foreach ($name in @("nephele.exe", "nephele-launcher.exe", "nephele-prepare.exe", "research-lock.json")) {
        $path = Join-Path $versionRoot $name
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        $size = (Get-Item -LiteralPath $path).Length
        $url = "https://github.com/Casualdv/Nephele-Client/releases/download/$tag/$name"
        $files[$name] = [ordered]@{ url = $url; sha256 = $hash; size = $size }
    }
    $releaseUrl = "https://github.com/Casualdv/Nephele-Client/releases/tag/$tag"
    $changeLines = @(& git -C $repoRoot log -8 --format="* %s")
    if ($LASTEXITCODE -ne 0 -or $changeLines.Count -eq 0) {
        $changeLines = @("See the release page for changes and known limitations.")
    }
    $releaseNotes = $changeLines -join "`n"
    $manifest = [ordered]@{
        schema = 1
        tag = $tag
        version = $Version
        channel = $channel
        released_at = [DateTime]::UtcNow.ToString("o")
        notes = $releaseNotes
        news = @(@{
            title = "Nephele $Version"
            body = "Version $Version is available in the $channel channel.`n$releaseNotes"
            url = $releaseUrl
        })
        platforms = [ordered]@{ "windows-x86_64" = $files }
    }
    $manifestPath = Join-Path $stageRoot "update-manifest.json"
    $manifestJson = ConvertTo-Json -InputObject $manifest -Depth 10
    [System.IO.File]::WriteAllText($manifestPath, $manifestJson, (New-Object System.Text.UTF8Encoding($false)))

    [System.IO.File]::WriteAllBytes($privateKeyPath, [Convert]::FromBase64String($env:NEPHELE_UPDATE_SIGNING_KEY_B64))
    & cargo run -p nephele-launcher --example sign_update_manifest -- $privateKeyPath $manifestPath (Join-Path $stageRoot "update-manifest.sig")
    if ($LASTEXITCODE -ne 0) {
        throw "Update manifest signing failed with exit code $LASTEXITCODE"
    }
    Remove-Item -LiteralPath $privateKeyPath -Force
    if (Test-Path -LiteralPath $pfxPath) { Remove-Item -LiteralPath $pfxPath -Force }
    if ($certificate) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Installer: $installerPath"
    Write-Host "Signed release assets and update manifest staged under $stageRoot"
} else {
    if (Test-Path -LiteralPath $pfxPath) { Remove-Item -LiteralPath $pfxPath -Force }
    Write-Host "Installer: $installerPath"
    Write-Warning "This was a local package build. It was not published as a signed update release."
}
