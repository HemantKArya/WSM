#!/bin/sh
# wsm base builder, stage 1: runs once in single-user BaseSystem (root shell on the serial console,
# typed by the host after `mount -uw /` and extracting the seed disk to /). Installs the builder
# daemon so every later multi-user boot of this BaseSystem copy runs the job on the job disk.
set -e
mkdir -p /System/Library/WSM
cp /wsm-build/build.sh /System/Library/WSM/build.sh
chmod 755 /System/Library/WSM/build.sh
cp /wsm-build/com.wsm.build.plist /System/Library/LaunchDaemons/com.wsm.build.plist
chown 0:0 /System/Library/LaunchDaemons/com.wsm.build.plist
chmod 644 /System/Library/LaunchDaemons/com.wsm.build.plist
sync
echo "WSM-BUILD: stage1 done"
