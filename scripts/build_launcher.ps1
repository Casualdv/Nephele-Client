# Build the launcher and the two binaries it spawns, then assemble dist\.
#
# The three executables are not optional: the launcher starts `nephele` and
# `nephele-prepare` as sibling processes, so a launcher without them next to it
# reports "nephele is not next to the launcher; build the workspace".
#
# For day-to-day use prefer `Launch Nephele.cmd`, which runs the binaries out of
# target\release directly - rebuilding with cargo updates them in place, so no
# step is needed between editing code and launching.

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root "dist"

Write-Host "Building release binaries..."
cargo build --release -p nephele-launcher -p nephele -p nephele-prepare

New-Item -ItemType Directory -Force -Path $dist | Out-Null

$binaries = @("nephele-launcher.exe", "nephele.exe", "nephele-prepare.exe")
foreach ($exe in $binaries) {
    $source = Join-Path $root "target\release\$exe"
    if (-not (Test-Path $source)) {
        throw "missing $source after the build"
    }
    Copy-Item $source (Join-Path $dist $exe) -Force
}

foreach ($extra in @("LICENSE", "LICENSE-MIT", "LICENSE-APACHE")) {
    $source = Join-Path $root $extra
    if (Test-Path $source) {
        Copy-Item $source $dist -Force
    }
}

$total = (Get-ChildItem $dist -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("dist ready: {0:N1} MB in {1}" -f $total, $dist)
Write-Host "Double-click nephele-launcher.exe there, or zip dist\ to share it."
