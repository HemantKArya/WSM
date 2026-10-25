"""P6 acceptance suite: drives wsm.exe only (no direct QEMU/QMP) and records medians.

usage: p6_acceptance.py INSTANCE [--gpu GPU_INSTANCE] [--quick] [--out DIR]
Needs WSM_HOME (or the installed %LOCALAPPDATA%\\wsm) and WSM_BIN (default work/r3/bin/wsm.exe).
Writes DIR/p6-<time>.json and prints the medians (per-run detail stays in the JSON). Items that can't run on this setup are
recorded as "NOT RUN" with the reason, never skipped silently.
"""
import argparse, json, os, queue, re, statistics, subprocess, sys, threading, time, tomllib
from pathlib import Path

# Absolute: CreateProcess doesn't resolve a relative path written with forward slashes.
WSM = str(Path(os.environ.get("WSM_BIN", "work/r3/bin/wsm.exe")).resolve())
ENV = dict(os.environ, MSYS_NO_PATHCONV="1")


def run(args, timeout=600):
    t = time.perf_counter()
    p = subprocess.run([WSM, *args], capture_output=True, text=True, timeout=timeout, env=ENV)
    return p.returncode, p.stdout + p.stderr, time.perf_counter() - t


def gx(name, cmd, timeout=600):
    """Guest shell snippet via `wsm exec`; returns (rc, stdout)."""
    rc, out, _ = run(["exec", "--name", name, "--", cmd], timeout)
    return rc, out.strip()


def med(xs):
    xs = [x for x in xs if x is not None]
    return {"median": round(statistics.median(xs), 2), "min": round(min(xs), 2), "max": round(max(xs), 2), "n": len(xs)} if xs else None


def secs(pattern, text):
    m = re.search(pattern, text)
    return float(m.group(1)) if m else None


def wait_ssh(name, t0, limit=180):
    while time.perf_counter() - t0 < limit:
        if run(["exec", "--name", name, "--", "true"], 30)[0] == 0:
            return time.perf_counter() - t0
        time.sleep(0.25)
    return None


def serial_shell(name, t0, limit=120):
    """Prove a command executed through `wsm shell --serial`, excluding echoed input."""
    p = subprocess.Popen([WSM, "shell", "--serial", "--name", name], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=ENV)
    chunks = queue.Queue()
    def reader():
        while data := p.stdout.read(1):
            chunks.put(data)
    threading.Thread(target=reader, daemon=True).start()
    text, sent = b"", 0
    try:
        while time.perf_counter() - t0 < limit and p.poll() is None:
            now = time.perf_counter()
            if now - sent >= 1:
                p.stdin.write(b"\recho WSM-P6-SERIAL-$((6*7))\r"); p.stdin.flush(); sent = now
            try:
                text += chunks.get(timeout=0.1)
            except queue.Empty:
                pass
            if b"WSM-P6-SERIAL-42" in text:
                return time.perf_counter() - t0
        return None
    finally:
        if p.poll() is None:
            p.stdin.write(b"\x1d"); p.stdin.flush()
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                p.kill(); p.wait()


def cold_boot(name, n):
    rows = []
    for _ in range(n):
        run(["stop", "--shutdown", "--name", name])
        t0 = time.perf_counter()
        rc, out, _ = run(["start", "--cold", "--no-wait", "--name", name])
        qemu_up = time.perf_counter() - t0 if rc == 0 else None
        # Serial and SSH are measured concurrently from the same t0: neither wait may delay the other.
        serial = {}
        th = threading.Thread(target=lambda: serial.update(s=serial_shell(name, t0) if rc == 0 else None))
        th.start()
        ssh_s = wait_ssh(name, t0)
        th.join()
        rows.append({"start_rc": rc, "qemu_up_s": qemu_up, "serial_shell_s": serial.get("s"), "ssh_s": ssh_s})
    return {"qemu_up_s": med([r["qemu_up_s"] for r in rows]), "serial_shell_s": med([r["serial_shell_s"] for r in rows]),
            "ssh_s": med([r["ssh_s"] for r in rows]), "runs": rows}


