#!/usr/bin/env bash
# Boot the installed Tahoe disk with the MAXZVER master build (reims-vgpu-pci). Prints the log dir.
# Env: RAM(MiB, 8192) SMP(8) WINDOW(off|on) REIMS_VGPU_* passthrough, DISK (work/r2/mz/tahoe.qcow2), INCOMING (migrate URI)
cd "$(dirname "$0")/.."
R=$(cygpath -m "$PWD"); B=$R/tools/build/reims-vgpu; L=$R/work/r2/mz/logs/$(date +%Y%m%d-%H%M%S); mkdir -p "$L"
DISK=${DISK:-work/r2/mz/tahoe.qcow2}
[ -f "$DISK" ] || tools/qemu/qemu-img.exe create -q -f qcow2 -F qcow2 -b "D:/MyProjects/wsm/work/reims/vm/disks/current.qcow2" "$DISK"
[ -f work/r2/mz/OVMF_VARS.fd ] || cp work/reims/vm/firmware/OVMF_VARS.fd work/r2/mz/OVMF_VARS.fd
export PATH="$(cygpath -u "$R")/tools/msys64/ucrt64/bin:$PATH" REIMS_VGPU_WINDOW=${WINDOW:-off} REIMS_VGPU_LOG=${REIMS_VGPU_LOG:-info} REIMS_VGPU_STATS=${REIMS_VGPU_STATS:-1}
A=(-L "$B/vendor/qemu/pc-bios" -name wsm-mz -machine q35 -accel ${ACCEL:-whpx,hyperv=off}
  -cpu "Haswell-noTSX,stepping=3,vendor=GenuineIntel,+ssse3,+sse4.2,+popcnt,+avx,+avx2,+aes,+xsave,+xsaveopt,-arat,-pcid,-tsc-deadline${CPU_EXTRA:-,+invtsc,vmware-cpuid-freq=on},enforce"
  -smp ${SMP:-8},sockets=1,cores=${SMP:-8},threads=1 -m ${RAM:-8192}M
  -global ICH9-LPC.disable_s3=1 -global ICH9-LPC.disable_s4=1
  -device "isa-applesmc,osk=ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc"
  -drive if=pflash,format=raw,readonly=on,file=work/reims/vm/firmware/OVMF_CODE.fd
  -drive if=pflash,format=raw,file=work/r2/mz/OVMF_VARS.fd -smbios type=2
  -device ich9-ahci,id=sata,addr=0x2
  -drive id=opencore,if=none,format=raw,snapshot=on,file=${OCIMG:-work/reims/vm/firmware/opencore-1.0.8/OpenCore.img}
  -device ide-hd,bus=sata.2,drive=opencore,bootindex=1
  -drive id=disk,if=none,format=qcow2,file="$DISK" -device ide-hd,bus=sata.3,drive=disk,rotation_rate=1
  -netdev user,id=net0,ipv6=off,hostfwd=tcp:127.0.0.1:2322-:22 -device virtio-net-pci,netdev=net0,id=net0,mac=52:54:00:c4:59:12,addr=0x3
  -vga none -display none
  -device pci-bridge,chassis_nr=5,id=pci.5,bus=pcie.0,addr=1e.0
  -device "reims-vgpu-pci,id=reimsvgpu,romfile=$B/crates/reims-vgpu-efi/out/reims-vgpu-gop.rom,rombar=1,bus=pci.5,addr=00.0"
  -device qemu-xhci,id=xhci,addr=0x5 -device usb-kbd,bus=xhci.0 -device usb-tablet,bus=xhci.0
  -monitor none -serial "file:$L/serial.log" -msg timestamp=on -qmp tcp:127.0.0.1:4544,server=on,wait=off)
[ -n "${INCOMING:-}" ] && A+=(-incoming "$INCOMING")
printf '%s\n' "${A[@]}" > "$L/args.txt"
"$(cygpath -u "$B")/vendor/qemu/build/qemu-system-x86_64.exe" "${A[@]}" > "$L/stdout.log" 2> "$L/stderr.log" &
echo "$L pid=$!"
