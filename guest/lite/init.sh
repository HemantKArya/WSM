#!/bin/sh
# com.wsm.lite: per-boot setup of the lite (BaseSystem-derived) base. Runs as root from launchd.
# The base holds no identity: user, hostname and the public key come from the config disk (a raw
# ustar with wsm-config.json); host keys and the home directory live on the per-instance data disk
# (APFS volumes WSMData and WSMLocal), because BaseSystem's own Data volume is a tmpfs.
exec >/dev/console 2>&1
echo "WSM-LITE: init start"
W=/System/Library/WSM
mount -uw / 2>/dev/null
mkdir -p $W/run /var/db/wsm
C=$W/run/cfg
rm -rf $C $W/run/sshd_config.ready; mkdir -p $C

# 1. config disk: the disk whose first block is a ustar header naming wsm-config.json
CFG=""; i=0
while [ -z "$CFG" ] && [ $i -lt 80 ]; do
    for d in /dev/rdisk[0-9]*; do
        case "$d" in *s[0-9]*) continue ;; esac
        if dd if="$d" bs=512 count=1 2>/dev/null | head -c 20 | grep -q '^wsm-config.json'; then CFG=$d; break; fi
    done
    [ -z "$CFG" ] && { sleep 0.25; i=$((i + 1)); }
done
[ -n "$CFG" ] || { echo "WSM-LITE: FAIL no config disk"; exit 1; }
tar -xf "$CFG" -C "$C" || { echo "WSM-LITE: FAIL config unreadable"; exit 1; }
USER_NAME=$(plutil -extract user raw -o - "$C/wsm-config.json")
HOST=$(plutil -extract hostname raw -o - "$C/wsm-config.json")
case "$USER_NAME" in ''|*[!a-z0-9_-]*) echo "WSM-LITE: FAIL bad user"; exit 1 ;; esac
case "$HOST" in ''|*[!A-Za-z0-9-]*) echo "WSM-LITE: FAIL bad hostname"; exit 1 ;; esac

