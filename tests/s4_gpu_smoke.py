"""S4: on a running, cold-booted GPU instance: gui prewarm/start, 30 s animation, per-second reims fps."""
import json, sys, time
sys.path.insert(0, "tests")
import p6_gpu as g
n = sys.argv[1]; anim_local = sys.argv[2]
st = g.Stats(n)
r = {}
g.run(n, "exec", "--", "sudo rm -f /var/db/wsm/no-prewarm", check=False)
t = time.perf_counter()
r["prewarm_status"] = g.run(n, "gui", "prewarm", check=False)[-200:]
while "session: up" not in g.run(n, "gui", "status", check=False) and time.perf_counter() - t < 180:
    time.sleep(2)
r["session_up_after_s"] = round(time.perf_counter() - t, 1)
t = time.perf_counter(); r["gui_start"] = g.run(n, "gui", "start", timeout=180, check=False)[-200:]
r["gui_start_s"] = round(time.perf_counter() - t, 1)
g.run(n, "cp", anim_local, ":wsm-anim"); g.run(n, "exec", "--", "chmod 755 ~/wsm-anim")
st.mark()
r["app"] = g.run(n, "exec", "--", g.as_user("$HOME/wsm-anim 30"), timeout=180, check=False).splitlines()[-1:]
rates = st.rates(); r["fps_all"] = rates
steady = rates[2:]
r["fps"] = g.summary(steady)
print(json.dumps(r))
