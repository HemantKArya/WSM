"""Launch QEMU (serial on stdio), time until regex(es) appear in output. Logs everything.

usage: python src/serial_timer.py LOG TIMEOUT PATTERN[,PATTERN2...] -- qemu args...
Prints elapsed seconds for each pattern in order; exits after last pattern (kills QEMU unless --keep).
"""
import re, subprocess, sys, time, threading

def main():
    keep = "--keep" in sys.argv
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    sep = argv.index("--")
    log, timeout, pats = argv[0], float(argv[1]), argv[2].split(",,")
    cmd = argv[sep + 1:]
    t0 = time.time()
    p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    buf = ""
    idx = 0
    out = open(log, "w", encoding="utf-8", errors="replace")
    stop = [False]
    def killer():
        while not stop[0] and time.time() - t0 < timeout:
            time.sleep(0.5)
        if not stop[0]:
            print(f"TIMEOUT after {timeout}s waiting for {pats[idx] if idx < len(pats) else '?'}", flush=True)
            p.kill()
    threading.Thread(target=killer, daemon=True).start()
    while True:
        b = p.stdout.read1(4096)
        if not b:
            break
        s = b.decode("utf-8", "replace")
        out.write(s); out.flush()
        buf += s
        while idx < len(pats):
            pat, _, send = pats[idx].partition("=>")
            m = re.search(pat, buf)
            if not m:
                break
            print(f"{time.time() - t0:7.2f}s  matched: {pat}", flush=True)
            buf = buf[m.end():]
            if send:
                for ch in send.encode().decode("unicode_escape").encode():
                    p.stdin.write(bytes([ch])); p.stdin.flush(); time.sleep(0.01)
            idx += 1
        if idx == len(pats):
            stop[0] = True
            if not keep:
                p.kill()
            break
    stop[0] = True
    print(f"exit; total {time.time() - t0:.2f}s, rc={p.poll()}")

if __name__ == "__main__":
    main()
