"""Host-driven Finder drag; preserve the matching reims log slice and frame statistics."""
import json
from pathlib import Path
import re
import statistics
import time
from gpu_followup import qmp

log = Path('logs/G1-followup/aljus-4g/stderr.log')
offset = log.stat().st_size
def event(events):
    qmp('input-send-event', {'events': events})
def move(x, y):
    event([{'type': 'abs', 'data': {'axis': 'x', 'value': int(x*32767/1919)}},
           {'type': 'abs', 'data': {'axis': 'y', 'value': int(y*32767/1079)}}])
start = time.monotonic()
for cycle in range(6):
    move(800, 210)
    event([{'type': 'btn', 'data': {'button': 'left', 'down': True}}])
    for i in range(80):
        phase = i/79
        dx = 240*(phase*2 if phase < .5 else (1-phase)*2)
        move(800+dx, 210+dx/5)
        time.sleep(.04)
    event([{'type': 'btn', 'data': {'button': 'left', 'down': False}}])
elapsed = time.monotonic()-start
time.sleep(1)
with log.open('rb') as f:
    f.seek(offset)
    data = f.read().decode(errors='replace')
Path('logs/G1-followup/aljus-4g/drag-reims.log').write_text(data)
fps = [float(x) for x in re.findall(r'stats: port 0: guest ([\d.]+)/s', data)]
result = dict(duration_s=elapsed, samples=len(fps), rates=fps,
              median=statistics.median(fps) if fps else None,
              mean=statistics.mean(fps) if fps else None,
              minimum=min(fps) if fps else None, maximum=max(fps) if fps else None)
Path('logs/G1-followup/aljus-4g/drag-stats.json').write_text(json.dumps(result, indent=2))
print(json.dumps(result))
