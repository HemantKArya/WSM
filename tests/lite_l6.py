"""Lite L6: 5 busy suspend/resume cycles. A detached CPU-busy process (own process group: lite's sshd
kills the session's group on disconnect) is the same-session marker, with kern.bootsessionuuid.
usage: lite_l6.py INSTANCE   (WSM_BIN, WSM_HOME set). Prints one JSON line."""
import json, os, subprocess, sys, time

W, N = os.environ["WSM_BIN"], sys.argv[1]
ENV = dict(os.environ, MSYS_NO_PATHCONV="1")


def run(*a, t=300):
    s = time.perf_counter()
    p = subprocess.run([W, *a], capture_output=True, text=True, timeout=t, env=ENV)
    return p.returncode, (p.stdout + p.stderr).strip(), time.perf_counter() - s


BUSY = ("perl -e 'if(fork()==0){setpgrp(0,0); open STDIN,\"</dev/null\"; open STDOUT,\">/dev/null\";"
        " open STDERR,\">/dev/null\"; $|=1; my $x=0; while(1){$x++}} else {sleep 1}'; ps -axo pid,command | awk '/[p]erl -e if/ {print $1; exit}' > /tmp/l6pid;"
        " sysctl -n kern.bootsessionuuid > /tmp/l6boot")
CHECK = 'kill -0 $(cat /tmp/l6pid) && [ "$(sysctl -n kern.bootsessionuuid)" = "$(cat /tmp/l6boot)" ] && echo SAME'
run("exec", "--name", N, "--", BUSY)
out = {"suspend_s": [], "resume_to_ssh_s": [], "same_session": [], "state_bytes": []}
for _ in range(5):
    rc, msg, ds = run("stop", "--name", N)
    st = os.path.join(os.environ["WSM_HOME"], "instances", N, "state.bin")
    out["state_bytes"].append(os.path.getsize(st) if os.path.exists(st) else None)
    rc2, msg2, dr = run("start", "--name", N)
    rc3, o, _ = run("exec", "--name", N, "--", CHECK)
    out["suspend_s"].append(round(ds, 2)); out["resume_to_ssh_s"].append(round(dr, 2)); out["same_session"].append("SAME" in o)
run("exec", "--name", N, "--", "kill $(cat /tmp/l6pid)")
print(json.dumps(out))
