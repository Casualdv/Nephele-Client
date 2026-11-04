param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"
$parent = Split-Path -Parent $OutputPath
if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
    throw "Icon output folder does not exist: $parent"
}

Add-Type -AssemblyName System.Drawing
if (-not ("NepheleIconNative" -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NepheleIconNative {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool DestroyIcon(IntPtr handle);
}
'@
}
$bitmap = New-Object System.Drawing.Bitmap 64, 64
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.Color]::FromArgb(10, 16, 27))

$accent = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(250, 171, 79))
$mint = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(92, 212, 194))
$graphics.FillRectangle($accent, 8, 18, 16, 16)
$graphics.FillRectangle($mint, 24, 10, 16, 24)
$graphics.FillRectangle($accent, 40, 18, 16, 16)
$graphics.FillRectangle($mint, 16, 34, 32, 6)

$handle = $bitmap.GetHicon()
$icon = [System.Drawing.Icon]::FromHandle($handle)
$stream = [System.IO.File]::Create($OutputPath)
try {
    $icon.Save($stream)
}
finally {
    $stream.Dispose()
    $icon.Dispose()
    $graphics.Dispose()
    $bitmap.Dispose()
    [void][NepheleIconNative]::DestroyIcon($handle)
}
