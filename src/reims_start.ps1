# Start aljus7's default VM. usage: reims_start.ps1 [-WithoutInstaller] [-Window]   extra env (REIMS_VGPU_*) is inherited.
param([switch]$WithoutInstaller, [switch]$Window)
$env:REIMS_DEBUG = '1'
if (!$Window) { $a = @{ Headless = $true } } else { $a = @{} }
if ($WithoutInstaller) { $a.WithoutInstaller = $true }
& D:\MyProjects\wsm\work\reims\Start-VM.ps1 @a 2>&1 | Select-Object -Last 3
(Get-ChildItem D:\MyProjects\wsm\work\reims\vm\logs | Sort-Object Name | Select-Object -Last 1).FullName
