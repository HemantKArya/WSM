"""Boot the VM (argv from src/boot.sh via env), time serial markers and first successful SSH command.

usage: python src/boot_to_ssh.py LOG TIMEOUT [--keep] [--cmd "remote command"] -- qemu argv...
Prints: t(kernel), t(sshd-ready marker), t(first ssh ok). With --keep the VM keeps running (pid printed).
"""
import os, re, subprocess, sys, time, threading

KEY = "work/ssh/id_wsm"
SSH = ["ssh", "-i", KEY, "-p", os.environ.get("TSSH_PORT", "2222"), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
       "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=2", "-o", "BatchMode=yes", os.environ.get("TSSH_USER", "root") + "@127.0.0.1"]
MARKS = [("handoff", r"HANDOFF TO XNU"), ("kernel", r"Darwin Kernel Version"), ("root", r"BSD root|Got boot device"),
         ("init", r"WSM-INIT start"), ("ip", r"WSM-INIT ip=\S+"), ("sshd", r"WSM-SSHD-READY")]

def main():
    a = sys.argv[1:]
    sep = a.index("--")
    log, timeout, opts, qemu = a[0], float(a[1]), a[2:sep], a[sep + 1:]
    keep = "--keep" in opts
    cmd = opts[opts.index("--cmd") + 1] if "--cmd" in opts else "sw_vers -productVersion"
    t0 = time.time()
    p = subprocess.Popen(qemu, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    seen = {}
    out = open(log, "w", encoding="utf-8", errors="replace")

    def reader():
        buf = ""
        while True:
            b = p.stdout.read1(4096)
            if not b:
                return
            s = b.decode("utf-8", "replace"); out.write(s); out.flush(); buf = (buf + s)[-20000:]
            for name, pat in MARKS:
                if name not in seen:
                    m = re.search(pat, buf)
                    if m:
                        seen[name] = time.time() - t0
                        print(f"{seen[name]:7.2f}s  {name}: {m.group(0)[:60]}", flush=True)
    threading.Thread(target=reader, daemon=True).start()
    ok = None
    while time.time() - t0 < timeout and p.poll() is None:
        r = None
        if "sshd" in seen or time.time() - t0 > 8:
            try:  # PROBE_TIMEOUT: a probe stuck behind slirp's SYN backoff would otherwise hide when sshd is really reachable
                r = subprocess.run(SSH + [cmd], capture_output=True, text=True, timeout=float(os.environ.get("PROBE_TIMEOUT", "15")))
            except subprocess.TimeoutExpired:
                pass
        if r is not None and r.returncode == 0:
            ok = time.time() - t0
            print(f"{ok:7.2f}s  ssh ok: {r.stdout.strip()[:200]}", flush=True)
            break
        time.sleep(0.5)
    if ok is None:
        print(f"NO SSH within {timeout}s (qemu rc={p.poll()})")
    if keep and ok is not None:
        print(f"VM kept running, pid={p.pid}")
        # keep draining serial so the guest never blocks on a full pipe
        threading.Event().wait() if "--block" in opts else None
        while p.poll() is None:
            time.sleep(1)
    else:
        p.kill()

if __name__ == "__main__":
    main()
