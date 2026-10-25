"""Resolve file/directory/symlink conflicts only on the new WSMHybrid volume."""
import os
from pathlib import Path
import plistlib
import stat
import subprocess

dest = Path('/Volumes/WSMHybrid')
assert dest.resolve() == dest and dest.is_mount()
info = plistlib.loads(subprocess.check_output(['diskutil', 'info', '-plist', str(dest)]))
assert info['VolumeName'] == 'WSMHybrid'
assert os.stat(dest).st_dev != os.stat('/').st_dev
backup = dest / '.wsm-layer-type-backups'
paths = ['usr/bin', 'usr/lib', 'usr/libexec', 'System/Library/Frameworks',
         'System/Library/PrivateFrameworks', 'System/Library/Extensions']
count = 0
for relative in paths:
    source = Path('/') / relative
    for parent, dirs, files in os.walk(source, followlinks=False):
        for name in dirs + files:
            src = Path(parent) / name
            rel = src.relative_to('/')
            dst = dest / rel
            src_kind = stat.S_IFMT(src.lstat().st_mode)
            if os.path.lexists(dst) and stat.S_IFMT(dst.lstat().st_mode) != src_kind:
                parked = backup / rel
                assert not os.path.lexists(parked), parked
                parked.parent.mkdir(parents=True, exist_ok=True)
                dst.rename(parked)
                count += 1
                print('PARK type conflict:', rel, flush=True)
            if stat.S_ISDIR(src.lstat().st_mode):
                dst.mkdir(parents=True, exist_ok=True)
print('Type conflicts parked:', count)
