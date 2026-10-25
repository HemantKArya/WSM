# Roadmap and known gaps

Status after v0.1. "NOT RUN" means implemented or planned but never measured; see [RESULTS.md](../RESULTS.md).

## Next (small, well understood)
- Lite base: `wsm` command to install the Command Line Tools and Homebrew (today: manual steps in the README).
- Lite base: wire `local.sh` earlier or hold the SSH-ready marker until it has run (first `exec` right after boot can race the CLT links).
- Lite base: guest does not power off by itself on `wsm stop --shutdown` (60 s timeout fallback).
- Lite base: a working serial login and a working `id`/group list (needs a membership service BaseSystem lacks); sudo is replaced by `wsm exec --root`.
- Lite base: `shasum`, `top`, `uptime` (missing Perl modules / dyld cache libraries).
- Ship the tools-pack build as a script (today: tar of binaries taken from a full base).

## Needs hardware or time (NOT RUN)
- Intel CPU hosts (the Intel OpenCore image exists and is untested).
- 1 h idle, 20-cycle suspend A/B, 30 min GUI soak, Finder drag, two GPU instances.
- Drive mapping with SSHFS-Win/WinFsp (WebDAV `wsm mount` is the supported path).

## Larger ideas
- GPU instances that can be suspended (the paravirtual GPU blocks migration today).
- 60 fps presentation and smoother window drag (30-45 fps, ~12 Hz drag in v0.1).
- Newer macOS: Tahoe 26.x is the last Intel release, so the base cannot go further.
- Upstream the six QEMU WHPX patches (`src/patches/`) and the AMD OpenCore notes.
- Detached jobs that survive a cold boot (launchd-managed jobs) and a `wsm jobs --log` reader.
- CI that builds the package from a clean checkout.

## Not planned
- Bundling any Apple software, installer or base image.
