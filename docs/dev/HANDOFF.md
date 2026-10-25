# WSM round 3 - current handoff

Updated 2026-10-25 (continuation 2 at the top). Work stopped for handoff at the user's request. Read this first, then
[ROUND3_TASK.md](ROUND3_TASK.md) and the Round 3 sections of [RESULTS.md](RESULTS.md).
The product is unfinished. Do not interpret implemented features or the preview ZIP as qualification.

## WORK RULES FOR THE NEXT SESSION (the user was upset about wasted tokens; follow these)

What went wrong last session: ~10 side projects and 3-4 repeated test runs happened while the one real
blocker (base rebuild from the ISO failing) stayed open. Do this instead:

1. **Only blocker first.** Order is fixed: (a) A/B the QEMU patch 06 vs 05 (20 busy cycles each, one script
   run per binary), (b) rebuild the base once, (c) run each existing test script ONCE. Nothing else until (b) passes.
2. **No tweak-and-remeasure loops.** If a number misses its target, read the log for the cause first. Cold boot
   (~21 s vs 20 s target) and drag fps (12 Hz) are known and accepted for now: do not tune them.
3. **No new features** (no more WebDAV/share/package/README work) until the base passes P3 + P6. Those are done in source.
4. **Keep output small:** `grep`/`tail` logs; read test JSON with the summary one-liners; never cat serial logs or whole-file diffs.
5. **File writing:** use the Write tool for anything with quotes or backslashes; Git Bash (scratchpad env.sh pattern) for
   `wsm exec`; QEMU sources are CRLF, so make patches with `git diff` or keep line endings.
6. **Update status.txt / RESULTS.md / HANDOFF.md as each step finishes** (one short entry), not in a big batch at the end.
7. Do not delete evidence: logs/r3, work/r3/oc-503m-originals, work/r3/qemu-v030/p05-archive.

## Continuation 3 (2026-10-25 03:20-05:20) - PAUSED BY THE USER; READ THIS FIRST (supersedes the resume steps in Continuation 2)

Everything is stopped (no QEMU/wsm/wsmd). Compiled outputs in `target/` were deleted at the pause: rebuild with the
cargo commands in "Environment and commands" below (clean build ~15 min at -j2, incremental ~13 s).

**What was done and decided**
- A/B of QEMU patch 05 vs 06, 20 busy suspend/resume cycles each on hl1: **identical**, p05 19/20 (cycle 10 changed
  session, 1 panic report), p06 19/20 (cycle 20, 1 panic report). The ~1-in-20 busy-resume panic is NOT caused by patch 06. Keep p06
  (`runtime/qemu` = sha256 d077a713...; copies in `work/r3/qemu-v030/p05-archive` and `p06-archive`). Logs: `logs/r3/ab-p05.txt`,
  `ab-p06.txt` (p06 file also contains junk from an invalid first run that wrote the same file; trust the last SUMMARY lines).
  Suspend 1.2-1.5 s / 1.5-1.65 GB, resume->SSH 2.2-2.3 s, ~25 s per cycle, 20 cycles ~9 min.
- Clippy on the newest source (uninstall, handle inheritance): **clean**. 15 min 21 s at `-j 2` BelowNormal (clean target dir).
- Base rebuild from the ISO with patch 06: **install now passes** (no AHCI timeout this time), the installed OS boots after resume.
  New failure at stage 3: `WSM-BUILD: FAIL no sqlite3 in BaseSystem`. Cause (verified by `7z l work/BaseSystem.img`): Recovery BaseSystem
  has libsqlite3 but no sqlite3 tool. **Fix written (in `guest/build/build.sh` ~line 123, wsm.exe rebuilt with it, NOT yet proven in a run):**
  mount the installed System volume read-only, copy its `usr/bin/sqlite3` to /tmp, unmount, add /tmp to PATH. Risk: the binary may not run
  under Recovery's dyld cache (script then fails with "installed sqlite3 does not run in BaseSystem"). Fallback: write the 4 TCC rows
  another way (e.g. a tiny tool run from the installed system is impossible under SIP; consider shipping the rows via a LaunchDaemon-less path,
  or perl DBD::SQLite if BaseSystem has /usr/bin/perl).

