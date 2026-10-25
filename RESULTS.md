# WSM feasibility experiments — RESULTS

Host: AMD Ryzen 7 7840HS (8C/16T, Zen 4), 15.2 GB RAM, Radeon 780M + RTX 4050 Laptop, Windows 11 Home 10.0.26300.
Tools fetched into `tools/` (portable, nothing installed): 7-Zip 24.09 (MSI admin-extract → `tools/7zip`), QEMU 11.1.0 (weilnetz 20260811 NSIS → `tools/qemu`), Alpine virt 3.20.3 ISO (`tools/dl`).
Reference repos: `refs/` (shallow clones).
Scripts: `src/serial_timer.py` (launch QEMU, time regex matches on serial, optionally type commands), `src/boot.sh` (QEMU argv), `src/make_esp.py` (tuned OpenCore ESP), `src/dirsizes.py`.

## X0 — Host + hypervisor — PASS
- Windows features: `HypervisorPlatform` and `VirtualMachinePlatform` already enabled (no change made).
- `qemu-system-x86_64 -accel help` → `tcg, whpx`.
- Alpine virt ISO, 2 vCPU / 1 GB, serial console (`logs/X0/`):

| accel | boot → `login:` | `seq 3000000 \| md5sum` in guest | guest sees |
|---|---|---|---|
| whpx | 7.0–8.9 s | **1.01 s** | `Hypervisor detected: Microsoft Hyper-V` |
| tcg  | 12.5–19.1 s | 8.59 s | — |

  WHPX is real (8.5× faster than TCG on CPU-bound work).
- Vulkan: Radeon 780M (AMD proprietary, Vulkan 1.3.280), RTX 4050 (NVIDIA, Vulkan 1.4.312) — `logs/X0/vulkaninfo.txt`.
- AMD CPU ⇒ guest gets `vendor=GenuineIntel` CPU model; AMD_Vanilla kernel patches available if needed (`src/make_esp.py --amd`).

## X1 — Dissect the Tahoe image — PASS
`macOS Tahoe 26.3.iso` (20 GiB) is **not ISO9660**: Apple Partition Map + one HFS+ partition (a `createinstallmedia` USB image). Listing: `logs/X1/iso-list.txt`.

```
Install macOS Tahoe/                       (HFS+, 19.1 GB used)
├── Install macOS Tahoe.app/Contents/SharedSupport/SharedSupport.dmg   17.5 GB  (full OS payload)
├── BaseSystem/BaseSystem.dmg                                          1.37 GB  (UDIF, uncompressed)
├── System/Library/CoreServices/boot.efi                               0.7 MB
├── System/Library/KernelCollections/BootKernelExtensions.kc          64 MB    (+ im4m tickets)
└── usr/standalone/i386/ (SecureBoot.bundle)                           13 MB
```
BaseSystem.dmg → `qemu-img convert -f dmg -O raw` (1.5 s) → `work/BaseSystem.img`: **GPT + one APFS container, 1.47 GB raw, 2.25 GB of files (53,859 files)**. Tree sizes (`logs/X1/basesystem-dirsizes.txt`):

| path | MB |
|---|---|
| System/Library/dyld (shared cache) | 914 |
| System/Library/PrivateFrameworks | 215 |
| usr/share (firmware 57, icu 33, locale 30) | 130 |
| System/Library/Frameworks | 106 |
| System/Library/Fonts | 91 |
| System/Library/KernelCollections | 86 |
| System/Library/CoreServices | 78 |
| Preboot volume (boot.efi, kc) | 75 |
| System/Cryptexes | 54 |
| usr/libexec + sbin + bin + lib | 100 |
| Install macOS Tahoe.app (installer UI) | 36 |

Minimal base ≈ BaseSystem minus installer apps/fonts/locales/screenreader ≈ 1.2–1.4 GB; the dyld cache alone is ~40%.

## X2 — Boot BaseSystem headless under WHPX (AMD) — kernel + userland PASS; serial shell: see below
Command: `mapfile -t A < <(GFX=VGA QMP=4444 bash src/boot.sh)` with ESP from `python src/make_esp.py work/esp --topo`.
QEMU: q35, `-accel whpx`, `Skylake-Client,-hle,-rtm,-pcid,-tsc-deadline,-arat,kvm=off,vendor=GenuineIntel,+invtsc,vmware-cpuid-freq=on`, 4 vCPU, 4 GB, AppleSMC, OVMF (OSX-KVM), OpenCore ESP via `fat:rw:` (vvfat), BaseSystem.img `snapshot=on`, `-device VGA` (needed for OC GOP), `-nographic -serial mon:stdio`.

**Measured (logs/X2/try16.log):** QEMU start → `HANDOFF TO XNU` 4.5 s → `Darwin Kernel Version 25.3.0 … xnu-12377.81.4~5/RELEASE_X86_64` 4.5 s → root device 5.9 s → launchd + Recovery GUI processes ~9 s. 0 panics over 180 s.

### Root-cause chain (each found by reading the panic/trap frame, not guessing)
| # | Symptom | Cause | Fix | Source |
|---|---|---|---|---|
| 1 | OC: "Missing compatible GOP" → halt | no display device | `-device VGA` (still `-nographic`) | — |
| 2 | OC: "Found previous image, aborting" | picker off + ESP is first entry → OC boots itself | `ScanPolicy=0x10F0103` (no ESP) | OpenCorePkg `Application/OpenCore/OpenCore.c:308`, QMP screenshot of OSX-KVM picker |
| 3 | boot.efi `EB.MM.AKMR Err(0xE)` → "Aborted" (intermittent) | KASLR slide hits unusable memory | `Booter.Quirks.ProvideCustomSlide=true` | — |
| 4 | panic shows raw `%016llx`, RIP `<ptr>` | OSX-KVM "send panic/early prints to serial" patches replace `_panic` | keep only "Enable early serial output on RELEASE kernel" | — |
| 5 | #GP right after handoff (CR4=PAE only) | **`wrmsr SYSENTER_EIP/ESP` (0x176/0x175) with 64-bit value; 32-bit on AMD and Hyper-V doesn't widen them for Intel-vendor guests** (KVM does: `svm.c sysenter_eip_hi`) | 2 kernel patches NOP those wrmsr (64-bit macOS uses SYSCALL only) | trap frame read via QMP `x/160gx $rsp`, disasm `x/40i` |
| 6 | divide error after `mig_table_max_displ` | CPUID leaves 2/4/5 not in QEMU WHPX `cpuidExitList` → AMD hardware returns 0 for leaf 4 → cores_per_package=0 | AMD_Vanilla "Force cpuid_cores_per_package (13.3+)" (=vCPU count) + "_cpuid_set_cache_info" | qemu `target/i386/whpx/whpx-all.c:3042`; fork AGENTS.md (same finding on Intel hybrid) |
| 7 | #GP in `_commpage_post_ucode_update` (RCX=0x1a0) | Hyper-V #GPs Intel-only MSRs on AMD *without* exiting to QEMU, so `ignore-unknown-msr` never sees them | AMD_Vanilla MSR patches: commpage rdmsr, 0x8B wr/rd, 0x17 flag, i386_init 3×rdmsr; + non-monotonic-time panic removal | backtrace via `keepsyms=1` |

### Serial shell — PASS (logs/X2/try18-single.log)
Boot-args `-v -s …` (single-user). QEMU start → kernel 3.7 s → **`-sh-3.2#` root shell on serial 7.0 s** → first command executed 10.3 s (input before the AHCI probe finishes is echoed but Enter is lost; send after `Successfully initialized AHCI`):
```
Darwin localhost 25.3.0 Darwin Kernel Version 25.3.0: Wed Jan 28 20:53:14 PST 2026; root:xnu-12377.81.4~5/RELEASE_X86_64 x86_64
ProductVersion: 26.3
```
Root FS is read-only (sealed BaseSystem), launchd services not running (logd, notifyd, opendirectoryd) — that's X3's job.

