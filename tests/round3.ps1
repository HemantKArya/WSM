# Fixed, project-confined operations for round-3 acceptance on the owner-protected VM files.
param([ValidateSet('Progress','Smoke','Acceptance','Package','Generic','Quit','ResetGeneric','InspectFirstboot','StorageCheck','DiskUsage','CleanupFailed','ArchiveInfo','CleanupBuildCache','RepackTarget','SetupDev','LaunchCheck')][string]$Action = 'Progress')
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Set-Location -LiteralPath $project
$env:WSM_HOME = Join-Path $project 'work/r3/home'
$env:WSM_BIN = Join-Path $project 'target/round3-next/release/wsm.exe'
$env:TEMP = Join-Path $project 'work/r3/tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
switch ($Action) {
    'Quit' { & $env:WSM_BIN quit }
    'LaunchCheck' {
        & (Join-Path $project '.venv/Scripts/python.exe') -I tests/p4_launcher.py
        if ($LASTEXITCODE -ne 0) { throw 'Detached launcher test failed' }
    }
    'SetupDev' {
        & (Join-Path $project '.venv/Scripts/python.exe') -I tests/setup_dev.py basecheck
        if ($LASTEXITCODE -ne 0) { throw 'Guest development-tool setup failed' }
    }
    'RepackTarget' {
        if (Get-Process qemu-system-x86_64 -ErrorAction SilentlyContinue) { throw 'A VM is running; target repack refused' }
        $imageTool = Join-Path $env:WSM_HOME 'runtime/qemu/qemu-img.exe'
        $original = [IO.Path]::GetFullPath((Join-Path $env:WSM_HOME 'build/tahoe-26.3/target.qcow2'))
        $repacked = [IO.Path]::GetFullPath((Join-Path $env:WSM_HOME 'build/tahoe-26.3/target-repacked.qcow2'))
        $backup = [IO.Path]::GetFullPath((Join-Path $env:WSM_HOME 'build/tahoe-26.3/target-before-repack.qcow2'))
        foreach ($path in @($original,$repacked,$backup)) {
            if (-not $path.StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Path outside project: $path" }
        }
        if ((Test-Path -LiteralPath $repacked) -or (Test-Path -LiteralPath $backup)) { throw 'Previous repack artifacts require inspection' }
        $info = (& $imageTool info --output=json $original) | ConvertFrom-Json
        if ($info.'backing-filename' -or $info.snapshots) { throw 'Repack only accepts the standalone builder target without internal snapshots' }
        $measure = (& $imageTool measure --output=json -O qcow2 $original) | ConvertFrom-Json
        $free = (Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='D:'").FreeSpace
        if ($free -lt ($measure.required + 2GB)) { throw 'Insufficient space for safe target repack' }
        & $imageTool convert -p -O qcow2 $original $repacked
        if ($LASTEXITCODE -ne 0) { throw 'Repack failed; original is untouched' }
        & $imageTool check $repacked
        if ($LASTEXITCODE -ne 0) { throw 'Repacked image check failed; original is untouched' }
        & $imageTool compare -f qcow2 -F qcow2 $original $repacked
        if ($LASTEXITCODE -ne 0) { throw 'Virtual disk comparison failed; original is untouched' }
        Move-Item -LiteralPath $original -Destination $backup
        try { Move-Item -LiteralPath $repacked -Destination $original }
        catch { Move-Item -LiteralPath $backup -Destination $original; throw }
        Remove-Item -LiteralPath $backup
        Write-Output 'Builder target repacked after complete virtual-disk comparison; guest bytes are unchanged.'
        Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='D:'" | Select-Object FreeSpace | ConvertTo-Json
    }
    'CleanupBuildCache' {
        if (Get-Process cargo,rustc,ninja -ErrorAction SilentlyContinue) { throw 'A compiler is active; cache cleanup refused' }
        $folders = @('tools/build/reims-vgpu/target','work/r3/reims-v030/.work/target/release','work/r3/reims-v030/.work/target/debug','work/r3/reims-v030/build/qemu','work/r3/reims-v030/build/qemu-dbg','target/debug')
        foreach ($relative in $folders) {
            $full = [IO.Path]::GetFullPath((Join-Path $project $relative))
            if (-not $full.StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Cache path outside project: $full" }
            if (Test-Path -LiteralPath $full) {
                Write-Output "Removing generated build cache $relative"
                Remove-Item -LiteralPath $full -Recurse -Force
            }
        }
        $duplicate = Join-Path $project 'work/gpu-followup/ventura-recovery.img'
        $original = Join-Path $project 'tools/gpu-followup/ventura/BaseSystem.dmg'
        if ((Test-Path -LiteralPath $original) -and (Test-Path -LiteralPath $duplicate)) {
            Write-Output 'Removing generated Ventura raw recovery copy; original DMG is retained'
            Remove-Item -LiteralPath $duplicate -Force
        }
        Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='D:'" | Select-Object FreeSpace | ConvertTo-Json
    }
    'ArchiveInfo' {
        $imageTool = Join-Path $env:WSM_HOME 'runtime/qemu/qemu-img.exe'
        $image = Join-Path $project 'work/reims/vm/disks/current.qcow2'
        & $imageTool info --output=json $image
        & $imageTool check $image
        & $imageTool measure --output=json -O qcow2 $image
    }
    'CleanupFailed' {
        foreach ($manifest in Get-ChildItem -LiteralPath (Join-Path $env:WSM_HOME 'instances') -Filter manifest.toml -Recurse) {
            if ((Get-Content -LiteralPath $manifest.FullName -Raw).Contains('base-tahoe-26.3-fd62cbf42ae6.qcow2')) { throw 'The rejected base still has a dependent instance' }
        }
        $files = @(
            'work/r3/home/build/tahoe-26.3/base.qcow2',
            'work/r3/home/bases/base-tahoe-26.3-fd62cbf42ae6.qcow2',
            'work/r3/home/bases/base-tahoe-26.3-fd62cbf42ae6.qcow2.sha256',
            'work/r3/home/bases/base-tahoe-26.3-fd62cbf42ae6.json',
            'work/r3/p1-trace.state',
            'work/r3/home/instances/hl1/state.bin.failed'
        )
        foreach ($relative in $files) {
            $full = [IO.Path]::GetFullPath((Join-Path $project $relative))
            if (-not $full.StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Cleanup path outside project: $full" }
            if (Test-Path -LiteralPath $full) {
                $entry = Get-Item -LiteralPath $full
                if ($entry.PSIsContainer) { throw 'Cleanup only accepts the named generated files' }
                Write-Output ("Removing {0} ({1} bytes)" -f $relative,$entry.Length)
                Remove-Item -LiteralPath $full -Force
            }
        }
        Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='D:'" | Select-Object FreeSpace | ConvertTo-Json
    }
    'DiskUsage' {
        & (Join-Path $project '.venv/Scripts/python.exe') -I tests/project_usage.py
    }
    'StorageCheck' {
        Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='D:'" | Select-Object DeviceID,FileSystem,FreeSpace,Size | ConvertTo-Json
        $imageTool = Join-Path $env:WSM_HOME 'runtime/qemu/qemu-img.exe'
        foreach ($file in 'target.qcow2','base.qcow2') {
            $image = Join-Path $env:WSM_HOME "build/tahoe-26.3/$file"
            if (Test-Path -LiteralPath $image) {
                Write-Output $file
                & $imageTool info --output=json $image
                & $imageTool check $image
            }
        }
    }
    'InspectFirstboot' {
        & $env:WSM_BIN --name basecheck exec -- 'sudo tail -80 /var/log/wsm-firstboot.log; sudo launchctl print system/com.wsm.firstboot | head -35; sudo ls -l /var/db/wsm; sudo du -sk /System/Volumes/Update /System/Volumes/Preboot; csrutil status'
    }
    'ResetGeneric' {
        foreach ($name in 'basecheck','basecheck2') {
            $manifest = Join-Path $env:WSM_HOME "instances/$name/manifest.toml"
            if (Test-Path -LiteralPath $manifest) {
                $text = Get-Content -LiteralPath $manifest -Raw
                if (-not ($text.Contains('base-tahoe-26.3-fd62cbf42ae6.qcow2') -or $text.Contains('base-tahoe-26.3-7f70d08f4532.qcow2'))) { throw "Unexpected base for disposable test instance $name" }
                & $env:WSM_BIN --name $name stop --shutdown
                if ($LASTEXITCODE -ne 0) { throw "Could not stop disposable instance $name" }
                & $env:WSM_BIN --name $name delete
                if ($LASTEXITCODE -ne 0) { throw "Could not delete disposable instance $name" }
            }
        }
    }
    'Progress' {
        foreach ($stage in 'install','install-resume','seed','inject','tools') {
            $log = Join-Path $env:WSM_HOME "build/tahoe-26.3/$stage/serial.log"
            if (Test-Path -LiteralPath $log) {
                Write-Output $stage
                $all = Get-Content -LiteralPath $log -Encoding UTF8
                $all | Select-String 'Kernel boot args:|WSM-BUILD:|WSM-CLT:|CommandTimeout|panic\(cpu|^BSD root:|^.*percent [0-9]+%|ramrod_display_set_granular_progress_forced:' | Select-Object -Last 6 | ForEach-Object { $_.Line }
                $all | Select-Object -Last 5
            }
        }
        Get-Process qemu*,wsm* -ErrorAction SilentlyContinue | Select-Object Id,ProcessName,CPU,WorkingSet64
        Get-ChildItem -LiteralPath (Join-Path $env:WSM_HOME 'bases') | Select-Object Name,Length
        Get-ChildItem -LiteralPath (Join-Path $env:WSM_HOME 'build/tahoe-26.3') -Filter '*.qcow2' | Select-Object Name,Length
        foreach ($name in 'basecheck','basecheck2') {
            $serial = Join-Path $env:WSM_HOME "instances/$name/logs/serial.log"
            if (Test-Path -LiteralPath $serial) {
                Write-Output "instance $name serial tail"
                Get-Content -LiteralPath $serial -Tail 15 -Encoding UTF8
            }
        }
        Get-Date
    }
    'Smoke' {
        & $env:WSM_BIN --name hl1 start
        if ($LASTEXITCODE -ne 0) { throw 'Legacy smoke start failed' }
        & (Join-Path $project '.venv/Scripts/python.exe') -I tests/p4_transport.py hl1
        if ($LASTEXITCODE -ne 0) { throw 'Transport smoke failed' }
        & $env:WSM_BIN --name hl1 suspend
        if ($LASTEXITCODE -ne 0) { throw 'Smoke suspend failed' }
    }
    'Acceptance' {
        & (Join-Path $project '.venv/Scripts/python.exe') -I tests/p6_acceptance.py basecheck
        if ($LASTEXITCODE -ne 0) { throw 'Acceptance suite failed' }
    }
    'Generic' {
        & (Join-Path $project '.venv/Scripts/python.exe') -I tests/p3_generic.py
        if ($LASTEXITCODE -ne 0) { throw 'Generic-base acceptance failed' }
    }
    'Package' {
        & (Join-Path $project '.venv/Scripts/python.exe') -I src/build/package.py --home $env:WSM_HOME --bin target/round3-next/release
        if ($LASTEXITCODE -ne 0) { throw 'Packaging failed' }
    }
}
