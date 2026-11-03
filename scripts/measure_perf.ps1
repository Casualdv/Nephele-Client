<#
.SYNOPSIS
    Measure Nephele's per-window frame budget and startup stages on integrated graphics.

.DESCRIPTION
    Builds the release CLI, then runs a deterministic local world with generated
    flat-colour placeholder textures (no game assets). The client requests a
    low-power adapter and logs its actual adapter name/type.

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
    [ValidateRange(30, 3600)]
    [int]$DurationSeconds = 75,

    [ValidateRange(2, 16)]
    [int]$ViewDistance = 8,

    [string]$IntegratedAdapterPattern = 'Radeon|Iris|UHD|Vega|Arc',

    [string]$BaselinePath = 'scripts/perf-baseline.json',

    [double]$MaxP99RegressionPct = 10.0,

    [int]$MaxOverBudgetRegression = 2,

    [string]$OutputPath,

    [string]$TargetDir = $env:CARGO_TARGET_DIR,

    [switch]$StartupOnly,

    [switch]$UpdateBaseline,

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
    $env:NEPHELE_VIEW_DISTANCE = [string]$ViewDistance
    $env:NEPHELE_VSYNC = '0'
    $env:NEPHELE_OCCLUSION = '1'
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
    if ($jsonLines.Count -lt 1) {
        throw "No 30-second frame window was logged."
    }
    if (-not $StartupOnly -and $jsonLines.Count -lt 2) {
        throw "Only one 30-second frame window was logged; run long enough to include startup and steady state, or pass -StartupOnly."
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
    if (-not $StartupOnly -and $last.elapsed_s -lt 55) {
        throw "The final perf report is not a full post-warmup measurement window: $($last | ConvertTo-Json -Compress)"
    }
    if ($first.frame_samples -lt 1000) {
        throw "The startup window has too few frame samples for a defensible p99."
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
    Write-Host ("Startup window: frame p50 {0} ms, p99 {1} ms, over-100ms {2}" -f
        (Format-Ms $first.startup_p50_ms),
        (Format-Ms $first.startup_p99_ms),
        $first.frame_over_100ms)
    Write-Host 'Hardware gap: this run is valid only for the adapter above. Intel HD-class is UNVERIFIED; the 60 fps product bar is not proven by this measurement.'
    Write-Host ''
    Write-Host 'Windows:'
    Write-Host ('{0,6} {1,10} {2,9} {3,12} {4,12} {5,10} {6,9} {7,12} {8,12} {9,9} {10,13} {11,10} {12,8} {13,8} {14,10} {15,9}' -f
        'window', 'elapsed_s', 'frames', 'frame_p50_ms', 'frame_p99_ms', 'over_100ms',
        'mesh_n', 'mesh_p50_ms', 'mesh_p99_ms', 'uploads', 'upload_total_ms', 'p99',
        'chunks', 'meshes', 'vram_mb', 'rss_mb')
    foreach ($window in $windows) {
        $memory = $window.memory
        Write-Host ('{0,6} {1,10:F1} {2,9} {3,12} {4,12} {5,10} {6,9} {7,12} {8,12} {9,9} {10,13:F3} {11,10} {12,8} {13,8} {14,10:F2} {15,9:F1}' -f
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
            (Format-Budget $window.p99_within_budget),
            $memory.chunks,
            $memory.meshes,
            ([double]$memory.vram_estimate_bytes / 1MB),
            ([double]$memory.rss_bytes / 1MB))
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

    $baselineFullPath = if ([System.IO.Path]::IsPathRooted($BaselinePath)) {
        $BaselinePath
    } else {
        Join-Path $repoRoot $BaselinePath
    }
    $baseline = $null
    if (Test-Path -LiteralPath $baselineFullPath -PathType Leaf) {
        $baseline = [System.IO.File]::ReadAllText($baselineFullPath) | ConvertFrom-Json
    } elseif (-not $UpdateBaseline) {
        throw "No baseline at '$baselineFullPath'. Run once with -UpdateBaseline to create it."
    }

    if ($baseline -and -not $StartupOnly) {
        $baselineLast = @($baseline.windows)[-1]
        $currentLast = $last
        $p99Limit = [double]$baselineLast.frame_p99_ms * (1.0 + $MaxP99RegressionPct / 100.0)
        $overBudgetLimit = [int]$baselineLast.frame_over_budget + $MaxOverBudgetRegression
        if ([double]$currentLast.frame_p99_ms -gt $p99Limit) {
            throw ("Frame p99 regressed: {0:F3} ms > baseline {1:F3} ms + {2:F1}% (limit {3:F3} ms)." -f
                [double]$currentLast.frame_p99_ms,
                [double]$baselineLast.frame_p99_ms,
                $MaxP99RegressionPct,
                $p99Limit)
        }
        if ([int]$currentLast.frame_over_budget -gt $overBudgetLimit) {
            throw ("Over-budget frame count regressed: {0} > baseline {1} + {2} (limit {3})." -f
                $currentLast.frame_over_budget,
                $baselineLast.frame_over_budget,
                $MaxOverBudgetRegression,
                $overBudgetLimit)
        }
    }

    if ($first.p99_within_budget -eq $false -or $last.p99_within_budget -eq $false) {
        throw 'A measured window exceeded the 60 Hz frame budget; the regression gate is not satisfied.'
    }

    $payload = [ordered]@{
        schema = 'nephele-perf-baseline-v2'
        workload = [ordered]@{
            seed = 12345
            width = 1280
            height = 720
            requested_view_distance = $ViewDistance
            streamed_columns = $chunkUploads
            duration_seconds = $DurationSeconds
            screen = 'local'
            gpu = 'integrated'
            adapter = $adapterLine
            hardware_note = 'AMD integrated verified; Intel HD-class unverified; 60 fps product bar not proven by this run.'
        }
        adapter = $adapterLine
        thresholds = [ordered]@{
            max_p99_regression_pct = $MaxP99RegressionPct
            max_over_budget_regression = $MaxOverBudgetRegression
            frame_budget_ms = 16.667
        }
        startup_p50_ms = $first.startup_p50_ms
        startup_p99_ms = $first.startup_p99_ms
        windows = $windows
    }
    $jsonText = $payload | ConvertTo-Json -Depth 16

    if ($UpdateBaseline) {
        $baselineDir = Split-Path -Parent $baselineFullPath
        if ($baselineDir -and -not (Test-Path -LiteralPath $baselineDir -PathType Container)) {
            New-Item -ItemType Directory -Path $baselineDir -Force | Out-Null
        }
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($baselineFullPath, $jsonText, $utf8NoBom)
        Write-Host "Baseline updated at $baselineFullPath"
    }

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
