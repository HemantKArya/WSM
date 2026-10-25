#!/usr/bin/env bash
# Cold boot -> SSH time, then idle metrics. usage: src/measure.sh LABEL   (VM left running afterwards)
cd "$(dirname "$0")/.."
L=${1:-run}; mkdir -p logs/X3
taskkill //F //IM qemu-system-x86_64.exe >/dev/null 2>&1
mapfile -t A < <(DISK=work/wsm-base.qcow2 DISK_FMT=qcow2 DISK_SNAP=off EXTRA_DISK=work/wsm-data.qcow2 GFX=VGA QMP=4444 bash src/boot.sh)
.venv/Scripts/python src/boot_to_ssh.py logs/X3/$L.log 150 --keep -- "${A[@]}" > logs/X3/$L.boot.txt 2>&1 &
until grep -q -E 'ssh ok|NO SSH' logs/X3/$L.boot.txt 2>/dev/null; do sleep 1; done
grep -v warning logs/X3/$L.boot.txt
sleep 45
src/gssh.sh 'echo PROCS=$(ps ax | wc -l); ps axo rss= | awk "{s+=\$1} END {print \"GUEST_RSS_SUM_MB=\" int(s/1024)}"; ps axo rss=,comm= | sort -n -r | head -12' | tee logs/X3/$L.idle.txt
powershell -NoProfile -Command '$p=Get-Process qemu-system-x86_64; $c1=$p.CPU; Start-Sleep 30; $p.Refresh(); "HOST_QEMU WS_MB={0:N0} Private_MB={1:N0} CPU%_30s={2:N2}" -f ($p.WorkingSet64/1MB),($p.PrivateMemorySize64/1MB),(($p.CPU-$c1)/30*100/[Environment]::ProcessorCount)' | tee -a logs/X3/$L.idle.txt
