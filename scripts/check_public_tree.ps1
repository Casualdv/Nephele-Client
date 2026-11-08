<#
.SYNOPSIS
    Fails if anything Mojang-derived is present in the public tree.

.DESCRIPTION
    Nephele is an original client: the Minecraft EULA lets us distribute a Mod
    (our own code) but not the game, its assets, or altered game files.

    This script is the gate that keeps that true.  It is deliberately dumb and
    deliberately paranoid: it looks at names, extensions and the first bytes of
    every file, so a renamed client.jar is still caught.

    Run it before every release, and in CI.  Exit code 1 means "do not publish".

    What it scans by default:
      - if the tree is a git checkout, exactly the files git tracks (those are
        what a clone or a source archive would publish);
      - otherwise the working tree, minus the local caches that .gitignore
        already excludes (target, assets/vanilla, references, .local).

.PARAMETER Root
    Tree to scan. Defaults to the repository root.

.PARAMETER WorkingTree
    Force a working-tree scan even inside a git checkout.

.PARAMETER Quiet
    Only print problems.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\check_public_tree.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\scripts\check_public_tree.ps1 -WorkingTree
#>
[CmdletBinding()]
param(
    [string]$Root = "",
    [switch]$WorkingTree,
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"

if (-not $Root) {
    $Root = Split-Path -Parent $PSScriptRoot
}
$Root = (Resolve-Path -LiteralPath $Root).Path

# Path fragments that never belong in a published tree.  These are checked in
# both modes, so a file that was force-added with `git add -f` is still caught.
$deniedPaths = @(
    "references",
    "versions",
    "libraries",
    ".local",
    "assets\vanilla",
    "assets/vanilla",
    "assets\objects",
    "assets/objects",
    "assets\indexes",
    "assets/indexes"
)

# Extensions Mojang ships, or the game produces.
$deniedExtensions = @(
    ".jar",
    ".class",
    ".exe",
    ".msi",
    ".dll",
    ".mca",
    ".mcr",
    ".dat",
    ".dat_old"
)

# Local caches that .gitignore already excludes: skipped by a working-tree scan
# so a normal development checkout still passes. `dist` is the release bundle
# written by scripts/package_windows.ps1: generated, gitignored, and never
# published from the source tree (the zip is uploaded separately).
$localCaches = @(
    "target",
    ".git",
    "node_modules",
    "bin",
    "obj",
    "dist",
    "references",
    ".local",
    "assets\vanilla",
    "assets/vanilla",
    "assets\objects",
    "assets/objects",
    "assets\indexes",
    "assets/indexes"
)

# Extensions cheap to read as text, so the zip sniff is skipped for them.
$textExtensions = @(
    ".rs", ".toml", ".lock", ".md", ".txt", ".ps1", ".bat", ".cmd", ".sh",
    ".wgsl", ".json", ".yml", ".yaml", ".gitignore", ".gitkeep", ".cfg",
    ".editorconfig", ".gitattributes"
)

# Compile-time file embedding: the only way an asset could end up inside the
# binary.  Our shaders are fine; anything pointing at assets is not.
$embedPattern = 'include_(?:bytes|str)!\s*\([^)]*(?:assets[\\/]|\.png|\.lang|\.json|splashes)'

$hasGit = (Test-Path -LiteralPath (Join-Path $Root ".git")) -and
    [bool](Get-Command git -ErrorAction SilentlyContinue)
$useTracked = $hasGit -and -not $WorkingTree

function Get-FilesToScan {
    if ($useTracked) {
        $paths = & git -C $Root ls-files
        if ($LASTEXITCODE -ne 0) {
            throw "git ls-files failed; is '$Root' a git checkout?"
        }
        foreach ($relative in $paths) {
            if (-not $relative) { continue }
            $full = Join-Path $Root $relative
            if (Test-Path -LiteralPath $full -PathType Leaf) {
                [PSCustomObject]@{ FullName = $full; Relative = ($relative -replace '/', '\') }
            }
        }
        return
    }

    Get-ChildItem -LiteralPath $Root -Recurse -File -Force -ErrorAction SilentlyContinue |
        Where-Object {
            $relative = $_.FullName.Substring($Root.Length).TrimStart('\', '/')
            foreach ($cache in $localCaches) {
                if ($relative.StartsWith($cache, [System.StringComparison]::OrdinalIgnoreCase)) {
                    return $false
                }
            }
            $true
        } |
        ForEach-Object {
            [PSCustomObject]@{
                FullName = $_.FullName
                Relative = $_.FullName.Substring($Root.Length).TrimStart('\', '/')
            }
        }
}

function Test-ZipMagic {
    param([string]$Path)
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            if ($stream.Length -lt 4) { return $false }
            $buffer = New-Object byte[] 4
            if ($stream.Read($buffer, 0, 4) -lt 4) { return $false }
            # "PK\x03\x04": a zip, and every Minecraft jar/asset bundle is a zip.
            return ($buffer[0] -eq 0x50 -and $buffer[1] -eq 0x4B -and
                $buffer[2] -eq 0x03 -and $buffer[3] -eq 0x04)
        }
        finally {
            $stream.Dispose()
        }
    }
    catch {
        return $false
    }
}

$violations = @{}
$warnings = New-Object System.Collections.Generic.List[string]

function Add-Violation {
    param([string]$Reason, [string]$Path)
    if (-not $violations.ContainsKey($Reason)) {
        $violations[$Reason] = New-Object System.Collections.Generic.List[string]
    }
    $violations[$Reason].Add($Path)
}

if (-not $Quiet) {
    Write-Host ("check_public_tree: scanning {0}" -f $Root)
    Write-Host ("  mode: {0}" -f $(if ($useTracked) {
                "git-tracked files (what a clone would publish)"
            }
            else {
                "working tree, local caches skipped"
            }))
}

foreach ($file in Get-FilesToScan) {
    $relative = $file.Relative
    $lowerRelative = $relative.ToLowerInvariant()
    $extension = [System.IO.Path]::GetExtension($relative).ToLowerInvariant()

    if ($deniedExtensions -contains $extension) {
        Add-Violation "Mojang artifact by extension ($extension)" $relative
        continue
    }

    $deniedPath = $deniedPaths | Where-Object { $lowerRelative.StartsWith($_) } | Select-Object -First 1
    if ($deniedPath) {
        Add-Violation "inside '$deniedPath', which never ships" $relative
        continue
    }

    if (($textExtensions -notcontains $extension) -and (Test-ZipMagic $file.FullName)) {
        Add-Violation "zip/jar content, whatever the file is called" $relative
        continue
    }

    if ($extension -eq ".rs" -or $extension -eq ".toml") {
        $text = Get-Content -LiteralPath $file.FullName -Raw -ErrorAction SilentlyContinue
        if ($text -and ($text -match $embedPattern)) {
            Add-Violation "embeds an asset into the binary at compile time" $relative
            continue
        }
    }

    $size = (Get-Item -LiteralPath $file.FullName).Length
    if ($size -gt 2MB) {
        $warnings.Add(("{0}  ({1:N1} MB, confirm it is ours)" -f $relative, ($size / 1MB)))
    }
}

foreach ($warning in $warnings) {
    Write-Host ("  WARN  {0}" -f $warning) -ForegroundColor Yellow
}

$total = 0
foreach ($reason in ($violations.Keys | Sort-Object)) {
    $paths = $violations[$reason]
    $total += $paths.Count
    Write-Host ("  FAIL  {0}  [{1} file(s)]" -f $reason, $paths.Count) -ForegroundColor Red
    foreach ($path in ($paths | Select-Object -First 5)) {
        Write-Host ("          {0}" -f $path) -ForegroundColor Red
    }
    if ($paths.Count -gt 5) {
        Write-Host ("          ... and {0} more" -f ($paths.Count - 5)) -ForegroundColor Red
    }
}

if ($total -gt 0) {
    Write-Host ""
    Write-Host ("{0} violation(s). Nothing Mojang ships may be committed or published." -f $total) -ForegroundColor Red
    Write-Host "Fix: delete the file, or move it outside this tree (Nephele-private/)."
    exit 1
}

if (-not $Quiet) {
    Write-Host "  OK    no Mojang artifacts found." -ForegroundColor Green
}
exit 0
