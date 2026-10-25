"""Move a GPT's backup header/entries to the end of an enlarged disk image (via qemu-io), keeping partitions as they are.
usage: python src/gpt_grow.py IMAGE [fmt=qcow2]"""
import re, struct, subprocess, sys, zlib
img = sys.argv[1]; fmt = sys.argv[2] if len(sys.argv) > 2 else "qcow2"
import os
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
QIO = os.path.join(ROOT, "tools", "qemu", "qemu-io.exe"); QIMG = os.path.join(ROOT, "tools", "qemu", "qemu-img.exe")
def qio(*cmds):
    a = [QIO, "-f", fmt, img]
    for c in cmds: a += ["-c", c]
    return subprocess.run(a, capture_output=True, text=True, check=True).stdout
size = int(re.search(r"virtual size: .*\((\d+) bytes\)", subprocess.run([QIMG, "info", img], capture_output=True, text=True).stdout).group(1))
N = size // 512
out = qio("read -v 512 17408")
buf = bytearray()
for line in out.splitlines():
    m = re.match(r"^[0-9a-f]{8}:\s+((?:[0-9a-f]{2}\s+)+)", line)
    if m: buf += bytes.fromhex(m.group(1).replace(" ", ""))
buf = bytes(buf[:17408 - 512 + 512 - 512])  # hexdump of 17408 bytes starting at LBA1 -> header + 32 sectors of entries
hdr = bytearray(buf[:512]); entries = bytes(buf[512:512 + 128 * 128])
assert hdr[:8] == b"EFI PART", hdr[:16]
hs = struct.unpack_from("<I", hdr, 12)[0]
old_backup = struct.unpack_from("<Q", hdr, 32)[0]
struct.pack_into("<QQQ", hdr, 24, 1, N - 1, 34); struct.pack_into("<Q", hdr, 48, N - 34)
struct.pack_into("<Q", hdr, 72, 2)
hdr[16:20] = b"\0\0\0\0"; struct.pack_into("<I", hdr, 16, zlib.crc32(bytes(hdr[:hs])) & 0xffffffff)
bh = bytearray(hdr); struct.pack_into("<QQ", bh, 24, N - 1, 1); struct.pack_into("<Q", bh, 72, N - 33)
bh[16:20] = b"\0\0\0\0"; struct.pack_into("<I", bh, 16, zlib.crc32(bytes(bh[:hs])) & 0xffffffff)
tmp = "work/r2/_gpt.bin"
open(tmp, "wb").write(bytes(hdr)); qio(f"write -s {tmp} 512 512")
open(tmp, "wb").write(entries + bytes(bh)); qio(f"write -s {tmp} {(N - 33) * 512} {len(entries) + 512}")
print(f"GPT grown: old backup LBA {old_backup} -> {N - 1}; usable 34..{N - 34}")
