# WSM round 3 - standing objective and acceptance

Read HANDOFF.md for latest state; RESULTS.md is the source of measured rounds 1-3 evidence.
Build a working, installable WSM v0.1 from proven pieces, not another feasibility investigation.

## Scope and fixed decisions

- Host: Windows 11 Home, Ryzen 7 7840HS, 15.2 GB RAM, RTX 4050 Laptop 6 GB + Radeon 780M, WHPX.
- Latest user constraint: NO changes outside D:\MyProjects\wsm. Downloads/portable tools in tools/;
  runtime/package tests inside project. No PATH/registry/driver/Defender/feature/LOCALAPPDATA changes.
- One QEMU: aljus7 reims v0.3.0 09f13b32e0a438502409b282e2a6750ad50f879b,
  vendor QEMU 7100064d13b42bb1be16671fcf18435822b13c3c plus WSM patches. Pins/hashes are in
  work/r3/qemu-v030/build.json; numbered patches in src/patches.
- Main candidate is currently identical to the tag. Keep known-good pin; do not rebuild an identical
  candidate or reopen GPU feasibility without a concrete regression.
- Tahoe 26.3 x86-64 from user's ISO; full trimmed install, stock seal, launchctl disable data lists,
  SIP on. Hybrid remains research. No swapped kernel collections or thin-userland workarounds.
- Generic immutable compressed base + SHA-256/manifest, instance qcow2 overlays. No user/password,
  SSH keys, machine SMBIOS identity or CPU patches in the base. Per-host Intel/AMD firmware and
  per-instance SMBIOS/MAC/config disk; firstboot creates hostname/user/keys.
- GPU RAM <=4 GiB, display 1920x1080. GPU migration blocked by device; stop shuts it down, next start cold.
- Serial early shell, SSH exec/SFTP/multiple sessions. Headless stop/idle saves, resume <=5 s.
- GUI on demand, hidden prewarm, correct GUI-user preferences, static wallpaper.
- Projects inside APFS; SFTP copies, optional SFTP drive, guest-to-host SMB. Current user forbids
  installing the absent WinFsp/SSHFS-Win kernel driver, so mount qualification is NOT RUN.
- Rust workspace: wsm-cli, wsmd, wsm-core, wsm-builder. Python only harness/build tooling.
- Do not pursue custom VMM/Hyper-V replacement, Darling, virtiofs/9p, arm64 guests, Linux host,
  per-app window projection or Linux-only userfaultfd/postcopy fast snapshots.

## Architecture

wsm.exe -> per-user named-pipe JSON RPC -> wsmd owns QEMU, QMP, serial, SSH pool, lifecycle and idle.
WSM_HOME contains runtime/, bases/, instances/<name>/{manifest,overlay,state,keys,logs}.
QMP/serial/migration currently use owner-only AF_UNIX; only guest SSH uses loopback TCP.

Commands: shell, exec, start, stop, suspend, resume, gui, status, logs, create, delete, doctor, cp,
mount/unmount, base build/import/export/list, uninstall. Default instance default, --name for others.
One GPU VM at a time. Final acceptance goes through CLI, not manual QEMU invocations.

## P1 - unified binary

Metal clear/shader + GPU desktop on one QEMU; headless resume <=5 s; GPU migration outcome recorded.
Known tested result: irqchip=on with WHPX patches; GPU device itself blocks migration.
Preserve APIC restore, AP startup, pause-time and pending-event fixes. Do not rewrite working components.

## P2 - candidate

One-session comparison of main vs v0.3.0: boot, Metal, warm FPS, Finder drag, first-session stall.
Retain baseline absent measured improvement. Current main/tag equality is recorded in HANDOFF.
Apply transparency/motion/static wallpaper through correct user context; root writes were rejected.

## P3 - generic reproducible builder

ISO -> unattended install -> Recovery config -> trim -> compact -> read-only base + manifest/SHA.
Target <=20 GB; SSH <=20 s. Optional CLT. Preserve original ISO and record source hash.
Generate CPU firmware/topology and identities outside base. Firstboot consumes config disk, then disables itself.
Export/import SHA verification; two fresh instances must have distinct SMBIOS, MAC, SSH host and user keys.
Intel-style validation as far as WHPX permits; at minimum prove regenerated identities between instances.
Record actual complete build time and final size. A repaired/resumed image is not proof of a full unattended run.

## P4 - CLI/service

Auto-start daemon, resume saved state or cold boot, attach shell. Early serial (~7 s), SSH switch/--ssh.
Exec preserves stdin/stdout/stderr/exit code; cp both ways through SFTP.
Stop suspends by default, shutdown clean, kill last resort; document GPU cold-boot limitation.
Stale PID/port/state detection, QMP timeouts, recoverable crashes, rotated logs, default idle suspend 15 min.
Doctor: WHPX availability, CPU, Vulkan/GPU info/preference, free space and versions.
Clippy clean, useful unit tests, opt-in real-VM integration tests. Full acceptance via CLI only.

## P5 - integration

Drive mount detects prerequisites and prints exact optional setup instructions. Do not install forbidden driver.
Document/test authorized SMB route. Verify actual GUI display, start/stop and hidden prewarm; a loaded
launchd job is not proof of a running WindowServer or presented frames.

## P6 - real measured acceptance

1. Cold serial command + SSH: median of 5.
2. Save duration/size and resume-to-shell median of 5; marker/session integrity. Clock correction shifts
   kern.boottime, so use bootsessionuuid + RAM marker and explain the distinction.
3. Warm exec true median of 20.
4. clang C hello, swiftc hello, git clone, source Homebrew hello + an actual bottle.
5. Metal clear/shader; warm FPS median/min; first-session stall with/without prewarm; Finder drag median Hz.
6. Ten consecutive start/suspend/resume cycles; one REAL hour idle with idle suspend; 30 REAL minutes GUI;
   zero panics/device loss. Short runs do not substitute for duration or frames.
7. Idle host CPU, headless/GPU working set, base + overlay disk use.

Unknowns are NOT RUN. Harnesses must not mask failed individual checks with an overall zero exit.

## P7 - package and final report

Tool-only wsm-setup.zip: CLI/service, unified QEMU runtime, firmware/AMD+Intel templates, README/pins.
macOS base separate, imported with SHA verification or built from ISO.
Portable installation/removal QA only in a disposable project folder with NoPath/portable mode.
Rebuild ZIP after final code/docs updates.

Final report: P1-P7 table, actual P6 numbers, pinned versions/hashes, named evidence-based limitations,
one-time optional user steps, disk used, changes outside project. Keep source, patches and evidence.
Read applicable AGENTS before dependency-source changes; no destructive git reset/clean of this workspace.
