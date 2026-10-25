#!/bin/sh
# Optional base-builder tools stage. Runs on the installed full OS, without creating a user.
set -eu
exec >/dev/console 2>&1
trap 'rc=$?; [ "$rc" -eq 0 ] || echo "WSM-CLT: FAIL status $rc"' 0
PINS=/usr/local/share/wsm/clt-pins.json
W=/private/var/tmp/wsm-clt
mkdir -p "$W"
i=0
while [ "$i" -lt 3 ]; do
    name=$(plutil -extract "$i.name" raw -o - "$PINS")
    url=$(plutil -extract "$i.url" raw -o - "$PINS")
    sum=$(plutil -extract "$i.sha256" raw -o - "$PINS")
    echo "WSM-CLT: downloading $name"
    curl -fL --retry 5 --retry-delay 2 --connect-timeout 20 --max-time 1800 -o "$W/$name" "$url"
    echo "$sum  $W/$name" | shasum -a 256 -c -
    pkgutil --check-signature "$W/$name"
    /usr/sbin/installer -pkg "$W/$name" -target /
    rm -f "$W/$name"
    i=$((i + 1))
done
xcode-select -s /Library/Developer/CommandLineTools
clang --version
swiftc --version
git --version
rm -f /Library/LaunchDaemons/com.wsm.clt.plist
rm -rf "$W"
sync
echo 'WSM-CLT: done'
trap - 0
shutdown -h now
