#!/bin/sh
set -eu
source=/Volumes/WSMHybrid
uuid=$(diskutil info -plist "$source" | plutil -extract VolumeUUID raw -o - -)
[ "$uuid" = 5363448F-62A5-4F78-908D-FB40803CB209 ] || exit 1
container=$(diskutil info -plist "$source" | plutil -extract APFSContainerReference raw -o - -)
if [ "${1:-}" = --resume ]; then
    diskutil mount EAC96B20-97FB-4AA9-819D-75328B420F7D
    dest=$(diskutil info -plist EAC96B20-97FB-4AA9-819D-75328B420F7D | plutil -extract MountPoint raw -o - -)
else
    dest=/Volumes/WSMHybridSystemTarget
    [ ! -e "$dest" ] || exit 1
    mkdir "$dest"
    diskutil apfs addVolume "$container" APFS WSMHybridSystem -role S -mountpoint "$dest"
fi
[ "$(diskutil info -plist "$dest" | plutil -extract APFSContainerReference raw -o - -)" = "$container" ] || exit 1
mount -uw "$dest"
ditto --rsrc --extattr --acl --preserveHFSCompression "$source" "$dest"
diskutil mount "${container}s2"
preboot=$(diskutil info -plist "${container}s2" | plutil -extract MountPoint raw -o - -)
newuuid=$(diskutil info -plist "$dest" | plutil -extract VolumeUUID raw -o - -)
ditto --rsrc --extattr --acl "$preboot/$uuid" "$preboot/$newuuid"
bless --folder "$dest/System/Library/CoreServices" --bootefi --verbose
diskutil info "$dest"
bless --info "$dest"
sync
echo HYBRID-SYSTEM-ROLE-READY
