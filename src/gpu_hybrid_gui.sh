#!/bin/sh
# On-demand probe only; does not enable WindowServer for the next boot.
echo "BEFORE_GUI $(date -u)"
defaults write /Library/Preferences/.GlobalPreferences METAL_ERROR_MODE -int 5
defaults write -g METAL_ERROR_MODE -int 5
start=$(date +%s)
launchctl bootstrap system /System/Library/WSM/disabled/LaunchDaemons/com.apple.WindowServer.plist
launchctl kickstart system/com.apple.WindowServer
echo "WINDOWSERVER_REQUESTED $(date -u)"
METAL_ERROR_MODE=5 script -q /tmp/wsm-metal.log /System/Library/WSM/metaltest </dev/null
echo "AFTER_METAL $(date -u)"
METAL_ERROR_MODE=5 script -q /tmp/wsm-gui.log /System/Library/WSM/guitest </dev/null &
child=$!
i=0
while [ "$i" -lt 100 ]; do
    if grep -q 'window shown' /tmp/wsm-gui.log; then
        echo "FIRST_WINDOW_REPORTED elapsed_seconds=$(( $(date +%s) - start ))"
        break
    fi
    i=$((i+1)); sleep 0.2
done
wait "$child"
echo "AFTER_GUI $(date -u)"
