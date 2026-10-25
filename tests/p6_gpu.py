"""P6 item 5 (GPU): Metal readback, first-session stall with/without pre-warm, warm frame rate, window drag.

usage: p6_gpu.py GPU_INSTANCE --anim ANIM_BINARY [--metal METALTEST] [--drag-tool QMP_DRAG_EXE]
Drives wsm.exe; frame rates come from reims' per-second `stats: port 0: guest N/s` lines in the
instance's qemu.err.log (round 2 method). The drag uses QMP tablet input (harness only).
Writes logs/r3/p6/gpu-<time>.json; exits non-zero when a measured item fails.
"""
import argparse, json, os, re, statistics, subprocess, sys, threading, time
from pathlib import Path

WSM = str(Path(os.environ["WSM_BIN"]).resolve())
HOME = Path(os.environ["WSM_HOME"])
STAT = re.compile(rb"stats: port 0: guest ([\d.]+)/s")


def run(name, *args, timeout=600, check=True):
    p = subprocess.run([WSM, "--name", name, *args], capture_output=True, timeout=timeout)
    out = p.stdout.decode(errors="replace") + p.stderr.decode(errors="replace")
    if check and p.returncode:
        raise RuntimeError(f"{args}: {p.returncode}: {out[-1500:]}")
    return out.strip()


class Stats:
    """Per-second reims present rates appended to qemu.err.log after `mark()`."""
    def __init__(self, name):
        self.log = HOME / "instances" / name / "logs" / "qemu.err.log"
        self.offset = 0

    def mark(self):
        self.offset = self.log.stat().st_size

    def rates(self):
        with self.log.open("rb") as f:
            f.seek(self.offset)
            return [float(x) for x in STAT.findall(f.read())]


def summary(xs):
    return {"median": statistics.median(xs), "min": min(xs), "max": max(xs), "n": len(xs)} if xs else None


def as_user(cmd):
    """Run in the logged-in GUI user's session (window server connection)."""
    return f"sudo launchctl asuser $(id -u) sudo -u $(id -un) {cmd}"


def first_session(name, stats, prewarm, anim):
    """Cold boot, then time a first GUI session: seconds until reims presents >= 30 frames/s."""
    run(name, "start")
    run(name, "exec", "--", "sudo rm -f /var/db/wsm/no-prewarm" if prewarm else "sudo touch /var/db/wsm/no-prewarm")
    run(name, "stop", timeout=300)                       # GPU instances shut down (session ends too)
    t0 = time.perf_counter()
    run(name, "start", timeout=300)
    ssh_s = time.perf_counter() - t0
    prewarm_s = None
    if prewarm:
        # com.wsm.init logs the desktop session in while the host window is hidden; a user's first
        # `wsm gui start` normally comes later, modelled as 60 s after that session exists.
        t = time.perf_counter()
        while "session: up" not in run(name, "gui", "status") and time.perf_counter() - t < 180:
            time.sleep(2)
        prewarm_s = round(time.perf_counter() - t0, 1)
        time.sleep(60)
    ws_before = run(name, "exec", "--", "pgrep -x WindowServer >/dev/null && echo up || echo down")
    t1 = time.perf_counter()
    gui = run(name, "gui", "start", timeout=180)
    gui_s = time.perf_counter() - t1
    stats.mark()
    out = run(name, "exec", "--", as_user(f"{anim} 30"), timeout=180)
    rates = stats.rates()
    flowing = next((i for i, r in enumerate(rates) if r >= 30), None)
    return {"prewarm": prewarm, "cold_boot_to_ssh_s": round(ssh_s, 1), "session_prewarmed_at_s": prewarm_s, "windowserver_before_gui": ws_before,
            "gui_start_s": round(gui_s, 1), "gui": gui, "app": out.splitlines()[-1:],
            "stall_s": flowing if flowing is not None else f">{len(rates)}", "rates": rates}


