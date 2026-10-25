#!/bin/sh
# Run once in single-user BaseSystem (seed disk extracted to /wsm-lite): turns the BaseSystem copy into
# the generic lite base. Nothing machine-specific is written.
S=/wsm-lite
W=/System/Library/WSM
set -x
mount -uw / || exit 1
mkdir -p $W/run $W/disabled/LaunchDaemons $W/disabled/LaunchAgents
cp $S/init.sh $S/serial.sh $S/sshd.sh $W/
chmod 755 $W/init.sh $W/serial.sh $W/sshd.sh
chown -R root:wheel $W
for p in com.wsm.lite.plist com.wsm.serial.plist com.wsm.sshd.plist; do
    cp $S/$p /System/Library/LaunchDaemons/$p
    chmod 644 /System/Library/LaunchDaemons/$p; chown root:wheel /System/Library/LaunchDaemons/$p
done
# GUI / installer / Recovery stack off (moved, not deleted: restore by moving back)
for f in $(cat $S/disable-daemons.txt); do
    [ -f /System/Library/LaunchDaemons/$f ] && mv /System/Library/LaunchDaemons/$f $W/disabled/LaunchDaemons/
done
for f in $(cat $S/disable-agents.txt); do
    [ -f /System/Library/LaunchAgents/$f ] && mv /System/Library/LaunchAgents/$f $W/disabled/LaunchAgents/
done
# /etc lives on the runtime tmpfs here, and BaseSystem has no zsh: PATH comes from sshd SetEnv (init.sh)
set +x
echo "--- remaining LaunchDaemons: $(ls /System/Library/LaunchDaemons | wc -l)"
df -h / | tail -1
sync
echo WSM-LITE-SETUP-DONE
