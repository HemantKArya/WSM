"""Cold-boot timeline: when do the boot phases appear on the serial console, and when does SSH answer?

usage: boot_timeline.py INSTANCE [--seconds 90]   (WSM_HOME set; drives wsm.exe + wsmd's serial relay)
Writes logs/r3/boot-timeline-<time>.txt: every serial line with its arrival time since `wsm start`,
then the first time each marker was seen and the first successful SSH command.
"""
import argparse, json, os, re, socket, subprocess, threading, time
from pathlib import Path

WSM = str(Path(os.environ.get("WSM_BIN", "work/r3/bin/wsm.exe")).resolve())
ENV = dict(os.environ, MSYS_NO_PATHCONV="1")
MARKERS = [
    ("kernel banner", r"Darwin Kernel Version"),
    ("root device", r"BSD root:|root device"),
    ("launchd", r"launchd\[1\]|com\.apple\.launchd"),
    ("AHCI ready", r"AHCI"),
    ("network link", r"Link up|link up|AppleVirtIO.*[Nn]et|en0"),
    ("first IOUC/userland msg", r"IOUC |AMFI:"),
]


def main():
    ap = argparse.ArgumentParser(); ap.add_argument("instance"); ap.add_argument("--seconds", type=float, default=90)
    a = ap.parse_args()
    home = Path(os.environ.get("WSM_HOME") or Path(os.environ["LOCALAPPDATA"]) / "wsm")
    subprocess.run([WSM, "stop", "--kill", "--name", a.instance], env=ENV, capture_output=True)
    t0 = time.perf_counter()
    subprocess.run([WSM, "start", "--cold", "--no-wait", "--name", a.instance], env=ENV, capture_output=True)
    up = time.perf_counter() - t0
    info = json.loads((home / "wsmd.json").read_text())
    s = socket.create_connection(("127.0.0.1", info["port"]), timeout=5)
    s.sendall((json.dumps({"token": info["token"], "cmd": "serial", "name": a.instance}) + "\n").encode())
    s.settimeout(0.5)
    lines, ssh_at = [], [None]

    def poll_ssh():
        while ssh_at[0] is None and time.perf_counter() - t0 < a.seconds:
            if subprocess.run([WSM, "exec", "--name", a.instance, "--", "true"], env=ENV, capture_output=True, timeout=60).returncode == 0:
                ssh_at[0] = time.perf_counter() - t0
            time.sleep(0.25)

    threading.Thread(target=poll_ssh, daemon=True).start()
    buf = b""
    while time.perf_counter() - t0 < a.seconds and (ssh_at[0] is None or time.perf_counter() - t0 < ssh_at[0] + 3):
        try:
            b = s.recv(65536)
            if not b:
                break
            buf += b
            *done, buf = buf.split(b"\n")
            now = time.perf_counter() - t0
            lines += [(now, l.decode(errors="replace").rstrip("\r")) for l in done]
        except socket.timeout:
            pass
    first = {}
    for name, rx in MARKERS:
        for t, l in lines:
            if re.search(rx, l):
                first[name] = (round(t, 1), l[:120]); break
    out = Path("logs/r3") / f"boot-timeline-{time.strftime('%Y%m%d-%H%M%S')}.txt"
    with out.open("w", encoding="utf-8") as f:
        f.write(f"qemu_up {up:.1f}s  ssh {ssh_at[0] and round(ssh_at[0], 1)}s  serial_lines {len(lines)}\n")
        for k, v in first.items():
            f.write(f"{k:26s} {v[0]:6.1f}s  {v[1]}\n")
        f.write("\n")
        for t, l in lines:
            f.write(f"{t:7.2f} {l}\n")
    print(out.read_text(encoding="utf-8").split("\n\n")[0])
    # Gaps: the longest silences on the console point at where boot waits.
    gaps = sorted(((lines[i + 1][0] - lines[i][0], lines[i][0], lines[i][1][:90], lines[i + 1][1][:90])
                   for i in range(len(lines) - 1)), reverse=True)[:6]
    for g, t, a1, b1 in gaps:
        print(f"gap {g:5.1f}s at {t:6.1f}s after: {a1!r}\n            before: {b1!r}")


if __name__ == "__main__":
    main()
