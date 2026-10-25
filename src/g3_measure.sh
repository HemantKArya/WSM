#!/usr/bin/env bash
# Cold boot -> SSH for the full-OS variant, then idle stats. usage: src/g3_measure.sh LABEL [RAM_MB] [extra qemu env...]
cd "$(dirname "$0")/.."
L=$1; RAM=${2:-3072}; mkdir -p logs/G3
taskkill //F //IM qemu-system-x86_64.exe >/dev/null 2>&1; sleep 1
mapfile -t A < <(SSH_PORT=2322 VARS=work/r2/g3-vars.fd ESP_DRIVE="format=raw,file=work/r2/esp-g4.img" DISK=${FULL:-work/r2/g3-full.qcow2} DISK_FMT=qcow2 DISK_SNAP=off EXTRA_DISK=${EXTRA-} GFX=VGA QMP=4444 RAM=$RAM SMP=${SMP:-4} bash src/boot.sh)
TSSH_PORT=2322 TSSH_USER=dev PROBE_TIMEOUT=2 .venv/Scripts/python.exe src/boot_to_ssh.py logs/G3/$L.log 200 --keep --cmd "sw_vers -productVersion" -- "${A[@]}" > logs/G3/$L.boot.txt 2>&1 &
until grep -q -E "ssh ok|NO SSH" logs/G3/$L.boot.txt 2>/dev/null; do sleep 1; done
grep -E "kernel|ssh ok|NO SSH" logs/G3/$L.boot.txt | tr '\n' ' '; echo
grep -q "ssh ok" logs/G3/$L.boot.txt || exit 1
sleep ${IDLE_WAIT:-60}
src/tssh.sh 'echo PROCS=$(ps ax | wc -l); ps axo rss= | awk "{s+=\$1} END {print \"GUEST_RSS_SUM_MB=\" int(s/1024)}"; top -l 1 -n 0 | grep -E "PhysMem|Load Avg"; vm_stat | awk "/Pages free/ {f=\$3} /Pages speculative/ {sp=\$3} END {printf \"FREE_MB=%d\n\", (f+sp)*4096/1048576}"; ps axo rss=,comm= | sort -n -r | head -8' 2>&1 | tee logs/G3/$L.idle.txt
powershell -NoProfile -Command '$p=Get-Process qemu-system-x86_64; $c1=$p.CPU; Start-Sleep 20; $p.Refresh(); "HOST_QEMU WS_MB={0:N0} CPU%_20s={1:N2}" -f ($p.WorkingSet64/1MB),(($p.CPU-$c1)/20*100/[Environment]::ProcessorCount)' | tee -a logs/G3/$L.idle.txt
