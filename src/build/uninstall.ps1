param([Parameter(Mandatory=$true)][string]$Runtime, [Parameter(Mandatory=$true)][int]$LauncherPid)
$ErrorActionPreference = 'Stop'
$target = [IO.Path]::GetFullPath($Runtime)
$scriptPath = [IO.Path]::GetFullPath($PSCommandPath)
try {
    if (-not (Test-Path -LiteralPath (Join-Path $target 'package-manifest.json'))) { throw 'Runtime package marker is missing' }
    Wait-Process -Id $LauncherPid -Timeout 30 -ErrorAction SilentlyContinue
    if (Get-Process -Id $LauncherPid -ErrorAction SilentlyContinue) { throw 'Uninstall launcher did not exit' }
    # The CLI validated this exact runtime; keep the native file operation in this shell.
    Remove-Item -LiteralPath $target -Recurse -Force
    Remove-Item -LiteralPath $scriptPath -Force
} catch {
    $_ | Out-File -LiteralPath ($scriptPath + '.log')
    exit 1
}
