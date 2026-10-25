#!/usr/bin/env bash
# G5: cold boot -> SSH on round-2 overlays. usage: src/g5.sh LABEL [vvfat|img]  (VM killed afterwards unless KEEP=1)
cd "$(dirname "$0")/.."
L=${1:-run}; mkdir -p logs/G5
taskkill //F //IM qemu-system-x86_64.exe >/dev/null 2>&1
[ "${2:-vvfat}" = img ] && export ESP_DRIVE="format=raw,file=work/r2/esp.img"
mapfile -t A < <(VARS=work/r2/OVMF_VARS.fd DISK=work/r2/g5-base.qcow2 DISK_FMT=qcow2 DISK_SNAP=off EXTRA_DISK=work/r2/g5-data.qcow2 GFX=VGA QMP=4444 bash src/boot.sh)
.venv/Scripts/python src/boot_to_ssh.py logs/G5/$L.log 150 ${KEEP:+--keep} -- "${A[@]}" 2>&1 | grep -v -i warning | tee logs/G5/$L.boot.txt
