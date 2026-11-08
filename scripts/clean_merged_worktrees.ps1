# Remove worktrees whose branch is already merged into main, and report the
# space that merged, unmerged and dirty worktrees are each holding.
#
# A worktree is safe to drop when its branch is merged and its tree is clean:
# the branch ref and every commit stay in the repository, and nothing on disk is
# unique. Recreating one later is `git worktree add`, and the only real cost is
# a rebuild of target\.
#
# Usage:
#   scripts\clean_merged_worktrees.ps1           # dry run, prints the plan
#   scripts\clean_merged_worktrees.ps1 -Force    # actually remove them

param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$worktrees = & git -C $root worktree list --porcelain |
    Select-String '^worktree ' |
    ForEach-Object { $_.Line.Substring(9) } |
    Where-Object { $_ -ne $root.Replace('\', '/') }

$removable = 0.0
$kept = 0.0

foreach ($path in $worktrees) {
    $dir = $path -replace '/', '\'
    if (-not (Test-Path $dir)) {
        Write-Host "stale   $dir (directory is gone; run git worktree prune)"
        continue
    }

    $size = 0.0
    $target = Join-Path $dir "target"
    if (Test-Path $target) {
        $size = (Get-ChildItem $target -Recurse -File -ErrorAction SilentlyContinue |
            Measure-Object Length -Sum).Sum / 1GB
    }

    $branch = (& git -C $dir rev-parse --abbrev-ref HEAD) 2>$null
    $dirty = @(& git -C $dir status --porcelain 2>$null).Count
    $merged = $false
    if ($branch -and $branch -ne "HEAD") {
        & git -C $root merge-base --is-ancestor $branch main 2>$null
        $merged = $LASTEXITCODE -eq 0
    }

    $name = Split-Path $dir -Leaf
    if ($merged -and $dirty -eq 0) {
        $removable += $size
        if ($Force) {
            & git -C $root worktree remove $dir
            if ($LASTEXITCODE -eq 0) {
                Write-Host ("removed {0,-24} {1,6:N2} GB ({2})" -f $name, $size, $branch)
            } else {
                Write-Host ("FAILED  {0,-24} is something using it?" -f $name)
                $removable -= $size
            }
        } else {
            Write-Host ("would remove {0,-24} {1,6:N2} GB ({2}, merged)" -f $name, $size, $branch)
        }
    } else {
        $kept += $size
        $reason = if ($dirty -gt 0) { "$dirty uncommitted files" } elseif (-not $merged) { "not merged" } else { "detached HEAD" }
        Write-Host ("keep    {0,-24} {1,6:N2} GB ({2})" -f $name, $size, $reason)
    }
}

Write-Host ""
Write-Host ("reclaimable from merged worktrees: {0:N1} GB; held by work in progress: {1:N1} GB" -f $removable, $kept)
if (-not $Force -and $removable -gt 0) {
    Write-Host "Run again with -Force to remove them."
}

$scratch = Join-Path $env:TEMP "opencode"
if (Test-Path $scratch) {
    $temp = (Get-ChildItem $scratch -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "tgt-*" } |
        ForEach-Object {
            $_.Name + " " + [math]::Round(((Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue |
                Measure-Object Length -Sum).Sum / 1GB), 2) + " GB"
        }) -join ", "
    if ($temp) {
        Write-Host "per-agent CARGO_TARGET_DIR scratch in $scratch : $temp"
        Write-Host "Those are build output only; delete once the owning lane has reported."
    }
}