**Timing / size log (what took what)**
| Item | Time | Size / note |
|---|---|---|
| hl1 `start --cold` (daemon restarted) | 71.7 s | includes waiting for SSH; the 20-21 s target is for the base, not legacy hl1 |
| first rebuild attempt (03:39) | stage 2 prepare ~13 min, killed ~03:50 | killed by Claude Code memory-pressure reaper while Clippy ran in parallel |
| resume 1 (04:51 -> 05:00) | ~9 min to stage 3 | installed OS boot + builder daemon; ACPI shutdown not handled (expected, see below) |
| resume 2 (05:01 -> 05:17) | no progress line after "resuming installation" for 16 min, killed on pause | cause UNKNOWN; resume 1 got further in 9 min. Look at serial/QMP (`watch_install` example) first |
| partial install target | 18 GB at 03:42, 37.5 GB (38,167,379,968 B) at 04:02 | `work/r3/home/build/tahoe-26.3/target.qcow2`, KEPT, resumable |
| D: free space | 73 GB -> 37 GB (target growth) -> 106 GB after old research disks deleted | |
| incremental cargo build of wsm | 12.6 s | |

**Problems hit this session (do not repeat)**
1. `WSM_HOME=$PWD/...` in Git Bash gives `/d/...`, which `wsm.exe` cannot read ("instance does not exist"). Always export Windows paths
   (`D:\MyProjects\wsm\work\r3\home`) with `MSYS_NO_PATHCONV=1`. The first 20-cycle run was invalid for this reason and still wrote into the same log.
2. `pkill` does not exist in Git Bash: use `taskkill //F //IM` or PowerShell `Stop-Process`. A leftover old stress script kept writing the log.
3. Do NOT run Clippy/cargo next to a VM build: memory pressure killed the build wrapper (QEMU died with it). The target survives; use `--resume-install --from-stage 2`.
4. `sleep N` and blocked chained sleeps: wait with `run_in_background` or the Monitor tool.
5. Claude Code's classifier blocked `rm -rf` of large folders; the user deleted them in PowerShell instead.
6. A background "completed" notification for `Start-Process` only means the launcher exited; check the real log.

**Cleanup done:** removed `work/reims/vm`, `work/r2`, old `work/gpu-followup/*.qcow2|img|state` (kept `hybrid.qcow2`, `metaltest`, `anim.swift`),
`work/esp*`, `seed.img`, `OVMF_VARS.fd`, old `target/release`, `target/round3-check`, `work/r3/tmp/*`, and at the pause all of `target/`.
Kept: ISO, `logs/`, `work/r3/oc-503m-originals`, `p05-archive`, `p06-archive`, hl1, gpu1, `tools/` toolchains, `work/dl-clt`, source.

**Resume plan (smallest steps)**
1. Rebuild host binaries (cargo build --release --locked, `CARGO_TARGET_DIR=target/round3-next`).
2. Run ONLY: `wsm base build --iso "macOS Tahoe 26.3.iso" --resume-install --from-stage 2 > logs/r3/rebuild-resume3.txt` with nothing else running.
   If it prints nothing after "resuming installation" for >10 min, run `watch_install` (see Continuation 2) before anything else.
   If stage 3 fails on sqlite3, switch the fallback above. If the target is judged corrupt, delete `build/tahoe-26.3` and run the full build (~1-2 h, needs ~75 GB free).
3. After the base exists: the order in Continuation 2 "After the rebuild, in order" (P3, SetupDev+P6, share, GPU, stability, package, README/RESULTS).

## Continuation 2 (2026-10-25 ~03:00) (partly superseded by Continuation 3)

Stopped at the user's request. Full evidence: RESULTS.md "Round 3 continuation 2". Status line: status.txt.

**State right now:** NO base image exists (the old one was deleted for the rebuild; the ISO and all
tools are intact). Instances left: `hl1` (legacy, suspended), `gpu1` (legacy, stopped). Nothing is running
except possibly wsmd. The failed rebuild left `work/r3/home/build/tahoe-26.3/` (install stage output; safe
to delete before rebuilding). D: had 73.8 GB free before it.

