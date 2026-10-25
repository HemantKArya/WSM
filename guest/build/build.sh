#!/bin/sh
# wsm base builder daemon (BaseSystem, every multi-user boot). Reads the job disk the host attached
# (a raw ustar archive whose first member is wsm-job.env) and runs one job:
#   MODE=install  erase the blank target disk and run startosinstall onto it (the installer reboots
#                 into its own phases on the target; the host watches the serial console)
#   MODE=inject   put the generic first-boot service, trim list and setup flags on the installed
#                 Data volume, then power off
# Every line the host matches starts with "WSM-BUILD:". Output goes to /dev/console = serial.
say() { echo "WSM-BUILD: $*"; }
fail() { say "FAIL $*"; sync; sleep 2; halt; exit 1; }
# Never print a completion marker after a failed copy, plist write or filesystem operation.
set -e
trap 'rc=$?; [ "$rc" -eq 0 ] || fail "command failed (status $rc)"' 0

JOB=""
for d in /dev/rdisk[0-9]*; do
    case "$d" in *s[0-9]*) continue ;; esac
    if dd if="$d" bs=512 count=1 2>/dev/null | head -c 12 | grep -q '^wsm-job.env'; then JOB=$d; break; fi
done
[ -n "$JOB" ] || { say "no job disk; idle"; exit 0; }
W=/tmp/wsm-job; rm -rf "$W"; mkdir -p "$W"
tar -xf "$JOB" -C "$W" || fail "cannot read job disk $JOB"
. "$W/wsm-job.env"
say "job $MODE from $JOB"

# Whole disk (no slice) whose size in bytes equals $1.
disk_of_size() {
    for d in $(diskutil list | awk '/^\/dev\/disk[0-9]+ /{print $1}'); do
        b=$(diskutil info "$d" | awk -F'[()]' '/Disk Size:/{split($2, a, " "); print a[1]}')
        [ "$b" = "$1" ] && { echo "$d"; return; }
    done
}

case "$MODE" in
install)
    # This runs at boot, before DiskArbitration/diskutil are ready: retry instead of failing.
    T=""; i=0
    while [ $i -lt 60 ]; do
        T=$(disk_of_size "$TARGET_BYTES") || true; [ -n "$T" ] && break
        sleep 2; i=$((i + 1))
    done
    [ -n "$T" ] || fail "no target disk of $TARGET_BYTES bytes after 120 s"
    say "target $T"
    diskutil eraseDisk APFS WSM GPT "$T" || fail "eraseDisk"
    APP=""
    i=0
    while [ $i -lt 120 ]; do
        for v in /Volumes/Install\ macOS*; do
            [ -x "$v/Install macOS Tahoe.app/Contents/Resources/startosinstall" ] && APP="$v/Install macOS Tahoe.app"
        done
        [ -n "$APP" ] && break
        # The installer image is an APM disk with one HFS+ partition; mount it if automount hasn't.
        for p in $(diskutil list | awk '/Apple_HFS/{print $NF}'); do diskutil mount readOnly "$p" >/dev/null 2>&1; done
        sleep 1; i=$((i + 1))
    done
    [ -n "$APP" ] || fail "installer volume not found"
    say "installer $APP"
    say "startosinstall begin"
    # Tahoe's startosinstall has no --nointeraction; as root in Recovery no --user is needed.
    rc=0
    "$APP/Contents/Resources/startosinstall" --agreetolicense --volume /Volumes/WSM || rc=$?
    say "startosinstall exit $rc"
    trap - 0
    ;;