def suspend_resume(name, n):
    rows = []
    for k in range(n):
        u0 = gx(name, "sysctl -n kern.bootsessionuuid")[1]
        gx(name, f"echo p6-{k} > /tmp/wsm-p6-marker")
        rc, out, _ = run(["suspend", "--name", name])
        save = {"rc": rc, "save_s": secs(r"suspended: ([\d.]+)s", out),
                "mb": secs(r", (\d+) MB", out), "migrate_s": secs(r"migrate ([\d.]+)s", out)}
        t0 = time.perf_counter()
        rc2, out2, _ = run(["start", "--name", name])
        resume_s = secs(r"in ([\d.]+)s \(resume\)", out2)
        ssh_s = wait_ssh(name, t0)
        u1 = gx(name, "sysctl -n kern.bootsessionuuid")[1]
        mk = gx(name, "cat /tmp/wsm-p6-marker")[1]
        off = gx(name, "date -u +%s")[1]
        offset = abs(int(time.time()) - int(off)) if off.isdigit() else None
        rows.append({**save, "resume_rc": rc2, "resume_ssh_s": resume_s or ssh_s,
                     "same_session": u0 == u1 and mk == f"p6-{k}", "clock_offset_s": offset})
    return {"save_s": med([r["save_s"] for r in rows]), "state_mb": med([r["mb"] for r in rows]),
            "resume_to_ssh_s": med([r["resume_ssh_s"] for r in rows]),
            "same_session": f"{sum(r['same_session'] for r in rows)}/{n}",
            "max_clock_offset_s": max((r["clock_offset_s"] or 0) for r in rows), "runs": rows}


def warm_exec(name, n):
    run(["exec", "--name", name, "--", "true"])
    rows = [run(["exec", "--name", name, "--", "true"]) for _ in range(n)]
    result = med([row[2] * 1000 for row in rows])
    result["failed"] = sum(row[0] != 0 for row in rows)
    return result


def dev_loop(name):
    res = {}
    if gx(name, "xcode-select -p")[0] != 0:
        return {"status": "NOT RUN", "reason": "no Command Line Tools in this image (P3 --clt)"}
    steps = {
        "clang_hello": "cd /tmp && printf '#include <stdio.h>\\nint main(){puts(\"hi\");}' > h.c && clang h.c -o h && ./h",
        "swiftc_hello": "cd /tmp && echo 'print(\"hi\")' > h.swift && swiftc h.swift -o hs && ./hs",
        "git_clone": "cd /tmp && rm -rf jq && git clone -q --depth 1 https://github.com/jqlang/jq jq && test -f jq/README.md",
    }
    if gx(name, "command -v brew")[0] == 0:
        steps["brew_bottle"] = "HOMEBREW_NO_AUTO_UPDATE=1 brew install -q ca-certificates && brew info --json=v2 ca-certificates | grep -Eq '\"poured_from_bottle\"[[:space:]]*:[[:space:]]*true'"
        steps["brew_source"] = "if brew list --versions hello >/dev/null 2>&1; then HOMEBREW_NO_AUTO_UPDATE=1 brew reinstall --build-from-source hello; else HOMEBREW_NO_AUTO_UPDATE=1 brew install --build-from-source hello; fi && hello"
    for k, cmd in steps.items():
        t = time.perf_counter(); rc, out = gx(name, cmd, 1800)
        res[k] = {"ok": rc == 0, "s": round(time.perf_counter() - t, 2), **({} if rc == 0 else {"out": out[-300:]})}
    if "brew_bottle" not in steps:
        res["brew"] = {"status": "NOT RUN", "reason": "Homebrew not installed in this image"}
    return res


def ps(cmd):
    p = subprocess.run(["powershell", "-NoProfile", "-Command", cmd], capture_output=True, text=True)
    return p.stdout.strip()


def resources(name, home):
    pid = (home / "instances" / name / "qemu.pid").read_text().strip()
    cpu = ps(f"$p=Get-Process -Id {pid}; $a=$p.CPU; Start-Sleep 30; $p.Refresh(); [math]::Round(($p.CPU-$a)/30/[Environment]::ProcessorCount*100,2)")
    ws = ps(f"[math]::Round((Get-Process -Id {pid}).WorkingSet64/1MB)")
    inst = home / "instances" / name
    man = tomllib.loads((inst / "manifest.toml").read_text())
    base = Path(man["base"])
    size = lambda p: round(p.stat().st_size / 1e9, 2) if p.exists() else None
    return {"idle_host_cpu_pct_30s": float(cpu) if cpu else None, "qemu_working_set_mb": int(ws) if ws else None,
            "base_gb": size(base), "instance_disk_gb": size(inst / "disk.qcow2")}


def gpu_metal(name):
    rc, out, _ = run(["start", "--name", name], 300)
    if rc != 0:
        return {"status": "FAIL", "out": out[-300:]}
    run(["cp", "--name", name, "work/gpu-followup/metaltest", ":/Users/dev/wsm-metaltest"])
    _, out = gx(name, "chmod +x ~/wsm-metaltest; METAL_ERROR_MODE=5 script -q /dev/null ~/wsm-metaltest")
    res = {"clear": "clear=PASS" in out, "shader": "shader=PASS" in out,
           "clear_ms": secs(r"clear: .*? ([\d.]+) ms", out), "shader_ms": secs(r"shader: .*? ([\d.]+) ms", out)}
    run(["stop", "--name", name], 180)
    return res


