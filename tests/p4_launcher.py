"""Regression: auto-started detached wsmd must not hold a captured CLI's output pipes open."""
import json
import os
from pathlib import Path
import subprocess
import time

wsm = os.environ["WSM_BIN"]
started = time.perf_counter()
p = subprocess.Popen([wsm, "--name", "hl1", "status"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
try:
    out, err = p.communicate(timeout=15)
except subprocess.TimeoutExpired:
    subprocess.run([wsm, "quit"], timeout=10)
    out, err = p.communicate(timeout=5)
    raise AssertionError("CLI capture waited on detached daemon EOF")
elapsed = time.perf_counter() - started
assert p.returncode == 0, err
assert json.loads(out)["name"] == "hl1", out
result = {"status": "PASS", "autostart_capture_seconds": round(elapsed, 3)}
Path("logs/r3/p4-launcher.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result))