**Why the rebuild failed and what was done:** QEMU+WHPX loses level-triggered IOAPIC interrupts when the
APIC-EOI exit runs `ioapic_eoi_broadcast()` without the BQL (AHCI timeout, `startosinstall exit 138`).
Fix = `src/patches/06-qemu-whpx-eoi-bql.patch`, QEMU rebuilt and installed in `work/r3/home/runtime/qemu`
(sha256 d077a713...). **Not yet proven:** it was never run through an install, and a 10-cycle busy
suspend test on hl1 had 1 panic in 10 (patch 05 had 0/10). Decide before trusting it:
  1. A/B 20 busy cycles each with the p06 binary and the archived p05 binary (`work/r3/qemu-v030/p05-archive/`,
     swap into runtime/qemu), `tests/stress_suspend.sh hl1 20 --busy` with a fresh `wsm --name hl1 start --cold` first.
     If p06 panics more, look at whether taking the BQL in the EOI exit changes migration timing, not at the base.
  2. Re-run `wsm base build --iso "macOS Tahoe 26.3.iso"` (unattended, ~1-2 h). `watch_install` example
     (`target/round3-next/release/examples/watch_install.exe <build>/install <out.txt>`) captures AHCI/IOAPIC state if it stalls again.

**Guest/host changes waiting for that rebuild to take effect (all in source, none verified in a booted new base):**
serial getty on /dev/console; /etc/zshenv path_helper; remote-login TCC grants (builder verifies 4 rows);
gui prewarm keeps a hidden logged-in session; `wsm share`; vCPU default; OpenCore logging off.
Binaries already built: `target/round3-next/release/{wsm,wsmd}.exe` (latest source incl. auto-create of `default`).

**Measured on the old base (valid for the design):** resume->SSH 1.9 s, save 1.4 s, warm exec 39 ms,
cold boot->SSH 20.6-21.1 s at 8 vCPU (target 20 s), GPU Metal PASS, warm 60 fps, first GUI session 0.7 s with
prewarm, drag median 12.5 Hz (uneven, unfixed), `wsm mount` PASS (built-in WebDAV, no installs).

**After the rebuild, in order:** (1) `tests/round3.ps1 -Action Generic` (P3: needs fresh names basecheck/basecheck2;
`-Action ResetGeneric` first if they exist); (2) `-Action SetupDev` then `tests/p6_acceptance.py basecheck`;
(3) `wsm share` + `ls ~/Documents` over `wsm exec` to confirm the TCC grant; (4) `tests/p6_gpu.py <gpu inst> --anim work/r3/gpu/anim`
(anim is compiled; metaltest at work/gpu-followup/metaltest); (5) `tests/p6_stability.py` (10 cycles + 1 h idle; `--gpu` 30 min);
(6) `tests/p7_package.ps1 -Base <base.qcow2>` (package + portable install in tools/wsm-portable-test + uninstall);
(7) update README numbers (cold boot ~21 s) and RESULTS P1-P7 table. Unknowns stay NOT RUN.

**Gotchas learned:** PowerShell 5.1 mangles quotes in `wsm exec -- '...'`: use Git Bash (scratchpad env.sh pattern: WSM_HOME,
WSM_BIN, MSYS_NO_PATHCONV=1). Bash heredocs with apostrophes/backslashes broke twice: use the Write tool for files.
CRLF source files (QEMU whpx-all.c): preserve line endings when generating patches. `wsm quit` before rebuilding wsmd.
Do not delete `work/r3/oc-503m-originals`, `work/r3/qemu-v030/p05-archive`, `logs/r3/*` evidence.

## User constraints

- **No changes outside D:\MyProjects\wsm.** Downloads and portable toolchains belong in tools/.
- Keep all builds, runtime/package tests and TEMP/TMP inside the project.
- No PATH, registry, driver, Windows feature, Defender or LOCALAPPDATA changes.
- The user rejected WinFsp/SSHFS-Win installation. Do not ask again. Drive mounting is NOT RUN;
  a kernel driver cannot be provided merely by extracting it portably.
- No sub-agents were used. No commit/push was made; most of the workspace is untracked. Do not clean/reset it.

## Status

| Phase | Acceptance status |
|---|---|
| P1 unified QEMU | PASS from earlier work; preserve known-good binary and five patches |
| P2 candidate | PASS: remote main and v0.3.0 are the same source revision |
| P3 generic builder | IN PROGRESS: readiness race, identities/export/import, full clean rerun |
| P4 host CLI/service | IN PROGRESS: pool/IPC tested; newest uninstall source unbuilt |
| P5 integration/GUI | NOT QUALIFIED; GUI tests pending, drive prerequisite forbidden |
| P6 acceptance | IN PROGRESS; complete new-base suite and real-duration tests NOT RUN |
| P7 package | STALE PREVIEW ONLY; isolated portable installation/removal QA pending |

