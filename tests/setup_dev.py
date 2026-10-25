"""Provision only the disposable development-test guest, through wsm CLI and pinned local CLT packages."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
WSM = os.environ["WSM_BIN"]
NAME = sys.argv[1]
BREW_COMMIT = "51da7f234a4ca944ddc88d02cbbb0d11062eefcc"


def run(*args, timeout=1800):
    p = subprocess.run([WSM, "--name", NAME, *args], capture_output=True, timeout=timeout)
    output = p.stdout.decode(errors="replace") + p.stderr.decode(errors="replace")
    if p.returncode:
        raise RuntimeError(f"{args}: {p.returncode}: {output[-2500:]}")
    return output.strip()


started = time.perf_counter()
pins = json.loads((ROOT / "guest/build/clt-pins.json").read_text())
for pkg in pins:
    local = ROOT / "work/dl-clt" / pkg["name"]
    with local.open("rb") as f:
        assert hashlib.file_digest(f, "sha256").hexdigest() == pkg["sha256"], f"bad cached CLT package {local}"
    print(f"installing {pkg['name']}", flush=True)
    run("cp", str(local), f":/tmp/{pkg['name']}")
    run("exec", "--", f"sudo /usr/sbin/installer -pkg /tmp/{pkg['name']} -target / && rm -f /tmp/{pkg['name']}")
run("exec", "--", "sudo xcode-select -s /Library/Developer/CommandLineTools; d=/Library/Developer/CommandLineTools/SDKs; test -e $d/MacOSX.sdk || sudo ln -s MacOSX26.sdk $d/MacOSX.sdk; xcrun --sdk macosx --show-sdk-path")
print("installing pinned Homebrew checkout", flush=True)
run("exec", "--", "sudo mkdir -p /usr/local/Homebrew /usr/local/bin && sudo chown dev:staff /usr/local/Homebrew /usr/local/bin")
run("exec", "--", f"git -C /usr/local/Homebrew init -q && (git -C /usr/local/Homebrew remote add origin https://github.com/Homebrew/brew.git 2>/dev/null || git -C /usr/local/Homebrew remote set-url origin https://github.com/Homebrew/brew.git) && git -C /usr/local/Homebrew fetch -q --depth 1 origin {BREW_COMMIT} && git -C /usr/local/Homebrew checkout -q --detach FETCH_HEAD && ln -sf ../Homebrew/bin/brew /usr/local/bin/brew")
versions = run("exec", "--", "clang --version | head -1; swiftc --version; git --version; /usr/local/bin/brew --version")
result = {"instance": NAME, "brew_commit": BREW_COMMIT, "clt_packages": pins, "versions": versions,
          "seconds": round(time.perf_counter() - started, 1), "status": "PASS"}
(ROOT / "logs/r3/dev-setup.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
