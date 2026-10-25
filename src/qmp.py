"""Minimal QMP client. usage: qmp.py PORT CMD [JSON_ARGS]   e.g. qmp.py 4444 screendump '{"filename":"x.ppm"}'
CMD 'hmp' runs a human-monitor command: qmp.py 4444 hmp "info status"
Prints the response and the round-trip time."""
import json, socket, sys, time

def qmp(port, cmd, args=None):
    s = socket.create_connection(("127.0.0.1", port), timeout=600)
    f = s.makefile("rwb")
    json.loads(f.readline())
    f.write(b'{"execute":"qmp_capabilities"}\n'); f.flush(); f.readline()
    if cmd == "hmp":
        cmd, args = "human-monitor-command", {"command-line": args}
    t = time.time()
    f.write(json.dumps({"execute": cmd, **({"arguments": args} if args else {})}).encode() + b"\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            return r, time.time() - t

if __name__ == "__main__":
    a = sys.argv[3] if len(sys.argv) > 3 else None
    if a and sys.argv[2] != "hmp":
        a = json.loads(a)
    r, dt = qmp(int(sys.argv[1]), sys.argv[2], a)
    print(json.dumps(r)[:4000]); print(f"rtt {dt:.3f}s")