inject)
    DEV=""; i=0
    while [ "$i" -lt 60 ]; do
        DEV=$(diskutil list | awk '/APFS Volume WSM - Data /{print $NF}' | head -1)
        [ -n "$DEV" ] && break
        sleep 2; i=$((i + 1))
    done
    [ -n "$DEV" ] || fail "no 'WSM - Data' volume after 120 s"
    diskutil mount "$DEV" >/dev/null || fail "mount $DEV"
    D=$(diskutil info "$DEV" | awk -F': *' '/Mount Point:/{print $2}')
    [ -d "$D" ] || fail "no mount point for $DEV"
    say "data volume $DEV at $D"
    SYS=$(diskutil list | awk '/APFS Volume WSM / && !/WSM - Data/ {print $NF}' | head -1)
    [ -n "$SYS" ] || fail "no installed system volume"
    diskutil apfs listSnapshots "$SYS" | tee "$W/system-snapshots.txt"
    grep -q 'com.apple.os.update-' "$W/system-snapshots.txt" || fail "installation has no sealed OS snapshot; installer phases are incomplete"
    # The installer's staging folder (UpdateBundle, BaseSystem.dmg: ~16 GB) is normally removed by the
    # first boot after the install; drop it if it is still there so the base stays small.
    if [ -d "$D/macOS Install Data" ]; then
        say "removing installer staging: $(du -sk "$D/macOS Install Data" | cut -f1) KB"
        rm -rf "$D/macOS Install Data"
    fi
    P="$W/payload"
    mkdir -p "$D/usr/local/libexec" "$D/Library/LaunchDaemons" "$D/private/var/db/com.apple.xpc.launchd"
    # BaseSystem has no install(1) (round 1 X4): cp + chmod.
    put() { cp "$1" "$2" && chown 0:0 "$2" && chmod "$3" "$2"; }
    for s in wsm-firstboot.sh wsm-init.sh wsm-serial.sh; do put "$P/$s" "$D/usr/local/libexec/$s" 755; done
    put "$P/wsm-gui.sh" "$D/usr/local/libexec/wsm-gui" 755
    for p in com.wsm.firstboot com.wsm.init com.wsm.serial; do put "$P/$p.plist" "$D/Library/LaunchDaemons/$p.plist" 644; done
    mkdir -p "$D/usr/local/share/wsm"
    put "$P/launchctl-user-disable.txt" "$D/usr/local/share/wsm/launchctl-user-disable.txt" 644
    if [ "${CLT:-0}" = 1 ]; then
        put "$P/clt.sh" "$D/usr/local/libexec/wsm-clt.sh" 755
        put "$P/clt-pins.json" "$D/usr/local/share/wsm/clt-pins.json" 644
        put "$P/com.wsm.clt.plist" "$D/Library/LaunchDaemons/com.wsm.clt.plist" 644
    fi
    # Offline `launchctl disable` for the trim list; sshd enabled (it is off on a fresh install).
    L="$D/private/var/db/com.apple.xpc.launchd/disabled.plist"
    {
        echo '<?xml version="1.0" encoding="UTF-8"?>'
        echo '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
        echo '<plist version="1.0"><dict>'
        grep -v '^#' "$P/launchctl-disable.txt" | grep . | while read -r l; do echo "<key>$l</key><true/>"; done
        echo '<key>com.openssh.sshd</key><false/>'
        echo '</dict></plist>'
    } > "$L"
    chmod 644 "$L"
    # All users, including root, need this before their first Metal process starts (G1).
    defaults write "$D/Library/Preferences/.GlobalPreferences" METAL_ERROR_MODE -int 5
    chmod 644 "$D/Library/Preferences/.GlobalPreferences.plist"
    # `wsm exec`/`wsm shell` are remote-login sessions. Without this grant macOS privacy protection
    # (TCC) returns EPERM for ~/Documents, ~/Desktop, ~/Downloads and network volumes to every SSH
    # process, root included (round 3: `ls ~/Documents` -> Operation not permitted; mount_webdav
    # mounted but every access failed). This is the System Settings > Sharing > Remote Login option
    # "Allow full disk access for remote users"; SIP blocks writing it while macOS runs, so it is set
    # offline here. Requirement blob = csreq of 'identifier "com.apple.sshd-keygen-wrapper" and anchor apple'.
    TCC="$D/Library/Application Support/com.apple.TCC/TCC.db"
    # Best-effort: if sqlite3 cannot be obtained or run, skip the grants (wsm share and ~/Documents
    # access over SSH are then unsupported) and continue.
    TCC_OK=1
    [ -f "$TCC" ] || { say "WARN: no system TCC database at $TCC; skipping remote-login TCC grants"; TCC_OK=0; }
    if [ "$TCC_OK" = 1 ] && ! command -v sqlite3 >/dev/null; then
        # BaseSystem ships libsqlite3 but no sqlite3 tool; try the installed OS copy from a read-only
        # mount of the sealed System volume (the seal is untouched).
        SYSDEV=$(diskutil list | awk '/APFS Volume WSM / && !/WSM - Data/ {print $NF}' | head -1)
        if [ -n "$SYSDEV" ] && diskutil mount readOnly "$SYSDEV" >/dev/null 2>&1; then
            SYSMNT=$(diskutil info "$SYSDEV" | awk -F': *' '/Mount Point:/{print $2}')
            cp "$SYSMNT/usr/bin/sqlite3" /tmp/sqlite3 2>/dev/null && chmod 755 /tmp/sqlite3
            diskutil unmount "$SYSDEV" >/dev/null 2>&1
            export PATH="/tmp:$PATH"
        fi
        sqlite3 -version >/dev/null 2>&1 || { say "WARN: no runnable sqlite3 in BaseSystem; skipping remote-login TCC grants (wsm share and ~/Documents over SSH unsupported)"; TCC_OK=0; }
    fi
    if [ "$TCC_OK" = 1 ]; then
    REQ="X'fade0c000000003c0000000100000006000000020000001d636f6d2e6170706c652e737368642d6b657967656e2d7772617070657200000000000003'"
    for svc in kTCCServiceSystemPolicyAllFiles kTCCServiceSystemPolicyNetworkVolumes; do
        for client in "com.apple.sshd-keygen-wrapper 0" "/usr/libexec/sshd-keygen-wrapper 1"; do
            set -- $client
            sqlite3 "$TCC" "INSERT OR REPLACE INTO access (service, client, client_type, auth_value, auth_reason, auth_version, csreq, flags, last_modified) VALUES ('$svc', '$1', $2, 2, 4, 1, $REQ, 0, CAST(strftime('%s','now') AS INTEGER));"
        done
    done
    say "remote-login TCC grants: $(sqlite3 "$TCC" "SELECT count(*) FROM access WHERE client LIKE '%sshd-keygen-wrapper' AND auth_value = 2;")"
    fi
    # `wsm exec` runs a non-login `zsh -c`; macOS applies /etc/paths (with /usr/local/bin, where
    # Homebrew and CLT shims live) only in /etc/zprofile, i.e. for login shells. zshenv is read by
    # every zsh. (Round 3 P6: `command -v brew` failed under `wsm exec` with Homebrew installed.)
    Z="$D/private/etc/zshenv"
    if ! grep -q 'wsm: path_helper' "$Z" 2>/dev/null; then
        printf '%s\n' '# wsm: path_helper for non-login shells too (wsm exec, ssh host cmd)' \
            'if [ -x /usr/libexec/path_helper ]; then eval "$(/usr/libexec/path_helper -s)"; fi' >> "$Z"
        chmod 644 "$Z"
    fi
    # A generic disk must never carry credentials generated by the installer environment.
    rm -f "$D"/private/etc/ssh/ssh_host_* "$D/private/etc/kcpassword"
    touch "$D/private/var/db/.AppleSetupDone"
    # The installer leaves a terms-of-service cookie; while it exists loginwindow starts Setup
    # Assistant's springboard as _mbsetupuser (+ software WindowServer, ~200 % CPU for minutes) at
    # every boot even with .AppleSetupDone. Verified round 3: log "System requires license: 1" ->
    # "0", load 90 s after boot 14.8 -> 1.8, `wsm exec true` ~2 s -> 0.45 s.
    rm -f "$D/private/var/db/.AppleSetupTermsOfService"
    # Removing the compressed installer payload does not zero its old APFS extents. Without this,
    # qemu-img convert retains the deleted payload and the distributable stays needlessly large.
    # Keep 2 GiB free for APFS metadata; QEMU detects these zero writes and unmaps their clusters.
    free_kb=$(df -k "$D" | awk 'NR == 2 {print $4}')
    blocks=$(( (free_kb - 2097152) / 8192 ))
    if [ "$blocks" -gt 0 ]; then
        say "zeroing $blocks free-space blocks (8 MiB), retaining 2 GiB free"
        dd if=/dev/zero of="$D/.wsm-zero-free-space" bs=8m count="$blocks" || {
            rm -f "$D/.wsm-zero-free-space"; fail "zero free space"
        }
        sync
        rm -f "$D/.wsm-zero-free-space"
    fi
    say "injected $(grep -c . "$P/launchctl-disable.txt") disabled services"
    sync
    trap - 0
    say "inject done"
    halt
    ;;
*)
    fail "unknown MODE '$MODE'"
    ;;
esac
