"""P3: fresh-instance identity, read-only base integrity, and export/import/create via CLI."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def main():
    home = Path(os.environ["WSM_HOME"])
    wsm = os.environ["WSM_BIN"]
    bases = sorted((home / "bases").glob("base-*.qcow2"), key=lambda p: p.stat().st_mtime)
    assert bases, "no built base"
    base = bases[-1]
    result = {"base": str(base), "base_bytes": base.stat().st_size, "instances": {}, "status": "RUNNING"}
    out = Path("logs/r3/p3-generic.json")

    def save():
        out.write_text(json.dumps(result, indent=2))

    def run(*args, timeout=300):
        p = subprocess.run([wsm, *args], capture_output=True, timeout=timeout)
        if p.returncode:
            raise RuntimeError(f"{args}: {p.returncode}: {p.stderr.decode(errors='replace')[-1500:]}")
        return p.stdout.decode(errors="replace").strip()

    def inspect(name, source):
        assert not (home / "instances" / name / "manifest.toml").exists(), f"test needs a fresh name: {name}"
        run("--name", name, "create", "--base", str(source))
        started = time.perf_counter()
        run("--name", name, "start")
        def guest(cmd):
            return run("--name", name, "exec", "--", cmd)
        info = {"first_boot_s": round(time.perf_counter() - started, 2),
                "hostname": guest("hostname"), "user": guest("id -un"),
                "uid": guest("id -u"), "os": guest("sw_vers -productVersion"),
                "ssh_host_fingerprint": guest("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub"),
                "authorized_key": guest("cat ~/.ssh/authorized_keys"),
                "sip": guest("csrutil status"),
                "sshd": guest("sudo /usr/sbin/sshd -T | egrep '^(passwordauthentication|kbdinteractiveauthentication|permitrootlogin) '"),
                "firstboot": guest("sudo test -f /var/db/wsm/firstboot.done && echo complete")}
        info["manifest"] = tomllib.loads((home / "instances" / name / "manifest.toml").read_text())
        public = (home / "instances" / name / "keys/id_ed25519.pub").read_text().strip()
        assert info["authorized_key"] == public, "instance authorized key does not match its config disk"
        assert info["user"] == "dev" and info["uid"] == "501" and info["os"] == "26.3", info
        assert info["firstboot"] == "complete"
        assert "passwordauthentication no" in info["sshd"] and "kbdinteractiveauthentication no" in info["sshd"]
        run("--name", name, "stop", "--shutdown")
        result["instances"][name] = info
        save()
        return info

    try:
        expected = base.with_suffix(".qcow2.sha256").read_text().split()[0]
        assert sha(base) == expected, "initial base checksum mismatch"
        a = inspect("basecheck", base)
        exported = Path("work/r3/exported-base.qcow2").resolve()
        assert shutil.disk_usage(exported.parent).free > base.stat().st_size + (2 << 30), "insufficient disk for export/import acceptance"
        run("base", "export", str(exported), "--base", str(base), timeout=1800)
        assert sha(exported) == expected
        imported = run("base", "import", str(exported), timeout=1800)
        result["import"] = imported
        b = inspect("basecheck2", base)
        for key in ["mac", "smbios_serial", "smbios_uuid", "smbios_mlb"]:
            assert a["manifest"][key] != b["manifest"][key], f"shared {key}"
        for key in ["ssh_host_fingerprint", "authorized_key", "hostname"]:
            assert a[key] != b[key], f"shared {key}"
        assert sha(base) == expected, "base changed after instance boots"
        result["base_sha256"] = expected
        result["status"] = "PASS"
        # The verified source stays in bases/. Remove only this test's duplicate export.
        exported.chmod(0o600)
        exported.unlink()
        exported.with_suffix(".qcow2.sha256").unlink()
    except Exception as error:
        result["status"] = "FAIL"
        result["error"] = repr(error)
        raise
    finally:
        save()
        print(out)


if __name__ == "__main__":
    main()
