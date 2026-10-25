#!/usr/bin/env bash
# Boot the installed full Tahoe (snapshot, discarded) with the G3 base attached as 2nd disk (writable). usage: src/g3_boot.sh [RAM_MB=3072]
cd "$(dirname "$0")/.."
taskkill //F //IM qemu-system-x86_64.exe >/dev/null 2>&1
mapfile -t A < <(SSH_PORT=2322 VARS=work/r2/g3-vars.fd ESP_DRIVE="format=raw,file=work/r2/esp-g4.img" DISK=${FULL:-work/r2/g3-full.qcow2} DISK_FMT=qcow2 DISK_SNAP=off EXTRA_DISK=${EXTRA-work/r2/g3-base.qcow2} GFX=VGA QMP=4444 RAM=${1:-3072} SMP=4 bash src/boot.sh)
.venv/Scripts/python.exe src/boot_to_ssh.py logs/G3-full-boot.log 240 --keep --cmd "sw_vers -productVersion" -- "${A[@]}" > logs/G3-full-boot.txt 2>&1 &
s=$(date +%s); until TSSH_PORT=2322 src/tssh.sh true 2>/dev/null; do sleep 2; [ $(( $(date +%s)-s )) -gt 240 ] && echo NO-SSH && exit 1; done; echo "ssh after $(( $(date +%s)-s ))s"
