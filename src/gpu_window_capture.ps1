param([int]$VmProcessId, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WsmGpuWindow {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
}
'@
$vmWindow = Get-Process -Id $VmProcessId
if ($vmWindow.MainWindowHandle -eq [IntPtr]::Zero) { throw 'VM has no native window' }
[void][WsmGpuWindow]::ShowWindow($vmWindow.MainWindowHandle, 3)
[void][WsmGpuWindow]::SetForegroundWindow($vmWindow.MainWindowHandle)
[void][WsmGpuWindow]::SetWindowPos($vmWindow.MainWindowHandle, [IntPtr](-1), 0, 0, 0, 0, 0x13)
try {
    Start-Sleep -Milliseconds 700
    & "$PSScriptRoot/../tools/gpu-followup/take_screenshot.ps1" -WindowHandle $vmWindow.MainWindowHandle.ToInt32() -Path $OutputPath
} finally {
    [void][WsmGpuWindow]::SetWindowPos($vmWindow.MainWindowHandle, [IntPtr](-2), 0, 0, 0, 0, 0x13)
}
