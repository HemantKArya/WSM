"""Real-duration acceptance. Drives wsm CLI only; never substitutes short waits for stability.

WSM_HOME and WSM_BIN choose an isolated runtime. --idle-seconds defaults to one hour.
Run GUI stability separately with --gpu; its duration defaults to thirty minutes.
"""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import time


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("instance")
    ap.add_argument("--cycles", type=int, default=10)
    ap.add_argument("--idle-seconds", type=int, default=3600)
    ap.add_argument("--gpu", action="store_true")
    ap.add_argument("--gui-seconds", type=int, default=1800)
    ap.add_argument("--anim", default="$HOME/wsm-anim", help="guest path of tests/guest/anim.swift's binary")
    a = ap.parse_args()
    wsm = str(Path(os.environ.get("WSM_BIN", "work/r3/bin/wsm.exe")).resolve())
    out = Path("logs/r3/p6") / f"stability-{a.instance}-{time.strftime('%Y%m%d-%H%M%S')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    result = {"instance": a.instance, "cycles": [], "complete": False}

    def save():
        out.write_text(json.dumps(result, indent=2))

    def run(*args, timeout=180):
        p = subprocess.run([wsm, "--name", a.instance, *args], capture_output=True, timeout=timeout)
        text = p.stdout.decode(errors="replace").strip()
        if p.returncode:
            raise RuntimeError(f"{args}: {p.returncode}: {p.stderr.decode(errors='replace')[-1000:]}")
        return text

    def guest(cmd):
        return run("exec", "--", cmd)

    try:
        run("start")
        uuid = guest("sysctl -n kern.bootsessionuuid")
        assert uuid, "missing boot session identity"
        baseline = guest("find /Library/Logs/DiagnosticReports -maxdepth 1 -iname '*panic*' -print | sort")
        guest("echo stability > /tmp/wsm-stability-marker")
        if a.gpu:
            qlog = Path(os.environ["WSM_HOME"]) / "instances" / a.instance / "logs" / "qemu.err.log"
            result["gui_start"] = run("gui", "start")
            started = time.monotonic()
            observed, frames, next_frames = [], [], 0.0
            while time.monotonic() - started < a.gui_seconds:
                time.sleep(max(0, min(30, a.gui_seconds - (time.monotonic() - started))))
                assert guest("sysctl -n kern.bootsessionuuid") == uuid, "guest rebooted"
                status = run("gui", "status")
                assert "session: up" in status and "windowserver: up" in status, status
                elapsed = time.monotonic() - started
                observed.append({"elapsed_s": round(elapsed, 1), "status": status})
                if elapsed >= next_frames:
                    # A live session is not proof of frame delivery: render for 10 s and read reims' rates.
                    offset = qlog.stat().st_size
                    guest(f"sudo launchctl asuser $(id -u) sudo -u $(id -un) {a.anim} 10")
                    with qlog.open("rb") as f:
                        f.seek(offset)
                        rates = [float(x) for x in re.findall(rb"stats: port 0: guest ([\d.]+)/s", f.read())][2:]
                    frames.append({"elapsed_s": round(elapsed, 1), "median_fps": statistics.median(rates) if rates else 0, "rates": rates})
                    assert rates and statistics.median(rates) >= 30, f"frames stalled at {elapsed:.0f} s: {rates}"
                    next_frames = elapsed + 300
                result["gui_observations"], result["gui_frames"] = observed, frames
                save()
            result["gui_duration_s"] = round(time.monotonic() - started, 1)
            lost = [l for l in qlog.read_text(errors="replace").splitlines() if re.search(r"device (was )?lost|fatal", l, re.I)]
            assert not lost, f"reims reported: {lost[:3]}"
        else:
            for i in range(a.cycles):
                before = time.monotonic()
                saved = run("suspend")
                resumed = run("start")
                assert guest("sysctl -n kern.bootsessionuuid") == uuid, "resume cold-booted"
                assert guest("cat /tmp/wsm-stability-marker") == "stability", "lost RAM marker"
                result["cycles"].append({"n": i + 1, "elapsed_s": round(time.monotonic() - before, 2),
                                          "suspend": saved, "resume": resumed})
                save()
            # Status probes do not touch the idle timer. No exec or shell during this interval.
            started = time.monotonic()
            transitions = []
            previous = None
            while time.monotonic() - started < a.idle_seconds:
                time.sleep(max(0, min(30, a.idle_seconds - (time.monotonic() - started))))
                status = json.loads(run("status"))
                if status["state"] != previous:
                    previous = status["state"]
                    transitions.append({"elapsed_s": round(time.monotonic() - started, 1), "state": previous})
                result["idle_transitions"] = transitions
                save()
            assert any(t["state"] == "suspended" for t in transitions), "idle policy never suspended VM"
            result["idle_duration_s"] = round(time.monotonic() - started, 1)
            run("start")
            assert guest("sysctl -n kern.bootsessionuuid") == uuid, "idle resume cold-booted"
        final = guest("find /Library/Logs/DiagnosticReports -maxdepth 1 -iname '*panic*' -print | sort")
        assert final == baseline, "new guest panic report"
        result["complete"] = True
        result["status"] = "PASS"
    except Exception as error:
        result["status"] = "FAIL"
        result["error"] = repr(error)
        raise
    finally:
        save()
        print(out)


if __name__ == "__main__":
    main()
