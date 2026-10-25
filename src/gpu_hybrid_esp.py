"""Make an isolated, unrestricted-scan ESP for the copied APFS volume."""
from pathlib import Path
import plistlib
import shutil
import subprocess

root = Path('work/gpu-followup/hybrid-esp-root')
assert not root.exists()
# Extract esp-g4.img with project-local 7-Zip first; work/esp is an older tree.
shutil.copytree('work/gpu-followup/esp-g4-extracted', root)
path = root / 'EFI/OC/config.plist'
c = plistlib.loads(path.read_bytes())
c['Misc']['Security']['ScanPolicy'] = 17760515
c['Misc']['Debug']['Target'] = 9
for entry in c['ACPI']['Add']:
    if not (path.parent / 'ACPI' / entry['Path']).exists():
        entry['Enabled'] = False
path.write_bytes(plistlib.dumps(c, sort_keys=False))
subprocess.run(['tools/qemu/qemu-img.exe', 'convert', '-f', 'vvfat', '-O', 'raw',
                'fat:' + root.as_posix(), 'work/gpu-followup/hybrid-esp.img'], check=True)
