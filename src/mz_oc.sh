#!/usr/bin/env bash
# Build work/r2/mz/oc-<name>.img from aljus7's pristine OC config + early serial + a chosen subset of WSM/AMD patches.
# usage: src/mz_oc.sh NAME "comment-substring|..."   (empty = serial only)
cd "$(dirname "$0")/.."
N=$1; SEL=${2:-}
rm -rf work/r2/mz/ocroot && cp -r work/reims/vm/firmware/opencore-1.0.8/root work/r2/mz/ocroot
cp work/r2/mz/ocroot/EFI/OC/config.plist.wsm-amd /tmp/amd.plist 2>/dev/null
.venv/Scripts/python.exe -I - "$SEL" <<'PY'
import plistlib, sys, re
sel = sys.argv[1]
amd = plistlib.load(open('work/reims/vm/firmware/opencore-1.0.8/root/EFI/OC/config.plist.wsm-amd', 'rb'))['Kernel']['Patch']
p = 'work/r2/mz/ocroot/EFI/OC/config.plist'
c = plistlib.load(open('work/r2/mz/ocroot/EFI/OC/config.plist.pre-wsm', 'rb'))
keep = [k for k in amd if k['Comment'] == 'Enable early serial output on RELEASE kernel' or (sel and re.search(sel, k['Comment']))]
c["Kernel"]["Patch"] = keep
import os
if os.environ.get("PCCI"): c["Kernel"]["Quirks"]["ProvideCurrentCpuInfo"] = True
plistlib.dump(c, open(p, 'wb'), sort_keys=False)
print([k['Comment'][:50] for k in keep])
PY
rm -f work/r2/mz/oc-$N.img && tools/qemu/qemu-img.exe convert -f vvfat -O raw fat:work/r2/mz/ocroot work/r2/mz/oc-$N.img && echo work/r2/mz/oc-$N.img
