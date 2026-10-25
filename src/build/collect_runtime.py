"""Assemble the QEMU runtime folder: our build + every MSYS2 UCRT64 DLL it needs (transitively) + pc-bios.

usage: collect_runtime.py BUILD_DIR OUT_DIR
BUILD_DIR holds qemu-system-x86_64.exe, qemu-img.exe, reims_vgpu_c.dll, ReimsVgpu.rom (build-qemu.sh output).
OUT_DIR is <runtime>/qemu.
"""
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
UCRT = ROOT / "tools/msys64/ucrt64/bin"
OBJDUMP = UCRT / "objdump.exe"
BIOS = ROOT / "work/reims/vendor/qemu/pc-bios"  # aljus7 v0.3.0 release's pc-bios (same QEMU source pin)


def imports(path):
    out = subprocess.run([str(OBJDUMP), "-p", str(path)], capture_output=True, text=True, check=True).stdout
    return [l.split(":", 1)[1].strip() for l in out.splitlines() if "DLL Name:" in l]


def main(build, out):
    build, out = pathlib.Path(build), pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    for f in ("qemu-system-x86_64.exe", "qemu-img.exe", "reims_vgpu_c.dll", "ReimsVgpu.rom"):
        shutil.copy2(build / f, out / f)
    todo, seen = ["qemu-system-x86_64.exe", "qemu-img.exe", "reims_vgpu_c.dll"], set()
    while todo:
        name = todo.pop()
        if name.lower() in seen:
            continue
        seen.add(name.lower())
        src = out / name if (out / name).exists() else UCRT / name
        if not src.exists():
            continue  # system DLL (kernel32, api-ms-win-*, ...)
        if src.parent != out:
            shutil.copy2(src, out / name)
        todo.extend(imports(out / name))
    if BIOS.exists():
        shutil.copytree(BIOS, out / "pc-bios", dirs_exist_ok=True)
    print(f"{len(list(out.iterdir()))} entries in {out}")


if __name__ == "__main__":
    main(*sys.argv[1:3])
