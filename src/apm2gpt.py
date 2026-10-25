"""Write GPT head/tail blobs describing one Apple_HFS partition at the same LBAs as an APM installer image.

usage: python src/apm2gpt.py START_LBA SECTORS TOTAL_SECTORS OUTDIR  -> OUTDIR/head.bin (LBA0-33), OUTDIR/tail.bin (last 33 LBAs)
"""
import struct, sys, uuid, zlib, pathlib
start, count, total, out = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), pathlib.Path(sys.argv[4])
HFS = uuid.UUID("48465300-0000-11AA-AA11-00306543ECAC").bytes_le
entry = HFS + uuid.uuid4().bytes_le + struct.pack("<QQQ", start, start + count - 1, 0) + "Install macOS".encode("utf-16-le").ljust(72, b"\0")
entries = entry.ljust(128 * 128, b"\0")
ecrc = zlib.crc32(entries) & 0xffffffff
disk_guid = uuid.uuid4().bytes_le
def header(cur, backup, ent_lba):
    h = struct.pack("<8sIIIIQQQQ16sQIII", b"EFI PART", 0x10000, 92, 0, 0, cur, backup, 34, total - 34, disk_guid, ent_lba, 128, 128, ecrc)
    return (h[:16] + struct.pack("<I", zlib.crc32(h) & 0xffffffff) + h[20:]).ljust(512, b"\0")
mbr = bytearray(512)
mbr[446:462] = bytes([0, 0, 2, 0, 0xEE, 0xFF, 0xFF, 0xFF]) + struct.pack("<II", 1, min(total - 1, 0xFFFFFFFF))
mbr[510:512] = b"\x55\xaa"
(out / "head.bin").write_bytes(bytes(mbr) + header(1, total - 1, 2) + entries)
(out / "tail.bin").write_bytes(entries + header(total - 1, 1, total - 33))
print("head", len(bytes(mbr)) + 512 + len(entries), "tail", len(entries) + 512)
