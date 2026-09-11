#!/usr/bin/env python3
"""
Generate Bilusic's app icons (PNG / ICNS / ICO) with only the stdlib.
Design: slate rounded-square background with a white play triangle.

Run:  python3 scripts/gen_icons.py
"""
import os
import struct
import zlib

ICON_DIR = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
BG = (100, 116, 139, 255)   # slate-500
FG = (255, 255, 255, 255)


def png(width: int, height: int, rows: bytes) -> bytes:
    def chunk(typ: bytes, data: bytes) -> bytes:
        body = typ + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    raw = b"".join(b"\x00" + rows[y * width * 4 : (y + 1) * width * 4] for y in range(height))
    idat = zlib.compress(raw, 9)
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def render(size: int) -> bytes:
    # Background
    px = bytearray(BG * (size * size))
    # Play triangle, scaled to the canvas
    a = (size * 0.41, size * 0.30)   # top-left
    b = (size * 0.41, size * 0.70)   # bottom-left
    c = (size * 0.72, size * 0.50)   # right tip

    def sign(p1, p2, p3):
        return (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])

    for y in range(size):
        for x in range(size):
            d1 = sign((x, y), a, b)
            d2 = sign((x, y), b, c)
            d3 = sign((x, y), c, a)
            has_neg = (d1 < 0) or (d2 < 0) or (d3 < 0)
            has_pos = (d1 > 0) or (d2 > 0) or (d3 > 0)
            if not (has_neg and has_pos):  # inside or on edge
                i = (y * size + x) * 4
                px[i:i + 4] = bytes(FG)
    return png(size, size, bytes(px))


def icns(images):
    body = b""
    for typ, data in images:
        body += typ + struct.pack(">I", len(data) + 8) + data
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def ico(png_bytes: bytes) -> bytes:
    hdr = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png_bytes), 22)
    return hdr + entry + png_bytes


def main():
    os.makedirs(ICON_DIR, exist_ok=True)
    p32 = render(32)
    p128 = render(128)
    p256 = render(256)
    p512 = render(512)

    with open(os.path.join(ICON_DIR, "32x32.png"), "wb") as f:
        f.write(p32)
    with open(os.path.join(ICON_DIR, "128x128.png"), "wb") as f:
        f.write(p128)
    with open(os.path.join(ICON_DIR, "128x128@2x.png"), "wb") as f:
        f.write(p256)
    with open(os.path.join(ICON_DIR, "icon.ico"), "wb") as f:
        f.write(ico(p256))
    with open(os.path.join(ICON_DIR, "icon.icns"), "wb") as f:
        f.write(icns([(b"ic07", p128), (b"ic08", p256), (b"ic09", p512)]))
    print("icons written to", os.path.abspath(ICON_DIR))


if __name__ == "__main__":
    main()
