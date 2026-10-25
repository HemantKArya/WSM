"""S3 core smoke, one run: cold boot, warm exec, 5 suspend/resume cycles with session check.
usage: s3_core_smoke.py INSTANCE   (WSM_HOME and WSM_BIN set). Prints one JSON line."""
import json, os, statistics, subprocess, sys, time

W = os.environ["WSM_BIN"]
N = sys.argv[1]
ENV = dict(os.environ, MSYS_NO_PATHCONV="1")


def run(*a, t=300):
    s = time.perf_counter()
    p = subprocess.run([W, *a], capture_output=True, text=True, timeout=t, env=ENV)
    return p.returncode, (p.stdout + p.stderr).strip(), time.perf_counter() - s


def ex(cmd):
    return run("exec", "--name", N, "--", cmd)


out = {}
run("stop", "--name", N, "--shutdown")
time.sleep(5)
rc, _, dt = run("start", "--name", N, "--cold")
out["cold_start_to_ssh_s"] = round(dt, 1)
assert ex("true")[0] == 0
# session marker: a background process and a boot-time id that must survive suspend/resume
ex("nohup sleep 100000 >/dev/null 2>&1 & echo $! > /tmp/s3pid; sysctl -n kern.bootsessionuuid > /tmp/s3boot")
warm = [ex("true")[2] * 1000 for _ in range(20)]
out["warm_exec_ms"] = {"median": round(statistics.median(warm)), "max": round(max(warm))}
resumes, same = [], []
for i in range(5):
    rc, msg, dts = run("stop", "--name", N)
    rc2, msg2, dtr = run("start", "--name", N)
    rc3, o, dte = ex('kill -0 $(cat /tmp/s3pid) && [ "$(sysctl -n kern.bootsessionuuid)" = "$(cat /tmp/s3boot)" ] && echo SAME')
    resumes.append(round(dtr, 2))
    same.append("SAME" in o)
    out.setdefault("suspend_s", []).append(round(dts, 2))
    out.setdefault("first_exec_after_resume_ms", []).append(round(dte * 1000))
out["resume_to_ssh_s"] = resumes
out["same_session"] = same
w2 = [ex("true")[2] * 1000 for _ in range(20)]
out["warm_exec_after_resume_ms_median"] = round(statistics.median(w2))
print(json.dumps(out))
