#!/usr/bin/env bash
# Print the QEMU argv (one per line) for a headless macOS boot. Consumed by serial_timer.py via xargs-free mapfile.
# Env: ACCEL CPU SMP CORES RAM ESP DISK DISK_FMT EXTRA_DISK GFX NET
cd "$(dirname "$0")/.."
ACCEL=${ACCEL:-whpx}
CPU=${CPU:-Skylake-Client,-hle,-rtm,-pcid,-tsc-deadline,-arat,kvm=off,vendor=GenuineIntel,+invtsc,vmware-cpuid-freq=on}
SMP=${SMP:-4}; CORES=${CORES:-$SMP}; RAM=${RAM:-4096}
ESP=${ESP:-work/esp}
DISK=${DISK:-work/BaseSystem.img}; DISK_FMT=${DISK_FMT:-raw}; DISK_SNAP=${DISK_SNAP:-on}
GFX=${GFX:-none}
args=(
  ${QEMU_BIN:-D:/MyProjects/wsm/tools/qemu/qemu-system-x86_64.exe}
  -accel "$ACCEL" -machine q35 -m "$RAM" -cpu "$CPU" -smp "$SMP",cores="$CORES",sockets=1
  -device isa-applesmc,osk="ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc"
  -smbios type=2
  -drive if=pflash,format=raw,readonly=on,file=refs/OSX-KVM/OVMF_CODE_4M.fd
  -drive if=pflash,format=raw,file=${VARS:-work/OVMF_VARS.fd}
  -device ich9-ahci,id=sata
  -drive id=esp,if=none,${ESP_DRIVE:-format=raw,file=fat:rw:$ESP}
  -device ide-hd,bus=sata.0,drive=esp
  -drive id=sys,if=none,format="$DISK_FMT",snapshot="$DISK_SNAP",file="$DISK"
  -device ide-hd,bus=sata.1,drive=sys
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:${SSH_PORT:-2222}-:22${NET_EXTRA:-} -device ${NIC:-vmxnet3},netdev=n0,mac=52:54:00:c9:18:27
  -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0
  -nographic -serial mon:stdio
)
[ "$GFX" != none ] && args+=(-device "$GFX")
[ "$GFX" = none ] && args+=(-vga none)
[ -n "${EXTRA_DISK:-}" ] && args+=(-drive id=extra,if=none,format=qcow2,file="$EXTRA_DISK" -device ide-hd,bus=sata.2,drive=extra)
[ -n "${SEED:-}" ] && args+=(-drive id=seed,if=none,format=raw,snapshot=on,file="$SEED" -device ide-hd,bus=sata.3,drive=seed)
[ -n "${QMP:-}" ] && args+=(-qmp tcp:127.0.0.1:"$QMP",server=on,wait=off)
printf '%s\n' "${args[@]}" ${EXTRA_ARGS:-}
