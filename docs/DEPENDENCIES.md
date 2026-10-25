# What you download separately, and what WSM is built from

The release `wsm-setup.zip` contains the WSM programs and a QEMU runtime. It contains **no Apple software**.
You supply the macOS installer yourself; everything else below is either bundled in the zip or only needed to build from source.

## 1. You must download: the macOS installer (base image source)

| item | exact source | check |
|---|---|---|
| macOS Tahoe 26.3 installer ISO (`macOS Tahoe 26.3.iso`, 21,474,836,480 bytes) | https://archive.org/download/mac-os-tahoe-26.3/macOS%20Tahoe%2026.3.iso | sha256 `6737e0969732eb621695d1eae79d23919b14751ad4bfe8475f1913324cd3d567` (the ISO used for every number in RESULTS.md) |

The ISO is third-party content hosted on archive.org. Use an installer you are entitled to use; WSM does not ship, redistribute or modify it.
From the ISO, `wsm base build-lite --iso "macOS Tahoe 26.3.iso"` (about 8 min, 1.0 GB light base) or `wsm base build --iso ...`
(1-2 h, 17.9 GB full base) produces your own base image. Bases are never uploaded anywhere.

## 2. Downloaded by WSM or by you inside the guest (only if you use the feature)

| item | used for | source |
|---|---|---|
| Xcode Command Line Tools 26.x packages (`CLTools_Executables_Universal.pkg`, `CLTools_macOSLMOS_SDK.pkg`, `CLTools_macOSNMOS_SDK.pkg`) | compilers, git, make on a light base; the `--clt` option of the full-base build | Apple's software-update CDN (swcdn.apple.com); exact URLs and SHA-256 pins in `guest/build/clt-pins.json` |
| Homebrew (`brew`, commit `51da7f234a4c`) and its portable Ruby, bottles and source tarballs | packages on a light base | https://github.com/Homebrew/brew , https://ghcr.io (bottles) |
| libffi 3.4.6 (built in the guest, shipped in the tools pack) | Homebrew's Ruby needs `/usr/lib/libffi.dylib` | https://github.com/libffi/libffi/releases/tag/v3.4.6 |
| the "tools pack" (diff, cmp, patch, install, du, gzip, bzip2, file, stat, base64, sftp-server, xcrun shims) | light-base conveniences | copied from a full base you built; contains Apple binaries, so it is never published |

## 3. Bundled in `wsm-setup.zip` (with licences under `licenses/`)

| component | role | source |
|---|---|---|
| QEMU (vendor tree `7100064d13b4`) + WSM patches 01-06 (`src/patches/`) | the hypervisor front end on WHPX | https://github.com/MAXZVER/qemu-reims-vgpu lineage, as vendored by reims-vgpu v0.3.0 |
| reims-vgpu v0.3.0 (`09f13b32e0a4`), DLL build `7bdb4ec0` | paravirtual Apple GPU (Vulkan) | https://github.com/aljus7/reims-vgpu |
| OpenCore (acidanthera) boot images, AMD/Intel templates | boots macOS under QEMU | https://github.com/acidanthera/OpenCorePkg |
| AMD_Vanilla kernel patches (merged by `src/build/merge_amd_oc.py`) | macOS on AMD CPUs | https://github.com/AMD-OSX/AMD_Vanilla |
| OVMF firmware (EDK II) | UEFI | bundled with the QEMU build |
| 7-Zip | reads the ISO's HFS+ volume (LGPL) | https://www.7-zip.org |
| Rust crates (ssh2, vendored OpenSSL, clap, serde, ...) | wsm.exe / wsmd.exe | `Cargo.lock` |

## 4. Needed only to build from source (portable, kept under `tools/`, nothing installed system-wide)

Rust toolchain via rustup (https://rustup.rs), MSYS2 (https://www.msys2.org; ucrt64 clang/gcc, make, meson for QEMU),
PowerShell 7 (https://github.com/PowerShell/PowerShell), Python 3 with `uv` (https://docs.astral.sh/uv/) for the helper scripts.

## 5. Studied as references (not shipped)

OSX-KVM (https://github.com/kholia/OSX-KVM), Docker-OSX (https://github.com/sickcodes/Docker-OSX), quickemu
(https://github.com/quickemu-project/quickemu), and the other reims-vgpu forks: steelbrain/reims-vgpu, Jakehallmark/reims-vgpu,
Nouradawy/reims-vgpu, bisaalz007/reims-vgpu-windows. Thanks to their authors.

## Host requirements
Windows 11 x86-64 (AMD or Intel), Virtual Machine Platform + Windows Hypervisor Platform enabled, 16 GB RAM recommended, 40 GB free disk.