## Immediate next fix: readiness race

Current base:

    work/r3/home/bases/base-tahoe-26.3-7f70d08f4532.qcow2

17,257,791,488 bytes (17.26 GB), read-only, SHA-256 and JSON sidecars, no CLT. Manifest includes ISO hash.
The old fd62 candidate was rejected and deleted after checking no dependent instance remained.

Latest P3 test fails an immediate `sudo test -f /var/db/wsm/firstboot.done` after SSH becomes available.
**This latest failure is a race, not an ongoing provisioning failure.** The subsequent guest inspection proved:

- firstboot.done exists, root-owned;
- firstboot launchd service ran once, exit code 0, no longer running;
- SIP enabled.

`crates/wsm-core/src/ssh.rs::ready()` currently probes `true`, which succeeds before firstboot finishes.
Suggested next change (NOT IMPLEMENTED): require the completion marker when the firstboot plist exists,
while allowing legacy images without that service. Preserve the 8-second kill bound on every SSH probe.

```sh
if [ -f /Library/LaunchDaemons/com.wsm.firstboot.plist ]; then
    sudo -n test -f /var/db/wsm/firstboot.done
else
    true
fi
```

Alternatively/additionally, make P3 explicitly wait for completion with a bounded loop. Do not rebuild the
OS just to fix this race. The earlier root/TCC preference error was real but is already fixed in this base.

Last observed processes at handoff: QEMU PID **30648** (disposable basecheck, headless 2 GiB), wsmd PID
**5004**. Recheck PIDs; basecheck may auto-suspend after 15 minutes. hl1 should be suspended. No build/test
supervisor or watcher appeared in the last process check. Preserve hl1/gpu1; only basecheck/basecheck2 are
disposable fixtures created by this continuation.

## Environment and commands

**Use target/round3-next/release/{wsm,wsmd}.exe.** work/r3/bin and target/release are older.
The separate target directory avoided replacing a CLI held by a running build.

```powershell
$env:WSM_HOME='D:\MyProjects\wsm\work\r3\home'
$env:WSM_BIN='D:\MyProjects\wsm\target\round3-next\release\wsm.exe'
$env:RUSTUP_HOME='D:\MyProjects\wsm\tools\rustup\home'
$env:CARGO_HOME='D:\MyProjects\wsm\tools\rustup\cargo'
$env:CARGO_TARGET_DIR='D:\MyProjects\wsm\target\round3-next'
$env:OPENSSL_STATIC='1'
$env:TEMP='D:\MyProjects\wsm\work\r3\tmp'
$env:TMP=$env:TEMP
$env:PATH='D:\MyProjects\wsm\tools\rustup\cargo\bin;D:\MyProjects\wsm\tools\msys64\ucrt64\bin;D:\MyProjects\wsm\tools\msys64\usr\bin;'+$env:PATH
& $env:WSM_BIN quit
cargo build --release --locked
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release --locked
```

Quit wsmd before overwriting its executable; QEMU survives a daemon quit. Let active commands finish.
Runtime ACLs deliberately exclude the sandbox token, so protected operations require owner-context
escalation. Do not weaken ACLs to avoid this.

`tests/round3.ps1 -Action ...` provides fixed project-local operations:

- Progress, InspectFirstboot, StorageCheck, DiskUsage;
- LaunchCheck (captured daemon auto-start), Smoke (hl1 transport + suspend);
- Generic (two fresh instances/export/import), ResetGeneric (ONLY checked disposable fixtures);
- SetupDev, Acceptance, Package, Quit.
- CleanupFailed/CleanupBuildCache/RepackTarget already ran. Do not repeat blindly.

Example: `powershell.exe -NoProfile -File tests/round3.ps1 -Action InspectFirstboot`.
Inspect actual script actions before using them. ResetGeneric currently allows the exact fd62 and 7f70 test bases.

## Verified host implementation

- Persistent ssh2 **0.9.6** pool; binary stdin/stdout, separate stderr, remote exit status, concurrent
  clients and cancellation. Interactive SSH/scp still use Windows OpenSSH.
