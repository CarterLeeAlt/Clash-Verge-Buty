# Regenerates the full-body Watchdog icon by recoloring the complete blue cat
# (icons/128x128.png) with the original coral-orange palette of the stock
# system-proxy tray icon (icons/tray-icon-sys.png). The tray icon itself is
# never modified. Mapping is a per-channel linear RGB transform anchored at
# the dominant body colors, so gradients match the stock shading.
$ErrorActionPreference = 'Stop'
$icons = Join-Path $PSScriptRoot '..\src-tauri\icons'
Add-Type -AssemblyName System.Drawing

function Get-DominantColor([Drawing.Bitmap]$bitmap) {
    $counts = @{}
    for ($y = 0; $y -lt $bitmap.Height; $y++) {
        for ($x = 0; $x -lt $bitmap.Width; $x++) {
            $pixel = $bitmap.GetPixel($x, $y)
            if ($pixel.A -gt 200) {
                $key = '{0:X2}{1:X2}{2:X2}' -f $pixel.R, $pixel.G, $pixel.B
                $counts[$key] = [int]$counts[$key] + 1
            }
        }
    }
    $best = $counts.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1
    return [Drawing.Color]::FromArgb(255,
        [Convert]::ToInt32($best.Key.Substring(0, 2), 16),
        [Convert]::ToInt32($best.Key.Substring(2, 2), 16),
        [Convert]::ToInt32($best.Key.Substring(4, 2), 16))
}

$source = [Drawing.Bitmap]::FromFile((Join-Path $icons '128x128.png'))
$palette = [Drawing.Bitmap]::FromFile((Join-Path $icons 'tray-icon-sys.png'))
try {
    $baseColor = Get-DominantColor $source
    $targetColor = Get-DominantColor $palette
    $scaleR = [Math]::Min(4.0, $targetColor.R / [Math]::Max(1.0, [double]$baseColor.R))
    $scaleG = [Math]::Min(4.0, $targetColor.G / [Math]::Max(1.0, [double]$baseColor.G))
    $scaleB = [Math]::Min(4.0, $targetColor.B / [Math]::Max(1.0, [double]$baseColor.B))
    Write-Output ("base=#{0:X2}{1:X2}{2:X2} target=#{3:X2}{4:X2}{5:X2} scales=({6:F3},{7:F3},{8:F3})" -f `
        $baseColor.R, $baseColor.G, $baseColor.B, $targetColor.R, $targetColor.G, $targetColor.B, $scaleR, $scaleG, $scaleB)

    $yellow = [Drawing.Bitmap]::new($source.Width, $source.Height)
    for ($y = 0; $y -lt $source.Height; $y++) {
        for ($x = 0; $x -lt $source.Width; $x++) {
            $pixel = $source.GetPixel($x, $y)
            if ($pixel.A -eq 0) { continue }
            $maxc = [Math]::Max($pixel.R, [Math]::Max($pixel.G, $pixel.B))
            $minc = [Math]::Min($pixel.R, [Math]::Min($pixel.G, $pixel.B))
            if ($maxc -eq 0 -or ($maxc - $minc) / $maxc -lt 0.12) {
                $yellow.SetPixel($x, $y, $pixel)
                continue
            }
            $yellow.SetPixel($x, $y, [Drawing.Color]::FromArgb($pixel.A,
                [Math]::Min(255, [int][Math]::Round($pixel.R * $scaleR)),
                [Math]::Min(255, [int][Math]::Round($pixel.G * $scaleG)),
                [Math]::Min(255, [int][Math]::Round($pixel.B * $scaleB))))
        }
    }
    $yellow.Save((Join-Path $icons 'watchdog.png'), [Drawing.Imaging.ImageFormat]::Png)

    $images = @()
    foreach ($size in 16, 32, 48, 64, 128, 256) {
        $bitmap = [Drawing.Bitmap]::new($size, $size)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.DrawImage($yellow, 0, 0, $size, $size)
        $graphics.Dispose()
        $stream = [IO.MemoryStream]::new()
        $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
        $images += [pscustomobject]@{ Size = $size; Bytes = $stream.ToArray() }
        $stream.Dispose()
        $bitmap.Dispose()
    }
    $output = [IO.File]::Create((Join-Path $icons 'watchdog.ico'))
    $writer = [IO.BinaryWriter]::new($output)
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$images.Count)
        $offset = 6 + 16 * $images.Count
        foreach ($image in $images) {
            $dimension = if ($image.Size -eq 256) { 0 } else { $image.Size }
            $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
            $writer.Write([byte]0); $writer.Write([byte]0)
            $writer.Write([uint16]1); $writer.Write([uint16]32)
            $writer.Write([uint32]$image.Bytes.Length); $writer.Write([uint32]$offset)
            $offset += $image.Bytes.Length
        }
        foreach ($image in $images) { $writer.Write([byte[]]$image.Bytes) }
    } finally { $writer.Dispose(); $output.Dispose() }
    Write-Output 'Generated full-body icon with the original stock orange palette.'
} finally {
    $source.Dispose(); $palette.Dispose(); $yellow.Dispose()
}
