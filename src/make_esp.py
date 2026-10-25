"""Build a speed-tuned OpenCore ESP directory (served to QEMU via fat:rw:<dir>).

usage: python src/make_esp.py OUT_DIR [--boot-args "..."] [--amd]
Source: refs/OSX-KVM/OpenCore (EFI + config.plist).
"""
import argparse, plistlib, shutil, pathlib

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("--boot-args", default="-v keepsyms=1 serial=3 debug=0x100 -no_compat_check")
ap.add_argument("--amd", action="store_true", help="merge AMD_Vanilla kernel patches")
ap.add_argument("--core-count", type=int, default=4)
ap.add_argument("--topo", action="store_true", help="AMD_Vanilla topology patches only (WHPX on AMD)")
a = ap.parse_args()

src = pathlib.Path("refs/OSX-KVM/OpenCore")
out = pathlib.Path(a.out)
shutil.rmtree(out, ignore_errors=True)
shutil.copytree(src / "EFI", out / "EFI")
p = plistlib.load(open(src / "config.plist", "rb"))

boot = p["Misc"]["Boot"]
boot.update(Timeout=0, ShowPicker=False, HideAuxiliary=False, PickerMode="Builtin",
            PollAppleHotKeys=False, TakeoffDelay=0, PickerAttributes=0)
p["Misc"]["Debug"].update(Target=0x09, DisplayLevel=0x80000042, AppleDebug=True, ApplePanic=True,
                          DisableWatchDog=True)
p["Misc"]["Serial"]["Init"] = False
p["Booter"]["Quirks"]["ProvideCustomSlide"] = True  # skip KASLR slides that overlap unusable memory (EB.MM.AKMR abort)
p["Misc"]["Security"]["ScanPolicy"] = 0x10F0103  # APFS/HFS on SATA/NVMe/PCI; excludes the ESP so OC never boots itself
for d in p["UEFI"]["Drivers"]:
    if d["Path"] in ("OpenCanopy.efi", "ToggleSipEntry.efi", "ResetNvramEntry.efi"):
        d["Enabled"] = False
for k in p["Kernel"]["Patch"]:
    # Only un-mute serial; the panic/early-print replacements print raw format strings and hide RIP.
    if k["Comment"] in ("Enable early serial output on RELEASE kernel",):
        k["Enabled"] = True
nv = p["NVRAM"]["Add"]["7C436110-AB2A-4BBB-A880-FE41995C9F82"]
nv["boot-args"] = a.boot_args
nv["csr-active-config"] = bytes.fromhex("67000000")  # SIP mostly off (allow edits in X3)
p["NVRAM"].setdefault("Delete", {})["7C436110-AB2A-4BBB-A880-FE41995C9F82"] = ["boot-args", "csr-active-config"]

# WHPX on AMD: SYSENTER_EIP/ESP (0x176/0x175) are 32-bit on AMD and Hyper-V does not widen them
# for Intel-vendor guests (KVM does, svm.c sysenter_eip_hi), so XNU's cpu_syscall_init wrmsr of a
# 64-bit address takes #GP right after handoff. 64-bit macOS uses SYSCALL only, so drop both writes.
def kpatch(comment, find, replace):
    return dict(Arch="x86_64", Base="", Comment=comment, Count=0, Enabled=True, Find=bytes.fromhex(find),
                Identifier="kernel", Limit=0, Mask=b"", MaxKernel="", MinKernel="",
                Replace=bytes.fromhex(replace), ReplaceMask=b"", Skip=0)
p["Kernel"]["Patch"] += [
    kpatch("WSM: skip wrmsr SYSENTER_EIP (AMD+WHPX #GP)", "89d048c1ea20b9760100000f30", "89d048c1ea20b976010000" "9090"),
    kpatch("WSM: skip wrmsr SYSENTER_ESP (AMD+WHPX #GP)", "b9750100000f30b9800000c0", "b975010000" "9090" "b9800000c0"),
]

for k in p["Kernel"]["Add"]:
    if not (out / "EFI/OC/Kexts" / k["BundlePath"] / k.get("PlistPath", "Contents/Info.plist")).exists():
        k["Enabled"] = False
        print("disabled missing kext", k["BundlePath"])

# WHPX answers CPUID 0x2/0x4/0x5 from AMD hardware (not in QEMU's cpuidExitList), so leaf 4 is
# zero and XNU divides by cores_per_package=0. Take only AMD_Vanilla's topology patches.
if a.topo:
    amd = plistlib.load(open("refs/AMD_Vanilla/patches.plist", "rb"))["Kernel"]["Patch"]
    for k in amd:
        if "13.3+" in k["Comment"] and "cores_per_package" in k["Comment"]:
            k["Replace"] = bytes([0xBA, a.core_count, 0, 0, 0])
            p["Kernel"]["Patch"].append(k)
        elif any(s in k["Comment"] for s in (
                "_cpuid_set_cache_info",
                # Intel-only MSRs Hyper-V #GPs on AMD without exiting to QEMU (ignore-unknown-msr can't help)
                "_commpage_populate | Remove rdmsr", "Remove wrmsr(0x8B)", "Replace rdmsr(0x8B)",
                "Set flag=1", "_i386_init | Remove 3 rdmsr",
                "non-monotonic time panic")):
            p["Kernel"]["Patch"].append(k)

if a.amd:
    amd = plistlib.load(open("refs/AMD_Vanilla/patches.plist", "rb"))
    patches = amd["Kernel"]["Patch"]
    for k in patches:
        if "algrey - Force cpuid_cores_per_package" in k.get("Comment", "") or "cores_per_package" in k.get("Comment", ""):
            # core count patch: replace placeholder byte with core count
            r = bytearray(k["Replace"])
            if r and r[1:2] == b"\x00":
                r[1] = a.core_count
            k["Replace"] = bytes(r)
    p["Kernel"]["Patch"] += patches
    p["Kernel"]["Quirks"]["ProvideCurrentCpuInfo"] = True

plistlib.dump(p, open(out / "EFI/OC/config.plist", "wb"))
print("ESP ready:", out, "| boot-args:", a.boot_args, "| amd:", a.amd)
