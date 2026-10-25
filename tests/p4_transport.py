"""CLI acceptance for persistent exec: binary IO, separate stderr, exit status and concurrent clients."""
import concurrent.futures
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

wsm = os.environ["WSM_BIN"]
name = sys.argv[1]
output_file = Path("logs/r3/p4-transport.json")
output_file.write_text(json.dumps({"status": "RUNNING", "started": time.strftime('%Y-%m-%d %H:%M:%S')}))
def record_failure(kind, error, traceback):
    output_file.write_text(json.dumps({"status": "FAIL", "error": repr(error), "time": time.strftime('%Y-%m-%d %H:%M:%S')}, indent=2))
    sys.__excepthook__(kind, error, traceback)
sys.excepthook = record_failure


def run(command, data=b"", timeout=60):
    started = time.perf_counter()
    p = subprocess.run([wsm, "--name", name, "exec", "--", command], input=data, capture_output=True, timeout=timeout)
    return p, (time.perf_counter() - started) * 1000


sample = bytes(range(256)) * 4096
p, ms = run("cat; printf transport-stderr >&2; exit 7", sample)
assert p.returncode == 7, (p.returncode, p.stderr[-500:])
assert p.stdout == sample, (len(p.stdout), len(sample))
assert p.stderr == b"transport-stderr", p.stderr[-500:]
warm = [run("true") for _ in range(20)]
assert all(p.returncode == 0 and not p.stdout and not p.stderr for p, _ in warm)
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    futures = [pool.submit(run, f"sleep 1; printf client-{i}") for i in range(4)]
    concurrent = [f.result() for f in futures]
assert all(p.returncode == 0 and p.stdout == f"client-{i}".encode() for i, (p, _) in enumerate(concurrent))
result = {"status": "PASS", "binary_roundtrip_bytes": len(sample), "binary_exit": 7,
          "binary_ms": round(ms, 2), "warm_exec_ms_median_20": round(statistics.median(t for _, t in warm), 2),
          "concurrent_ms": [round(t, 2) for _, t in concurrent]}
output_file.write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
