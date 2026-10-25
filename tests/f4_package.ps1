# F4 single-package QA (lite base), confined to the project: build wsm-setup.zip, install it portably under
# tools/wsm-portable-test (no PATH change), then drive ONLY the installed copy with WSM_HOME unset
# (the runtime must find itself next to wsm.exe), and uninstall it. Never touches %LOCALAPPDATA%.
param([Parameter(Mandatory = $true)][string]$Base)
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Set-Location -LiteralPath $project
$test = Join-Path $project 'tools\wsm-portable-test'
$pkg = Join-Path $test 'pkg'
$root = Join-Path $test 'root'
$log = Join-Path $project 'logs\lite\f4-package.json'
$result = [ordered]@{ time = (Get-Date).ToString('s'); steps = [ordered]@{}; status = 'RUNNING' }
function Save { $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $log -Encoding utf8 }
function Step([string]$name, [scriptblock]$body) {
    $t = [Diagnostics.Stopwatch]::StartNew()
    $ErrorActionPreference = 'Continue'   # native stderr progress lines are not errors on PS 5.1
    $out = & $body 2>&1 | Out-String
    $ErrorActionPreference = 'Stop'
    $code = $LASTEXITCODE
    $result.steps[$name] = [ordered]@{ s = [math]::Round($t.Elapsed.TotalSeconds, 1); exit = $code; out = $out.Trim() }
    Save
    if ($code -ne 0) { throw "$name failed ($code): $out" }
    $out
}
$localAppWsm = Join-Path $env:LOCALAPPDATA 'wsm'
$localBefore = Test-Path -LiteralPath $localAppWsm
$userPathBefore = [Environment]::GetEnvironmentVariable('Path', 'User')
try {
    if (Test-Path -LiteralPath $test) { throw "$test exists from an earlier run; inspect and remove it first" }
    $env:TEMP = Join-Path $project 'work\r3\tmp'; $env:TMP = $env:TEMP
    $py = Join-Path $project '.venv\Scripts\python.exe'
    Step 'package' { & $py -I src/build/package.py --home work/r3/home --bin target/round3-next/release --out work/lite/dist/wsm-setup.zip } | Out-Null
    $zip = Join-Path $project 'work\lite\dist\wsm-setup.zip'
    $result.zip = [ordered]@{ bytes = (Get-Item $zip).Length; sha256 = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() }
    New-Item -ItemType Directory -Force -Path $pkg | Out-Null
    Expand-Archive -LiteralPath $zip -DestinationPath $pkg
    Step 'install' { powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $pkg 'install.ps1') -Root $root -NoPath } | Out-Null
    Remove-Item Env:WSM_HOME -ErrorAction SilentlyContinue
    $wsm = Join-Path $root 'wsm.exe'
    # Recorded, not gating: doctor also fails on host conditions (e.g. low free disk) unrelated to the package.
    $result.doctor = (& $wsm doctor 2>&1 | Out-String).Trim()
    Save
    if ($result.doctor -notmatch [regex]::Escape($root)) { throw 'installed runtime did not locate itself' }
    Step 'import' { & $wsm base import $Base } | Out-Null
    Step 'create' { & $wsm --name p7 create } | Out-Null
    Step 'first_boot' { & $wsm --name p7 start } | Out-Null
    $result.os = Step 'exec' { & $wsm --name p7 exec -- sw_vers -productVersion }
    Step 'root' { & $wsm --name p7 exec --root -- id -u }
    Step 'libffi_link' { & $wsm --name p7 exec -- 'ls /usr/lib/libffi.dylib; ls /usr/local/libexec/wsm-lite-local.sh' } | Out-Null
    Step 'detach' { & $wsm --name p7 run --detach -- sleep 300 } | Out-Null
    Step 'jobs' { & $wsm --name p7 jobs } | Out-Null
    Step 'suspend' { & $wsm --name p7 stop } | Out-Null
    Step 'resume' { & $wsm --name p7 start } | Out-Null
    Step 'exec_after_resume' { & $wsm --name p7 exec -- true } | Out-Null
    Step 'jobs_after_resume' { & $wsm --name p7 jobs } | Out-Null
    Step 'kill_job' { $id = ((& $wsm --name p7 jobs | Select-Object -First 1) -split '\s+')[0]; & $wsm --name p7 kill $id } | Out-Null
    Step 'shutdown' { & $wsm --name p7 stop --shutdown } | Out-Null
    Step 'delete' { & $wsm --name p7 delete } | Out-Null
    Step 'uninstall' { & $wsm uninstall --yes --portable } | Out-Null
    $t = [Diagnostics.Stopwatch]::StartNew()
    while ((Test-Path -LiteralPath $root) -and $t.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 500 }
    if (Test-Path -LiteralPath $root) { throw 'portable runtime still present 60 s after uninstall' }
    if ((Test-Path -LiteralPath $localAppWsm) -ne $localBefore) { throw '%LOCALAPPDATA%\wsm changed' }
    if ([Environment]::GetEnvironmentVariable('Path', 'User') -ne $userPathBefore) { throw 'user PATH changed' }
    if (-not (Test-Path -LiteralPath $Base)) { throw 'uninstall removed the source base (hard link handling)' }
    $result.status = 'PASS'
} catch {
    $result.status = 'FAIL'; $result.error = "$_"
    throw
} finally {
    Save
    Write-Output $log
}

