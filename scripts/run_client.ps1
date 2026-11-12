# Launch the client for one development session, with a machine-wide lock.
#
# Why this exists: several sessions share one machine. Worktrees keep the source
# apart, but a running client is a single resource - it steals focus, it locks
# target\release\nephele.exe so the next build fails, and it invalidates any
# performance measurement happening at the same time.
#
# Usage:
#   .\scripts\run_client.ps1 -Session m3 -Server mc.hypixel.net
#   .\scripts\run_client.ps1 -Session m6 -Local
#   .\scripts\run_client.ps1 -Session m3 -Server 127.0.0.1:25565 -Offline
#   .\scripts\run_client.ps1 -Session m3 -DryRun
#
# Each session gets its own NEPHELE_DATA_DIR, so worlds, saves and the auth
# session never cross between sessions.
param(
    [Parameter(Mandatory = $true)][string]$Session,
    [string]$Server = "mc.hypixel.net",
    [switch]$Local,
    [switch]$Offline,
    [switch]$Trace,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$binary = Join-Path $repoRoot 'target\release\nephele.exe'

if (-not (Test-Path $binary)) {
    throw "nephele.exe is not built. Run: cargo build --release -p nephele"
}

# One client at a time. The lock records who holds it and when, so a stale lock
# from a crashed session can be recognised instead of blocking everyone forever.
$lockPath = Join-Path $env:TEMP 'nephele-client.lock'
if (Test-Path $lockPath) {
    $held = Get-Content $lockPath -Raw | ConvertFrom-Json
    $owner = Get-Process -Id $held.pid -ErrorAction SilentlyContinue
    if ($owner -and $owner.ProcessName -eq 'nephele') {
        throw ("The client is already running for session '{0}' (pid {1}, since {2}). " -f
            $held.session, $held.pid, $held.started) +
            "Ask that session to finish, or close the window."
    }
    Write-Warning "Clearing a stale client lock left by session '$($held.session)'."
    Remove-Item $lockPath -Force
}

# Separate data per session: saves, settings and auth must never be shared.
$dataDir = Join-Path $env:TEMP "nephele-dev\$Session"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null

if ($Trace) {
    $env:NEPHELE_TRACE_INPUT = '1'
}

$arguments = @('play')
if ($Local) {
    $arguments += '--seed'
    $arguments += '12345'
    # `play --seed` only stores the seed for the world-create screen; the
    # single-player world is entered through NEPHELE_SCREEN=local.
    $env:NEPHELE_SCREEN = 'local'
}
else {
    $arguments += $Server
    if (-not $Offline) {
        $arguments += '--online'
    }
}

$env:NEPHELE_DATA_DIR = $dataDir

if ($DryRun) {
    "session : $Session"
    "data    : $dataDir"
    "command : $binary $($arguments -join ' ')"
    return
}

$record = [pscustomobject]@{
    session = $Session
    pid     = $PID
    started = (Get-Date).ToString('s')
}
$record | ConvertTo-Json | Set-Content -Path $lockPath -Encoding utf8

try {
    & $binary @arguments
}
finally {
    Remove-Item $lockPath -Force -ErrorAction SilentlyContinue
}
