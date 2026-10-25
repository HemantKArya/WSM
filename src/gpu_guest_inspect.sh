#!/bin/sh
echo '=== identity and preferences ==='
id
defaults read -g METAL_ERROR_MODE
defaults read /Library/Preferences/com.apple.loginwindow autoLoginUser
ls -ld ~/Library/Preferences
echo '=== display ==='
system_profiler SPDisplaysDataType
ioreg -rc AppleParavirtGPUControl -l | head -30
kextstat | grep -E 'ReimsVgpu|Lilu|ParavirtGPU'
echo '=== GUI processes ==='
ps axo pid,comm | grep -E 'Setup|loginwindow|Dock|WindowServer'
