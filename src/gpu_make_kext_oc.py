"""Add only upstream's Tahoe driver shim to an isolated copy of our AMD OpenCore."""
from pathlib import Path
import hashlib
import json
import plistlib
import shutil
import subprocess
import zipfile

root = Path('work/gpu-followup/oc-kext')
if root.exists():
    raise SystemExit('Refusing to overwrite an existing OpenCore experiment')
shutil.copytree('work/r2/mz/ocroot', root)
with zipfile.ZipFile('tools/gpu-followup/Lilu-1.7.2-DEBUG.zip') as z:
    z.extractall('tools/gpu-followup/lilu')
lilu = next(Path('tools/gpu-followup/lilu').rglob('Lilu.kext'))
kexts = root / 'EFI/OC/Kexts'
kexts.mkdir(exist_ok=True)
shutil.copytree(lilu, kexts / 'Lilu.kext')
shutil.copytree('refs/reims-util/kext/ReimsVgpu.kext', kexts / 'ReimsVgpu.kext')
path = root / 'EFI/OC/config.plist'
c = plistlib.loads(path.read_bytes())
assert c['Kernel']['Quirks']['ProvideCurrentCpuInfo']
for name in ('Lilu', 'ReimsVgpu'):
    c['Kernel']['Add'].append(dict(Arch='x86_64', BundlePath=name + '.kext',
        Comment='WSM: upstream Tahoe driver shim', Enabled=True,
        ExecutablePath='Contents/MacOS/' + name, MaxKernel='25.99.99',
        MinKernel='25.0.0', PlistPath='Contents/Info.plist'))
path.write_bytes(plistlib.dumps(c, sort_keys=False))
out = 'work/gpu-followup/oc-kext.img'
subprocess.run(['tools/qemu/qemu-img.exe', 'convert', '-f', 'vvfat', '-O', 'raw',
    'fat:' + str(root).replace('\\', '/'), out], check=True)
hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest()
          for p in (Path(out), kexts/'Lilu.kext/Contents/MacOS/Lilu',
                    kexts/'ReimsVgpu.kext/Contents/MacOS/ReimsVgpu')}
Path('logs/G1-followup/kext-oc-hashes.json').write_text(json.dumps(hashes, indent=2))
print(out)