def verdict(r):
    """Check every recorded item against the round-3 targets. Returns {item: PASS|FAIL|NOT RUN|ERROR}."""
    def m(item, key):
        v = r.get(item, {}).get(key)
        return v["median"] if isinstance(v, dict) and "median" in v else None
    v = {}
    cold = r.get("1_cold_boot", {})
    if "status" in cold:
        v["1_cold_boot"] = cold["status"]
    else:
        ok_runs = all(x["start_rc"] == 0 and x["ssh_s"] is not None for x in cold["runs"])
        v["1_cold_ssh_le_20s"] = "PASS" if ok_runs and (m("1_cold_boot", "ssh_s") or 999) <= 20 else "FAIL"
        v["1_cold_serial_le_10s"] = "PASS" if (m("1_cold_boot", "serial_shell_s") or 999) <= 10 else "FAIL"
    sr = r.get("2_suspend_resume", {})
    if "status" in sr:
        v["2_suspend_resume"] = sr["status"]
    else:
        n = len(sr["runs"])
        ok_runs = all(x["rc"] == 0 and x["resume_rc"] == 0 for x in sr["runs"])
        v["2_resume_le_5s"] = "PASS" if ok_runs and (m("2_suspend_resume", "resume_to_ssh_s") or 999) <= 5 else "FAIL"
        v["2_same_session"] = "PASS" if sr["same_session"] == f"{n}/{n}" and sr["max_clock_offset_s"] <= 3 else "FAIL"
    we = r.get("3_warm_exec_ms", {})
    v["3_warm_exec"] = we.get("status") or ("PASS" if we.get("failed") == 0 else "FAIL")
    dev = r.get("4_dev_loop", {})
    if "status" in dev:
        v["4_dev_loop"] = dev["status"]
    else:
        for k, x in dev.items():
            v[f"4_{k}"] = x.get("status") or ("PASS" if x["ok"] else "FAIL")
    g = r.get("5_gpu_metal", {})
    v["5_gpu_metal"] = g.get("status") or ("PASS" if g.get("clear") and g.get("shader") else "FAIL")
    res = r.get("7_resources", {})
    v["7_resources"] = res.get("status") or ("PASS" if res.get("qemu_working_set_mb") else "FAIL")
    return v


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("instance"); ap.add_argument("--gpu"); ap.add_argument("--quick", action="store_true")
    ap.add_argument("--out", default="logs/r3/p6")
    a = ap.parse_args()
    home = Path(os.environ.get("WSM_HOME") or Path(os.environ["LOCALAPPDATA"]) / "wsm")
    n5, n20 = (2, 5) if a.quick else (5, 20)
    r = {"instance": a.instance, "time": time.strftime("%Y-%m-%d %H:%M:%S"), "wsm": WSM}
    steps = [("1_cold_boot", lambda: cold_boot(a.instance, n5)),
             ("2_suspend_resume", lambda: suspend_resume(a.instance, n5)),
             ("3_warm_exec_ms", lambda: warm_exec(a.instance, n20)),
             ("4_dev_loop", lambda: dev_loop(a.instance)),
             ("7_resources", lambda: resources(a.instance, home))]
    if a.gpu:
        steps.append(("5_gpu_metal", lambda: (run(["stop", "--name", a.instance]), gpu_metal(a.gpu))[1]))
    else:
        r["5_gpu_metal"] = {"status": "NOT RUN", "reason": "no --gpu instance given"}
    r["6_stability"] = {"status": "NOT RUN", "reason": "run stability suite separately; short timing runs do not establish long-run stability"}
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    output_file = out / f"p6-{time.strftime('%Y%m%d-%H%M%S')}.json"
    for k, step in steps:
        print(f"[p6] {k} ...", file=sys.stderr, flush=True)
        try:
            r[k] = step()
        except Exception as e:  # record and continue: one broken item must not hide the others
            r[k] = {"status": "ERROR", "error": repr(e)}
        output_file.write_text(json.dumps(r, indent=2))
    r["verdict"] = verdict(r)
    output_file.write_text(json.dumps(r, indent=2))
    print(json.dumps({k: (v if not isinstance(v, dict) else {x: y for x, y in v.items() if x != "runs"}) for k, v in r.items()}, indent=2))
    print(f"[p6] written {output_file}", file=sys.stderr)
    # NOT RUN is reported, not failed; anything measured and missing its target fails the run.
    sys.exit(1 if any(s in ("FAIL", "ERROR") for s in r["verdict"].values()) else 0)


if __name__ == "__main__":
    main()
