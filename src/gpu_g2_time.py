"""Poll SSH on the helper's port after a `start`; report seconds since QEMU launch (run.json)."""
import json, subprocess, sys, time
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
started = json.loads((ROOT / 'logs/G1-followup' / sys.argv[1] / 'run.json').read_text())['started']
ssh = [str(ROOT / 'tools/msys64/usr/bin/ssh.exe'), '-i', str(ROOT / 'work/ssh/id_wsm'), '-p', '2422',
       '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null', '-o', 'LogLevel=ERROR',
       '-o', 'ConnectTimeout=1', '-o', 'BatchMode=yes', 'root@127.0.0.1', 'sysctl -n kern.boottime; uptime']
while time.time() - started < 180:
    r = subprocess.run(ssh, capture_output=True, text=True, timeout=10)
    if r.returncode == 0:
        print(f'first SSH command OK {time.time() - started:.2f} s after QEMU launch'); print(r.stdout.strip()); break
    time.sleep(0.5)
else:
    print('NO SSH')
