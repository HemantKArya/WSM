# Run from the extracted wsm-setup.zip. -Root and -NoPath support isolated acceptance tests.
param([string]$Root = (Join-Path $env:LOCALAPPDATA 'wsm'), [switch]$NoPath)
$ErrorActionPreference = 'Stop'
$source = [IO.Path]::GetFullPath($PSScriptRoot)
$destination = [IO.Path]::GetFullPath($Root)
if ($source -eq $destination) { throw 'Extract the package to a separate directory before installing.' }
$manifest = Get-Content -LiteralPath (Join-Path $source 'package-manifest.json') -Raw | ConvertFrom-Json
foreach ($entry in $manifest.files.PSObject.Properties) {
    $inputFile = [IO.Path]::GetFullPath((Join-Path $source $entry.Name))
    $outputFile = [IO.Path]::GetFullPath((Join-Path $destination $entry.Name))
    if (-not $inputFile.StartsWith($source + '\', [StringComparison]::OrdinalIgnoreCase) -or
        -not $outputFile.StartsWith($destination + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Invalid package path: $($entry.Name)"
    }
    if ((Get-FileHash -LiteralPath $inputFile -Algorithm SHA256).Hash -ne $entry.Value.sha256) {
        throw "Package checksum failed: $($entry.Name)"
    }
}
foreach ($entry in $manifest.files.PSObject.Properties) {
    $outputFile = Join-Path $destination $entry.Name
    New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($outputFile)) | Out-Null
    Copy-Item -LiteralPath (Join-Path $source $entry.Name) -Destination $outputFile -Force
}
Copy-Item -LiteralPath (Join-Path $source 'package-manifest.json') -Destination $destination -Force
if (-not $NoPath) {
    [string]$oldPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($oldPath -split ';') -notcontains $destination) {
        [Environment]::SetEnvironmentVariable('Path', ($oldPath.TrimEnd(';') + ';' + $destination).TrimStart(';'), 'User')
    }
}
Write-Output "Installed WSM $($manifest.version) to $destination."
Write-Output 'The macOS base is separate. Use wsm base import FILE --sha256 HASH or wsm base build --iso FILE.'
if ($NoPath) { Write-Output 'PATH was not changed. Set WSM_HOME to this directory for isolated testing.' }
