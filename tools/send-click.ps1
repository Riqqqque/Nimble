# Posts a left button down at client coords (x,y) of the nimble window.
param([int]$X = 30, [int]$Y = 13, [switch]$Up)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class W32C {
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
}
'@

$p = Get-Process nimble -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Output "nimble not running"; exit 1 }
$l = [IntPtr]($X -bor ($Y -shl 16))
[void][W32C]::PostMessageW($p.MainWindowHandle, 0x0201, [IntPtr]1, $l) # WM_LBUTTONDOWN
if ($Up) { [void][W32C]::PostMessageW($p.MainWindowHandle, 0x0202, [IntPtr]0, $l) }
Write-Output "clicked ($X,$Y)"
