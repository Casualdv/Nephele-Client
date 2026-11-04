<#
.SYNOPSIS
    Measure Nephele's per-window frame budget and startup stages on integrated graphics.

.DESCRIPTION
    Builds the release CLI, then runs a deterministic local world with a generated
    flat-color placeholder texture (no game assets). The client requests a
    low-power adapter and logs its actual adapter name/type. VSync is disabled so
    the percentiles measure render headroom rather than the display refresh. The
    run must report frame and mesh p50/p99 values plus non-zero mesh/upload counts
    before it can be treated as a valid measurement. The workload uses the client
    default render distance of 8 chunks (a 17x17 column area around spawn).

    Every 30-second frame window is printed, including a bad startup window. The
    first five seconds are broken into shader compilation, texture-atlas upload,
    initial mesh builds, chunk uploads, and first-frame work. JSON output contains
    the same per-window and startup data for a committed baseline.

    Run for at least 75 seconds to collect a startup window and a steady-state
    window.

.PARAMETER DurationSeconds
    Wall-clock measurement duration. Defaults to 75 seconds; minimum is 75.

.PARAMETER ViewDistance
    Requested render distance. The client must use this value before local
    streaming starts.

.PARAMETER IntegratedAdapterPattern
    Regex required in the selected adapter line in addition to the IntegratedGpu
    device type. Defaults to common integrated adapter names.

.PARAMETER OutputPath
    Write the JSON baseline to this path. The file is UTF-8 without a BOM.

.PARAMETER TargetDir
    Cargo target directory. Defaults to CARGO_TARGET_DIR when set.

.PARAMETER Json
    Print the JSON baseline to stdout after the human-readable table.

.PARAMETER SkipBuild
    Use the existing release binary instead of building it.

.EXAMPLE
    .\scripts\measure_perf.ps1

.EXAMPLE
    .\scripts\measure_perf.ps1 -ViewDistance 6 -Json -OutputPath scripts/perf-baseline.json
