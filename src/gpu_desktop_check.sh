#!/bin/sh
echo '=== power and console ==='
sudo pmset -a sleep 0 displaysleep 0 disksleep 0
stat -f '%Su' /dev/console
ps axo pid,comm | grep -E 'Setup Assistant|loginwindow|Dock.app|WindowServer|Finder.app'
echo '=== guest diagnostics ==='
uptime
system_profiler SPDisplaysDataType
echo '=== open Finder in the logged-in GUI session ==='
open -a Finder
echo '=== windows ==='
osascript -e 'tell application "Finder" to get name of every window'
