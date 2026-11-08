# Build the release binaries with pinned, machine-independent inputs.
#
# A build is only "reproducible" if a second run on the same toolchain produces
# the same bytes. This script fixes everything that usually drifts:
#
#   * Cargo.lock is enforced (--locked);
#   * the source checkout path is remapped out of debug info;
#   * debug info is dropped and symbols stripped;
#   * the linker is asked for reproducible PE timestamps (/Brepro);
#   * SOURCE_DATE_EPOCH is the commit time, not "now".
#
# -Verify does the second run and compares SHA-256 hashes. Expect it to take a
# while: changing RUSTFLAGS invalidates the dependency cache once.
#
# Usage:
#   .\scripts\reproducible_build.ps1
#   .\scripts\reproducible_build.ps1 -Verify
[CmdletBinding()]
param(
    [string]$OutDir = "",
    [switch]$Verify,
    [string[]]$Packages = @('nephele', 'nephele-launcher', 'nephele-prepare')
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $root 'dist' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$epoch = (& git -C $root show -s --format=%ct HEAD 2>$null)
if (-not $epoch) { $epoch = '0' }
$env:SOURCE_DATE_EPOCH = $epoch
$env:CARGO_INCREMENTAL = '0'

$flags = @(
    '-C', 'codegen-units=1',
    '-C', 'debuginfo=0',
    '-C', 'strip=symbols',
    "--remap-path-prefix=$root=/nephele",
    '-C', 'link-arg=/Brepro'
)
# CARGO_ENCODED_RUSTFLAGS uses US (0x1f) separators, so a checkout path with
# spaces survives; plain RUSTFLAGS would split it.
$env:CARGO_ENCODED_RUSTFLAGS = ($flags -join [char]0x1f)

function Build {
    Write-Host "Building: $($Packages -join ', ')" -ForegroundColor Cyan
    $arguments = @('build', '--release', '--locked')
    foreach ($package in $Packages) { $arguments += @('-p', $package) }
    # Windows PowerShell turns native stderr into a terminating error when
    # $ErrorActionPreference is Stop; cargo writes its progress there.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo @arguments
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($code -ne 0) { throw "cargo build failed with exit code $code" }
}

function Get-Hashes {
    $hashes = [ordered]@{}
    foreach ($package in $Packages) {
        $name = if ($package -eq 'nephele') { 'nephele.exe' } else { "$package.exe" }
        $path = Join-Path $root "target\release\$name"
        if (-not (Test-Path -LiteralPath $path)) { throw "$name was not built" }
        $hashes[$name] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return $hashes
}

function Show-Hashes($hashes) {
    foreach ($name in $hashes.Keys) { Write-Host ("  {0}  {1}" -f $hashes[$name], $name) }
}

Build
$first = Get-Hashes
Show-Hashes $first
$record = [pscustomobject]@{
    epoch      = $epoch
    toolchain  = (& rustc --version)
    rustflags  = $flags
    packages   = $Packages
    hashes     = $first
}
$record | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $OutDir 'build-hashes.json') -Encoding UTF8
Write-Host "  record: $(Join-Path $OutDir 'build-hashes.json')"

if (-not $Verify) {
    Write-Host ''
    Write-Host 'Run with -Verify to rebuild from clean and compare the hashes.' -ForegroundColor Yellow
    exit 0
}

Write-Host ''
Write-Host 'Cleaning the packages and rebuilding for comparison...' -ForegroundColor Cyan
foreach ($package in $Packages) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo clean --release -p $package
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($code -ne 0) { throw "cargo clean -p $package failed" }
}
Build
$second = Get-Hashes
Show-Hashes $second

$mismatched = @()
foreach ($name in $first.Keys) {
    if ($first[$name] -ne $second[$name]) { $mismatched += $name }
}
if ($mismatched.Count -gt 0) {
    Write-Host ''
    Write-Host "NOT REPRODUCIBLE: $($mismatched -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host ''
Write-Host 'Reproducible: both builds produced identical SHA-256 hashes.' -ForegroundColor Green