- Enable BOTH vendored-openssl and openssl-on-win32. Vendored alone selected WinCNG for libssh2 and
  could not negotiate the guest keys; build.rs established the cause.
- IPC protocol **2**, owner+SYSTEM, local-only named pipes; QMP/serial/migration remain owner-only
  AF_UNIX. Only guest SSH uses TCP.
- interprocess **2.4.7** generic wrapper had 512-byte pipe buffers and unsupported Windows timeouts.
  Current ipc.rs uses raw 1 MiB buffers, bounded chunks, PeekNamedPipe and nonblocking deadlines.
- `logs/r3/p4-transport.json`: PASS, 1,048,576 bytes binary round trip, separate stderr, exit 7,
  four concurrent commands. Warm exec **46.06 ms median/20**, legacy hl1. Binary round trip 4.918 s.
  These are NOT final new-base P6 numbers.
- Named-pipe smoke: hl1 resume 2.1 s to SSH; save 2.1 s, 1,572 MB.
- Detached wsmd retained a captured launcher's output pipe after CLI exit. The fresh P3 create call
  timed out despite the CLI already being gone. `private_standard_handles()` clears inheritance at
  CLI/daemon entry. `tests/p4_launcher.py`: **PASS, 0.128 s** auto-start/captured completion.
- Native PID checks replace tasklist; creation-time sidecar checks reused PIDs. GPU-start and same-name
  create locks, refreshed SSH ports, readiness cache. Daemon errors go to WSM_HOME/wsmd.log;
  instance serial output now persists in logs/serial.log.
- Instance firmware disables the template's confirmed ShowPicker=true/Timeout=10 delay. Measure actual
  new-base cold boot; do not assume the <=20 s target is now met.
- Last complete checks: 13 unit tests PASS, 1 real-VM test explicitly ignored, Clippy clean. Later
  handle-inheritance/newest uninstall changes need final checks. The handle fix was compiled and tested.
- Older serial_tail.py/boot_timeline.py use obsolete TCP discovery; adapt if needed. P6 serial timing
  drives `wsm shell --serial` instead.

## Builder history and state

Earlier successful-build claims were false: Recovery has loginwindow; later installer boots drop
autoinstall but still use -restore/-rootdmg-ramdisk, followed by -bsdmgroot. Current predicate rejects all
observed installer modes, with regression coverage. A supervisor killed ramrod at sealing progress 28%;
the target was resumed with original installer NVRAM, then reached normal OS boot.

`--resume-install --from-stage 2` preserves target/NVRAM. A **complete unattended corrected ISO rerun
is still NOT RUN**. Resumed-stage timing is not total build time.

Stage 3 refreshes the script INSIDE BaseSystem through stage 1, retries disk readiness, fails on guest
errors, checks an OS snapshot, injects payload, removes host keys/kcpassword/installer staging/license
cookie, sets METAL_ERROR_MODE=5, zeroes free extents, syncs and halts Recovery. SIP and sealed stock OS
remain intact. ACPI shutdown of the installed OS was not reliably handled; Recovery replays APFS and
checks the snapshot afterward. Do not call that ACPI timeout a clean shutdown.

Root could not write the protected com.apple.universalaccess.plist even with PlistBuddy. Current
firstboot no longer depends on that write; GUI helper applies preferences as GUI user after login,
which remains untested. Do not disable SIP to work around it.

Current builder target: work/r3/home/build/tahoe-26.3/target.qcow2. Repacked from 38.44 GB to 21.45 GB
after clean qemu-img check and complete comparison: **Images are identical**. logs/r3/repack-target.txt.
Final corrected base completed in p3-compact-10.txt; injection is p3-inject-9.txt.

One compaction failed because D: had **1.3 MB free**, not a demonstrated QEMU codec failure. Both QCOW2
checks were clean. New preflight uses qemu-img measure live allocation plus 2 GiB headroom; it also
records ISO SHA-256. Check space before full builds and temporary exports.

An earlier AHCI timeout recurred (stall-evidence.txt), disproving the one-off claim. Its mechanism is
still unproven. Do not attribute that historical timeout to disk full without contemporaneous evidence.
Read-only QMP diagnostic: crates/wsm-core/examples/watch_install.rs, optional --now.

