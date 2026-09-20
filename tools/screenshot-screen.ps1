# Screen-captures the region around the nimble window (includes popups/menus,
# unlike PrintWindow). Usage: tools\screenshot-screen.ps1 [out.png]
param([string]$Out = "screen.png", [int]$ExtraW = 320, [int]$ExtraH = 360)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class W32S {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@ -ReferencedAssemblies System.Drawing

Add-Type -AssemblyName System.Drawing

$p = Get-Process nimble -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Output "nimble not running"; exit 1 }
$rc = New-Object W32S+RECT
[void][W32S]::GetWindowRect($p.MainWindowHandle, [ref]$rc)
$w = [Math]::Min($rc.Right - $rc.Left + $ExtraW, 1500)
$h = [Math]::Min($rc.Bottom - $rc.Top + $ExtraH, 1100)
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($rc.Left, $rc.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
$g.Dispose()
$bmp.Save((Join-Path $pwd $Out), [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $Out"
