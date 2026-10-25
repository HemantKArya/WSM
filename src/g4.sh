#!/usr/bin/env bash
# G4: suspend-to-file / resume of the headless base on the MAXZVER QEMU (+WSM migration patch).
# usage: src/g4.sh LABEL      logs -> logs/G4/
cd "$(dirname "$0")/.."
L=${1:-run}; P=.venv/Scripts/python.exe; ST=$(cygpath -m "$PWD")/work/r2/g4.state
export PATH="$PWD/tools/msys64/ucrt64/bin:$PATH" WSM_WHPX_ALLOW_MIGRATION=1
export REIMS_VGPU_LAB_RAM_RESIDENT=off REIMS_VGPU_LAB_RAM_POPULATE=off QEMU_WHPX_MMIO_FAST=0
args() { QEMU_BIN=$(cygpath -m "$PWD")/tools/build/reims-vgpu/vendor/qemu/build/qemu-system-x86_64.exe ESP_DRIVE="format=raw,file=work/r2/esp-g4.img" \
  VARS=work/r2/g4-vars.fd DISK=work/r2/g4-base.qcow2 DISK_FMT=qcow2 DISK_SNAP=off EXTRA_DISK=work/r2/g4-data.qcow2 GFX=VGA QMP=4444 ACCEL=${ACCEL:-whpx} bash src/boot.sh; }
taskkill //F //IM qemu-system-x86_64.exe >/dev/null 2>&1; rm -f work/r2/g4.state
mapfile -t A < <(args)
echo "== cold boot"; PROBE_TIMEOUT=2 $P src/boot_to_ssh.py logs/G4/$L-cold.log 180 --keep -- "${A[@]}" > logs/G4/$L-cold.txt 2>&1 &
until grep -q -E "ssh ok|NO SSH" logs/G4/$L-cold.txt 2>/dev/null; do sleep 1; done; grep -E "kernel|ssh ok|NO SSH" logs/G4/$L-cold.txt
grep -q "ssh ok" logs/G4/$L-cold.txt || exit 1
sleep 20; src/gssh.sh 'echo marker-$(date +%s) > /tmp/g4marker; cat /tmp/g4marker; uptime'
echo "== save"; t0=$(date +%s.%N)
$P src/qmp.py 4444 stop >/dev/null
$P -I src/migpipe.py sink 5555 work/r2/g4.state > logs/G4/$L-sink.txt 2>&1 &
until grep -q listening logs/G4/$L-sink.txt 2>/dev/null; do sleep 0.05; done
$P src/qmp.py 4444 migrate '{"uri":"tcp:127.0.0.1:5555"}'
until $P src/qmp.py 4444 query-migrate | grep -q -E '"status": "(completed|failed)"'; do sleep 0.2; done
t1=$(date +%s.%N); $P src/qmp.py 4444 query-migrate | head -c 600; echo
sleep 0.5; cat logs/G4/$L-sink.txt; echo "save_s=$(awk "BEGIN{print $t1-$t0}") size=$(stat -c %s work/r2/g4.state)"
taskkill //F //IM qemu-system-x86_64.exe >/dev/null 2>&1; sleep 2
echo "== restore"; t0=$(date +%s.%N)
"${A[0]}" "${A[@]:1}" -incoming tcp:127.0.0.1:5556 > logs/G4/$L-restore.log 2>&1 < /dev/null &
$P -I src/migpipe.py source 5556 work/r2/g4.state > logs/G4/$L-source.txt 2>&1 &
until $P src/qmp.py 4444 query-status 2>/dev/null | grep -q '"status": "paused"\|"status": "running"\|postmigrate'; do sleep 0.1; tasklist | grep -q qemu-system || { echo "QEMU died on restore:"; tail -3 logs/G4/$L-restore.log; exit 2; }; done
t1=$(date +%s.%N); $P src/qmp.py 4444 query-status | head -1
$P src/qmp.py 4444 cont >/dev/null
until src/gssh.sh 'cat /tmp/g4marker; sysctl -n kern.boottime' 2>/dev/null; do sleep 0.2; t=$(awk "BEGIN{print $(date +%s.%N)-$t0}"); [ ${t%.*} -gt 60 ] && echo "NO SSH after restore" && break; done
t2=$(date +%s.%N); echo "load_s=$(awk "BEGIN{print $t1-$t0}") restore_to_ssh_s=$(awk "BEGIN{print $t2-$t0}")"
