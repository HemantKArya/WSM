"""Print the serial backlog of a WSM instance via wsmd (non-interactive).
usage: serial_tail.py NAME [seconds]   (needs WSM_HOME if not the default)"""
import json, os, pathlib, socket, sys, time

home = pathlib.Path(os.environ.get("WSM_HOME") or pathlib.Path(os.environ["LOCALAPPDATA"]) / "wsm")
info = json.loads((home / "wsmd.json").read_text())
s = socket.create_connection(("127.0.0.1", info["port"]), timeout=5)
s.sendall((json.dumps({"token": info["token"], "cmd": "serial", "name": sys.argv[1]}) + "\n").encode())
s.settimeout(float(sys.argv[2]) if len(sys.argv) > 2 else 2.0)
buf = b""
end = time.time() + (float(sys.argv[2]) if len(sys.argv) > 2 else 2.0)
while time.time() < end:
    try:
        b = s.recv(65536)
        if not b:
            break
        buf += b
    except socket.timeout:
        break
sys.stdout.buffer.write(buf)
