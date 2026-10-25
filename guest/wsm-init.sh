#!/bin/sh
# com.wsm.init: per-boot fixups. Must stay cheap: it runs on every cold boot.
exec >>/var/log/wsm-init.log 2>&1
# GPU instances: warm WindowServer once, off the critical path (measured: the first session after boot
# presents 0 frames for ~20 s; a warm one runs at 56-58 Hz).
if [ ! -f /var/db/wsm/no-prewarm ] && ioreg -r -c AppleParavirtGPUControl 2>/dev/null | grep -q AppleParavirtGPU; then
    ( sleep 5; /usr/local/libexec/wsm-gui prewarm ) &
fi
exit 0
