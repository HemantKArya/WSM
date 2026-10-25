#!/bin/sh
# Run as root only in hybrid-headless-builder, whose blank 20 GiB disk is disk2.
set -eu
target=disk2
source='/Volumes/macOS Base System'
dest=/Volumes/WSMHybrid
cache=/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld
bytes=$(diskutil info -plist "$target" | plutil -extract TotalSize raw -o - -)
[ "$bytes" = 21474836480 ] || { echo 'REFUSE: target size changed'; exit 1; }
[ "$(diskutil info -plist "$target" | plutil -extract WholeDisk raw -o - -)" = true ] || exit 1
[ -f "$source/System/Library/WSM/init.sh" ] || { echo 'REFUSE: source not the WSM base'; exit 1; }
[ -f "$cache/dyld_shared_cache_x86_64h" ] || exit 1
echo "BEGIN $(date -u) target=$target bytes=$bytes"
if [ "${1:-}" = --resume ]; then
    [ "$(diskutil info -plist "$dest" | plutil -extract VolumeName raw -o - -)" = WSMHybrid ] || exit 1
    [ -f "$dest/System/Library/WSM/init.sh" ] || exit 1
    /Library/Developer/CommandLineTools/usr/bin/python3 /Users/dev/gpu_layer_types.py
else
    diskutil eraseDisk APFS WSMHybrid GPT "$target"
    echo "COPY BASE $(date -u)"
    ditto --rsrc --extattr --acl --preserveHFSCompression "$source" "$dest"
fi
echo "COPY FULL-OS LAYER $(date -u)"
for path in usr/bin usr/lib usr/libexec System/Library/Frameworks System/Library/PrivateFrameworks System/Library/Extensions; do
    echo "COPY $path $(date -u)"
    ditto --rsrc --extattr --acl --preserveHFSCompression "/$path" "$dest/$path"
done
echo "COPY CACHE $(date -u)"
ditto --rsrc --extattr --acl --preserveHFSCompression "$cache" "$dest/System/Library/dyld"
echo 'LAYER COPY FINISHED'
diskutil info -plist "$dest" > /Users/dev/hybrid-volume.plist
container=$(diskutil info -plist "$dest" | plutil -extract APFSContainerReference raw -o - -)
diskutil apfs addVolume "$container" APFS Preboot -role B
diskutil apfs updatePreboot "$dest" || echo 'updatePreboot needs explicit boot files; inspect below'
bless --folder "$dest/System/Library/CoreServices" --bootefi --verbose || echo 'bless needs explicit preboot; inspect below'
du -sk "$dest" "$dest/System/Library/dyld" "$dest/System/Library/Frameworks" "$dest/System/Library/PrivateFrameworks" "$dest/usr"
diskutil list "$target"
bless --info "$dest" || true
sync
echo "BUILD COPY DONE $(date -u)"