#>
[CmdletBinding()]
param(
    [ValidateRange(75, 3600)]
    [int]$DurationSeconds = 75,

    [ValidateRange(2, 16)]
    [int]$ViewDistance = 8,

    [string]$IntegratedAdapterPattern = 'Radeon|Iris|UHD|Vega|Arc',

    [string]$OutputPath,

    [string]$TargetDir = $env:CARGO_TARGET_DIR,

    [switch]$Json,

    [switch]$SkipBuild,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArguments
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$jsonRequested = $Json -or ($RemainingArguments -contains '--json')

Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
. 'C:\Users\dariu\AppData\Local\Temp\opencode\cargo-env.ps1'

if (-not $SkipBuild) {
    $buildArgs = @(
        'build',
        '--manifest-path', (Join-Path $repoRoot 'Cargo.toml'),
        '-p', 'nephele',
        '--release',
        '--bin', 'nephele'
    )
    if ($TargetDir) {
        $buildArgs += @('--target-dir', $TargetDir)
    }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}

$targetRoot = if ($TargetDir) { $TargetDir } else { Join-Path $repoRoot 'target' }
$binary = Join-Path $targetRoot 'release\nephele.exe'
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
    'NEPHELE_PERF_JSON',
    'NEPHELE_GPU',
    'NEPHELE_SCREEN',
    'NEPHELE_VIEW_DISTANCE',
    'NEPHELE_DATA_DIR',
    'NEPHELE_PROFILE',
    'NEPHELE_VSYNC',
    'NEPHELE_OCCLUSION'
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
    $env:NEPHELE_PERF_JSON = '1'
    $env:NEPHELE_GPU = 'integrated'
    $env:NEPHELE_SCREEN = 'local'
    $env:NEPHELE_VSYNC = '0'
    $env:NEPHELE_OCCLUSION = '1'
    $env:NEPHELE_VIEW_DISTANCE = [string]$ViewDistance
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
    $reportLines = @($logLines | Where-Object { $_ -match '^\[perf\] window=' })
    $jsonLines = @($logLines | Where-Object { $_ -match '^\[perf-json\] ' })

    if ($adapterLines.Count -eq 0) {
        throw "No selected-adapter line was logged. Client stderr:`n$stderr"
    }
    $adapterLine = $adapterLines[0]
    if ($adapterLine -notmatch 'type=IntegratedGpu' -or
        $adapterLine -notmatch $IntegratedAdapterPattern) {
        throw "Requested a low-power adapter, but the selected device was not verified as integrated: $adapterLine"
    }
    if ($jsonLines.Count -eq 0) {
        throw "No JSON perf windows were logged. Client stderr:`n$stderr"
    }
    if ($reportLines.Count -ne $jsonLines.Count) {
        throw "The text and JSON window counts differ; refusing to publish an incomplete baseline."
    }
    if ($jsonLines.Count -lt 2) {
        throw "Only one 30-second frame window was logged; run long enough to include startup and steady state."
    }

    $windows = @()
    foreach ($line in $jsonLines) {
        $payloadText = $line -replace '^\[perf-json\]\s*', ''
        try {
            $windows += ($payloadText | ConvertFrom-Json)
        }
        catch {
            throw "Could not parse a perf JSON window: $line"
        }
    }

    $first = $windows[0]
    $last = $windows[-1]
    if ($null -eq $first.startup) {
        throw "The first window did not include startup-stage attribution."
    }
    if ($last.elapsed_s -lt 55) {
        throw "The final perf report is not a full post-warmup measurement window: $($last | ConvertTo-Json -Compress)"
    }
    if ($last.frame_samples -lt 1000) {
        throw "The final window has too few frame samples for a defensible p99."
    }
    $meshSamples = ($windows | Measure-Object -Property mesh_build_samples -Sum).Sum
    $chunkUploads = ($windows | Measure-Object -Property chunk_uploads -Sum).Sum
    if ($meshSamples -eq 0) {
        throw "The measurement reports no mesh-build samples in any window."
    }
    if ($chunkUploads -eq 0) {
        throw "The measurement reports no chunk uploads in any window."
    }

    function Format-Ms($value) {
        if ($null -eq $value) { return 'n/a' }
        return ('{0:F3}' -f [double]$value)
    }

    function Format-Budget($value) {
        if ($value -eq $true) { return 'PASS' }
        if ($value -eq $false) { return 'MISS' }
        return 'N/A'
    }

    Write-Host ("Workload: local generated world, seed 12345, 1280x720, requested view distance {0}, {1}s" -f $ViewDistance, $DurationSeconds)
    Write-Host "Adapter: $adapterLine"
    Write-Host ''
    Write-Host 'Windows:'
    Write-Host ('{0,6} {1,10} {2,9} {3,12} {4,12} {5,10} {6,9} {7,12} {8,12} {9,9} {10,13} {11,10}' -f
        'window', 'elapsed_s', 'frames', 'frame_p50_ms', 'frame_p99_ms', 'over_100ms',
        'mesh_n', 'mesh_p50_ms', 'mesh_p99_ms', 'uploads', 'upload_total_ms', 'p99')
    foreach ($window in $windows) {
        Write-Host ('{0,6} {1,10:F1} {2,9} {3,12} {4,12} {5,10} {6,9} {7,12} {8,12} {9,9} {10,13:F3} {11,10}' -f
            $window.window,
            [double]$window.elapsed_s,
            $window.frame_samples,
            (Format-Ms $window.frame_p50_ms),
            (Format-Ms $window.frame_p99_ms),
            $window.frame_over_100ms,
            $window.mesh_build_samples,
            (Format-Ms $window.mesh_build_p50_ms),
            (Format-Ms $window.mesh_build_p99_ms),
            $window.chunk_uploads,
            [double]$window.chunk_upload_total_ms,
            (Format-Budget $window.p99_within_budget))
    }

    Write-Host ''
    Write-Host 'Startup (first five seconds):'
    Write-Host ('{0,-30} {1,8} {2,12} {3,12} {4,18} {5,12}' -f
        'stage', 'samples', 'total_ms', 'max_ms', 'long_frame_overlap_ms', 'long_frames')
    foreach ($stage in @($first.startup.stages | Sort-Object -Property long_frame_overlap_ms -Descending)) {
        Write-Host ('{0,-30} {1,8} {2,12:F3} {3,12:F3} {4,18:F3} {5,12}' -f
            $stage.name,
            $stage.samples,
            [double]$stage.total_ms,
            [double]$stage.max_ms,
            [double]$stage.long_frame_overlap_ms,
            $stage.long_frames)
    }
    Write-Host ''
    Write-Host 'Long startup frames (>100 ms):'
    foreach ($frame in @($first.startup.long_frames)) {
        $stages = if ($frame.stages.Count -gt 0) { $frame.stages -join ',' } else { '-' }
        Write-Host ('  start {0,10:F1} ms  duration {1,10:F1} ms  stages {2}' -f
            [double]$frame.start_ms,
            [double]$frame.duration_ms,
            $stages)
    }
    if ($first.p99_within_budget -eq $false) {
        Write-Host 'WARNING: the first 30-second window missed the 60 Hz frame budget; that startup stutter is retained above.'
    }

    $payload = [ordered]@{
        schema = 'nephele-perf-baseline-v1'
        workload = [ordered]@{
            seed = 12345
            width = 1280
            height = 720
            requested_view_distance = $ViewDistance
            streamed_columns = $chunkUploads
            duration_seconds = $DurationSeconds
            screen = 'local'
            gpu = 'integrated'
        }
        adapter = $adapterLine
        windows = $windows
    }
    $jsonText = $payload | ConvertTo-Json -Depth 16

    if ($OutputPath) {
        $fullOutputPath = if ([System.IO.Path]::IsPathRooted($OutputPath)) {
            $OutputPath
        } else {
            Join-Path (Get-Location) $OutputPath
        }
        $outputDir = Split-Path -Parent $fullOutputPath
        if ($outputDir -and -not (Test-Path -LiteralPath $outputDir -PathType Container)) {
            New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
        }
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($fullOutputPath, $jsonText, $utf8NoBom)
        Write-Host "Baseline written to $fullOutputPath"
    }
    if ($jsonRequested -and -not $OutputPath) {
        Write-Output $jsonText
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
