"""Merge WSM's AMD-on-WHPX kernel patches (+ early serial) into another project's OpenCore config.plist.

usage: python src/merge_amd_oc.py CONFIG.plist --cores N [--no-serial]
Same patch set as make_esp.py --topo (round 1, X2 fixes #5-#7). Idempotent: drops earlier 'WSM:'/AMD_Vanilla entries first.
"""
import argparse, plistlib, pathlib, shutil
ap = argparse.ArgumentParser()
ap.add_argument("config"); ap.add_argument("--cores", type=int, required=True); ap.add_argument("--no-serial", action="store_true")
a = ap.parse_args()
p = pathlib.Path(a.config)
if not p.with_suffix(".plist.pre-wsm").exists():
    shutil.copy(p, p.with_suffix(".plist.pre-wsm"))
c = plistlib.loads(p.read_bytes())
root = pathlib.Path(__file__).resolve().parent.parent
def kpatch(comment, find, replace):
    return dict(Arch="x86_64", Base="", Comment=comment, Count=0, Enabled=True, Find=bytes.fromhex(find),
                Identifier="kernel", Limit=0, Mask=b"", MaxKernel="", MinKernel="",
                Replace=bytes.fromhex(replace), ReplaceMask=b"", Skip=0)
mine = [kpatch("WSM: skip wrmsr SYSENTER_EIP (AMD+WHPX #GP)", "89d048c1ea20b9760100000f30", "89d048c1ea20b976010000" "9090"),
        kpatch("WSM: skip wrmsr SYSENTER_ESP (AMD+WHPX #GP)", "b9750100000f30b9800000c0", "b975010000" "9090" "b9800000c0")]
for k in plistlib.load(open(root / "refs/AMD_Vanilla/patches.plist", "rb"))["Kernel"]["Patch"]:
    cm = k["Comment"]
    if "13.3+" in cm and "cores_per_package" in cm:
        k["Replace"] = bytes([0xBA, a.cores, 0, 0, 0]); mine.append(k)
    elif any(s in cm for s in ("_cpuid_set_cache_info", "_commpage_populate | Remove rdmsr", "Remove wrmsr(0x8B)",
                               "Replace rdmsr(0x8B)", "Set flag=1", "_i386_init | Remove 3 rdmsr", "non-monotonic time panic")):
        mine.append(k)
if not a.no_serial:
    for k in plistlib.load(open(root / "refs/OSX-KVM/OpenCore/config.plist", "rb"))["Kernel"]["Patch"]:
        if k["Comment"] == "Enable early serial output on RELEASE kernel":
            k["Enabled"] = True; mine.append(k)
names = {k["Comment"] for k in mine}
c["Kernel"]["Patch"] = [k for k in c["Kernel"]["Patch"] if k["Comment"] not in names] + mine
p.write_bytes(plistlib.dumps(c, sort_keys=False))
for k in mine: print(" +", k["Comment"][:90])
