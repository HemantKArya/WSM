"""Isolated G1 follow-up VM launcher; never writes a round-1/2 disk.

start NAME [--software] [--import-mode off|surface] [--ram 6144]
probe NAME [--seconds 120] ; stop NAME
"""
import argparse
import base64
import datetime
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
os.chdir(ROOT)
BUILD = ROOT / 'tools/gpu-followup/reims-vgpu'
QEMU = BUILD / 'vendor/qemu/build/qemu-system-x86_64.exe'
WORK = ROOT / 'work/gpu-followup'
LOGS = ROOT / 'logs/G1-followup'

def ssh_argv(command):
    # Preserve shell quotes when a native Windows parent invokes MSYS2 ssh.
    payload = base64.b64encode(command.encode()).decode()
    command = 'echo ' + payload + ' | base64 -D | /bin/sh'
    return [str(ROOT / 'tools/msys64/usr/bin/ssh.exe'), '-i', 'work/ssh/id_wsm', '-p', '2422',
        '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null',
        '-o', 'LogLevel=ERROR', '-o', 'ConnectTimeout=3', '-o', 'BatchMode=yes',
        os.environ.get('WSM_GUEST_USER', 'dev') + '@127.0.0.1', command]

def ssh(command):
    return subprocess.run(ssh_argv(command), capture_output=True, text=True, timeout=15)

def qmp(command, arguments=None):
    with socket.create_connection(('127.0.0.1', 4644), timeout=8) as s:
        s.settimeout(8)
        f = s.makefile('rwb')
        f.readline()
        for req in [{'execute': 'qmp_capabilities'},
                    {'execute': command, 'arguments': arguments or {}}]:
            f.write(json.dumps(req).encode() + b'\n'); f.flush()
            while True:
                ans = json.loads(f.readline())
                if 'return' in ans or 'error' in ans:
                    break
        return ans

