"""Verify a downloaded recovery image using the reference downloader's signed chunklist."""
import hashlib
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location('macrecovery', 'refs/OSX-KVM/fetch-macOS-v2.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
base = Path('tools/gpu-followup/ventura')
count = 0
with (base / 'BaseSystem.dmg').open('rb') as f:
    for count, (size, expected) in enumerate(module.verify_chunklist(str(base / 'BaseSystem.chunklist')), 1):
        data = f.read(size)
        if len(data) != size or hashlib.sha256(data).digest() != expected:
            raise SystemExit(f'FAIL: recovery chunk {count}')
    if f.read(1):
        raise SystemExit('FAIL: extra image data')
print(f'PASS: {count} chunks match; chunklist RSA signature verified')
print('SHA256:', hashlib.sha256((base / 'BaseSystem.dmg').read_bytes()).hexdigest())
