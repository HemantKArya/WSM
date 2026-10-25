#!/bin/sh
# com.wsm.firstboot: one-shot, per-instance identity. Runs as root at first boot of an instance.
# The base image contains NO host keys, user, password or authorized key; all of it comes from
# the config disk (a raw ustar archive on a SATA disk) that `wsm create` generates.
exec >>/var/log/wsm-firstboot.log 2>&1
set -eu
umask 077
DONE=/var/db/wsm/firstboot.done
[ -f "$DONE" ] && exit 0
mkdir -p /var/db/wsm

# 1. find the config disk: the one disk whose first block is a ustar header naming wsm-config.json
CFG=""
for d in /dev/rdisk[0-9]*; do
    case "$d" in *s[0-9]*) continue ;; esac
    if dd if="$d" bs=512 count=1 2>/dev/null | head -c 20 | grep -q '^wsm-config.json'; then CFG=$d; break; fi
done
if [ -z "$CFG" ]; then echo "no config disk"; exit 1; fi
W=$(mktemp -d /var/db/wsm/cfg.XXXXXX)
tar -xf "$CFG" -C "$W" || exit 1

# plutil reads JSON and is always present; /usr/bin/python3 is only a CLT-installer stub on a base without CLT.
USER_NAME=$(plutil -extract user raw -o - "$W/wsm-config.json")
HOST=$(plutil -extract hostname raw -o - "$W/wsm-config.json")
case "$USER_NAME" in ''|*[!a-z0-9_-]*) echo "bad user name"; exit 1 ;; esac
case "$HOST" in ''|*[!A-Za-z0-9-]*) echo "bad hostname"; exit 1 ;; esac

# 2. hostname
scutil --set HostName "$HOST"; scutil --set LocalHostName "$HOST"; scutil --set ComputerName "$HOST"

# 3. SSH host keys (unique per instance)
ssh-keygen -A

# 4. user: random password (never leaves the guest; only needed for GUI autologin), key-only SSH, passwordless sudo
PW_FILE=/var/db/wsm/firstboot-password
if [ ! -s "$PW_FILE" ]; then
    LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 24 > "$PW_FILE"
fi
PW=$(cat "$PW_FILE")
[ "${#PW}" -eq 24 ] || exit 1
if ! id "$USER_NAME" >/dev/null 2>&1; then
    sysadminctl -addUser "$USER_NAME" -fullName "$USER_NAME" -UID 501 -password "$PW" -admin -home "/Users/$USER_NAME" -shell /bin/zsh
fi
id "$USER_NAME" >/dev/null
dscl . -read "/Users/$USER_NAME" AuthenticationAuthority | grep -q ShadowHash
HOME_DIR=$(dscl . -read "/Users/$USER_NAME" NFSHomeDirectory | awk '{print $2}')
[ -d "$HOME_DIR" ] || createhomedir -c -u "$USER_NAME"
mkdir -p "$HOME_DIR/.ssh"
cp "$W/authorized_keys" "$HOME_DIR/.ssh/authorized_keys"
chown -R "$USER_NAME" "$HOME_DIR/.ssh"; chmod 700 "$HOME_DIR/.ssh"; chmod 600 "$HOME_DIR/.ssh/authorized_keys"
mkdir -p /etc/sudoers.d /etc/ssh/sshd_config.d
echo "$USER_NAME ALL=(ALL) NOPASSWD: ALL" > "/etc/sudoers.d/wsm-$USER_NAME"; chmod 440 "/etc/sudoers.d/wsm-$USER_NAME"
visudo -cf "/etc/sudoers.d/wsm-$USER_NAME"
cat > /etc/ssh/sshd_config.d/010-wsm.conf <<EOF
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
AllowUsers $USER_NAME
EOF
chmod 644 /etc/ssh/sshd_config.d/010-wsm.conf
/usr/sbin/sshd -t

# 5. GUI autologin material (used only when `wsm gui start` enables the session)
/usr/bin/perl -e '
    my @k = (0x7d,0x89,0x52,0x23,0xd2,0xbc,0xdd,0xea,0xa3,0xb9,0x1f);
    my @p = unpack("C*", $ARGV[0]); push @p, 0;
    push @p, 0 while (@p % 12);
    my $o = ""; for my $i (0..$#p) { $o .= chr($p[$i] ^ $k[$i % 11]); }
    open(F, ">/etc/kcpassword") or die; binmode F; print F $o; close F;
' "$PW"
chmod 600 /etc/kcpassword
echo "$USER_NAME" > /var/db/wsm/user

# Accessibility preferences are protected even from root while macOS is running. The GUI helper
# applies them as the logged-in GUI user after its session exists; do not block headless provisioning.

# 6. skip Setup Assistant for the new user
touch /var/db/.AppleSetupDone

# 7. never sleep: a sleeping guest takes the NIC and every SSH session down (S3/S4 are also off in QEMU)
pmset -a sleep 0 disksleep 0 displaysleep 0 standby 0 autopoweroff 0 hibernatemode 0 powernap 0

# 8. headless instances: no consumer/AI/indexing user agents (round 3 P6: they kept the guest at load
#    7-24 after every login). GPU instances keep them: the GUI session needs icons, fonts, etc.
if ! ioreg -r -c AppleParavirtGPUControl 2>/dev/null | grep -q AppleParavirtGPU; then
    UID_N=$(id -u "$USER_NAME")
    grep -v '^#' /usr/local/share/wsm/launchctl-user-disable.txt | grep . | while read -r l; do
        launchctl disable "user/$UID_N/$l" || true
    done
fi

rm -rf "$W"
date > "$DONE"
rm -f "$PW_FILE"
launchctl disable system/com.wsm.firstboot
exit 0
