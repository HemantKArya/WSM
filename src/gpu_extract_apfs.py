"""Extract the existing Apple EFI APFS driver using documented jumpstart extents."""
from pathlib import Path
import hashlib
import json
import plistlib
import struct
import shutil
import subprocess

source = Path('work/gpu-followup/base-for-apfs.img')
with source.open('rb') as f:
    f.seek(1024 + 32)
    base = struct.unpack('<Q', f.read(8))[0] * 512
    f.seek(base)
    header = f.read(4096)
    assert header[32:36] == b'NXSB'
    blocksize = struct.unpack_from('<I', header, 36)[0]
    jump = struct.unpack_from('<Q', header, 1272)[0]
    assert jump and blocksize == 4096
    f.seek(base + jump * blocksize)
    record = f.read(blocksize)
    assert record[32:36] == b'JSDR'
    size, count = struct.unpack_from('<II', record, 40)
    assert 0 < size < 16*1024*1024 and 0 < count < 100
    data = b''
    for i in range(count):
        start, length = struct.unpack_from('<QQ', record, 176 + i*16)
        f.seek(base + start*blocksize)
        data += f.read(length*blocksize)
    data = data[:size]
assert data[:2] == b'MZ' and len(data) == size
root = Path('work/gpu-followup/hybrid-apfs-esp-root')
assert not root.exists()
shutil.copytree('work/gpu-followup/esp-g4-extracted', root)
(root/'EFI/OC/Drivers/ApfsFromBase.efi').write_bytes(data)
p = root/'EFI/OC/config.plist'
c = plistlib.loads(p.read_bytes())
c['UEFI']['Drivers'].append(dict(Arguments='', Comment='Apple driver extracted from original BaseSystem jumpstart',
    Enabled=True, LoadEarly=False, Path='ApfsFromBase.efi'))
c['UEFI']['APFS']['EnableJumpstart'] = False
p.write_bytes(plistlib.dumps(c, sort_keys=False))
subprocess.run([str(Path('tools/qemu/qemu-img.exe').resolve()), 'convert', '-f', 'vvfat', '-O', 'raw',
    'fat:'+root.as_posix(), 'work/gpu-followup/hybrid-apfs-esp.img'], check=True)
meta = dict(source=str(source), bytes=size, sha256=hashlib.sha256(data).hexdigest())
Path('logs/G1-followup/apfs-driver.json').write_text(json.dumps(meta, indent=2))
print(meta)
