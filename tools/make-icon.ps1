# Generates nimble.ico (multi-size, transparent background) from assets\nimble.png.
# Run from the repo root:  powershell -File tools\make-icon.ps1
param(
    [string]$Source = "assets\nimble.png",
    [string]$Output = "nimble.ico"
)

Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class IconTools
{
    // Set alpha=0 for pixels that are near-white (threshold on R,G,B).
    public static void KeyWhite(Bitmap b, byte thr)
    {
        var rect = new Rectangle(0, 0, b.Width, b.Height);
        var d = b.LockBits(rect, ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
        int n = Math.Abs(d.Stride) * d.Height;
        byte[] px = new byte[n];
        Marshal.Copy(d.Scan0, px, 0, n);
        for (int i = 0; i < n; i += 4)
        {
            // BGRA order
            if (px[i + 2] > thr && px[i + 1] > thr && px[i] > thr)
                px[i + 3] = 0;
        }
        Marshal.Copy(px, 0, d.Scan0, n);
        b.UnlockBits(d);
    }
}
'@ -ReferencedAssemblies System.Drawing

$src = [System.Drawing.Bitmap]::FromFile((Join-Path $pwd $Source))
$sizes = 16, 24, 32, 40, 48, 64, 128, 256
$blobs = @()

foreach ($s in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($src, 0, 0, $s, $s)
    $g.Dispose()
    [IconTools]::KeyWhite($bmp, 235)
    $mem = New-Object IO.MemoryStream
    $bmp.Save($mem, [System.Drawing.Imaging.ImageFormat]::Png)
    $blobs += , $mem.ToArray()
    $bmp.Dispose()
}
$src.Dispose()

$mem = New-Object IO.MemoryStream
$bw = New-Object IO.BinaryWriter $mem
$bw.Write([uint16]0)                 # reserved
$bw.Write([uint16]1)                 # type: icon
$bw.Write([uint16]$sizes.Count)      # image count
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $sz = $sizes[$i]
    $bw.Write([byte]($sz -band 0xFF)) # width  (256 -> 0)
    $bw.Write([byte]($sz -band 0xFF)) # height
    $bw.Write([byte]0)                # colors in palette
    $bw.Write([byte]0)                # reserved
    $bw.Write([uint16]1)              # planes
    $bw.Write([uint16]32)             # bpp
    $bw.Write([uint32]$blobs[$i].Length)
    $bw.Write([uint32]$offset)
    $offset += $blobs[$i].Length
}
foreach ($d in $blobs) { $bw.Write($d) }
[IO.File]::WriteAllBytes((Join-Path $pwd $Output), $mem.ToArray())
Write-Output "wrote $Output ($($mem.Length) bytes, sizes: $($sizes -join ', '))"
