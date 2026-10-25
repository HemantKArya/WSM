# Build dist\wsm-setup.zip + SHA256SUMS from the current build and (optionally) create a draft GitHub release.
param([Parameter(Mandatory = $true)][string]$Version, [switch]$Publish,
      [string]$Home_ = 'work\r3\home', [string]$Bin = 'target\round3-next\release')
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Set-Location -LiteralPath $project
$dist = Join-Path $project 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$allowed = 'wsm-setup.zip', 'wsm-setup.zip.sha256', 'SHA256SUMS'
$extra = Get-ChildItem -LiteralPath $dist -File | Where-Object { $allowed -notcontains $_.Name }
if ($extra) { throw "dist\ has unexpected files (bases/ISOs must never be released): $($extra.Name -join ', ')" }
$py = Join-Path $project '.venv\Scripts\python.exe'
& $py -I src/build/package.py --home $Home_ --bin $Bin --out (Join-Path $dist 'wsm-setup.zip') | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'package.py failed' }
$hash = (Get-FileHash (Join-Path $dist 'wsm-setup.zip') -Algorithm SHA256).Hash.ToLower()
"$hash  wsm-setup.zip" | Set-Content -LiteralPath (Join-Path $dist 'SHA256SUMS') -Encoding ascii
$notes = "docs\releases\v$Version.md"
if (-not (Test-Path $notes)) { throw "missing $notes" }
$cmd = "gh release create v$Version dist\wsm-setup.zip dist\SHA256SUMS --draft --title `"WSM v$Version`" --notes-file $notes"
if ($Publish) { Invoke-Expression $cmd } else { Write-Output "Next: $cmd" }
