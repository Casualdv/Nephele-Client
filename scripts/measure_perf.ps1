<#
.SYNOPSIS
    Measure Nephele's frame budget and chunk meshing on an integrated GPU.

.DESCRIPTION
    Builds the release CLI, then runs a deterministic local world with a generated
    flat-color placeholder texture (no game assets). The client requests a
    low-power adapter and logs its actual adapter name/type. The run must report
    frame and mesh p50/p99 values plus non-zero mesh/upload counts before it can
    be treated as a valid measurement. The workload uses the client default render
    distance of 8 chunks (a 17x17 column area around spawn).

    Frame-time histograms reset every 30 seconds. Run for at least 75 seconds to
    collect a startup window and a separate steady-state window; the script checks
    the final window. Mesh-build percentiles and chunk-upload counts are cumulative.

.PARAMETER DurationSeconds
    Wall-clock measurement duration. Defaults to 75 seconds; minimum is 75.

.PARAMETER IntegratedAdapterPattern
    Regex required in the selected adapter line in addition to the IntegratedGpu
    device type. Defaults to common integrated adapter names.

.PARAMETER SkipBuild
    Use the existing target\release\nephele.exe instead of building it.

.EXAMPLE
    .\scripts\measure_perf.ps1

.EXAMPLE
    .\scripts\measure_perf.ps1 -DurationSeconds 120
#>
[CmdletBinding()]
param(
    [ValidateRange(75, 3600)]
    [int]$DurationSeconds = 75,

    [string]$IntegratedAdapterPattern = 'Radeon|Iris|UHD|Vega|Arc',

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path

Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
. 'C:\Users\dariu\AppData\Local\Temp\opencode\cargo-env.ps1'

if (-not $SkipBuild) {
    cargo build --manifest-path (Join-Path $repoRoot 'Cargo.toml') -p nephele --release --bin nephele
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}

$binary = Join-Path $repoRoot 'target\release\nephele.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Release binary not found at '$binary'; run without -SkipBuild first."
}

$tempBase = [System.IO.Path]::GetTempPath()
if (-not (Test-Path -LiteralPath $tempBase -PathType Container)) {
    throw "Temporary directory '$tempBase' does not exist."
}
$runDir = Join-Path $tempBase ("nephele-perf-{0}-{1}" -f $PID, [guid]::NewGuid().ToString('N'))
$blocksDir = Join-Path $runDir 'textures\blocks'
$guiDir = Join-Path $runDir 'textures\gui'
$dataDir = Join-Path $runDir 'user-data'
$stdoutPath = Join-Path $runDir 'client.stdout.log'
$stderrPath = Join-Path $runDir 'client.stderr.log'
New-Item -ItemType Directory -Path $runDir -Force | Out-Null
New-Item -ItemType Directory -Path $blocksDir -Force | Out-Null
New-Item -ItemType Directory -Path $guiDir -Force | Out-Null
New-Item -ItemType Directory -Path $dataDir -Force | Out-Null

$processEnvNames = @(
    'NEPHELE_PERF',
    'NEPHELE_GPU',
    'NEPHELE_SCREEN',
    'NEPHELE_VIEW_DISTANCE',
    'NEPHELE_DATA_DIR',
    'NEPHELE_PROFILE'
)
$previousEnv = @{}
foreach ($name in $processEnvNames) {
    $previousEnv[$name] = [System.Environment]::GetEnvironmentVariable($name, 'Process')
}

