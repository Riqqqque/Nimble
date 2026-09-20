Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon("$pwd\target\release\nimble.exe")
$bmp = $icon.ToBitmap()
$bmp.Save("$pwd\icon-check.png", [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "extracted $($icon.Width)x$($icon.Height) -> icon-check.png"
