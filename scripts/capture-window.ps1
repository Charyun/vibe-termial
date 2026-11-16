param([Parameter(Mandatory=$true)][int]$ClientProcessId, [Parameter(Mandatory=$true)][string]$OutputFile)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class VibeWindowCapture {
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
}
'@
[void][VibeWindowCapture]::SetThreadDpiAwarenessContext([IntPtr](-4))
$client = Get-Process -Id $ClientProcessId
$handle = $client.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) { throw 'The process has no native window.' }
$rect = New-Object VibeWindowCapture+Rect
if (-not [VibeWindowCapture]::GetWindowRect($handle, [ref]$rect)) { throw 'GetWindowRect failed.' }
$bitmap = New-Object Drawing.Bitmap ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top)
$graphics = [Drawing.Graphics]::FromImage($bitmap)
try {
 $hdc = $graphics.GetHdc()
 try {
  if (-not [VibeWindowCapture]::PrintWindow($handle, $hdc, 2)) { throw 'PrintWindow failed.' }
 } finally { $graphics.ReleaseHdc($hdc) }
 $bitmap.Save($OutputFile, [Drawing.Imaging.ImageFormat]::Png)
} finally { $graphics.Dispose(); $bitmap.Dispose() }
Write-Output $OutputFile
