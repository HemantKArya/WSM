"""Prepare the published ABI-v24 Nouradawy pair without changing refs or existing builds."""
from pathlib import Path
import json
import shutil

base = Path('tools/gpu-followup')
target = base / 'nour'
if (target / '.wsm-prepared').exists():
    raise SystemExit('Candidate already prepared')
if not target.exists():
    shutil.copytree(base / 'reims-vgpu-3bd1f790ba515443b0422965d3b30a4160b88524', target)
    try:
        shutil.copytree(base / 'qemu-reims-vgpu-768b7a951fa581d221e4cd9b310392be23b80c9e',
                       target / 'vendor/qemu', dirs_exist_ok=True)
    except shutil.Error as errors:
        if any('libvhost-user' not in e[0] or not e[0].endswith('linux') for e in errors.args[0]):
            raise
# Windows may refuse dereferencing this archive's relative symlink. Materialize
# its exact target instead (the same portable copy strategy used in round 2).
headers = target / 'vendor/qemu/subprojects/libvhost-user/standard-headers/linux'
if not headers.exists():
    shutil.copytree(target/'vendor/qemu/include/standard-headers/linux', headers)
name = 'metal2vulkan-c1fc19d539dbb299573192afb2a13a342b36f9ca'
path = target / 'Cargo.toml'
s = path.read_text()
old = 'metal2vulkan = { git = "https://github.com/MAXZVER/metal2vulkan.git", rev = "c1fc19d539dbb299573192afb2a13a342b36f9ca" }'
assert old in s
path.write_text(s.replace(old, 'metal2vulkan = { path = "../' + name + '" }'))
path = target / 'crates/reims-vgpu/Cargo.toml'
s = path.read_text()
old = 'metal2vulkan = { git = "https://github.com/steelbrain/metal2vulkan.git", rev = "9e0e99a41dc3cb8bb7e288b531f1698a79fd4b1c" }'
assert old in s
path.write_text(s.replace(old, 'metal2vulkan = { path = "../../../' + name + '" }'))
# Exact backport of the current fork's log-path override, as in the control.
path = target / 'crates/reims-vgpu-observe/src/sink.rs'
s = path.read_text()
for kind in ('fail', 'draw'):
    old = f'{kind.upper()}_PATH.get_or_init(|| "/tmp/reims-vgpu-{kind}.log".to_string())'
    new = f'{kind.upper()}_PATH.get_or_init(|| product_path("{kind}"))'
    assert old in s
    s = s.replace(old, new)
insert = '''#[cfg(not(any(test, feature = "testing")))]
fn product_path(kind: &str) -> String {
    match std::env::var("REIMS_VGPU_LAB_LOG_DIR") {
        Ok(dir) if !dir.is_empty() => format!("{dir}/reims-vgpu-{kind}.log"),
        _ => format!("/tmp/reims-vgpu-{kind}.log"),
    }
}

'''
s = s.replace('pub fn fail_log_path()', insert + 'pub fn fail_log_path()', 1)
path.write_text(s)
rom = target / 'crates/reims-vgpu-efi/out'
rom.mkdir(exist_ok=True)
control = base / 'reims-vgpu/crates/reims-vgpu-efi'
efi = target / 'crates/reims-vgpu-efi'
for p in list(efi.rglob('*.rs')) + [efi/'Cargo.toml']:
    assert p.read_bytes() == (control/p.relative_to(efi)).read_bytes(), p
shutil.copyfile(control/'out/reims-vgpu-gop.rom', rom/'reims-vgpu-gop.rom')
pins = dict(reims='3bd1f790ba515443b0422965d3b30a4160b88524',
            qemu='768b7a951fa581d221e4cd9b310392be23b80c9e',
            metal2vulkan='c1fc19d539dbb299573192afb2a13a342b36f9ca', abi=24)
(base/'nour-pins.json').write_text(json.dumps(pins, indent=2))
(target/'.wsm-prepared').write_text('ABI 24; source and translator pins in ../nour-pins.json\n')
print(target)