def start(a):
    # A listener means an old VM might otherwise answer this run's probes.
    for port in (2422, 4644):
        with socket.socket() as s:
            if s.connect_ex(('127.0.0.1', port)) == 0:
                raise SystemExit(f'Port {port} in use; stop the owning experiment first')
    log = LOGS / a.name
    log.mkdir(parents=True, exist_ok=False)
    disk = WORK / (a.name + '.qcow2')
    if disk.exists():
        raise SystemExit(f'Refusing to reuse writable disk {disk}')
    subprocess.run([str(ROOT / 'tools/qemu/qemu-img.exe'), 'create', '-q', '-f', 'qcow2',
        '-F', 'qcow2', '-b', str(ROOT / a.base), str(disk)], check=True)
    nvram = WORK / (a.name + '-vars.fd')
    shutil.copyfile('work/reims/vm/firmware/OVMF_VARS.fd', nvram)
    if a.stack == 'aljus':
        original = json.loads(Path('work/reims/vm/logs/20261024-064241-090/command.json').read_text())
        args = original['arguments']
        a.qemu = ROOT / 'work/reims/build/qemu/qemu-system-x86_64.exe'
    else:
        args = Path('work/r2/mz/logs/20261024-071542/args.txt').read_text().splitlines()
    for i, value in enumerate(args):
        args[i] = value.replace('D:/MyProjects/wsm/tools/build/reims-vgpu', BUILD.as_posix())
        if value.startswith('id=disk,'):
            args[i] = 'id=disk,if=none,format=qcow2,file=' + disk.as_posix()
        if value.startswith('if=pflash,format=raw,file='):
            args[i] = 'if=pflash,format=raw,file=' + nvram.as_posix()
        if value.startswith('file:'):
            args[i] = 'file:' + (log / 'serial.log').as_posix()
        args[i] = args[i].replace(':2322-', ':2422-').replace(':4544,', ':4644,')
        if a.oc and args[i].startswith('id=opencore,'):
            args[i] = 'id=opencore,if=none,format=raw,snapshot=on,file=' + a.oc
        if a.nic and args[i].startswith('virtio-net-pci,netdev=net0,'):
            args[i] = args[i].replace('virtio-net-pci,', a.nic + ',', 1)
        if i and args[i-1] == '-m':
            args[i] = str(a.ram) + 'M'
    if a.software:
        # Reproduce the G3 boot rail including its known-good NVRAM/ESP.
        shutil.copyfile('work/r2/g3-vars.fd', nvram)
        env_boot = os.environ.copy()
        env_boot.update(SSH_PORT='2422', VARS=nvram.as_posix(),
            ESP_DRIVE='format=raw,snapshot=on,file=' + (a.oc or 'work/r2/esp-g4.img'),
            DISK=disk.as_posix(), DISK_FMT='qcow2', DISK_SNAP='off',
            EXTRA_DISK='', GFX='VGA', QMP='4644', RAM=str(a.ram), SMP='4')
        boot = subprocess.run([str(ROOT / 'tools/msys64/usr/bin/bash.exe'), 'src/boot.sh'],
            env=env_boot, capture_output=True, text=True, check=True)
        args = boot.stdout.splitlines()[1:]
        args[args.index('-serial') + 1] = 'file:' + (log / 'serial.log').as_posix()
    elif a.vga:
        for i in range(len(args)-1):
            if args[i] == '-device' and args[i+1].startswith('reims-vgpu-pci,'):
                args[i+1] = 'VGA'
    env = os.environ.copy()
    env['PATH'] = str(ROOT / 'tools/msys64/ucrt64/bin') + os.pathsep + env['PATH']
    if a.stack == 'aljus':
        env['PATH'] = str(ROOT / 'work/reims/build/qemu') + os.pathsep + env['PATH']
    for k in list(env):
        if k.startswith(('REIMS_', 'QEMU_WHPX_', 'WSM_WHPX_')):
            del env[k]
    env.update(REIMS_VGPU_WINDOW='off' if a.software else 'on',
        REIMS_VGPU_LOG='info', REIMS_VGPU_STATS='1',
        REIMS_VGPU_LAB_LOG_DIR=log.as_posix(), TMP=log.as_posix(), TEMP=log.as_posix(),
        TMPDIR=log.as_posix())
    if a.lazy_off:
        env['REIMS_VGPU_LAZY_WRITEBACK'] = 'off'
    if a.stack == 'aljus':
        env['REIMS_DEBUG'] = '1'
        env['REIMS_VGPU_WINDOW'] = '1'
    else:
        if a.import_mode == 'off':
            env['REIMS_VGPU_GUEST_IMPORT'] = 'off'
        else:
            env['REIMS_VGPU_GUEST_IMPORT_SURFACE_WRITES'] = 'off'
    for index, (attachment, readonly) in enumerate([(p, True) for p in a.attach_readonly] + [(p, False) for p in a.attach]):
        assert index < 2, 'Only SATA ports 0 and 1 are reserved for attachments'
        attachment_path = (ROOT / attachment).resolve()
        assert attachment_path.is_relative_to(ROOT), 'Attachment must stay inside workspace'
        if readonly:
            # IDE hard disks require a writable node. A disposable overlay keeps
            # the original backing image read-only while satisfying that device.
            source_overlay = WORK / f'{a.name}-source-{index}.qcow2'
            if source_overlay.exists():
                raise SystemExit('Refusing to reuse an attachment overlay')
            subprocess.run([str(ROOT / 'tools/qemu/qemu-img.exe'), 'create', '-q', '-f', 'qcow2',
                '-F', 'qcow2', '-b', str(attachment_path), str(source_overlay)], check=True)
            attachment_path = source_overlay
        args += ['-drive', f'id=wsm-extra-{index},if=none,format=qcow2,file={attachment_path.as_posix()}',
                 '-device', f'ide-hd,bus=sata.{index + (2 if a.software else 0)},drive=wsm-extra-{index}']
    with open(log / 'stdout.log', 'wb') as out, open(log / 'stderr.log', 'wb') as err:
        p = subprocess.Popen([str(a.qemu or QEMU), *args], env=env, stdout=out, stderr=err,
            creationflags=subprocess.CREATE_NO_WINDOW)
    meta = {'pid': p.pid, 'started': time.time(), 'qemu': str(a.qemu or QEMU), 'args': args,
            'env': {k: v for k, v in env.items() if k.startswith('REIMS_')}}
    (log / 'run.json').write_text(json.dumps(meta, indent=2))
    print(json.dumps({'pid': p.pid, 'logs': str(log)}))

def probe(a):
    log = LOGS / a.name
    meta = json.loads((log / 'run.json').read_text())
    until = time.monotonic() + a.seconds
    while time.monotonic() < until:
        try:
            r = ssh('echo WSM-G1-SSH; sw_vers; sysctl kern.boottime; uptime')
            (log / 'ssh-last-error.txt').write_text(r.stderr)
            if r.returncode == 0:
                elapsed = time.time() - meta['started']
                text = f'SSH observed {elapsed:.2f} s after launch (includes probe delay)\n' + r.stdout
                (log / 'ssh.txt').write_text(text)
                print(text)
                return
        except subprocess.TimeoutExpired:
            pass
        time.sleep(3)
    print('NO SSH within probe window')

