"""X5: save running VM state to a file (QMP migrate file:), quit, restart with -incoming, time to first SSH.

usage: python src/suspend_resume.py STATE_FILE LOG -- qemu argv (same as the running VM, without -incoming)
Requires the running VM to expose QMP on 127.0.0.1:4444.
"""
import json, os, socket, subprocess, sys, threading, time

SSH = ["ssh", "-i", "work/ssh/id_wsm", "-p", "2222", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
       "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=2", "-o", "BatchMode=yes", "root@127.0.0.1"]

class QMP:
    def __init__(self, port=4444, timeout=30):
        end = time.time() + timeout
        while True:
            try:
                self.s = socket.create_connection(("127.0.0.1", port), timeout=600); break
            except OSError:
                if time.time() > end: raise
                time.sleep(0.05)
        self.f = self.s.makefile("rwb"); json.loads(self.f.readline()); self.cmd("qmp_capabilities")
    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, **({"arguments": args} if args else {})}).encode() + b"\n"); self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r or "error" in r:
                return r

def main():
    a = sys.argv[1:]; sep = a.index("--")
    state, log, qemu = os.path.abspath(a[0]).replace("\\", "/"), a[1], a[sep + 1:]
    q = QMP()
    t = time.time()
    q.cmd("stop")
    r = q.cmd("migrate", uri=f"file:{state}")
    if "error" in r:
        print("migrate error:", r); return
    while True:
        st = q.cmd("query-migrate")["return"]
        if st.get("status") in ("completed", "failed", "cancelled"):
            break
        time.sleep(0.05)
    save = time.time() - t
    print(f"save: status={st.get('status')} {save:.2f}s size={os.path.getsize(state)/2**20:.0f} MiB")
    q.cmd("quit"); time.sleep(1.5)

    t0 = time.time()
    p = subprocess.Popen(qemu + ["-incoming", f"file:{state}"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    out = open(log, "wb")
    threading.Thread(target=lambda: [out.write(b) or out.flush() for b in iter(lambda: p.stdout.read1(4096), b"")], daemon=True).start()
    q2 = QMP(timeout=60)
    while q2.cmd("query-status")["return"]["status"] != "running":
        time.sleep(0.02)
    print(f"resume: vCPUs running after {time.time()-t0:.2f}s")
    while time.time() - t0 < 120:
        r = subprocess.run(SSH + ["uptime"], capture_output=True, text=True)
        if r.returncode == 0:
            print(f"resume: first ssh ok after {time.time()-t0:.2f}s  [{r.stdout.strip()}]"); break
        time.sleep(0.1)
    else:
        print("resume: no ssh in 120 s")
    print(f"VM kept running pid={p.pid}")
    while p.poll() is None:
        time.sleep(1)

if __name__ == "__main__":
    main()
