"""Pack a directory into a raw 'seed' disk: a plain tar padded to 1 MiB (guest: tar -xf /dev/diskN).
usage: python src/mkseed.py SRC_DIR OUT_IMG"""
import io, sys, tarfile, pathlib
src, out = pathlib.Path(sys.argv[1]), sys.argv[2]
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w", format=tarfile.USTAR_FORMAT) as t:
    for f in sorted(src.iterdir()):
        ti = t.gettarinfo(str(f), arcname=f.name)
        ti.uid = ti.gid = 0; ti.uname = ti.gname = "root"
        ti.mode = 0o755 if f.suffix == ".sh" else 0o644
        t.addfile(ti, open(f, "rb"))
data = buf.getvalue()
data += b"\0" * (-len(data) % (1 << 20))
open(out, "wb").write(data)
print(out, len(data), "bytes")
