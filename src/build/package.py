"""Create the tool-only distribution, with every shipped file hashed. Never includes a base disk."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(8 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--home", type=Path, required=True)
    ap.add_argument("--bin", type=Path, default=ROOT / "target/release")
    ap.add_argument("--out", type=Path, default=ROOT / "work/r3/dist/wsm-setup.zip")
    a = ap.parse_args()
    runtime = a.home / "runtime"
    required = ["qemu/qemu-system-x86_64.exe", "qemu/qemu-img.exe", "qemu/reims_vgpu_c.dll",
                "qemu/ReimsVgpu.rom", "firmware/OVMF_CODE.fd", "firmware/OVMF_VARS.fd",
                "firmware/opencore-amd.img", "firmware/opencore-intel.img", "tools/7z.exe", "tools/7z.dll"]
    for relative in required:
        if not (runtime / relative).is_file():
            raise SystemExit(f"Missing runtime file: {relative}")
    pin = json.loads((ROOT / "work/r3/qemu-v030/build.json").read_text())
    for name, expected in pin["files"].items():
        if digest(runtime / "qemu" / name) != expected:
            raise SystemExit(f"Runtime differs from the tested build pin: {name}")
    files = {f"runtime/{p.relative_to(runtime).as_posix()}": p for p in runtime.rglob("*") if p.is_file()}
    for name in ["wsm.exe", "wsmd.exe"]:
        files[name] = a.bin / name
    for name in ["README.md", "RESULTS.md", "Cargo.lock"]:
        files[name] = ROOT / name
    files["install.ps1"] = ROOT / "src/build/install.ps1"
    files["sources/uninstall.ps1"] = ROOT / "src/build/uninstall.ps1"
    files["sources/lite/local.sh"] = ROOT / "guest/lite/local.sh"
    files["sources/qemu-build.json"] = ROOT / "work/r3/qemu-v030/build.json"
    files["sources/build-qemu.sh"] = ROOT / "src/build/build-qemu.sh"
    for p in (ROOT / "src/patches").glob("*.patch"):
        files[f"sources/patches/{p.name}"] = p
    for prefix, folder in [("qemu", ROOT / "work/r3/reims-v030/vendor/qemu"),
                           ("reims", ROOT / "work/r3/reims-v030")]:
        for p in folder.glob("COPYING*"):
            files[f"licenses/{prefix}/{p.name}"] = p
        for p in folder.glob("LICENSE*"):
            if p.is_file():
                files[f"licenses/{prefix}/{p.name}"] = p
    manifest = {"version": "0.1.0", "base_included": False, "reims": pin["reims"], "qemu": pin["qemu"],
                "files": {name: {"sha256": digest(p), "bytes": p.stat().st_size} for name, p in sorted(files.items())}}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(a.out, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for name, path in sorted(files.items()):
            z.write(path, name)
        z.writestr("package-manifest.json", json.dumps(manifest, indent=2))
    checksum = digest(a.out)
    a.out.with_suffix(".zip.sha256").write_text(f"{checksum}  {a.out.name}\n")
    print(json.dumps({"zip": str(a.out), "bytes": a.out.stat().st_size, "sha256": checksum,
                      "files": len(files), "base_included": False}, indent=2))


if __name__ == "__main__":
    main()