Not needed: vendor-spoof patches (QEMU's CPUID leaf 0 already exits → GenuineIntel), CPU model (Penryn/Haswell/Skylake identical), `kernel-irqchip=off`.

## X3 — Minimal writable base — PASS (boot-to-SSH), trimming in progress
Disk layout:
- `work/wsm-base.qcow2` — BaseSystem.img converted to qcow2 (1.37 GiB virtual, 1.19 GiB used), root remounted rw by init. 83 MB free on root → nothing big goes here.
- `work/wsm-data.qcow2` — 40 GB, APFS "WSMData" (formatted in-guest with `diskutil eraseDisk APFS WSMData GPT disk2`), mounted by UUID at `/Volumes/WSMData`; `/usr/local`, `/opt`, `/Library/Developer` are symlinks into it.
  **Do not** mount over `/System/Volumes/Data`: in BaseSystem it is a tmpfs holding the live local user DB — covering/unmounting it kills opendirectoryd and every login (observed: sshd-session `opendirectoryd.libinfo` lookup failures).

Provisioning (one-time, `logs/X3/setup.log`): boot single-user (`work/esp-single`, `-s`) with a **raw tar "seed" disk** (`src/mkseed.py work/seed work/seed.img`), then `tar -xf /dev/disk3; sh setup.sh`.
FAT/vvfat seeds don't work: `msdosfs.kext` isn't in the boot KC and kmutil can't load it in single-user ("Daemon is not reachable").
`setup.sh` installs `/System/Library/WSM/{init.sh,sshd_config,host keys,authorized_keys}` + `com.wsm.init` LaunchDaemon and moves GUI/installer plists to `/System/Library/WSM/disabled/` (reversible).
sshd: BaseSystem's `/usr/sbin/sshd` with own config (key-only root, `UsePAM no`); no `sftp-server` in BaseSystem → `scp` fails, use `ssh 'cat > f' < f`.

| iteration | cold boot → sshd ready | → first SSH cmd | guest procs | host QEMU WS | host idle CPU |
|---|---|---|---|---|---|
| v1 (init waited for DHCP addr 30 s) | 44.8 s | 49.0 s | 78 | — | — |
| v2 (no DHCP wait) | 14.1 s | **15.8 s** | 78 | 1,486 MB (private 4,669 MB) | 0.36 % of 16 threads |
| v3 (+ batch-2 trim: mobileassetd, apsd, iconservices, locationd, airportd, bluetoothd, imk, softwareupdated, analyticsd, cvmsComp*) | 13.8 s | **15.8 s** | **60** | **1,199 MB** (private 4,559 MB; guest RSS sum 362 MB) | 0.41 % |

Breakdown of the 15.8 s cold boot (v3): OVMF+OpenCore+boot.efi 3.8 s → kernel → root device 5.2 s → launchd reaches `com.wsm.init` 10.1 s → sshd 13.8 s → first ssh command 15.8 s.
Outbound HTTPS from guest via QEMU user-net works (github.com 200 in 0.58 s).
Disk: base 1.19 GiB qcow2 (BaseSystem, unmodified size) + data volume (empty: 36 KB).

### Bare fundamentals (processes alive in the trimmed base with working SSH, `logs/X3/trim2-procs.txt`)
- **Kernel/boot:** launchd, kernelmanagerd + kernelmanager_helper, amfid, AppleCredentialManagerDaemon, KernelEventAgent, UserEventAgent, powerd, timed
- **Storage:** diskarbitrationd, storagekitd, fskitd/fskit_helper/fskit.msdos, fseventsd, filecoordinationd, dirhelper
- **Identity/security (required for ssh login):** opendirectoryd, usermanagerd, securityd, trustd(+FileHelper), authd, secinitd, sandboxd, tccd, mobileactivationd
- **IPC/prefs/logging:** notifyd, distnoted, cfprefsd, logd, syslogd, remoted, runningboardd, smd, backgroundtaskmanagementd
- **Network:** configd, mDNSResponder(+Helper); sshd (ours)
- **LaunchServices family (candidates for next trim):** lsd, launchservicesd, coreservicesd, iconservicesd, extensionkitservice, containermanagerd_system, displaypolicyd, wifiFirmwareLoader, com.apple.ifdreader, instlogd
Disabled (moved to `/System/Library/WSM/disabled/`): 27 LaunchDaemons + 25 LaunchAgents (WindowServer, loginwindowLite, Recovery/installer UI, Wi-Fi/BT/location/push/asset/update/analytics). `installd`/`system_installd` kept for `installer -pkg`.

## X4 — Developer toolchain on the minimal base — PASS (with workarounds)
CLT **26.6** (product 140-17812 from Apple's public SUS catalog, no Apple ID): `CLTools_Executables_Universal.pkg` 776 MB + macOS NMOS/LMOS SDK pkgs, downloaded **inside the guest** with curl at ~6 MB/s (QEMU user-net).

| step | result |
|---|---|
| `installer -pkg … -target /` | **FAIL**: PackageKit preflight checks free space on the 1.4 GB BaseSystem root ("additional 3.53 GB needed") and ignores that `/Library/Developer` is a symlink to the data volume. |
| workaround: `pkgutil --expand-full` (20 s/pkg) + `ditto Payload/Library/Developer → /Volumes/WSMData/Library/Developer` (94 s) + `SDKs/MacOSX.sdk → MacOSX26.sdk` | PASS: Apple clang 21.0.0, Swift 6.3.3, git 2.50.1, SDK 26.5 |
| C hello (`clang -O2`) | PASS, `MH_MAGIC_64 X86_64 EXECUTE` |
| Swift hello (`swiftc -O`) | PASS, 0.29–0.30 s compile, Mach-O x86_64 |
| `git clone --depth 1` jq / Homebrew/brew | 2.7 s / 7.6 s |
| autoconf project (libffi 3.4.6 `./configure && make -j4`) | PASS, 29 s |
| `brew install hello` (no Intel-Tahoe bottle → source build, Tier 3) | **PASS, 51.9 s** (`🍺 /usr/local/Cellar/hello/2.12.3 … built in 50 seconds`) |

Compile/run vs WSL2 Debian on the same PC (`work/x4/bench.c`: sum of primes < 3M, `clang -O2`, 3 runs):

| | compile | run |
|---|---|---|
| macOS guest (WHPX, 4 vCPU), Apple clang 21 | 0.106–0.148 s | 0.447–0.452 s |
| WSL2 Debian 13 (16 vCPU), clang 19 | 0.169–0.192 s | 0.413–0.418 s |
Runtime within ~9 % of WSL2 → native-speed confirmed.

What it took to make Homebrew work on a BaseSystem-derived userland (each one observed, then fixed):
1. Missing userland binaries (`readlink`, `diff`, `patch`, `cmp`, `install`, `du`, `gzip`, `file`, `xcrun`, `xcode-select`): extracted from the installer's full-OS payload. The 17.5 GB MobileAsset zip inside `SharedSupport.dmg` was attached as a raw disk, chunks streamed out with guest `tar` (7.9 GB, 1705 s), then `aa extract -i payload.04x -include-path usr/bin/X` (host `ipsw ota` needs Apple's `aa`, macOS-only).
2. `shasum` is Perl and BaseSystem's Perl lacks `Fcntl`/`Digest::SHA` → Python shim in `/usr/local/bin/shasum`.
3. Portable Ruby's `fiddle.bundle` needs `/usr/lib/libffi.dylib`, which is only in the **full OS dyld shared cache** (cryptex), not BaseSystem's → built libffi from source, installed as `/usr/lib/libffi.dylib` (ad-hoc signed). Same class: `uptime` (libsbuf), `top` (libpanel) still broken.
4. Homebrew prefix reached through a symlink → Ruby requires files twice (`already initialized constant Sandbox::…`, "Unknown command: install") → `/usr/local` is now its own APFS volume (`WSMLocal`) mounted directly.
5. Build sandbox fails with `TMPDIR` on the data volume → `HOMEBREW_TEMP=/Volumes/WSMData/tmp`.
6. Superenv's make shim does `exec /usr/bin/make` → symlinked CLT tools (`make cc clang git ld …`) into `/usr/bin`; `xcode-select -s /Library/Developer/CommandLineTools`.
7. Homebrew refuses root, and `dscl`-created users are invisible to `getpwnam` in BaseSystem (libinfo reads flat files) → `dev` (uid 501) is appended to `/etc/master.passwd` + `/etc/passwd` by `init.sh` each boot.

**Lesson for the product:** BaseSystem's userland/dyld cache is too thin for development. The next base should take `/usr/bin`, `/usr/lib` and the OS cryptex dyld cache from the full-OS payload (`payloadv2` + `image_patches/cryptex-system-x86_64`) while keeping the BaseSystem-style boot (no Data-volume/firstboot ceremony).

## X5 — Instant attach — PARTIAL
| metric | value |
|---|---|
| cold boot → first SSH command (trimmed base, 4 GB) | **15.8 s** (range 15.8–18.3 s over 5 boots) |
| warm attach `ssh … true` (VM running) | **119 ms median** (104–147, n=10); `ssh … sw_vers` 133 ms |
| idle host cost (VM running, nothing to do) | QEMU working set 1.2 GB, 0.4 % of 16 threads |
| suspend/resume (`migrate file:` / `-incoming`) | **BLOCKED**: QEMU-WHPX migration blocker — `State blocked due to missing dirty memory tracking support, And some system register/state save-restore`. `savevm/loadvm` are blocked by the same blocker (and the vvfat ESP/raw pflash aren't snapshot-capable anyway). |
| memory balloon | **not available**: macOS has no virtio-balloon driver (`info balloon`: no device). |
| boot with 2 GB RAM | **FAIL**: `panic: userspace panic: boot task failure: restore-datapartition` — BaseSystem sizes its tmpfs Data volume from RAM. 4 GB works. |
So the WSL2 recipe here is "cold boot once per PC session (~16 s), keep warm (0.4 % CPU), attach in ~0.1 s". No resume-from-file until QEMU's WHPX backend gets dirty-page tracking (or a different VMM is used).

## X6 — File sharing — PASS (SMB for live mounts, tar/ssh for bulk)
Host share: `work/share` (Lua git checkout + 200 MB random blob). Results `logs/X6/results.txt`.

| method | setup | read | write | Lua build (onelua.c, -O2) | notes |
|---|---|---|---|---|---|
| guest local APFS disk (after copy) | — | — | — | **4.98 s** | baseline |
| tar \| ssh push (host → guest) | none | **30.3 MB/s** (lua+200 MB in 7.0 s) | — | — | fastest bulk path; slirp-bound |
| **SMB2** (impacket `SimpleSMBServer` on host :4445, `mount_smbfs -N //guest@10.0.2.2:4445/HOST`) | 0.93 s mount, no admin | 18.8 MB/s | 15.4 MB/s | 7.65 s in place | `git status` shows 111 spurious changes (mode bits via impacket) → use `core.fileMode=false` |
| WebDAV (wsgidav :8088, `mount_webdav`) | mounts in 0.72 s | **FAIL** | | | every access `Operation not permitted` in BaseSystem, as root and as dev |
| 9p / virtiofs | — | **N/A** | | | Windows QEMU build: `fsdev support is disabled`, no vhost-user-fs (guest *has* `mount_9p`/`mount_virtiofs` + AppleVirtIO) |
| *WSL2 reference* `/mnt/d` (9p) | — | 129.6 MB/s | — | 4.77 s (ext4: 4.68 s) | |

Pick: **SMB mount for editing/live projects, `tar|ssh` (or rsync once added) for bulk sync**, build on the guest's local disk. Ceiling is QEMU user-mode networking (~20–30 MB/s); a TAP/vmswitch NIC (needs admin) or virtiofs in a future QEMU-Windows would be the fix.
Windows Defender initially quarantined impacket (hack-tool signature); the user allowed it.

## X7 — GUI/GPU on demand — PARTIAL
| check | result |
|---|---|
| WindowServer off by default | yes (plist parked in `/System/Library/WSM/disabled/`) |
| start on demand: `launchctl bootstrap system …/com.apple.WindowServer.plist` | **PASS** — WindowServer runs (≈40–45 MB RSS), takes the VGA display (1280×800); `logs/X7/after-ws.png` |
| AppKit app on demand (`work/x4/guitest.swift`, swiftc in guest) | **PASS** — menu bar + window frame drawn (`logs/X7/window.png`); window contents not composited (software path, no accelerator) |
| stop again: `launchctl bootout system/com.apple.WindowServer` | PASS — back to headless |
| `ioreg -l \| grep ParavirtGPU` | 0 (expected: only `AppleBochVGAFB` on `-device VGA`) |
| Metal: `MTLCreateSystemDefaultDevice()` | **nil**, `MTLCopyAllDevices()` = [] → no Metal without a paravirt GPU |
| CPU CoreGraphics render + readback | PASS (RGBA 61 172 255 255 for (0.2,0.6,1.0)) |
| reims-vgpu PCI device under WHPX | **NOT RUN** — no prebuilt binaries in steelbrain/reims-vgpu or the Windows fork; needs the fork's QEMU (`vendor/qemu` submodule → tqmane/qemu-reims-vgpu) built in MSYS2 MINGW64 with Rust `x86_64-pc-windows-gnu`, LLVM + SPIRV-Tools on PATH, **Windows Developer Mode on** (meson symlinks). The fork's own AGENTS.md (2026-07-31) states no guest has attached to `reims-vgpu-pci` on the WHPX rail yet, and its Tahoe boot there was on an **Intel** host with vmware-svga. METAL_ERROR_MODE step skipped (no device to test). |
First broken layer for GPU: **the device itself isn't buildable here without the user enabling Developer Mode**; after that, the open risk is the fork's untested Tahoe/WHPX attach path (plus our AMD kernel patches, which the fork never needed).

## X8 — Darling comparison — SKIPPED (user request)
Partial work left in WSL Debian: release debs downloaded to `/opt/wsm-darling` (nothing installed — `darling-gui` needs Ubuntu-noble ffmpeg-6 libs missing on Debian 13).

---
## Summary
| exp | result | key numbers |
|---|---|---|
| X0 host/hypervisor | PASS | WHPX real: CPU loop 1.01 s vs TCG 8.59 s; Alpine boot 7 s |
| X1 dissect Tahoe image | PASS | APM+HFS+ installer; BaseSystem.dmg 1.37 GB → raw GPT/APFS 1.47 GB (2.25 GB files, dyld cache 914 MB) |
| X2 boot BaseSystem headless | PASS | 7 fixes incl. 3 AMD-on-WHPX kernel issues; kernel at 3.7 s, serial root shell 7.0 s |
| X3 minimal writable base | PASS | **cold boot → SSH 15.8 s**, 60 procs, guest RSS 362 MB, host 1.2 GB WS, idle 0.4 % CPU |
| X4 dev toolchain | PASS (workarounds) | CLT 26.6, C/Swift/git OK, `brew install hello` 52 s; bench run within 9 % of WSL2, compile faster |
| X5 instant attach | PARTIAL | warm SSH 119 ms; suspend/resume **blocked** by QEMU-WHPX; min RAM 4 GB; no balloon |
| X6 file sharing | PASS | SMB2 18.8/15.4 MB/s, tar\|ssh 30 MB/s (WSL2 9p: 130 MB/s) |
| X7 GUI/GPU on demand | PARTIAL | WindowServer + AppKit window on demand OK; Metal = nil; reims build not run |
| X8 Darling | SKIPPED | — |

## Changes outside this folder / cleanup
- **WSL Debian**: `apt install clang gcc libc6-dev` (remove: `sudo apt remove clang gcc libc6-dev && sudo apt autoremove`); `/opt/wsm-darling` (remove: `sudo rm -rf /opt/wsm-darling`); `apt update` refreshed package lists.
- **Windows Defender**: user allowed impacket (restore by removing that allow/exclusion entry in Windows Security → Protection history).
- Windows features: none changed (HypervisorPlatform/VirtualMachinePlatform were already on).
- Everything else lives in `D:\MyProjects\wsm` (tools/, refs/, work/ ≈ 60 GB incl. extracted SharedSupport 17.5 GB + 17.5 GB asset zip + data disk). Safe to delete `work/ss` (35 GB) now that the needed binaries are in the guest.

---
# Round 2 — GPU go/no-go + round-1 gaps

**Follow-up supersedes the original G1 RED verdict below:** G1 now meets the functional pass criteria on the RTX 4050 with aljus7 v0.3.0 and 4 GiB guest RAM. Practical verdict **YELLOW** because frame pacing is uneven. See “Round 2 follow-up” at the end for the working recipe and evidence. Original measurements below are retained as history.

Pinned sources (all under `tools/` or `refs/`): aljus7/reims-vgpu **v0.3.0** release ZIP (sha256 `4bb900b6…1cca`, source `09f13b3`); MAXZVER/reims-vgpu **master `d6f52d1`** + vendor/qemu **`319fddb`** (MAXZVER/qemu-reims-vgpu) + metal2vulkan **`d0414a2`**; PowerShell 7.6.8, MSYS2 (UCRT64, gcc 16.2, rust 1.99.1), rustup 1.99.1 + `x86_64-unknown-uefi` — all portable in `tools/`. Guest: macOS Tahoe 26.3 (25D125), installed from our ISO with `startosinstall` (installer image converted APM→GPT through a qcow2 overlay, `work/r2/media/`; ISO untouched). Scripts: `src/{reims_start.ps1,mz_start.sh,mz_oc.sh,merge_amd_oc.py,qin.py,shot.sh,tssh.sh,g4.sh,g5.sh,migpipe.py,apm2gpt.py}`, patch `src/patches/qemu-whpx-allow-migration.patch`.

## G1 — GPU on a full Tahoe install — RED (no accelerated desktop reached)
Install itself: PASS (Recovery boots on the AMD host with aljus7's stack once our AMD patches are merged; `startosinstall` 13 min prepare + restore stages; METAL_ERROR_MODE=5 written from Recovery; `dev` user + SSH + autologin added from Recovery).

Device attach on the **RTX 4050** (Windows graphics preference set; log: `device starting on Metal device NVIDIA GeForce RTX 4050 Laptop GPU`) — PASS on aljus7's build:
- `system_profiler SPDisplaysDataType`: **Apple Paravirtualized Graphics Device**, Metal Support: Metal 2, vendor 0x106b dev 0xeeee
- `ioreg`: `AppleParavirtGPUControl`, `MetalPluginName=AppleParavirtGPUMetal`; `kextstat`: `com.apple.driver.AppleParavirtGPU (15.0.0)`; WindowServer running
- `ReimsVgpu.kext`: NOT RUN (not needed to attach on 26.3; would be the next variable)

| stack | outcome | first broken layer |
|---|---|---|
| aljus7 v0.3.0 (their QEMU, their OpenCore + our AMD patches), installed Tahoe, 2 boots | `reims-vgpu: fatal … Vulkan device was lost`, QEMU exits; 0 frames presented | **guest-RAM import** — 232–256 no-copy imports (6.7–6.9 GB), then one GPU paging task took 6.6–17 s (>> 2 s Windows TDR). aljus7 reads no `GUEST_IMPORT*` variable, so import can't be switched off there. |
| MAXZVER master, defaults, import off | boot panics `pmap_pv_remove … null pv_list` / `pmap_recycle_page … is referenced` (guest memory corruption) | **QEMU lab memory paths** (`WP_VIEW`, dirty write-protect, RAM populate/resident, lockless MMIO) on this AMD host |
| MAXZVER master, those lab paths off (`REIMS_VGPU_LAB_DIRTY_WP=off QEMU_WHPX_WP_VIEW=off REIMS_VGPU_LAB_RAM_RESIDENT=off REIMS_VGPU_LAB_RAM_POPULATE=off REIMS_VGPU_LAB_READ_FAST=off QEMU_WHPX_MMIO_FAST=0`, import off) | boots, no panic, but black screen, 0 reims stats, SSH stalls at banner for >12 min, QEMU ≈3 host cores | **performance/first-boot stall** (cause not isolated) |

AMD-specific findings needed for MAXZVER QEMU on this CPU (each found by a boot, one change per boot): (1) OpenCore SYSENTER wrmsr patches still required (its WHPX fix doesn't cover our CPU → #GP after handoff); (2) `-accel whpx,hyperv=off` gives a real TSC leaf but leaves 1 of 8 vCPUs ("1 threads but 8 registered from MADT"); the working combination is default hyperv + OpenCore `Kernel/Quirks/ProvideCurrentCpuInfo=true` + `AMD_Vanilla` topology patches; (3) CPU model needs `-pcid,-tsc-deadline` (host lacks both; aljus7's default `enforce` refuses to start).
Step 4 (`GUEST_IMPORT_SURFACE_WRITES=off`, screenshots, GPU utilisation), step 5 Nouradawy/Jake forks, Metal pixel-readback (`work/x4/metaltest.swift` written, **NOT RUN** — no usable guest session), **step 6 Ventura baseline: NOT RUN** (no Ventura image/installer available here).

## G4 — Suspend/resume — PASS (headless VM, MAXZVER QEMU + 4 small changes)
Round-1 blocker removed. Needs: (a) `WSM_WHPX_ALLOW_MIGRATION=1` (skips the WHPX migration blocker; dirty tracking already exists in this fork); (b) restore writes **one TSC to all vCPUs** (saved per-vCPU TSCs were ~0.4 ms apart → guest panic `assertion failed: now >= *pkt_enq_time`, traced with `WSM_WHPX_TSC_TRACE=1`); (c) `-accel whpx,kernel-irqchip=off` (Hyper-V rejects restoring its APIC state: `WHvSetVirtualProcessorInterruptControllerState failed: c0350005`); (d) save through TCP → file (`migrate file:` fails on Windows: "Failed to set FD nonblocking").

| metric | value |
|---|---|
| save, 4 GB guest (migrate over TCP bridge) | **9.3 s**, state **1.07 GB**, downtime 0.1 s |
| load state | 3.5 s |
| restore → first SSH command | **4.1 s** (same session: `/tmp` marker present, same `kern.boottime`) |
| after resume | 0 panics; data/local volumes mounted; HTTPS to github.com 200 in 0.74 s |
| warm SSH after resume | 247–279 ms (round 1: 119 ms) |
| cost of `kernel-irqchip=off` | cold boot→SSH 23.3 s (vs 15.8 s), SSH round-trips ~2× slower |
Not done: `savevm/loadvm` with qcow2 internal snapshots; tested with a FAT-image ESP, qcow2 disks, no GPU device. Save/restore with the GPU device present: NOT RUN.

## G5 — Boot-time pass — no gain in boot→SSH (15.7–15.8 s); finding recorded
| change (one at a time, 3 boots each) | sshd ready | first SSH |
|---|---|---|
| round-1 baseline (reproduced) | 14.4–14.9 s | 15.8 s |
| ESP as FAT **image file** instead of vvfat | — | 15.8 s (kernel handoff 3.9→3.1 s) |
| static IP instead of DHCP | — | 15.8 s |
| sshd as its own LaunchDaemon (`/System/Library/LaunchDaemons`, not `/Library/…`; waits for writable root) | **9.4–10.9 s** | 15.7 s |
| `virtio-net-pci` instead of `vmxnet3` | 9.0–9.7 s | 15.7 s |
| static IP set when `en0` appears | 9.1–9.4 s | **13.2 s (1 of 3)**, 15.7 s (2 of 3) |
Cause: the NIC (`en0`) does not exist when launchd starts our scripts (`ipconfig set en0 … interface doesn't exist`); `ifnet_attach en0` happens at ~15 s regardless of NIC model, so nothing reaches the guest before then. LaunchServices-group trim: NOT RUN. (A first attempt put the plist in `/Library/LaunchDaemons`, which this base never reads → three boots timed out at 150 s; fixed.)

## G6 — File sharing — answer: no 9p/virtiofs in either Windows build
`fsdev support is disabled` (aljus7 build QEMU 11.1.50 and weilnetz 11.1.0); MAXZVER build not configured with it either (no `-fsdev`). virtio-net vs vmxnet3 throughput and `-netdev user` tuning: NOT RUN. Round-1 numbers stand (SMB 18.8/15.4 MB/s, tar|ssh 30 MB/s).

## G2 — GPU on the minimal base — NOT RUN (G1 did not pass)
## G3 — Full userland base — PASS as a *trimmed full install*; BaseSystem-style hybrid NOT achieved
**Hybrid (BaseSystem boot + full-OS `/usr` + full cryptex dyld cache):** blocked at step 1. The full OS cache alone is 2.0 GB (`dyld_shared_cache_x86_64h` + 6 subcaches + atlas), bigger than the whole 1.47 GB BaseSystem container, and the container cannot be grown: `diskutil apfs resizeContainer` → `-69606: A problem occurred while resizing APFS Container structures` (also when unmounted; `fsck_apfs` reports the container OK; the GPT already had room after the partition). Next step would be a new, larger container with the base volume copied in (`asr`/`ditto`) — NOT RUN.

**What was measured instead — the installed Tahoe 26.3 with 112 of 172 running system services disabled** (`launchctl disable system/<label>`, written to the Data volume's `disabled.plist`; sealed system volume untouched; WindowServer + autologin off). Overlay `work/r2/g3-full.qcow2` on the G1 install; ESP image + OpenCore with our AMD patches + `ProvideCurrentCpuInfo`; round-1 QEMU 11.1.0, 4 vCPU, VGA, vmxnet3.

| RAM | cold boot → SSH (dev) | processes | guest RSS sum | free after ~60 s | host QEMU WS |
|---|---|---|---|---|---|
| untrimmed full OS, 3 GB | 25 s | 387 | 2.25 GB | 139 MB (load avg 22) | — |
| **3 GB** | **17.2 s** (16.1 s on a second boot) | 166 | 1.07 GB | 1.18 GB | 3.1 GB, 7.4 % CPU (Spotlight still indexing) |
| **2 GB** | **16.6 s** | 164 | 1.06 GB | 261 MB | 2.1 GB, 4.2 % |
| **1 GB** | **18.4 s** | 164 | 0.83 GB | 38 MB (compressor 48 MB) | 1.1 GB, 6.3 % |
RAM floor: **1 GB boots** (2 GB comfortable) — round 1's 4 GB floor came from BaseSystem's RAM-sized tmpfs Data volume; a real Data volume removes it.

Round-1 workarounds, re-tested with none applied (`logs/G3/tools.txt`, `tools-brew.txt`):
| check | round 1 | now |
|---|---|---|
| `installer -pkg` CLT 26.6 (3 pkgs) | refused (free space on 1.4 GB root) | **PASS**, 223 s |
| clang / swift / git / SDKs | via `pkgutil --expand-full` + `ditto` | Apple clang 21.0.0, Swift 6.3.3, git 2.50.1, SDK 26.5; C + Swift compile and run |
| `brew` as non-root user | `/etc/passwd` hack, own APFS volume, `HOMEBREW_TEMP`, shims, own libffi | **PASS**: plain `git clone` into `/usr/local/Homebrew`, `dev` owns `/usr/local/*` (my first run forgot `/usr/local/var` → "Failed to install Portable Ruby"; fixed by ownership only) |
| brew bottle (`ca-certificates`, all-platform) | — | **PASS**, 32 s incl. Portable Ruby |
| brew source build (`hello`) | 52 s | **PASS**, 41 s (built in 31 s); Tier-3 warning for Intel Tahoe as before |
| `uptime`, `top`, `shasum` | broken (libsbuf, libpanel, Perl Fcntl) | work |
| `readlink diff patch cmp install du gzip file` | copied in from payload | all 8 present |
| scp / sftp | no `sftp-server` | work |
| non-root user via `dscl` | invisible to `getpwnam` | `dev` uid 501 created with `dscl -f` from Recovery, works for login, sudo, brew |
Disk: guest uses ~18 GiB (system 11 GiB incl. snapshot, Data 7.0 GiB incl. CLT 1.8 GiB). Host: install image `current.qcow2` 36.1 GiB (installer staging never trimmed; a `qemu-img convert` compaction would bring it near the guest's 18 GiB — NOT RUN) + G3 overlay 5.3 GiB. Round-1 base was 1.35 GiB + data volume.
**Trade-off:** full userland and 1 GB RAM floor vs ~13× the disk of the BaseSystem base; boot→SSH is the same ~16–17 s (both gated on NIC attach, see G5).

## Round 2 summary
| exp | result | key numbers |
|---|---|---|
| G1 GPU on full Tahoe | **RED** | device attaches on RTX 4050 (Paravirtualized GPU, Metal 2); no accelerated desktop: aljus7 → host GPU device lost (guest-RAM import); MAXZVER → guest memory corruption, or a stalled black screen with lab paths off |
| G2 GPU on minimal base | NOT RUN | needs G1 |
| G3 full-userland base | PASS (trimmed full install; hybrid blocked) | boot→SSH 16–17 s, **1 GB RAM floor**, CLT `installer` + brew bottle + source build with zero workarounds; ~18 GiB guest disk |
| G4 suspend/resume | **PASS** (headless) | save 9.3 s / 1.07 GB; restore → SSH **4.1 s** |
| G5 boot-time pass | no gain | sshd 4 s earlier, first SSH still 15.7 s: NIC (`en0`) attaches late |
| G6 file sharing | answered | no 9p/virtiofs in any Windows build |

1. **GPU verdict on this PC: RED.** First broken layer per stack — aljus7 v0.3.0: guest-RAM import (6.9 GB of no-copy imports, then a 6–17 s GPU paging task → Windows TDR → `Vulkan device was lost`; no switch to disable import in that build). MAXZVER master: QEMU lab memory paths corrupt guest page tables on this AMD host (`pmap_pv_remove … null pv_list`); with them off the guest boots but presents 0 frames and stalls (cause not isolated). Device attach, AppleParavirtGPU driver and Metal device enumeration are fine. Tahoe vs Ventura: Ventura baseline **NOT RUN** (no Ventura media here), so "host vs Tahoe" is not separated yet.
2. **Recipe to keep (best result reached):** aljus7 v0.3.0 release + our `merge_amd_oc.py` (SYSENTER + AMD_Vanilla topology/MSR patches) + CPU `-pcid,-tsc-deadline` + GPT installer overlay; Windows graphics preference = High performance (log confirms RTX 4050). This installs Tahoe and attaches the GPU; it does not yet give a usable desktop. For MAXZVER master `d6f52d1`/`319fddb`/`d0414a2` on this AMD host also add: OpenCore `ProvideCurrentCpuInfo=true` (WHPX reports TSC 0 otherwise; `hyperv=off` fixes TSC but strands 7 of 8 vCPUs), `REIMS_VGPU_LAB_DIRTY_WP=off QEMU_WHPX_WP_VIEW=off REIMS_VGPU_LAB_RAM_RESIDENT=off REIMS_VGPU_LAB_RAM_POPULATE=off REIMS_VGPU_LAB_READ_FAST=off QEMU_WHPX_MMIO_FAST=0 REIMS_VGPU_GUEST_IMPORT=off`, and drop `ICH9-LPC.acpi-pci-hotplug-with-bridge-support=off` (conflicts with its pci-bridge).
3. **Suspend/resume:** works on the headless VM with MAXZVER QEMU + `src/patches/qemu-whpx-allow-migration.patch` (`WSM_WHPX_ALLOW_MIGRATION=1`, equal TSC on restore), `-accel whpx,kernel-irqchip=off`, migration over TCP to a file (`src/migpipe.py`). Save 9.3 s → 1.07 GB (4 GB guest); restore → SSH 4.1 s; same session, 0 panics. Cost: kernel-irqchip=off makes cold boot 23 s and SSH round-trips ~2× slower. With the GPU device: NOT RUN.
4. **New base:** trimmed full install — 16–17 s boot→SSH, 1 GB RAM floor (2 GB comfortable), idle guest RSS ~1.07 GB / 166 procs, ~18 GiB guest disk (36 GiB host file before compaction). Workarounds gone: installer free-space refusal, pkgutil/ditto CLT, `/etc/passwd` user hack, brew volume/TEMP/shims, libffi/shasum shims, missing userland tools, no sftp. BaseSystem-style hybrid not built (APFS container won't grow).
5. **Changes outside this folder (and how to undo):**
   - `HKCU\Software\Microsoft\DirectX\UserGpuPreferences`, value `D:\MyProjects\wsm\work\reims\build\qemu\qemu-system-x86_64.exe` = `GpuPreference=2;` (High performance). Undo: Settings → System → Display → Graphics → remove that app, or `Remove-ItemProperty -Path HKCU:\Software\Microsoft\DirectX\UserGpuPreferences -Name 'D:\MyProjects\wsm\work\reims\build\qemu\qemu-system-x86_64.exe'`.
   - `C:\Users\iamhe\.claude\statusline-wsm.sh` edited (by the statusline agent, at the user's request) to show a round label from line 2 of `status.txt`.
   - Nothing else: Developer Mode not needed (release ZIP + MAXZVER tree copy symlinks), no admin/UAC, no reboots; PowerShell 7, MSYS2, rustup all portable under `tools/`. `install.ps1` ran with `-NoShortcuts` (no Start-menu/desktop links).
   - Disk: round 2 added 44 GB under `work/` (`work/reims` 37 GB incl. the 36 GiB Tahoe image; `work/r2` 7 GB: overlays, G4 state) and 7.6 GB under `tools/` (MSYS2 2.3, build tree 4.3, rustup 1.0).

---
# Round 2 follow-up — G1 recovered; G2 investigation

## G1 — functional PASS; practical GPU verdict YELLOW

The RTX 4050 **does work** with the existing Tahoe 26.3 (25D125) install. Keep aljus7/reims-vgpu **v0.3.0**, source **`09f13b32e0a438502409b282e2a6750ad50f879b`**, the release QEMU at `work/reims/build/qemu/qemu-system-x86_64.exe`, the existing AMD-patched OpenCore, and **4096 MiB guest RAM**. The successful writable guest is `work/gpu-followup/aljus-4g.qcow2`; retain its backing chain. Native Vulkan window enabled: `REIMS_VGPU_WINDOW=1 REIMS_VGPU_LOG=info REIMS_VGPU_STATS=1 REIMS_DEBUG=1`. Windows' existing High performance preference selects NVIDIA; the log confirms **NVIDIA GeForce RTX 4050 Laptop GPU**. Guest global and test-user `METAL_ERROR_MODE=5`.

| check | measured result |
|---|---|
| device/driver | Apple Paravirtualized Graphics Device, Metal 2; AppleParavirtGPU |
| Metal clear readback | PASS, corner and centre RGBA **51,153,255,255**, 7.7 ms |
| compiled Metal shader readback | PASS, corner and centre **255,128,0,255**, 11.2 ms; compile/pipeline 13.6 ms |
| command submission | 200 submits, **1.6 ms each** |
| visible desktop | Finder window, menu bar, icons and desktop rendered in native host window |
| frame rate | reached 60 Hz; welcome animation roughly 35–37 Hz |
| Finder dragging | 25.74 s, 27 rate samples: **mean 20.7 Hz, median 11 Hz, range 0–57 Hz**; visible stalls |
| repeatability | correct clear/shader readback again after guest reboot; no device-lost error in this successful run |

Evidence: `logs/G1-followup/aljus-4g/{metal.txt,native-desktop.png,drag-stats.json,drag-reims.log,gpu-counters.json,stderr.log,run.json}`. `work/x4/metaltest.swift` is the actual Metal test. The AppKit test's CoreGraphics readback alone does not count as a Metal pass.

Two earlier conclusions were too broad. First, the original 8 GiB run imported 6.7–6.9 GB and lost the host GPU; the successful 4 GiB run imported approximately 4.0–4.2 GB. Lower memory pressure is a plausible explanation, **not a clean isolated A/B proof**: native-window enablement and guest setup also changed. Second, QMP framebuffer dumps can freeze at wallpaper/boot text while the native Vulkan window renders correctly. Native-window capture is required to judge presentation.

The Recovery-created `dev` account lacked AuthenticationAuthority. SSH key login did not prove GUI authentication worked. Adding the standard ShadowHash authority, setting a policy-compliant disposable test password, regenerating `/etc/kcpassword`, and completing Setup Assistant made desktop login work. No Apple ID was used. Scripts: `src/gpu_repair_test_user.sh`, `src/gpu_followup.py`. Attempts to write Reduce Transparency/Motion defaults were rejected by the protected preference domain; **those settings are not claimed as applied**.

aljus7 v0.3.0 does **not** read either `REIMS_VGPU_GUEST_IMPORT*` switch. An early run metadata file contains `GUEST_IMPORT=off`, but that variable was ignored; the current launcher omits it for this stack. Do not advertise an import-off recipe for this build.

Reproduce on another disposable overlay (stop the current harness VM first):

```powershell
.venv/Scripts/python.exe -I src/gpu_followup.py start tahoe-working --stack aljus --base work/gpu-followup/aljus-4g.qcow2 --ram 4096
.venv/Scripts/python.exe -I src/gpu_followup.py probe tahoe-working --seconds 90
.venv/Scripts/python.exe -I src/gpu_followup.py metal tahoe-working
```

Use a new experiment name each time. The launcher refuses existing disks/log directories and occupied ports. Native capture helper: `src/gpu_window_capture.ps1`; QMP capture is useful for boot errors but is unreliable for the accelerated desktop.

## Alternative stacks and research controls

Research notes and primary-source links are in `logs/G1-followup/RESEARCH.md`. The [MAXZVER Tahoe issue](https://github.com/MAXZVER/reims-vgpu/issues/1) initially supplied a pinned control pair, then recommended matched master on October 19; the original master choice was reasonable. The [Nouradawy WHPX/Tahoe PR](https://github.com/steelbrain/reims-vgpu/pull/125) supplies additional fixes, but is not evidence that a different untested binary will pass here.

| candidate | exact pins / outcome |
|---|---|
| MAXZVER published Tahoe pair | reims `c34e6caf7971edde337331e4f06585e48a0fc63d`, QEMU `e92847870dea4760355889edac3d623af05a9223`, translator `c1fc19d539dbb299573192afb2a13a342b36f9ca`; built under `tools/gpu-followup/reims-vgpu`; import-off 6 GiB boots to SSH then `pmap_recycle_page` panic. Lazy-writeback-off also failed. This prevents blaming only newer master's experimental memory paths. |
| current ReimsVgpu.kext + Lilu control | isolated OpenCore image; host says **binary version 245 unavailable**, clear returns zero pixels, shader times out. Host/driver mismatch; not a fix for this old host build. |
| Nouradawy compatible feature pair | reims `3bd1f790ba515443b0422965d3b30a4160b88524`, QEMU `768b7a951fa581d221e4cd9b310392be23b80c9e`, translator `c1fc19d539dbb299573192afb2a13a342b36f9ca`, ABI 24; build PASS, VM test **NOT RUN** after aljus succeeded. Newer declared QEMU archive returned 404, so arbitrary revisions were not mixed. |
| Ventura baseline | Apple recovery downloaded into `tools/gpu-followup/ventura`; signed chunklist RSA verification and **68 chunks PASS**. SHA256 `c19bd12f5cb1651b87b74d04f02a636da762ea46b81c7ebc9f205fa2a976d599`. Installed Ventura / GPU comparison **NOT RUN**. |

Build changes were local Cargo dependency paths, an exact-source-matching GOP ROM reuse, Windows archive-link materialization, and a project-local log-directory backport. No new GPU architecture was written. Pin manifests: `tools/gpu-followup/{pins.json,nour-pins.json}`. Public Tahoe fixes and guest-driver versions must be matched to host ABI; don't add a current driver shim indiscriminately.

## G2 checkpoint — incomplete, saved for handoff

Built a larger BaseSystem-style hybrid in `work/gpu-followup/hybrid.qcow2` (20 GiB virtual). Copied the base plus full `/usr/bin`, `/usr/lib`, `/usr/libexec`, Frameworks, PrivateFrameworks, Extensions, cryptex dyld cache, matching full KernelCollections and Perl. Content occupies about **8.4 GiB per system copy**; broad starting layer, not a proven minimum. A second System-role copy was needed because role conversion after creation failed. Both staging copies remain; current container allocation therefore exceeds the intended final system size.

**No hybrid boot or G2 graphics pass yet.** Corrected an ESP config referencing missing SSDT-USBX.aml; corrected ESP boots the full G3 builder. The hybrid still stops at **`OCB: System has no boot entries`** before kernel, even with a blessed System-role volume. Next test is explicitly loading Apple's EFI APFS driver extracted from the pristine BaseSystem container. Extraction script `src/gpu_extract_apfs.py` was corrected from reversed signature `RDSJ` to actual `JSDR`; **corrected script NOT RUN**. Failed run `hybrid-explicit-apfs` exited immediately because the ESP hadn't been generated. Use a new run name after extraction succeeds.

**Resume instructions, exact UUIDs, commands, logs and pitfalls: [HANDOFF.md](HANDOFF.md).** Work paused at the user's checkpoint request. G2 boot time, GUI cold-start time, Metal readback and RAM floor are **NOT RUN**. Do not mark G2 PASS from successful disk copying/blessing.

## Revised closing summary at handoff

1. **GPU verdict YELLOW**, superseding original RED: full Tahoe desktop and correct Metal clear/shader readback on RTX 4050; uneven frame pacing (drag mean20.7Hz, median11Hz). Ventura installed comparison NOT RUN.
2. **Keep aljus7 v0.3.0 / `09f13b32e0a438502409b282e2a6750ad50f879b`, 4 GiB RAM**, native window1, logs/stats enabled, existing AMD-patched OpenCore, guest METAL_ERROR_MODE5, repaired GUI account. Successful disk `work/gpu-followup/aljus-4g.qcow2` plus backing chain. Import switches unsupported by that build.
3. **Suspend/resume unchanged:** headless save9.3s/1.07GB, restoreSSH4.1s; GPU suspend NOT RUN.
4. **Working base remains G3 trimmed full install:** ~18GiB,16–17sSSH,1GiB boot floor, previously recorded workarounds removed. New hybrid~8.4GiB content is built but **not boot-verified**; no new timing/RAM/workaround claims.
5. **Outside-folder changes this follow-up:** no new host graphics preference, Developer Mode, reboot or statusline edit. A legacy build appended to `D:\tmp\reims-vgpu-fail.log`; complete backup saved inside project, then its verified appended bytes removed with approval, restoring original4,950,369bytes. Prior round's graphics preference remains as documented above. Original disks preserved; all new test state is in project overlays.

# Round 2 follow-up (continued) — G2 PASS

## G2 — GPU on the minimal (BaseSystem-style) base — PASS, with a broad desktop layer

The hybrid (BaseSystem boot + full-OS layer, `work/gpu-followup/hybrid.qcow2`) now boots headless and starts an accelerated GUI on demand on the RTX 4050, using the same aljus7 v0.3.0 stack as G1. Final disk chain: `hybrid.qcow2` ← `hybrid-initfix.qcow2` ← `hybrid-gpu4.qcow2` ← `hybrid-gpu6.qcow2` ← **`hybrid-g2assets.qcow2`** (all overlays; `hybrid.qcow2` and the round-1 disks are unchanged).

Each blocker was found from logs and fixed with one change at a time:

| # | symptom | cause (evidence) | fix |
|---|---|---|---|
| 1 | `OCB: System has no boot entries` | The new APFS container has no usable EFI jumpstart driver; with ScanPolicy=0 OpenCore only found itself | Apple's APFS EFI driver extracted from the pristine BaseSystem container's `JSDR` jumpstart record (`src/gpu_extract_apfs.py`, 713,176 bytes, sha256 `bf5b343c14518873da305104e7de6d1ff83f7219315ec2cb73bfbabc6106660d`, `logs/G1-followup/apfs-driver.json`), loaded explicitly, `EnableJumpstart=false`. ESPs: `work/gpu-followup/hybrid-apfs-esp.img` (software rail), `hybrid-gpu-oc.img` (GPU rail). |
| 2 | GPU rail: init stops after `WSM-INIT /usr/local mounted`, no sshd (`hybrid-gpu1..3`) | The aljus OpenCore has **SIP enabled** (`csr-active-config=00000000`); the software rail uses 0x67. The hybrid's init/sshd state lives in `/System/Library/WSM` (writable only with SIP off). Switching the NIC (`--nic vmxnet3`) and moving `pmset` after sshd did **not** fix it. | `hybrid-gpu-oc2.img` = aljus OC + APFS driver + `csr-active-config=0x67`. SSH at 14 s guest uptime. (The `pmset`-after-sshd init change is kept but was not the fix.) |
| 3 | AppleParavirtGPU not loaded (34 kexts, no ParavirtGPU in ioreg) | XNU loads the pageable KC **by name** on a BaseSystem-style boot: `KextLog: Loading Pageable KC from file /System/Library/KernelCollections/BaseSystemKernelExtensions.kc`; it doesn't fall back to SystemKC (`loadKCFileSet for Pageable KC returned -603947003` when the file was moved away) | APFS clone of the full-OS `SystemKernelExtensions.kc` (built against the same BootKC) saved as `BaseSystemKernelExtensions.kc`; originals parked in `/System/Library/WSM/disabled/KernelCollections`. Then `com.apple.driver.AppleParavirtGPU (15.0.0)` loaded, with AppleParavirtAccelerator/Framebuffer/DisplayPipe instances. |
| 4 | Metal clear PASS but shader FAIL: `Unable to reach MTLCompilerService … lookup with error 3` | The hybrid's `/System/Library/xpc/launchd.plist` (BaseSystem's, 1,097 entries) doesn't list Metal's framework XPC service; the bundle itself is present | Full-OS 25D125 `launchd.plist` + `.sig` (sha256 `4392c278242dd937aaec5930ad772205caeedc7a2cf0b9df04ccdebdb96c8e92` / `75da5a654cfd5313fd5a589042ebd895fc085d9d9f604f32514261d6c52d9561`) copied from the G1 install (`work/gpu-followup/full-xpc/`); originals parked in `/System/Library/WSM/disabled/xpc`. Boot stays headless (59–61 processes, no WindowServer/loginwindow/Dock/Finder). |
| 5 | WindowServer + AppKit window: menu text shown as `?` boxes, window body not drawn | BaseSystem has only a font subset (58 files/93 MB vs 374/576 MB) and a reduced SystemAppearance.bundle (6.7 vs 25.7 MB) | Merged the full-OS `/System/Library/Fonts` and `SystemAppearance.bundle`. The CoreTypes.bundle (icons) merge hit a symlink conflict, so it stays partial. |

Harness fixes along the way: `src/gpu_followup.py` now calls qemu-img by absolute path (relative CreateProcess failed in this session) and has `--nic`. `src/gpu_hybrid_gui.sh` runs `script` with `</dev/null`; previously `script` swallowed the rest of the command file, so the AppKit step never ran. New: `src/gpu_g2_time.py` (boot→first SSH command from QEMU launch, 0.5 s polling).

### Measurements (fresh boot `hybrid-gpu8`, aljus GPU rail, 4 GiB; software rail `hybrid-sw-time`)

| check | result |
|---|---|
| root volume | `WSMHybridSystem` EAC96B20-97FB-4AA9-819D-75328B420F7D (BaseSystem-style, no sealed snapshot) |
| Metal (root, with on-demand WindowServer) | device "Apple Paravirtual device"; clear **51,153,255,255 PASS** (66 ms); compiled shader **255,128,0,255 PASS** (13.5 ms, compile+pipeline 650 ms); 200 submits 0.7–0.9 ms each (`logs/G1-followup/hybrid-gpu7/gui-run.txt`, `hybrid-gpu6/gui-run.txt`) |
| visible GUI | Native reims window shows the WindowServer surface, menu bar (Apple menu + "guitest" in system font) and the titled AppKit window "WSM on-demand GUI" with blue content: `logs/G1-followup/hybrid-gpu7/native-gui.png` (before the font fix: `hybrid-gpu6/native-gui2.png`) |
| GUI cold start | WindowServer kickstart **0.22 s**; request → AppKit window shown **3.41 s** (fresh boot, no Metal test in between) |
| headless boot → first SSH command | GPU rail **32.7 s** from QEMU launch (guest uptime 16 s; the rest is OVMF/OpenCore/reims GOP). Software rail **18.85 s** (round-1 base 15.8 s, G3 16–17 s), so about **3 s slower, not unchanged** |
| processes | 59–61 headless → 73 with WindowServer + app |
| idle RAM (active+inactive+wired) | software rail 0.93 GB; GPU rail 1.37 GB (wired 0.93 GB) of 4 GiB |
| system volume size | **9.07 GiB used** vs 1.19 GiB BaseSystem base → userland+desktop layer ≈ **7.9 GiB**. Largest: PrivateFrameworks 3.5 GiB, dyld cache 2.4 GiB, Frameworks 0.8 GiB, Fonts 0.64 GiB, Extensions 0.4 GiB, `/usr` 0.4 GiB, KCs 0.3 GiB |
| RAM floor for the hybrid | **2 GiB** (1 GiB panics in restore-datapartition); see G2 completion |

Not proven: that this layer is the *minimum*. It is a broad upper bound: whole Frameworks/PrivateFrameworks/Extensions/`/usr` copied. (Staging copy, icons, RAM floor and frame rate: see **G2 completion** below.)

Recipe to reproduce (PowerShell, from the project root; new NAME each run):

```powershell
.venv/Scripts/python.exe -I src/gpu_followup.py start NAME --stack aljus --oc D:/MyProjects/wsm/work/gpu-followup/hybrid-gpu-oc2.img --base work/gpu-followup/hybrid-g2assets.qcow2 --ram 4096 --attach-readonly work/wsm-data.qcow2
.venv/Scripts/python.exe -I src/gpu_g2_time.py NAME
.venv/Scripts/python.exe -I src/gpu_followup.py command NAME --user root --command-file src/gpu_hybrid_gui.sh --seconds 150
```

Pitfall: don't attach `aljus-4g.qcow2` to a hybrid run. OpenCore then boots the full install instead (`hybrid-gpu5`); that run was used deliberately as the source for the launchd cache. QMP `quit` on a GPU-rail VM can leave QEMU alive; check `run.json` for the PID and end only that process.

## Final closing summary (supersedes the handoff summary)

1. **GPU verdict: YELLOW.** On the RTX 4050, full Tahoe 26.3 and now the minimal hybrid both give correct Metal clear + shader readback and a visible accelerated GUI. Frame pacing is uneven on the full desktop (Finder drag mean 20.7 Hz, median 11 Hz). Ventura comparison NOT RUN.
2. **Recipe to keep:** aljus7/reims-vgpu v0.3.0 `09f13b32e0a438502409b282e2a6750ad50f879b` + its QEMU, 4 GiB guest, `REIMS_VGPU_WINDOW=1 REIMS_VGPU_LOG=info REIMS_VGPU_STATS=1 REIMS_DEBUG=1`, guest `METAL_ERROR_MODE=5`, AMD-patched OpenCore. For the hybrid, also: explicit Apple APFS driver, SIP 0x67, full SystemKC under the BaseSystem KC name, full-OS launchd.plist, full fonts/SystemAppearance. `REIMS_VGPU_GUEST_IMPORT*` is not read by this build.
3. **Suspend/resume:** unchanged. Headless save 9.3 s / 1.07 GB, restore → SSH 4.1 s. GPU-guest suspend NOT RUN.
4. **New base:** the BaseSystem-style hybrid boots headless to SSH in 18.85 s (software rail) / 32.7 s from launch (GPU rail). Its system volume is 9.07 GiB including the desktop layer, and GUI on demand comes up in 3.4 s. The G3 trimmed full install (≈18 GiB, 16–17 s, 1 GiB floor) remains the measured headless base. Hybrid RAM floor and a minimal layer set are NOT RUN.
5. **Outside the folder:** nothing new in this session. No graphics preference, Developer Mode, driver, registry or reboot changes. The round-1 NVIDIA "High performance" preference for QEMU remains (undo: Settings → System → Display → Graphics → remove the qemu-system-x86_64.exe entry). The earlier `D:\tmp\reims-vgpu-fail.log` restore stands.


## G2 completion — remaining items closed

Final hybrid disk: **`work/gpu-followup/hybrid-final.qcow2`** (overlay on `hybrid-g2assets.qcow2`), same OpenCore images as above.

| item | result | evidence |
|---|---|---|
| staging copy removed | Non-role `WSMHybrid` (UUID 5363448F-62A5-4F78-908D-FB40803CB209, verified before deleting) deleted with `diskutil apfs deleteVolume`. Container in use **10.1 GB** (was ~19 GB: two 9 GB copies). The System volume and Preboot are intact, and the hybrid still boots (GPU rail SSH 34.4 s, software rail 19.3 s). The host qcow2 files don't shrink without a convert/compact (not done). | `logs/G1-followup/g2-finish-full2/` |
| icons | Full-OS `CoreTypes.bundle` copied (**1226/1226 files**); BaseSystem copy parked as `/System/Library/WSM/disabled/CoreTypes.bundle.base`. System volume now **9.28 GiB used** → layer ≈ 8.1 GiB over the 1.19 GiB base | `logs/G1-followup/g2-icons2/` |
| RAM floor (headless, software rail) | **2 GiB PASS** (SSH 19.26 s, 32,742 free pages, 1 GiB tmpfs Data). **1 GiB FAIL**: `panic … boot task failure: restore-datapartition` (BaseSystem tmpfs Data). Floor = **2 GiB** (1.5 GiB NOT RUN). | `logs/G1-followup/hybrid-ram2048/`, `hybrid-ram1024/serial.log` |
| frame rate on the hybrid | 800×500 AppKit window recoloured every 1/60 s (`work/gpu-followup/anim.swift`), reims `present` stats on the RTX 4050. **Warm (second WindowServer session): mean 56.3 Hz, median 58 Hz, range 41–60 Hz** over 24 one-second samples; per-second max gap median 35 ms, worst 117 ms. **First WindowServer session after boot: 0 presents/s for the whole 20 s run** (the app itself ticked 54/s), with reims GPU-paging stalls up to 2,046 ms and 32 MB no-copy imports of 50–100 ms each; frames flowed only afterwards. | `logs/G1-followup/hybrid-fps/{fps-summary.json,fps-stats.log,anim-a.png,anim-b.png,stderr.log}` |

The simple-window frame rate is GREEN-class once warm, but the first GUI session stalls badly, and the full desktop (G1 Finder drag median 11 Hz) is still uneven. **Overall GPU verdict remains YELLOW**, with the named causes: cold-start guest-memory import/paging stalls and uneven pacing under full-desktop load. Workaround to try next: start WindowServer once at boot (hidden) so the first user-visible session is warm. NOT RUN.

Updated closing points: (4) hybrid base **9.28 GiB system volume, 10.1 GB container, headless SSH 18.9–19.3 s, RAM floor 2 GiB** (G3 full-install floor 1 GiB), GUI on demand 3.4 s. Items 1, 2, 3 and 5 are unchanged.


# Round 3 — production v0.1

## P1 — Unified QEMU — PASS

One QEMU: aljus7/reims-vgpu v0.3.0 `09f13b32e0a438502409b282e2a6750ad50f879b` + its QEMU `7100064d13b42bb1be16671fcf18435822b13c3c` + three WSM patches, built by `src/build/build-qemu.sh` with the project-local MSYS2/rustup. `qemu-system-x86_64.exe` sha256 `92c47deb590642ec0ff7be7725a5b30fe59ef5cef00d348c270c07c396b138a1` (record in `work/r3/qemu-v030/build.json`).

| patch | reason | sha256 |
|---|---|---|
| `qemu-whpx-allow-migration.patch` | allow WHPX migration + one TSC for all vCPUs on restore (round 2 G4) | `09b86ae9…` |
| `qemu-whpx-apic-restore.patch` | restore the in-kernel APIC from Hyper-V's own page plus guest-controlled registers; the zero-built page was rejected with `c0350005` and QEMU called `abort()` | `fde3b901…` |
| `qemu-whpx-resume-aps.patch` | after loading an incoming VM, clear `InternalActivityState`: the pre-load reset leaves halted APs in StartupSuspend (wait for SIPI), so APs 1–7 never ran again | `27dcb05f…` |

| acceptance item | result | evidence |
|---|---|---|
| (a) GPU Metal readback on this binary | **PASS** (clear + shader), `gpu1` 4 GiB, cold boot → SSH 34.8 s | `logs/r3/p1-metal-patched.txt`, `logs/r3-p1-metal.txt` |
| (b) headless suspend/resume via `wsm` | **PASS**: save 17.5–19.2 s, state 1.76–2.0 GB (2 GiB guest, 8 vCPU); **resume → QEMU/serial 3.0 s, → SSH 5.3 s**; same `kern.boottime`, `/tmp` marker present, 0 new panics | session log below |
| (c) GPU-instance suspend | **Not possible, by design of the device**: QMP `migrate` → `Migration state blocked by apple-gfx display device`. v0.1 rule: `wsm stop` on a GPU instance shuts down cleanly (9.8–11 s) and the next start cold-boots. | `logs/r3/p1-gpu-migrate-blocker.txt` |
| `kernel-irqchip` | `on` (default) is the only mode that boots on this QEMU (`off`: no SSH in 90 s, 8 vCPU). Suspend now works with `on`, so `off` is not needed. | `work/r3/try-irqon8.txt`, `try-vcpu8.txt` |

### Root causes found this round (each by reading the source or the live process, not by retrying)
1. **Save hung for 90 min**: QEMU on Windows never closes the migration socket after `completed` (`logs/r3/p1-save-trace2.txt`: no EOF within 60 s, VM in `postmigrate`). `suspend::save` waited for EOF without a timeout. Fix: drain until QMP reports `completed`, then stop at the first 1 s idle read. Thread dump of the stuck QEMU (no migration thread alive): `logs/r3/p1-stuck-save-bt.txt`.
2. **Resume took 123 s**: same socket behaviour on the incoming side; `suspend::feed` waited for close. Fix: keep the stream open and watch QMP for the end of the load.
3. **Resume aborted** with `WHvSetVirtualProcessorInterruptControllerState failed: c0350005` → patch 2.
4. **Resumed guest froze** (SSH banner timeout; `info registers -a`: CPUs 1–7 `HLT=1` with fixed RIPs, CPU 1 IRR=222 pending; `logs/r3/p1-resume-hang-cpus.txt`; next boot reported `mp_kdp_enter() timed-out on cpu 0`) → patch 3.
5. The aljus QEMU's `whpx_log_sync` marks all RAM dirty (no `WHvQueryGpaRangeDirtyBitmap`, which MAXZVER's fork has). Harmless for suspend because the VM is stopped first, so one pass is the whole state; it would matter for live migration only.

### Known issues (named causes)
- **Continuing a VM after a save** (the "suspend failed, leave it running" fallback) can panic the guest: `NMIPI for unresponsive processor: TLB flush timeout` (`logs/r3/p1-cont-panic.txt`). Cause: under WHPX the guest TSC keeps running while paused, so a CPU waiting on a TLB-flush ack sees the pause as a timeout. Restore is not affected (saved TSC is written back).
- Right after a resume the test guest showed load average 16–19: `iconservicesagent` at 228 % CPU (icon cache rebuild on the G3 image). Add to the P3 trim list.
- Save of a 2 GiB guest takes ~18 s (round 2 MAXZVER: 9.3 s for 4 GiB). Not yet profiled.

Fixed in the CLI on the way: `wsm stop` on a GPU instance printed `suspended: …, 0 MB`; it now says it shut down.

## P1 follow-up — known issues fixed at the root

| issue | root cause (how found) | fix | result |
|---|---|---|---|
| guest panics after `stop` + `cont` (`TLB flush timeout`) | WHPX keeps partition time (TSC) running while QEMU is paused; QEMU already uses `WHvSuspendPartitionTime` while writing the TSC, never on pause (source) | `04-qemu-whpx-pause-time.patch`: suspend partition time in the VM-stop handler; it resumes when a vCPU runs | 3 × (8 busy loops, 30 s pause, cont): 0 panics, same boot; each pause adds exactly 30 s guest-clock lag |
| guest clock behind by the whole suspend time (e.g. 1,477 s) | correct VM behaviour once time is frozen; a Mac re-reads its RTC on wake | `wsmd` sets the guest clock from the host once SSH answers after a resume | offset 0–1 s right after resume |
| save of 2 GiB took 14–18 s | per-thread CPU sampling: one QEMU thread 100 %; QEMU's Win32 `writev` calls `send()` once per iovec element = once per 4 KiB page; loopback TCP ≈ 33.5k pages/s (QEMU's own `pages-per-second`). `max-bandwidth` changes nothing (`logs/r3/p3-save-bandwidth.txt`, `p3-save-threads.txt`) | migration over an AF_UNIX socket file (next row) | **save 1.4–1.9 s** (migrate 1.3–1.7 s, fsync 0.1–0.2 s), state 1.4–1.6 GB |
| serial console (passwordless login), QMP and the migration stream on unauthenticated loopback TCP | any local process/user could get a shell, control the VM, read its RAM on save or inject state on resume | QMP, serial chardev and migration now use AF_UNIX socket files in the instance dir (`crates/wsm-core/src/afunix.rs`, Winsock FFI, no new crate); `wsmd` sets the instance dir ACL to owner + SYSTEM only on create/start. Only the key-only SSH forward stays on TCP. | QEMU's TCP listeners: only `127.0.0.1:<ssh>`; `qmp.sock` ACL = SYSTEM + owner |
| new instances got `accel = "whpx,kernel-irqchip=off"` | stale default from round 2; on this QEMU irqchip=off doesn't boot | default `whpx,hyperv=off` | — |
| `wsm-firstboot.sh` parsed JSON with `/usr/bin/python3` | that is a CLT-installer stub on a base without CLT | `plutil -extract … raw`; user/hostname validated; `_` in instance names becomes `-` in the hostname | not yet run in a guest (P3) |

**One corrupted resume** (`pmap_recycle_page: page … is referenced` in `iconservicesagent`, `logs/r3/p1-afunix-resume-panic.txt`) happened on the first AF_UNIX cycle right after a cold boot. Tracing the stopped vCPUs' event registers showed **an undelivered exception in 9 of 10 busy saves**: always #PF (vector 0x0e, error code 4/6/7, CR2 = user address) parked in `WHvRegisterPendingEvent`, which QEMU's WHPX code does not migrate (`/* Interrupt / Event Registers - Skipped */`). With it dropped, 10/10 busy cycles still continued the same session (`logs/r3/stress-busy-1.txt`): the faulting instruction re-executes and faults again, so the loss is usually benign. `05-qemu-whpx-migrate-events.patch` now carries it like KVM's vcpu events (pending → `cpu_pre_save` → injected + CR2 → `whpx_inject_exceptions` with CR2 as the parameter).

Test-method fix: `kern.boottime` is wall clock − uptime, so the clock resync shifts it; "same session" is now `kern.bootsessionuuid` + a `/tmp` marker (`tests/stress_suspend.sh`).

Patches are now numbered (`src/patches/NN-*.patch`) and applied as a series after resetting the touched files to the pinned commit (`build-qemu.sh`): they stack, so a per-patch "already applied" check can't work. Verified: the 5 patches on a clean tree reproduce the built sources exactly.

**`05-qemu-whpx-migrate-events.patch` verified:** 10 busy suspend/resume cycles (8 guest loops of `ls -R` + 64 MB `dd` churn), 1–7 undelivered #PF events carried per save, **10/10 same session (`kern.bootsessionuuid` + marker), 0 new panic reports**; save 1.8–3.6 s, resume → SSH 2.2–3.1 s (`logs/r3/stress-busy-events.txt`).

Two host-side hangs found on the way, both now fixed at the cause:
- **Headless guests went to ACPI S3** (`ACPI SLEEP … acpi_sleep_kernel hib=0` on serial): macOS idle-sleeps a VM with no user activity and the NIC goes with it, so every SSH client hangs. Only the GPU profile disabled S3/S4. Now `ICH9-LPC.disable_s3=1 disable_s4=1` for every instance (unit-tested); the P3 base also sets `pmset sleep 0`.
- **`wsm start` could block for minutes** (seen 3 min and 5 min 18 s): QEMU user-net accepts the forwarded TCP connection before the guest's sshd listens, so `ssh` waits for a banner indefinitely (`ConnectTimeout` is already satisfied). `ssh::ready` now kills a probe after 8 s.

## P6 — first full run (instance `hl1`: round-2 trimmed G3 image, 2 GiB, 8 vCPU) — `logs/r3/p6/p6-20261024-*.json`

| # | item | result | target |
|---|---|---|---|
| 1 | cold boot → QEMU up / → SSH (median of 5) | 0.3 s / **33.2 s** (32.8–34.0) | serial shell ~7 s; SSH ≤20 s → **FAIL, investigating** |
| 1 | cold boot → serial shell | NOT RUN (needs the P3 base's `com.wsm.serial`) | |
| 2 | suspend (median of 5) | **1.7 s**, 1.62 GB | |
| 2 | resume → SSH (median of 5) | **2.0 s** (1.8–2.3) | ≤5 s **PASS** |
| 2 | same session / clock offset after resume | 5/5 / ≤2 s | **PASS** |
| 3 | warm `wsm exec true` (median of 20) | **1,966 ms** (809–7,063) | round 1: 119 ms → **FAIL, cause below** |
| 4 | clang C hello / swiftc hello / `git clone --depth 1 jq` | PASS 89 s / PASS 104 s / PASS 6.1 s | times inflated by guest load |
| 4 | brew | NOT RUN (no Homebrew in this image) | |
| 5 | GPU | NOT RUN in this run (Metal PASS earlier on the same binary, P1) | |
| 6 | 10× start/suspend/resume | **10/10**, 0 panics (`tests/stress_suspend.sh --busy`) | **PASS** |
| 6 | 1 h idle with suspend-on-idle; 30 min GUI | NOT RUN | |
| 7 | idle host CPU (QEMU, 30 s) / QEMU working set | **29 %** / 2,123 MB | **FAIL, cause below** |

**Cause of items 3, 4 timings and 7:** the guest is busy, not the transport. Right after boot/login the top processes are per-user agents: `iconservicesagent` (6 CPU-min in 6 min), `intelligenceplatformd`, `assistantd`, `itunescloudd`, `callservicesd`, `accountsd`, `familycircled`, `amsaccountsd`, `CommCenter`, … (`logs/r3/p6-guest-busy.txt`). Round 2's trim covered only system daemons (`launchctl disable system/…`); an SSH login starts the user's own launchd domain (session type `Background`). Fix in progress: a per-user trim (`disabled.501.plist`) for headless instances (GUI instances need some of these, e.g. icons).

**Root cause of the P6 load (found from the guest's unified log, not by trimming more services):** `loginwindow` logged `[MacBuddyX] System requires license: 1 … Setup launch reason: terms and conditions` and launched `SetupAssistantSpringboard` as `_mbsetupuser`. That GUI session starts WindowServer on demand over IPC, so round 2's `launchctl disable com.apple.WindowServer` never applied (`launchctl blame` → `ipc (mach)`). The trigger is the installer's `/private/var/db/.AppleSetupTermsOfService` cookie, which survives `.AppleSetupDone` (`logs/r3/re-minibuddy-log.txt`, `re-license-log.txt`).
- Test: rename cookie, `sync`, clean shutdown, cold boot (`logs/r3/re-cookie-test2.txt`): `System requires license: 0`, no Setup Assistant/`_mbsetupuser` processes, **load 1.78 at 90 s (was 14.8)**, **`wsm exec true` 440–454 ms (was 0.7–4.3 s)**. A first attempt was invalid: `wsm stop --kill` dropped the unflushed rename.
- The P3 builder's inject stage removes the cookie. The per-user agent denylist (`guest/launchctl-user-disable.txt`) stays as a secondary trim for headless instances.
- Remaining gap: 0.45 s per `exec` vs round 1's 0.12 s (a full SSH login each time, Windows OpenSSH has no connection multiplexing). NOT yet investigated.

## Round 3 continuation — 2026-10-25 (work in progress)

**Scope:** the user reaffirmed that no installation, registry/PATH change or runtime deployment outside
`D:\MyProjects\wsm` is allowed. Downloads remain in `tools/`; development runtime remains in
`work/r3/home`. WinFsp and SSHFS-Win are absent; their driver installation is not authorized, so drive
mount acceptance is **NOT RUN**. No driver/Windows feature/graphics preference changes were made.

### Confirmed evidence

| item | measured result | evidence |
|---|---|---|
| P2 source comparison | GitHub `main` and `v0.3.0` both resolve to `09f13b32e0a438502409b282e2a6750ad50f879b`; no distinct candidate to compare, retain the pin | `git ls-remote origin refs/heads/main refs/tags/v0.3.0`, 2026-10-24 |
| persistent SSH pool, named-pipe CLI transport | 1,048,576 binary bytes preserved; stdout/stderr separated; remote exit 7 preserved; 4 concurrent clients pass | `logs/r3/p4-transport.json`, `tests/p4_transport.py` |
| warm `wsm exec true` | **46.06 ms median, n=20** on legacy headless instance `hl1` | same transport result; not yet the new-base P6 run |
| headless resume through named-pipe CLI | **2.1 s** to SSH; subsequent save **2.1 s**, state 1,572 MB | transport smoke run on `hl1` |
| first generic image candidate | **17,266,442,240 bytes (17.27 GB)**; fresh instance reaches SSH; SIP **enabled** | `base-tahoe-26.3-fd62cbf42ae6.qcow2`, `logs/r3/p3-inject-8.txt` |
| Rust checks | 13 unit tests PASS, 1 real-VM test explicitly ignored; Clippy `--all-targets -- -D warnings` clean | current source; CLI real-VM transport checked separately |

### Causes established while completing the builder

1. The earlier successful-build claim was invalid. Recovery has a `loginwindow`; later installer boots
   drop `autoinstall` while still running ramrod with `-restore`/`-rootdmg-ramdisk`, followed by
   `-bsdmgroot`. The builder now rejects all observed installer boot modes, with a regression test.
   The interrupted target was resumed using its original installer NVRAM (no erase), reached a normal
   OS boot, and Recovery then verified the OS snapshot before injection. A clean full unattended
   rerun of the corrected pipeline is **NOT RUN**, so reproducibility is not yet PASS.
2. An AHCI timeout did recur on the earlier stage-2 retry (`logs/r3/stall-evidence.txt`), disproving the
   one-off explanation. The subsequent observed install/repair did not hit it. Read-only QMP block,
   AHCI register and interrupt capture is in `crates/wsm-core/examples/watch_install.rs`; its occurrence
   mechanism is still unproven. Do not attribute it to a particular QEMU or host-storage bug yet.
3. The guest builder script lives inside BaseSystem. Stage 3 now refreshes that script, fails on guest
   command errors, verifies a system snapshot, removes installer staging and credentials, and zeroes
   free APFS extents with QEMU zero-detection/unmap before compaction. First candidate compaction stayed
   under the 20 GB target. Earlier "27-minute build" timing included an incomplete installer, and is
   not a valid full-build timing.
4. First-instance provisioning reached SSH but did not finish: SIP/TCC rejected root's attempt to write
   the user's `com.apple.universalaccess.plist` (`Operation not permitted`, guest firstboot log).
   Accessibility settings have been moved to the GUI-user helper; headless provisioning no longer
   depends on those protected preferences. The corrected image is building in
   `logs/r3/p3-inject-9.txt`. Fresh-instance completion/export/import tests remain pending.
5. The new SSH library defaulted to WinCNG even with vendored OpenSSL enabled; that backend could not
   negotiate this guest's key algorithms. Explicit `openssl-on-win32` selects the bundled OpenSSL
   backend. The stream test above then passed. Versions are fixed in Cargo.lock (ssh2 0.9.6).
6. The generic named-pipe wrapper used 512-byte buffers and did not support IO deadlines. Large exec
   frames stalled. WSM uses local-only pipes with owner+SYSTEM ACLs and 1 MiB buffers, caps writes, and
   uses PeekNamedPipe plus nonblocking IO for deadlines. The complete binary/concurrency test passes
   after this correction. Named-pipe library pin: interprocess 2.4.7.

Package scripts, Intel OpenCore template and opt-in CLT installation are implemented but their remaining
acceptance is **NOT RUN**. No final v0.1 production claim is made by this progress entry.

## Handoff snapshot - 2026-10-25

The user requested a Markdown handoff; implementation stopped for transfer. HANDOFF.md is the current
operational entry point, ROUND3_TASK.md preserves acceptance criteria. This supersedes stale progress above.

- Current base: **base-tahoe-26.3-7f70d08f4532.qcow2, 17,257,791,488 bytes**, read-only with checksum,
  manifest and ISO hash. Rejected fd62 candidate was removed after checking dependencies.
- Current firstboot **does finish**: latest inspection found the marker, launchd exit 0/runs 1, SIP enabled.
  Latest P3 failure is a readiness race: SSH true returns before completion. The suggested readiness
  completion guard is NOT IMPLEMENTED. Do not rebuild the OS for this host/test race.
- Named-pipe binary/concurrency test passes; warm exec median 46.06 ms/20 on legacy hl1, not final P6.
- Captured CLI auto-start hung because detached wsmd inherited the caller's output pipe. Clearing
  standard-handle inheritance fixed it: **p4-launcher.json PASS, 0.128 s**. Fix compiled and tested.
- D: filled during compaction (1.3 MB free). QCOW2 source checks remained clean. Cleanup stayed inside
  project, retained source/ISO/working disks. Builder target repacked 38.44 GB -> 21.45 GB after a complete
  virtual-disk comparison returned **Images are identical**. Last free-space check before compression:
  52.47 GB; remeasure current/final disk use. New preflight measures live qcow2 allocation plus headroom.
- Preview ZIP exists (50,606,272 bytes/235 files, no base) but is STALE: predates capture-handle fix and
  newest portable-uninstall source. New uninstall source is NOT BUILT/TESTED; package QA NOT RUN.
- Full new-base P6, real-duration stability, GPU/prewarm, clean unattended complete builder rerun and
  final packaging qualification remain NOT RUN. No production/P3/P7 PASS is claimed.
- No host installation, PATH/registry/driver/feature/Defender change was made. Mount remains NOT RUN
  under the user's explicit no-install constraint. See HANDOFF for exact cleanup and remaining actions.

## Round 3 continuation 2 - 2026-10-25 (stopped at the user's request)

Status line: P1 REVALIDATE | P2 PASS | P3 build FAIL (cause found, fix built, rebuild NOT RUN) | P4 RUN | P5 PARTIAL | P6 PARTIAL | P7 NOT RUN.
Every number below was measured this session; anything not listed is NOT RUN. No base image exists right now (old one deleted for the rebuild).

### P3 generic base (base 7f70d08f4532, since deleted)

| item | result | evidence |
|---|---|---|
| readiness race (SSH up before firstboot done) | fixed: `ssh::READY_PROBE` also requires `/var/db/wsm/firstboot.done` when the firstboot plist exists | `crates/wsm-core/src/ssh.rs` |
| two fresh instances | **PASS**: distinct MAC, SMBIOS serial/UUID/MLB, SSH host key, authorized key, hostname; user dev/501; SIP enabled; password/kbd auth off | `logs/r3/p3-generic.json` |
| first boot incl. provisioning | 51.0 s / 51.7 s | same |
| export -> import -> create | **PASS**, sha256 verified, base unchanged after both boots | same |
| full unattended rebuild from ISO | **FAIL** at stage 2 after 5.4 min: `startosinstall exit 138` after AHCI `CommandTimeout` on ports 1-3 | `logs/r3/p3-build-full.txt`, `work/r3/home/build/tahoe-26.3/install/serial.log` |

**Root cause of the recurring AHCI timeout (proven from a live capture, `logs/r3/stall-capture-full.txt`):**
AHCI `IS=0x0000000e` (ports 1-3 completed, commands cleared), interrupts enabled, INTx on IOAPIC pin 22
(level, vector 86, unmasked) with **IRR=1, Remote IRR=0 and nothing delivered**. That state needs a race:
`whpx_vcpu_run` runs without the BQL and its `WHvRunVpExitReasonX64ApicEoi` exit calls `ioapic_eoi_broadcast()`
without retaking it, while device interrupts call `ioapic_set_irq()` on the main loop under the BQL.
Fix: `src/patches/06-qemu-whpx-eoi-bql.patch`. QEMU rebuilt with patches 01-06
(`logs/r3/build-qemu-p06.log`, pin `work/r3/qemu-v030/build.json`, qemu-system-x86_64.exe sha256
`d077a713c28e2c78ba3995f217a982f778e87b60d70962c7a9a45390f102983c`); previous binaries in
`work/r3/qemu-v030/p05-archive/`. The race could also hang running instances under disk load.

**Patch-06 validation is incomplete:** hl1 (legacy image), 10 busy suspend/resume cycles: 9/10 same session,
1 guest panic (`Taking non-sleepable RW lock with preemption enabled`, APFS lookup in `ls`). Patch 05 had 10/10.
1/10 vs 0/10 is not a verdict; an A/B of 20 cycles each (p06 vs archived p05 binary) is NOT RUN. A state saved
by the p05 binary and restored with p06 also panicked (`pmap_recycle_page`), but that mixes binaries and proves nothing.

### P6 on the generic base (basecheck, 2 GiB) - `logs/r3/p6/p6-20261025-014018.json`

| # | item | result | target |
|---|---|---|---|
| 2 | suspend (median 5) | **1.4 s**, 1,838 MB | |
| 2 | resume -> SSH (median 5) | **1.9 s**, 5/5 same session, clock offset <=1 s | <=5 s **PASS** |
| 3 | warm `wsm exec true` (median 20) | **39.4 ms**, 0 failures (legacy image: 1,966 ms) | **PASS** |
| 4 | clang / swiftc / git clone | PASS 9.0 s / 50.1 s / 3.4 s | |
| 4 | brew | installed (`logs/r3/dev-setup.json`: 3 pinned CLT pkgs + Homebrew 51da7f2); suite said NOT RUN because of the PATH bug below (fixed); brew install tests NOT RUN | |
| 7 | idle host CPU / QEMU working set / instance disk | **0.83 %** / 2,127 MB / 4.56 GB (base 17.26 GB) | |
| 1 | cold boot -> SSH | 4 vCPU 23.6 s; **8 vCPU 20.6-21.1 s** | <=20 s: close, not met |
| 1 | cold boot -> serial shell | 21-30 s (works now) | ~7 s: not achievable in multi-user mode, see below |

Causes found from the guest and host:
- Serial getty used `/dev/tty.serial1`, which no driver creates (launchd exit 78, 22 respawns). Now `/dev/console`
  (works). It starts ~27 s after QEMU start because Background Task Management gates third-party LaunchDaemons.
  Round 1's 7 s was single-user mode. Instant start = resume (1.9 s).
- SSH banner at ~21.6 s (DHCP bound 16.7 s, sshd spawned 17.0 s). Our probe added up to 8 s because QEMU accepts the
  forwarded TCP before the guest answers: a 1.5 s banner check now gates it. Boot is CPU-bound: default vCPUs =
  host threads/2 (2..8). OpenCore logging off: kernel starts 0.5 s earlier.
- `wsm exec` PATH lacked /usr/local/bin (non-login zsh): base now writes `/etc/zshenv` path_helper (verified live).
- TCC blocks SSH sessions from ~/Documents, ~/Desktop and network volumes: builder now writes the "full disk access
  for remote users" grant offline and fails if 4 grants are not written. NOT yet verified in a booted base.

### GPU (g1 on base 7f70, RTX 4050) - `logs/r3/p6/gpu-20261025-023206.json`

| item | result |
|---|---|
| Metal clear + shader readback | **PASS** (3 runs) |
| first session WITHOUT prewarm | `gui start` 33.6 s, then 0 presents for >32 s |
| first session WITH new prewarm | `gui start` **0.7 s**, frames after 1 s, then ~60 fps |
| warm frame rate | **median 59.95 fps**, min 37 |
| window drag (QMP tablet, 6 cycles) | median 12.5 Hz, range 0-33: still uneven (round 2: 11 Hz) |
| device lost / fatal | 0 |

The stall is per user session. `wsm-gui prewarm` now logs the desktop session in hidden at boot and keeps it;
`gui stop` hides the window; `gui logout` ends the session. Static wallpaper via NSWorkspace verified.
reduceTransparency/Motion are TCC-protected even for root: NOT verified with the new grant.

### Host integration (P5)

| item | result |
|---|---|
| `wsm mount W:` (no WinFsp) | **PASS**: wsmd WebDAV bridge (`crates/wsmd/src/dav.rs`) + Windows' built-in WebClient; list, write, mkdir, copy, move, recursive delete, 20 MB round trip sha256-identical; outside-token 404, `..` 400; access to a suspended VM resumed it in 2.7 s |
| `wsm share DIR` | implemented; guest mount works but file access was EPERM (TCC) on the old base; NOT verified with the new grant |
| WebClient download limit | 50 MB default (system setting, not changed); larger files: `wsm cp` |

### Other changes (source; Clippy clean; 15 unit tests pass at last run)
`wsm list`; bare `wsm` creates `default` on first use; packaged runtime finds itself next to wsm.exe; uninstall
removes install.ps1's PATH entry; same-volume `base import` hard-links; builder free-space preflight and per-run
OpenCore cleanup; OpenCore templates 503 MiB -> 64 MiB (boot verified; originals in `work/r3/oc-503m-originals/`);
timestamped `serial-timed.log`; tests: P6 verdict/exit code, `tests/p6_gpu.py`, real-frame GUI stability,
`tests/p7_package.ps1` (NOT RUN). Known leak NOT fixed: QEMU `vl.*` temp files accumulate in instance logs dirs.

### Outside changes
None: no installs, PATH, registry, driver or feature changes. Temporary `net use` drives were removed.

## Round 3 continuation 3 (2026-10-25, paused by the user)

| Check | Result | Evidence |
|---|---|---|
| QEMU patch 05 vs 06, 20 busy suspend/resume cycles each (hl1, legacy base) | p05 19/20 and p06 19/20 (one session change + one panic report each). Equal: the rare busy-resume panic is not caused by patch 06. Suspend 1.2-1.5 s, 1.5-1.65 GB; resume to SSH 2.2-2.3 s | logs/r3/ab-p05.txt, ab-p06.txt |
| Clippy, newest source, -D warnings | PASS (clean build 15 m 21 s at -j2) | - |
| Base rebuild from ISO with patch 06 | Install stage PASSES (no AHCI timeout, installed OS boots). Stage 3 FAILED: no sqlite3 in BaseSystem. Fix written in guest/build/build.sh, wsm.exe rebuilt, NOT yet verified. Not complete: NO base image exists | logs/r3/rebuild-final.txt, rebuild-resume.txt, rebuild-resume2.txt |
| P3-P7 on a new base | NOT RUN (no base) | - |

Unresolved: resume 2 printed nothing after "resuming installation" for 16 min (unknown cause, not a verified hang). The first attempt was killed by memory pressure while Clippy ran in parallel. Details and timings: HANDOFF.md "Continuation 3".

## Round 4 / S1 — base (2026-10-25)
PASS. `wsm base build --from-stage 3` (stage 2 resume stalled in OpenCore "no boot entries"; stage 2 had already finished in resume 1, stage 3 only needs BaseSystem+target). build.sh TCC grant now best-effort; in practice installed sqlite3 ran, 4 TCC rows set (no WARN). Base: base-tahoe-26.3-1f6020294f4b.qcow2, 17,934,385,152 B, sealed read-only, qemu-img check clean (81% clusters already compressed), sha256+json sidecars. Build 612 s (inject 127 s, compact 485 s).

## Round 4 / S2 — shrink
SKIPPED (no gain). The builder's base is already ~81% compressed clusters; `convert -c -O qcow2 -o compression_type=zstd` gave 17,840,209,920 B vs 17,934,385,152 B (-0.5%, 94 MB). Not worth a second format/boot comparison; keeping the sealed base as is (uncompressed-vs-compressed size = 17.93 GB either way). Boot A/B NOT RUN.

## Round 4 / S3 — core smoke (instance core1, 2 GiB, 8 vCPU, tests/s3_core_smoke.py, logs/r4/s3.json)
PASS (one run). Cold start->SSH 26.8 s. Suspend 2.0-2.7 s. Resume->SSH 2.1-2.7 s (5 cycles), first exec after resume 0.36-0.73 s; session (background pid + kern.bootsessionuuid) identical in 5/5 cycles. Warm `exec true`: median 117 ms right after cold boot (max 188; target <100 MISSED), 59 ms median after the resume cycles (met). Auto-suspend: idle_minutes lowered 15->5 for the test (default 15 min; a literal 10 min idle was NOT run); suspended ~5 min after last access, state 1.93 GB. disk overlay 180 MB.

## Round 4 / S1b — base rebuilt (autologin fix)
New base base-tahoe-26.3-0a0a0ccadd84.qcow2 (stage 3 only, ~10 min). Cause of the GUI password prompt on a fresh instance: loginwindow logged "no autologin because previous user was not the autologin user" (lastUserName unset). guest/wsm-gui.sh now writes lastUserName with autoLoginUser. Old base 1f6020294f4b kept until S5. S3 repeated on the new base (core2, logs/r4/s3b.json): first-ever boot (incl. provisioning) 37.5 s, suspend 1.7-1.9 s, resume->SSH 2.0-2.5 s x5, first exec after resume 0.34-0.43 s, same session 5/5, warm exec median 57 ms (max 70). Idle auto-suspend: tested on the old base only (same daemon).

## Round 4 / S4 — gpu smoke (gpu3, 4 GiB, 1920x1080, backing = new base, overlay 4.96 GiB after use)
PASS with caveat. Metal readback clear+shader PASS (200 submits 1.4 ms each). Cold boot->SSH 23.8 s. First-ever GUI login (fresh home) is slow: 168 s on the first boot (prewarm script gives up at 60 s); later boots: prewarm 15 s, `wsm gui start` 1.8 s. 30 s animation (app 59 ticks/s): reims presented fps median 30 (min 12, max 56), repeat warm run median 45 (min 24, max 61); earlier rounds measured ~60, NOT tuned. `wsm stop` = clean shutdown 8.6 s, next start cold.
REGRESSION FOUND: runtime reims_vgpu_c.dll d9024afc (built 03:00 today, from reims-v030) makes every GPU submit ~200 ms (Metal: 207 ms/submit, screen frozen after a few frames). The p05-archive DLL 7bdb4ec0 gives 1.4 ms. Runtime now uses 7bdb4ec0 (backup of d9024afc: work/r3/tmp/reims_vgpu_c.v030.bak). Cause inside d9024afc not investigated. S6 must ship 7bdb4ec0, not d9024afc.

## Round 4 / S5 — disk report and cleanup
Deleted after S3+S4 passed: old base 1f6020294f4b (17.9 GB), build/tahoe-26.3 scratch (38 GB, incl. target.qcow2 and BaseSystem copy), test instances core1/gpu2 (old base), stale hl1 (9 GB) and gpu1 (2 GB) whose backing images no longer existed, work/r3/tmp screenshots. Free space on D: 64 GB -> 130 GB (+66 GB). ISO left in place (user decides).
Steady state: base 17.93 GB (sealed) + core instance 2.2 GB (overlay 0.18 + saved state 1.97 GB + OpenCore 0.07) + gpu instance 5.1 GB after first GUI login (0.7 GB fresh) = ~25 GB.

## Round 4 / S6 — package
PASS. work/r3/dist/wsm-setup.zip 49,767,184 B, sha256 92fadce82ea155effe56783891870811768adcd28c06f2e1251822feac17a830, 238 files: wsm.exe, wsmd.exe, QEMU runtime (patches 01-06 built in), reims DLL 7bdb4ec0, opencore-amd.img + opencore-intel.img, install/uninstall scripts, README; no base/ISO/dmg/qcow2/keys. tests/p7_package.ps1 (portable install in tools/wsm-portable-test, uninstall removes it, %LOCALAPPDATA% and PATH untouched): package 5 s, import 66 s, first boot 48.8 s, exec OK (26.3), suspend 2.6 s, resume 2.4 s, exec after resume OK, shutdown 10 s, uninstall 0.5 s. Fixes needed: build pin work/r3/qemu-v030/build.json now pins reims DLL 7bdb4ec0 (backup work/r3/tmp/build.json.bak); p7 harness no longer treats native stderr progress lines as errors (PS 5.1). Intel OpenCore image is untested on Intel hardware (NOT RUN). README updated with numbers and limits.

## Round 4 / S7 — publish/
DONE. publish/: patches/01..06, merge_amd_oc.py, AMD-OPENCORE-PATCHES.md (8 items, one-line cause each), RESULTS-summary.md, WRITEUP.md. No disks, no Apple files.

## Lite / L0 — inventory (2026-10-25)
Surviving: work/wsm-base.qcow2 (1.44 GB, round-1 provisioned), work/BaseSystem.img (1.47 GB raw), work/seed/{setup,init,disable}.sh + src/mkseed.py; wsm-data.qcow2 gone. Reusable from the full builder: guest/wsm-firstboot.sh (config-disk ustar with wsm-config.json), wsm-serial.sh, launchctl-disable.txt; Rust OpenCore generator (add_boot_args, oc_tweak) for csr/boot-args. Round-1 init.sh has a hardcoded dev user and old host keys: identity parts are NOT reused. 129 GB free (need ~8). All new files go under work/lite/ and logs/lite/.

## Lite / L1 — extract
PASS. 7z extracted BaseSystem.dmg (1,436,892,191 B, sha256 15613d8ab727…) from the ISO in 3.5 s; qemu-img dmg->raw 1.6 s: work/lite/l1.raw 1,465,943,552 B (sha256 0f99c5d5d825…, byte-identical to round-1 work/BaseSystem.img); l1.qcow2 1,399,848,960 B. ISO untouched; the .dmg copy was removed (re-extractable).

## Lite / L2 — provision (wsm base build-lite)
PASS. `wsm base build-lite --iso` (~8 min: extract, single-user seed provisioning, one boot to format the data template, compact) -> bases/base-lite-tahoe-26.3-9c6156594ebb.qcow2 1,008,343,552 B (read-only, sha256+json sidecars, kind=lite) + -data.qcow2 template 589,824 B (APFS WSMData + WSMLocal, 64 GiB virtual). Fresh instance (lite1): first boot to SSH 26.9 s as dev uid 501, hostname wsm-lite1, macOS 26.3, /usr/local = WSMLocal, PATH via sshd SetEnv; base sha256 unchanged after boot.
Fixes needed on the way (one rebuild each): blank-disk detection had to skip synthesized APFS disks and use `od -v`; container ref is field 4 of `diskutil list`; BaseSystem has no ssh-keygen (host generates the instance host key onto the config disk); no /bin/zsh (lite user shell is /bin/bash); /etc is on the runtime tmpfs (no zshenv in the base); sshd must be its own KeepAlive LaunchDaemon (launchd killed it with the run-once init job); CRLF from a Windows edit broke init.sh once. Lite instances get csr-active-config 0x67 in their OpenCore (kept per spec; not proven required). Known gap: `id` prints "failed to retrieve group list" (flat-file user, no opendirectoryd record).

## Lite / L3 — smoke and floors (one run each)
PASS with gaps. Cold boot -> SSH 27.4 s (second boot; init.sh start at 15 s, data volumes mounted at 25.7 s: slower than round 1's 15.8 s, not tuned). `wsm exec true` median of 10: 40 ms (max 47). Two instances from one base (lite1, lite2): MACs 52:54:00:a9:01:ff vs 52:54:00:1a:8c:67, hostnames wsm-lite1 vs wsm-lite2, host keys SHA256:F+HIiAxU… vs SHA256:y2gwm/cJ…. RAM: 4 GB and 3 GB boot to SSH; 2 GB fails (`Boot task failed: restore-datapartition`) -> floor 3 GB (default stays 4 GB). `wsm stop --shutdown` took 60 s (guest did not power off by itself; fell back to the timeout).
Gap: serial console has no working login on lite (`login -fp` gets "Login incorrect": BaseSystem login needs an opendirectory record); the `login:` prompt appears at ~17 s. SSH is the only way in.

## Lite / L4 — extras pack
PASS with gaps. Source: full-base instance core2 (resumed 2.3 s), `tar` of the binaries via `wsm exec`/`wsm cp`. Each tested in the lite guest. Pack work/lite/extras/wsm-lite-extras.tgz 1,433,838 B (bin/, libexec/sftp-server, share/file): readlink diff patch cmp install du gzip gunzip zcat file bzip2 stat base64 (run), sftp-server (runs; `wsm cp` now works on lite), xcrun xcode-select make (CLT shims; need CLT, L5). `wsm base build-lite --extras <tgz>` unpacks it into the data template's WSMLocal (= /usr/local, owned by the user at first boot); template 2.2 MB with extras.
Gaps (dyld-cache or Perl module dependencies, dropped): shasum (Perl Fcntl.pm missing), top (libpanel), uptime (libsbuf). `file` needs MAGIC=/usr/local/share/file/magic.mgc (next base rebuild). No sudo in BaseSystem (only su); root work happens in init.sh.

## Lite / L5 — dev on lite (one run)
PARTIAL. CLT 26.6 (3 cached pinned pkgs): `wsm cp` in 12 s (893 MB), `pkgutil --expand-full` + `ditto` to /Volumes/WSMData/Library/Developer 139 s (4.8 GB; ditto to `/` fails: /Library is a symlink into the tmpfs Data volume). With DEVELOPER_DIR + SDKROOT=MacOSX26.sdk: clang C hello 1 s, swiftc hello 50 s (first run), `git clone --depth 1` brew 8 s, Homebrew clone + pinned checkout 167 s.
STOPPED (new workaround needed, not chased): `brew install jq` fails. 1) Homebrew looks for /usr/bin/git (BaseSystem has none; HOMEBREW_GIT_PATH gets past it). 2) Then "Unknown command: brew install" with Ruby "already initialized constant Sandbox::…" warnings (Homebrew files loaded twice; likely a path/realpath mismatch). 3) Next step would be root-side /usr/bin shims for CLT (git, clang…) in init.sh and a brew-on-lite investigation. `brew install hello` (source) NOT RUN.

## Lite / L6 — suspend/resume (QEMU with patches 01-06)
PASS. lite1 (4 GB), 5 busy cycles (detached perl spin at ~100 % CPU as the session marker; tests/lite_l6.py, logs/lite/l6c.json): suspend 1.47-1.83 s, resume -> SSH 2.28-2.48 s, same session (process alive + same kern.bootsessionuuid) 5/5. Saved state 1.027-1.029 GB (full-base core: 1.97 GB). Idle check of the earlier run (tests/s3_core_smoke.py): first exec after resume 173-213 ms, warm exec 36-41 ms.
Note: on lite, sshd kills the SSH session's process group on disconnect (no PAM/launchd session), so `nohup cmd &` dies when `wsm exec` returns; long-running jobs need their own process group (setsid/perl setpgrp). Known gap.

## Lite / L7 — integrate and package
PASS (compression skipped: no gain). `wsm base build-lite --iso <iso> [--extras tgz]` (~8 min), `wsm create` picks the newest lite base by default (GPU instances only the full base), kind="lite" in the instance manifest (RAM default 4096, `-data.qcow2` overlay as sata.5), `wsm base export/import` carry the data template + sha256 sidecar (import of a lite base name base-lite-tahoe-26.3-<sha12>). Final base base-lite-tahoe-26.3-5fb9ab575eee.qcow2 1,008,343,552 B + data template 2,228,224 B (with extras, 1.43 MB tgz). zstd recompression: 993,789,440 B (-1.4 %); boot comparison NOT RUN (gain too small). Lite package QA (tests/lite_package.ps1, separate zip work/lite/dist/wsm-setup-lite.zip 49,783,974 B, sha256 3eacc576…c13): package 5 s, import 2.1 s, create 0.4 s, first boot 20 s, exec ok (26.3), suspend 2.3 s, resume 2.2 s, shutdown 60.5 s (guest does not power off itself), uninstall 0.5 s. cargo build/clippy -D warnings/tests clean. The shipped full-base zip, publish/ and the full base were not touched (work/r3/dist/wsm-setup.zip still the pre-lite build).

## Final polish / F1 — brew on lite (2026-10-25)
PASS. Three root causes, none of them the one assumed in round 1: (1) Homebrew's portable Ruby loads fiddle -> `/usr/lib/libffi.dylib`, absent from BaseSystem's dyld cache, so `sandbox.rb` half-loaded and brew printed "Unknown command: brew install" with "already initialized constant" warnings; fix: libffi 3.4.6 built with CLT, shipped as `lib/libffi.8.dylib` in the tools pack, linked to `/usr/lib/libffi.dylib` at boot. (2) git from CLT, symlinked into `/usr/bin`, finds `git-core`/templates relative to `/usr/bin`: `libexec/*` and `share/*` of the CLT are linked into `/usr/libexec` and `/usr/share` too. (3) superenv's PATH has no `/usr/local/bin`, so autoconf's "sed that does not truncate output" test could not find `diff`/`cmp` from the tools pack: regular files of `/usr/local/bin` are linked into `/usr/bin`. All done by `guest/lite/local.sh` (installed as `/usr/local/libexec/wsm-lite-local.sh`, the hook the base's init already runs as root at the end of boot), so the base was not rebuilt; it also writes `/usr/local/etc/homebrew/brew.env` (HOMEBREW_TEMP on the data disk). Results on lite1 (CLT unpack 4.8 GB, Homebrew pinned 51da7f23): `brew install hello` source build 3 min 00 s (configure dominates, each compiler call goes through brew's Ruby shim); `brew install jq` bottle (+ oniguruma, sonoma bottles) 12.9 s, `jq .a` works.

## Final polish / F2 — detached jobs
PASS. `wsm run --detach -- cmd` / `wsm jobs` / `wsm kill ID` (crates/wsm-cli/src/jobs.rs; guest side sh + perl, state in `~/.wsm/jobs/ID/{cmd,pid,log}`). launchd was not used: no user domain without a GUI session, and a forked `setpgrp` leader survives sshd's cleanup (BaseSystem perl has no POSIX module, so no `setsid`). Tested on lite1: `sleep 600`, `python3 -m http.server`, `nc -l`; after the SSH sessions ended and after `wsm quit` (all pooled connections closed) all three were still running, http answered 200, nc served a connection; `wsm kill` stopped sleep 600; suspend + resume with jobs running: the same processes were still running (http 200). Not covered: shutdown or cold boot ends all jobs.

## Final polish / F3 — sudo on lite
sudo NOT usable; `wsm exec --root` implemented. The full base's `/usr/bin/sudo` (single 1.58 MB binary, execute-only; read through `sudo cat` on core2) was copied in as setuid root with NOPASSWD and a minimal pam file: it starts but dies with `get_user_info: unable to allocate memory`, the same `getgrouplist` failure as `id` ("failed to retrieve group list"). A dscl record for the user did not help (no memberd/dsmemberutil in BaseSystem). Dropped. `wsm exec --root -- id -u` prints 0: local.sh adds root to sshd's AllowUsers (`PermitRootLogin prohibit-password`, same instance key, SIGHUP to sshd), the daemon opens the SSH channel as root (pool key `name#root`).

## Final polish / F4 — one package
PASS. One `wsm-setup.zip` (work/r3/dist/wsm-setup.zip, 49,795,082 B, sha256 b1c4920f241a5ea498d3463fb0d95ac9d838780887d18d19eb4b07086f06b62e, 238 files + manifest): wsm.exe, wsmd.exe, QEMU runtime (patches 01-06, reims 7bdb4ec0), OpenCore AMD/Intel templates, install/uninstall, README with the two-base table, `sources/lite/local.sh`; no Apple content, no base. The previous full-only zip is kept as wsm-setup.zip.prev; the separate lite zip was removed. New command `wsm base lite-data BASE --extras PACK` rebuilds only a lite base's data template (the base file is untouched, sha unchanged; ~37 s); the tools pack is now work/lite/f4/wsm-lite-extras2.tgz (1.49 MB, adds libffi and local.sh); template 2.29 MB. tests/f4_package.ps1 (portable install in tools/wsm-portable-test, nothing outside the project): package 6 s, install 1.2 s, import 2.2 s, create 0.5 s, first boot 26.6 s, exec 26.3, `exec --root` uid 0, libffi link and local.sh present, detached job started, suspend 3.2 s, resume 2.3 s, job still running after resume, kill, shutdown 60.5 s, delete, uninstall 0.5 s. cargo build and clippy -D warnings clean. Export dir work/lite/export holds the new template (old one in export/prev). The dev home work/r3/home still has the old template; lite1 overlays it, so it was not swapped.
Disk (this workspace): lite base 1.0 GB + 2.3 MB template; lite1 (CLT + Homebrew + hello, jq, libffi build) data disk 8.2 GB + 1.0 GB state; full base 17.9 GB; core2 2.2 GB; gpu3 5.3 GB.
