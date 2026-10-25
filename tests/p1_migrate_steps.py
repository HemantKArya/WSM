"""P1 debug: step through stop/migrate on a running instance with timing, one QMP command at a time.
usage: p1_migrate_steps.py QMP_PORT OUT_FILE [EXTRA_PARAMS_JSON]"""
import json, socket, sys, threading, time
port, out = int(sys.argv[1]), sys.argv[2]
def conn():
    s = socket.create_connection(("127.0.0.1", port), timeout=300); f = s.makefile("rwb")
    f.readline(); f.write(b'{"execute":"qmp_capabilities"}\n'); f.flush(); f.readline(); return f
def ex(f, cmd, args=None):
    t = time.time(); f.write(json.dumps({"execute": cmd, "arguments": args or {}}).encode() + b"\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            print(f"{cmd}: {time.time()-t:.2f}s {json.dumps(r)[:300]}", flush=True); return r
        print("  event:", json.dumps(r)[:200], flush=True)
ls = socket.socket(); ls.bind(("127.0.0.1", 0)); ls.listen(1); mport = ls.getsockname()[1]
n = [0]
eof = [None]
def sink():
    c, _ = ls.accept()
    with open(out, "wb") as fo:
        while True:
            b = c.recv(4 << 20)
            if not b: break
            fo.write(b); n[0] += len(b)
    eof[0] = time.time()
threading.Thread(target=sink, daemon=True).start()
f = conn()
ex(f, "query-status")
ex(f, "stop")
ex(f, "migrate-set-parameters", {"downtime-limit": 600000, **(json.loads(sys.argv[3]) if len(sys.argv) > 3 else {})})
ex(f, "query-migrate-parameters")
ex(f, "migrate", {"uri": f"tcp:127.0.0.1:{mport}"})
t0 = time.time()
while time.time() - t0 < 180:
    r = ex(f, "query-migrate")
    st = r.get("return", {}).get("status")
    print(f"  t={time.time()-t0:.1f}s bytes_in_file={n[0]}", flush=True)
    if st in ("completed", "failed", "cancelled"): break
    time.sleep(2)
tc = time.time()
while eof[0] is None and time.time() - tc < 60: time.sleep(0.1)
print("sink EOF", f"{eof[0]-tc:.2f}s after completed" if eof[0] else "NOT received within 60s", f"bytes={n[0]}", flush=True)
ex(f, "query-status")
