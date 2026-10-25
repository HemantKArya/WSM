# Rebuild aljus7's OpenCore.img after editing its config.plist (validates with ocvalidate).
$env:REIMS_PROJECT_ROOT_OVERRIDE='D:\MyProjects\wsm\work\reims'; $env:REIMS_MSYS2_ROOT='D:\MyProjects\wsm\work\reims\runtime\msys2'
$env:PYTHONHOME='D:\MyProjects\wsm\work\reims\runtime\msys2\ucrt64'
$env:PATH = "D:\MyProjects\wsm\work\reims\runtime\msys2\ucrt64\bin;D:\MyProjects\wsm\work\reims\build\qemu;" + $env:PATH
& D:\MyProjects\wsm\work\reims\scripts\windows\prepare-opencore.ps1 -Version 1.0.8 -FirmwareDirectory D:\MyProjects\wsm\work\reims\runtime\msys2\ucrt64\share\qemu -Rebuild | Select-Object -First 3
Get-Content D:\MyProjects\wsm\work\reims\vm\firmware\opencore-1.0.8\ocvalidate.log | Select-Object -Last 3
