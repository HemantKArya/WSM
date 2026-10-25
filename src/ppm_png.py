"""Encode a QEMU P6 screendump as PNG using only the project Python's stdlib."""
import pathlib
import struct
import sys
import zlib

source, destination = map(pathlib.Path, sys.argv[1:])
with source.open('rb') as f:
    assert f.readline().strip() == b'P6'
    line = f.readline()
    while line.startswith(b'#'):
        line = f.readline()
    width, height = map(int, line.split())
    assert f.readline().strip() == b'255'
    data = f.read()
assert len(data) == width * height * 3
def chunk(kind, body):
    return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
rows = b''.join(b'\0' + data[y*width*3:(y+1)*width*3] for y in range(height))
destination.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
    + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))
print(destination)
