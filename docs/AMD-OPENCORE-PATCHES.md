# AMD host + WHPX: OpenCore changes needed to boot macOS Tahoe 26.3 (x86-64)

Applied by `merge_amd_oc.py` (kernel patches) plus OpenCore config settings. Each was found by one boot, one change per boot.

| # | Change | One-line cause |
|---|---|---|
| 1 | QEMU `-device VGA` (still `-nographic`) | OpenCore halts with "Missing compatible GOP" without a display device. |
| 2 | `ScanPolicy=0x10F0103` (excludes the ESP) | With the picker off and the ESP first, OpenCore boots itself ("Found previous image, aborting"). |
| 3 | `Booter.Quirks.ProvideCustomSlide=true` | Intermittent `EB.MM.AKMR Err(0xE)`: the KASLR slide lands on unusable memory. |
| 4 | Keep only "Enable early serial output on RELEASE kernel" from OSX-KVM | The other OSX-KVM panic/serial patches replace `_panic` and print raw pointers. |
| 5 | Two kernel patches NOP `wrmsr SYSENTER_EIP/ESP` (0x176/0x175) | A 64-bit write to these MSRs #GPs on AMD/Hyper-V for an Intel-vendor guest; 64-bit macOS only uses SYSCALL. |
| 6 | AMD_Vanilla "Force cpuid_cores_per_package (13.3+)" (= vCPU count) and "_cpuid_set_cache_info" | CPUID leaves 2/4/5 are not exited to QEMU by WHPX, AMD returns 0 for leaf 4, so cores_per_package = 0 and the kernel divides by zero. |
| 7 | AMD_Vanilla MSR patches: commpage `rdmsr`, MSR 0x8B `wrmsr`/`rdmsr`, 0x17 flag, `_i386_init` 3x `rdmsr` | Hyper-V #GPs Intel-only MSRs on AMD without exiting to QEMU, so `ignore-unknown-msr` never sees them. |
| 8 | AMD_Vanilla "non-monotonic time panic" removal | Removes the kernel's non-monotonic-time panic, which the AMD+WHPX timing path can trigger (documented in RESULTS.md as part of the X2 #7 patch set). |

CPU flags used: `-pcid,-tsc-deadline` (plus `vendor=GenuineIntel`, which QEMU's CPUID leaf 0 exit already provides). Not needed: vendor-spoof patches, `kernel-irqchip=off`.
Source: RESULTS.md (X2 findings 1-7); AMD_Vanilla patches.plist is third-party and not included here.