def warm_fps(name, stats, anim):
    stats.mark()
    out = run(name, "exec", "--", as_user(f"{anim} 20"), timeout=120)
    rates = stats.rates()[2:]                           # drop window creation seconds
    return {"app": out.splitlines()[-1:], **(summary(rates) or {}), "rates": rates}


def drag(name, stats, anim, tool):
    p = subprocess.Popen([WSM, "--name", name, "exec", "--", as_user(f"{anim} 45 static")],
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    pos = None
    for line in p.stdout:
        m = re.match(rb"titlebar (\d+) (\d+)", line)
        if m:
            pos = (m.group(1).decode(), m.group(2).decode()); break
    if pos is None:
        p.kill(); return {"status": "ERROR", "error": "test window did not report its position"}
    threading.Thread(target=p.stdout.read, daemon=True).start()
    time.sleep(3)
    stats.mark()
    d = subprocess.run([str(Path(tool).resolve()), str(HOME / "instances" / name), *pos, "6"], capture_output=True, text=True, timeout=120)
    rates = stats.rates()
    p.wait(timeout=120)
    return {"titlebar": pos, "tool": d.stdout.strip() or d.stderr.strip()[-300:], **(summary(rates) or {}), "rates": rates}


def metal(name, binary):
    run(name, "cp", binary, ":/tmp/wsm-metaltest")
    out = run(name, "exec", "--", "chmod +x /tmp/wsm-metaltest; METAL_ERROR_MODE=5 /tmp/wsm-metaltest", check=False)
    return {"clear": "clear=PASS" in out, "shader": "shader=PASS" in out, "out": out[-600:]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("instance"); ap.add_argument("--anim", required=True)
    ap.add_argument("--metal", default="work/gpu-followup/metaltest")
    ap.add_argument("--drag-tool", default="target/round3-next/release/examples/qmp_drag.exe")
    a = ap.parse_args()
    name, stats = a.instance, Stats(a.instance)
    out = Path("logs/r3/p6") / f"gpu-{time.strftime('%Y%m%d-%H%M%S')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    r = {"instance": name, "time": time.strftime("%Y-%m-%d %H:%M:%S")}
    anim = "$HOME/wsm-anim"                              # survives the cold boots below
    # Frame-rate and drag numbers are only meaningful inside a visible desktop session, so they run
    # in the pre-warmed boot, after its first-session measurement.
    steps = [
        ("first_session_no_prewarm", lambda: first_session(name, stats, False, anim)),
        ("metal", lambda: metal(name, a.metal)),
        ("first_session_prewarm", lambda: first_session(name, stats, True, anim)),
        ("warm_fps", lambda: warm_fps(name, stats, anim)),
        ("drag", lambda: drag(name, stats, anim, a.drag_tool)),
    ]
    run(name, "start", timeout=300)
    run(name, "cp", a.anim, ":wsm-anim"); run(name, "exec", "--", "chmod 755 ~/wsm-anim")
    for k, step in steps:
        print(f"[gpu] {k} ...", file=sys.stderr, flush=True)
        try:
            r[k] = step()
        except Exception as e:
            r[k] = {"status": "ERROR", "error": repr(e)}
        out.write_text(json.dumps(r, indent=2))
    v = {
        "metal": "PASS" if r["metal"].get("clear") and r["metal"].get("shader") else "FAIL",
        "warm_fps_median_ge_50": "PASS" if (r["warm_fps"].get("median") or 0) >= 50 else "FAIL",
        "drag_measured": "PASS" if r["drag"].get("n") else "FAIL",
        "prewarm_stall_le_2s": "PASS" if isinstance(r["first_session_prewarm"].get("stall_s"), int)
                                and r["first_session_prewarm"]["stall_s"] <= 2 else "FAIL",
    }
    r["verdict"] = v
    out.write_text(json.dumps(r, indent=2))
    print(json.dumps({k: ({x: y for x, y in val.items() if x != "rates"} if isinstance(val, dict) else val) for k, val in r.items()}, indent=2))
    sys.exit(1 if "FAIL" in v.values() else 0)


if __name__ == "__main__":
    main()