def stop(a):
    # Only this harness's QMP endpoint; never kill by executable name.
    print(qmp('quit'))

def command(a):
    path = LOGS / a.name / f'command-{time.time_ns()}.txt'
    payload = Path(a.command_file).read_text() if a.command_file else a.command
    with path.open('wb') as out:
        proc = subprocess.Popen(ssh_argv(payload), stdout=out, stderr=subprocess.STDOUT)
        try:
            rc = proc.wait(timeout=a.seconds)
        except subprocess.TimeoutExpired:
            proc.kill(); proc.wait()
            rc = 124
    print(path.read_text(errors='replace'), end='')
    shutil.copyfile(path, LOGS / a.name / 'command-last.txt')
    if rc == 124:
        print(f'COMMAND TIMEOUT after {a.seconds}s; partial output retained at {path}')
    raise SystemExit(rc)

def metal(a):
    scp = ROOT / 'tools/msys64/usr/bin/scp.exe'
    if not a.staged:
        subprocess.run([str(scp), '-q', '-i', 'work/ssh/id_wsm', '-P', '2422',
            '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null',
            'work/gpu-followup/metaltest', 'dev@127.0.0.1:/Users/dev/wsm-metaltest'], check=True, timeout=20)
    cmd = 'chmod +x /Users/dev/wsm-metaltest; METAL_ERROR_MODE=5 script -q /dev/null /Users/dev/wsm-metaltest'
    argv = [str(ROOT / 'tools/msys64/usr/bin/ssh.exe'), '-i', 'work/ssh/id_wsm',
        '-p', '2422', '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null',
        '-o', 'LogLevel=ERROR', '-o', 'BatchMode=yes', 'dev@127.0.0.1', cmd]
    with (LOGS / a.name / 'metal.txt').open('wb') as out:
        try:
            r = subprocess.run(argv, stdout=out, stderr=subprocess.STDOUT, timeout=90)
            print('Metal probe exit:', r.returncode)
        except subprocess.TimeoutExpired:
            print('Metal probe TIMEOUT (90 seconds); retained partial output')
            ssh('pkill -x wsm-metaltest')
    print((LOGS / a.name / 'metal.txt').read_text())

def upload(a):
    subprocess.run([str(ROOT / 'tools/msys64/usr/bin/scp.exe'), '-q',
        '-i', 'work/ssh/id_wsm', '-P', '2422', '-o', 'StrictHostKeyChecking=no',
        '-o', 'UserKnownHostsFile=/dev/null', a.source, 'dev@127.0.0.1:' + a.destination],
        check=True, timeout=30)

def capture(a):
    log = LOGS / a.name
    ppm = log / (a.label + '.ppm')
    result = qmp('screendump', {'filename': ppm.as_posix()})
    if 'error' in result:
        raise SystemExit(result)
    subprocess.run([str(ROOT/'.venv/Scripts/python.exe'), '-I', 'src/ppm_png.py',
        str(ppm), str(log/(a.label+'.png'))], check=True)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['start', 'probe', 'stop', 'command', 'metal', 'upload', 'capture'])
    parser.add_argument('name')
    parser.add_argument('--software', action='store_true')
    parser.add_argument('--vga', action='store_true')
    parser.add_argument('--lazy-off', action='store_true')
    parser.add_argument('--staged', action='store_true')
    parser.add_argument('--stack', choices=['control', 'aljus'], default='control')
    parser.add_argument('--user', default='dev')
    parser.add_argument('--base', default='work/reims/vm/disks/current.qcow2')
    parser.add_argument('--qemu', type=Path)
    parser.add_argument('--oc')
    parser.add_argument('--nic', help='replace the GPU rail virtio-net-pci model')
    parser.add_argument('--command')
    parser.add_argument('--command-file')
    parser.add_argument('--source')
    parser.add_argument('--destination')
    parser.add_argument('--label', default='screen')
    parser.add_argument('--attach', action='append', default=[])
    parser.add_argument('--attach-readonly', action='append', default=[])
    parser.add_argument('--import-mode', choices=['off', 'surface'], default='off')
    parser.add_argument('--ram', type=int, choices=range(1024, 8193), default=6144)
    parser.add_argument('--seconds', type=int, default=120)
    a = parser.parse_args()
    os.environ['WSM_GUEST_USER'] = a.user
    {'start': start, 'probe': probe, 'stop': stop, 'command': command, 'metal': metal, 'upload': upload, 'capture': capture}[a.action](a)
