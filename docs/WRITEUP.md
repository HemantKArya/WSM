# WSM: a WSL-style macOS VM on Windows

WSM runs macOS Tahoe 26.3 (x86-64) on Windows 11 with QEMU on the Windows Hypervisor Platform. One command gives a shell. `wsm stop` saves the VM to a file and the next command resumes it in about 2 s. A GPU profile adds a paravirtual GPU (reims-vgpu on Vulkan) and a desktop window.

**One base, two products.** Both profiles are qcow2 overlays on the same read-only base (17.9 GB). The core profile is headless and suspendable. The GPU profile boots cold each time because the paravirtual GPU blocks migration.

**What had to be fixed to make this work on an AMD host** (patches in `src/patches/`, notes in this folder):
- Six QEMU WHPX patches (`src/patches/`): allow migration, restore the APIC, resume application processors, pause guest time while saved, migrate pending events, and take the BQL on APIC EOI exits (this one fixed a lost level-triggered interrupt that stalled AHCI).
- OpenCore kernel patches for AMD (`AMD-OPENCORE-PATCHES.md`, script `src/build/merge_amd_oc.py`).

**Measured** (see `RESULTS-summary.md`): resume 2.0-2.7 s, warm exec 57 ms, GPU animation 30-45 fps presented.

**Limits:** GPU instances cold boot; window dragging is uneven; the first GUI login takes 2-3 min; WebDAV drive mapping has Windows' own 50 MB per-file limit; Tahoe is the last Intel macOS; Intel hosts untested.

No Apple software or disk images are included. Build your own base from an installer you own with `wsm base build`.
