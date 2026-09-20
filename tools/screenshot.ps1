# Captures the running nimble.exe window to a PNG. Usage: tools\screenshot.ps1 [out.png]
param([string]$Out = "shot.png")

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class W32 {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, int flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@ -ReferencedAssemblies System.Drawing

Add-Type -AssemblyName System.Drawing

$p = Get-Process nimble -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Output "nimble not running"; exit 1 }
$h = $p.MainWindowHandle
$rc = New-Object W32+RECT
[void][W32]::GetWindowRect($h, [ref]$rc)
$w = $rc.Right - $rc.Left; $hh = $rc.Bottom - $rc.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[void][W32]::PrintWindow($h, $hdc, 2) # PW_RENDERFULLCONTENT
$g.ReleaseHdc($hdc)
$g.Dispose()
$bmp.Save((Join-Path $pwd $Out), [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $Out ($w x $hh)"
