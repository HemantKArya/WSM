#!/bin/sh
# wsm-lite-local.sh: installed at /usr/local/libexec/ (data disk) and run as root at the end of com.wsm.lite.
# Everything BaseSystem lacks for development is wired up here, every boot (root is rw, /usr/bin and /etc are
# not persistent). Pieces whose files are missing do nothing.
exec >/dev/console 2>&1
CLT=/Library/Developer/CommandLineTools
W=/System/Library/WSM/run

# root over SSH, key-only, with the instance key (`wsm exec --root`): BaseSystem has no working sudo
# (sudo needs getgrouplist(), which needs a membership service BaseSystem does not ship).
if ! grep -q '^AllowUsers .* root' $W/sshd_config; then
    sed -i '' -e 's/^AllowUsers .*/& root/' -e 's/^PermitRootLogin .*/PermitRootLogin prohibit-password/' $W/sshd_config
    i=0; while [ ! -s $W/sshd.pid ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    kill -HUP "$(cat $W/sshd.pid)" 2>/dev/null
fi

# Homebrew's portable ruby loads fiddle -> /usr/lib/libffi.dylib, which BaseSystem's dyld cache lacks (extras pack: /usr/local/lib)
[ -e /usr/local/lib/libffi.8.dylib ] && ln -sfn /usr/local/lib/libffi.8.dylib /usr/lib/libffi.dylib

# Homebrew scratch space on the data disk (the root volume is small); read by brew as system-wide env
if [ -d /usr/local/Homebrew ]; then
    mkdir -p /Volumes/WSMData/brewtmp /usr/local/etc/homebrew
    chown 501:20 /Volumes/WSMData/brewtmp
    printf 'HOMEBREW_TEMP=/Volumes/WSMData/brewtmp\nHOMEBREW_NO_ANALYTICS=1\nHOMEBREW_NO_AUTO_UPDATE=1\n' > /usr/local/etc/homebrew/brew.env
    chown -R 501:20 /usr/local/etc
fi

# Command Line Tools unpacked onto the data disk: expose them where Homebrew and configure scripts look
[ -x $CLT/usr/bin/clang ] || exit 0
for t in $CLT/usr/bin/*; do
    n=${t##*/}
    [ -e /usr/bin/$n ] && [ ! -L /usr/bin/$n ] && continue   # never shadow a real BaseSystem tool
    ln -sfn $t /usr/bin/$n
done
# helper trees the tools find relative to /usr/bin (git-core, templates, ...)
for d in libexec share; do
    for t in $CLT/usr/$d/*; do
        [ -e /usr/$d/${t##*/} ] || ln -sfn $t /usr/$d/${t##*/}
    done
done
[ -e /usr/include ] || ln -sfn $CLT/usr/include /usr/include
# extras-pack tools (regular files in /usr/local/bin; brew's own are symlinks): Homebrew's superenv PATH has no /usr/local/bin
for t in /usr/local/bin/*; do
    [ -f "$t" ] && [ ! -L "$t" ] && [ ! -e /usr/bin/${t##*/} ] && ln -sfn $t /usr/bin/${t##*/}
done
ln -sfn $CLT /var/db/xcode_select_link
echo "WSM-LITE: CLT shims in /usr/bin"
exit 0