# 2. template build only: format the blank data disk (sector 0 all zero) as WSMData + WSMLocal, then halt
if [ -f "$C/format-data" ]; then
    # physical disks only: synthesized APFS container disks also have an all-zero sector 0
    B=""; i=0
    while [ -z "$B" ] && [ $i -lt 60 ]; do
        for d in $(diskutil list physical | awk '/^\/dev\/disk[0-9]+ / {print $1}'); do
            if [ -z "$(dd if="/dev/r${d#/dev/}" bs=512 count=1 2>/dev/null | od -v -An -tx1 | tr -d ' \n0')" ]; then B=${d#/dev/}; break; fi
        done
        [ -z "$B" ] && { sleep 0.5; i=$((i + 1)); }
    done
    [ -n "$B" ] || { diskutil list; echo "WSM-LITE: FAIL no blank disk"; exit 1; }
    diskutil eraseDisk APFS WSMData GPT "/dev/$B" || { echo "WSM-LITE: FAIL eraseDisk"; exit 1; }
    CONT=$(diskutil list "$B" | awk '/Apple_APFS Container/ {print $4; exit}')
    diskutil apfs addVolume "$CONT" APFS WSMLocal || { echo "WSM-LITE: FAIL addVolume"; exit 1; }
    # optional extras pack (tools missing from BaseSystem, taken from a full base) into WSMLocal = /usr/local
    if [ -f "$C/extras.tgz" ]; then
        tar -xzf "$C/extras.tgz" -C /Volumes/WSMLocal && echo "WSM-LITE: extras installed"
    fi
    sync
    echo "WSM-LITE: FORMATTED"
    /sbin/halt
    exit 0
fi

# 3. data volumes: WSMData at /Volumes/WSMData, WSMLocal at /usr/local (Homebrew needs its own volume)
i=0
until diskutil mount WSMData >/dev/null 2>&1 || [ -d /Volumes/WSMData/Users ] || [ $i -gt 80 ]; do i=$((i + 1)); sleep 0.25; done
D=/Volumes/WSMData
[ -d $D ] || { echo "WSM-LITE: FAIL no WSMData volume"; exit 1; }
if [ -f $W/local-skip ]; then :; else
    diskutil unmount WSMLocal >/dev/null 2>&1
    [ -L /usr/local ] && rm /usr/local
    mkdir -p /usr/local
    diskutil mount -mountPoint /usr/local WSMLocal >/dev/null 2>&1 && echo "WSM-LITE: /usr/local mounted"
fi

# 4. user (BaseSystem's libinfo only sees the flat files; BaseSystem has bash but no zsh), home and key on the data disk
H=$D/Users/$USER_NAME
grep -q "^$USER_NAME:" /etc/master.passwd || echo "$USER_NAME:*:501:20::0:0:$USER_NAME:$H:/bin/bash" >> /etc/master.passwd
grep -q "^$USER_NAME:" /etc/passwd || echo "$USER_NAME:*:501:20:$USER_NAME:$H:/bin/bash" >> /etc/passwd
mkdir -p "$H/.ssh" "$D/ssh" "$D/tmp" "$D/opt" "$D/Library/Developer"
cp "$C/authorized_keys" "$H/.ssh/authorized_keys"
chown 501:20 "$H"; chown -R 501:20 "$H/.ssh"; chmod 700 "$H/.ssh"; chmod 600 "$H/.ssh/authorized_keys"
for l in opt Library/Developer; do [ -L /$l ] || { rm -rf /$l 2>/dev/null; ln -s $D/$l /$l; }; done
# no sudo in BaseSystem: the user owns its developer locations (Homebrew, CLT unpack) on its own volumes
if [ ! -f $D/.wsm-owned ]; then
    chown -R 501:20 /usr/local "$D/opt" "$D/Library/Developer" && touch $D/.wsm-owned
fi
# what `xcode-select -s` would do (/var/db is on the runtime tmpfs, so every boot)
[ -d $D/Library/Developer/CommandLineTools ] && ln -sfn /Library/Developer/CommandLineTools /var/db/xcode_select_link
echo "$USER_NAME" > /var/db/wsm/user

# 5. host name and per-instance host key
scutil --set HostName "$HOST"; scutil --set LocalHostName "$HOST"; hostname "$HOST"
# BaseSystem has no ssh-keygen: the host creates this instance's key on the config disk; kept on the data disk
if [ ! -f "$D/ssh/ssh_host_ed25519_key" ]; then
    if [ -f "$C/ssh_host_ed25519_key" ]; then cp "$C/ssh_host_ed25519_key" "$D/ssh/ssh_host_ed25519_key"
    else /usr/bin/ssh-keygen -q -t ed25519 -N '' -f "$D/ssh/ssh_host_ed25519_key"; fi
fi
chmod 600 "$D/ssh/ssh_host_ed25519_key"
pmset -a sleep 0 displaysleep 0 disksleep 0 2>/dev/null

# 6. sshd: key-only, our config generated per boot
cat > $W/run/sshd_config <<EOF
Port 22
ListenAddress 0.0.0.0
HostKey $D/ssh/ssh_host_ed25519_key
PermitRootLogin no
AllowUsers $USER_NAME
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
AuthorizedKeysFile $H/.ssh/authorized_keys
PidFile $W/run/sshd.pid
UseDNS no
SetEnv PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin MAGIC=/usr/local/share/file/magic.mgc
EOF
for s in /usr/libexec/sftp-server /usr/local/libexec/sftp-server; do
    [ -x $s ] && { echo "Subsystem sftp $s" >> $W/run/sshd_config; break; }
done
# com.wsm.sshd (KeepAlive) starts sshd once this file exists
/usr/sbin/sshd -t -f $W/run/sshd_config && touch $W/run/sshd_config.ready && echo "WSM-SSHD-READY"
[ -x /usr/local/libexec/wsm-lite-local.sh ] && /usr/local/libexec/wsm-lite-local.sh
exit 0
