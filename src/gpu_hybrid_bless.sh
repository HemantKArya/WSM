#!/bin/sh
set -eu
dest=/Volumes/WSMHybrid
uuid=$(diskutil info -plist "$dest" | plutil -extract VolumeUUID raw -o - -)
[ "$uuid" = 5363448F-62A5-4F78-908D-FB40803CB209 ] || exit 1
preboot=$(diskutil info -plist disk6s2 | plutil -extract MountPoint raw -o - -)
[ "$(diskutil info -plist "$preboot" | plutil -extract DeviceIdentifier raw -o - -)" = disk6s2 ] || exit 1
echo "Actual target Preboot mount: $preboot"
ditto --rsrc --extattr --acl --preserveHFSCompression /Volumes/WSMSourcePreboot/66D9F403-1A73-4DB0-83C7-139A1DEA789E "$preboot/$uuid"
bless --folder "$dest/System/Library/CoreServices" --bootefi --verbose
bless --info "$dest"
du -sk "$dest" "$preboot"
sync
echo HYBRID-BLESSED
