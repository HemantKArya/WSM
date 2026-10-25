#!/bin/sh
set -eu
dest=/Volumes/WSMHybrid
source='/Volumes/macOS Base System'
[ "$(diskutil info -plist "$dest" | plutil -extract VolumeUUID raw -o - -)" = 5363448F-62A5-4F78-908D-FB40803CB209 ] || exit 1
srcuuid=$(diskutil info -plist "$source" | plutil -extract VolumeUUID raw -o - -)
dstuuid=$(diskutil info -plist "$dest" | plutil -extract VolumeUUID raw -o - -)
[ -d "/Volumes/WSMSourcePreboot/$srcuuid" ] || exit 1
diskutil apfs changeVolumeRole disk6s1 S || echo 'System role conversion unavailable; test with explicit preboot UUID folder'
ditto --rsrc --extattr --acl --preserveHFSCompression "/Volumes/WSMSourcePreboot/$srcuuid" "/Volumes/WSMHybridPreboot/$dstuuid"
# The boot collections are byte-identical between Recovery and the full OS.
# Add the matching full system collection, which contains AppleParavirtGPU.
ditto --rsrc --extattr --acl --preserveHFSCompression /System/Library/KernelCollections "$dest/System/Library/KernelCollections"
if [ -d /System/Library/Perl ]; then
    ditto --rsrc --extattr --acl --preserveHFSCompression /System/Library/Perl "$dest/System/Library/Perl"
fi
cp /Users/dev/wsm-guitest "$dest/System/Library/WSM/guitest"
cp /Users/dev/wsm-metaltest "$dest/System/Library/WSM/metaltest"
chmod 755 "$dest/System/Library/WSM/guitest" "$dest/System/Library/WSM/metaltest"
bless --folder "$dest/System/Library/CoreServices" --bootefi --verbose
bless --info "$dest"
diskutil apfs updatePreboot "$dest" || true
du -sk "$dest" "$dest/System/Library/KernelCollections" "$dest/System/Library/Perl"
sync
echo BOOTFILES-DONE
