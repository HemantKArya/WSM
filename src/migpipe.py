"""Bridge QEMU TCP migration <-> a state file (QEMU on Windows can't migrate to file:).
usage: migpipe.py sink PORT FILE     (accept one connection, write the stream to FILE)
       migpipe.py source PORT FILE   (connect to QEMU's -incoming tcp listener, send FILE)"""
import socket, sys, time
mode, port, path = sys.argv[1], int(sys.argv[2]), sys.argv[3]
if mode == "sink":
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", port)); s.listen(1)
    print("listening", flush=True)
    c, _ = s.accept(); n = 0
    with open(path, "wb", buffering=8 << 20) as f:
        while True:
            b = c.recv(4 << 20)
            if not b: break
            f.write(b); n += len(b)
    print("wrote", n, flush=True)
else:
    for _ in range(200):
        try: c = socket.create_connection(("127.0.0.1", port)); break
        except OSError: time.sleep(0.05)
    with open(path, "rb") as f:
        while True:
            b = f.read(4 << 20)
            if not b: break
            c.sendall(b)
    c.shutdown(socket.SHUT_WR); c.recv(1); print("sent", flush=True)
