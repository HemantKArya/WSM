"""Read-only inventory of project-generated disk use, to select safe cleanup targets."""
import heapq
import json
import os
from pathlib import Path

root = Path(__file__).resolve().parents[1]
totals = {}
largest = []
for parent, directories, files in os.walk(root):
    rel = Path(parent).relative_to(root).parts
    group = "/".join(rel[:2]) or "."
    for name in files:
        path = Path(parent) / name
        try:
            size = path.stat().st_size
        except OSError:
            continue
        totals[group] = totals.get(group, 0) + size
        if size > 1 << 30:
            heapq.heappush(largest, (size, str(path.relative_to(root))))
            if len(largest) > 30:
                heapq.heappop(largest)
result = {"groups_gb": {k: round(v / 1e9, 2) for k, v in sorted(totals.items(), key=lambda v: -v[1])[:30]},
          "largest_files_gb": [[round(size / 1e9, 2), name] for size, name in sorted(largest, reverse=True)]}
(root / "logs/r3/project-usage.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