$bitmap = $null
$process = $null
try {
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new(16, 16)
    $color = [System.Drawing.Color]::FromArgb(255, 128, 128, 128)
    for ($y = 0; $y -lt 16; $y++) {
        for ($x = 0; $x -lt 16; $x++) {
            $bitmap.SetPixel($x, $y, $color)
        }
    }
    $bitmap.Save(
        (Join-Path $blocksDir 'stone.png'),
        [System.Drawing.Imaging.ImageFormat]::Png
    )
    $bitmap.Save(
        (Join-Path $guiDir 'icons.png'),
        [System.Drawing.Imaging.ImageFormat]::Png
    )
    $bitmap.Dispose()
    $bitmap = $null

    $env:NEPHELE_PERF = '1'
    $env:NEPHELE_GPU = 'integrated'
    $env:NEPHELE_SCREEN = 'local'
    Remove-Item Env:NEPHELE_VIEW_DISTANCE -ErrorAction SilentlyContinue
    $env:NEPHELE_DATA_DIR = $dataDir
    Remove-Item Env:NEPHELE_PROFILE -ErrorAction SilentlyContinue

    $quotedBlocksDir = '"{0}"' -f $blocksDir.Replace('"', '\"')
    $argumentLine = "play --seed 12345 --assets-dir $quotedBlocksDir"
    $process = Start-Process `
        -FilePath $binary `
        -ArgumentList $argumentLine `
        -WorkingDirectory $repoRoot `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -PassThru

    $exited = $process.WaitForExit($DurationSeconds * 1000)
    if (-not $exited) {
        $process.Refresh()
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            if (-not ('NephelePerfWindow' -as [type])) {
                Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NephelePerfWindow {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
}
'@
            }
            [void][NephelePerfWindow]::PostMessage(
                $process.MainWindowHandle,
                0x0010,
                [IntPtr]::Zero,
                [IntPtr]::Zero
            )
            $exited = $process.WaitForExit(10000)
        }
        if (-not $exited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }

    $stderr = if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
        [System.IO.File]::ReadAllText($stderrPath)
    } else {
        ''
    }
    $logLines = @($stderr -split '\r?\n')
    $adapterLines = @($logLines | Where-Object { $_ -match '^\[perf\] adapter ' })
    $reportLines = @($logLines | Where-Object { $_ -match '^\[perf\] report ' })

    if ($adapterLines.Count -eq 0) {
        throw "No selected-adapter line was logged. Client stderr:`n$stderr"
    }
    $adapterLine = $adapterLines[0]
    if ($adapterLine -notmatch 'type=IntegratedGpu' -or
        $adapterLine -notmatch $IntegratedAdapterPattern) {
        throw "Requested a low-power adapter, but the selected device was not verified as integrated: $adapterLine"
    }
    if ($reportLines.Count -eq 0) {
        throw "No 30-second perf report was logged. Client stderr:`n$stderr"
    }
    if ($reportLines.Count -lt 2) {
        throw "Only one 30-second frame window was logged; run long enough to include warm-up and steady state."
    }

    $reportLine = $reportLines[-1]
    $elapsedMatch = [regex]::Match($reportLine, 'elapsed_s=([0-9.]+)')
    if (-not $elapsedMatch.Success -or [double]$elapsedMatch.Groups[1].Value -lt 55) {
        throw "The final perf report is not a full post-warmup measurement window: $reportLine"
    }
    $frameSamplesMatch = [regex]::Match($reportLine, 'frame_samples=(\d+)')
    if (-not $frameSamplesMatch.Success -or [int64]$frameSamplesMatch.Groups[1].Value -lt 1000) {
        throw "Perf report has too few frame samples for a defensible p99: $reportLine"
    }
    $meshSamplesMatch = [regex]::Match($reportLine, 'mesh_build_samples=(\d+)')
    if (-not $meshSamplesMatch.Success -or [int64]$meshSamplesMatch.Groups[1].Value -eq 0) {
        throw "Perf report contains no mesh-build samples: $reportLine"
    }
    $uploadsMatch = [regex]::Match($reportLine, 'chunk_uploads=(\d+)')
    if (-not $uploadsMatch.Success -or [int64]$uploadsMatch.Groups[1].Value -eq 0) {
        throw "Perf report contains no chunk uploads: $reportLine"
    }

    Write-Output ("Workload: local generated world, seed 12345, 1280x720, view distance 8 (17x17 columns), {0}s" -f $DurationSeconds)
    Write-Output "Adapter: $adapterLine"
    Write-Output "Measurement: $reportLine"

    if ($reportLine -match 'p99_budget=MISS') {
        throw 'Measured frame p99 exceeded the 60 Hz (16.667 ms) frame budget.'
    }
}
finally {
    if ($null -ne $bitmap) {
        $bitmap.Dispose()
    }
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    foreach ($name in $processEnvNames) {
        $value = $previousEnv[$name]
        if ($null -eq $value) {
            Remove-Item "Env:$name" -ErrorAction SilentlyContinue
        } else {
            [System.Environment]::SetEnvironmentVariable($name, $value, 'Process')
        }
    }
    if (Test-Path -LiteralPath $runDir -PathType Container) {
        Remove-Item -LiteralPath $runDir -Recurse -Force
    }
}