## Storage cleanup already done

Removed only project artifacts: failed partial output; rejected fd62 base/sidecars with no dependents;
p1-trace.state; hl1/state.bin.failed (valid state.bin retained); inactive old build caches; generated
Ventura raw recovery copy (original tools/gpu-followup/ventura/BaseSystem.dmg retained).

Preserved ISO, source repos, portable toolchains, working VM disks and pinned runtime binaries.
The old QEMU/qemu-dbg object trees and debug-symbol executable were removed; rebuild if required.
Last free-space measurement BEFORE final compression: 52,466,102,272 bytes. Recheck live space.
logs/r3/project-usage.json is PRE-CLEANUP, not final disk use.

Import of an already-present verified base avoids an extra staging copy but hashes both source and
destination. P3 export needs one temporary copy and deletes it only after PASS. Inspect leftovers after failure.

## Remaining dev/GPU/package work

- tests/setup_dev.py is written, NOT RUN: three cached Apple CLT packages (work/dl-clt), hashes in
  guest/build/clt-pins.json; Homebrew main pinned to 51da7f234a4ca944ddc88d02cbbb0d11062eefcc.
  Make git remote setup idempotent before retrying partial runs.
- Optional builder --clt stage is implemented but NOT RUN. It creates no user and sanitizes keys again.
- tools/clt/CLTools_macOS_SDK.pkg has a SHA-1 mismatch against the cached Apple catalog; NOT pinned,
  used or shipped. Do not execute/add it without investigation. Proven three-package recipe remains.
- P6 script has 5 cold/resume samples, 20 warm calls, serial command timing, dev/bottle/source and
  resource checks. Review process exit/overall-pass logic: individual failures do not all fail its exit.
- tests/p6_stability.py implements real 10 cycles/1 hour idle and --gpu 30-minute checks. NOT RUN.
  GUI session alive is not continuous-frame proof; add actual stats measurements.
- Metal binary: work/gpu-followup/metaltest. Animation source: work/gpu-followup/anim.swift, compile in
  CLT guest and copy via CLI. New-base Metal/FPS/first-stall/prewarm/Finder drag NOT RUN.
- GUI ws_up() currently uses launchctl print (possibly loaded but inactive). Inspect actual process
  before trusting it. Hidden prewarm/window show-hide logic is not visually qualified. Static wallpaper
  setting is not implemented. A tiny AppKit helper using public NSWorkspace desktop-image API is an
  option after CLT is available; avoid unproven private preference hacks.
- Intel OC template exists in runtime; real Intel testing NOT RUN. Two AMD instances must at least
  prove regenerated identities. WHPX on AMD cannot emulate Intel-only MSRs by vendor spoof alone.
- Session identity uses kern.bootsessionuuid plus /tmp marker; clock resync shifts kern.boottime.

Preview ZIP: work/r3/dist/wsm-setup.zip, 50,606,272 bytes, 235 files, no base.
Preview hash: 126de51fc5667da87bb41d56c090a01a88c3c6c9ba9c913a99355e429b254448.
**STALE:** predates standard-handle fix and newest uninstall source. Rebuild before QA/release.

Newest unbuilt/untested source: uninstall --yes --portable skips GPU registry changes; package marker
validation; hidden PowerShell deferred self-removal because Windows cannot delete its running EXE;
src/build/uninstall.ps1 embedded and packaged. Test only a disposable project-local runtime, e.g.
tools/wsm-portable-test, install.ps1 -NoPath. Do not test in LOCALAPPDATA.

## Continuation order

1. Fix/test firstboot readiness race; preserve current base.
2. Build latest source, Clippy/tests, launcher/transport regressions if affected.
3. Reset only disposable fixtures; complete Generic identities/export/import.
4. SetupDev, full P6 timing/dev/resource tests, then GUI/Metal/prewarm/FPS/drag and real stability.
5. Rebuild/retest base if guest payload changes; genuinely unattended whole builder rerun with space.
6. Rebuild ZIP; portable install/import/create/exec/suspend/resume/uninstall QA inside project.
7. Final RESULTS P1-P7 table, actual P6 numbers, pins, named limits, disk used and outside changes.
   Unknowns are NOT RUN. A repaired image or code-only feature is not a production acceptance PASS.
