#!/usr/bin/env python3
"""Rasterize the Guild Foundry icon (public/icon.svg) into Tauri icon assets.

The SVG is plain rects + a circle, so it is re-drawn here with a tiny
supersampled rasterizer instead of pulling in an SVG library. Output is
32x32.png, 128x128.png, 128x128@2x.png, icon.ico and tray-32.png.
"""
import math
import os
import struct
import zlib

BG = (0x0A, 0x0A, 0x0A)
GOLD = (0xC9, 0xA2, 0x27)
TAN = (0xD2, 0xB4, 0x8C)
SS = 4  # supersample factor per axis


def blend(dst, src, alpha):
    return tuple(int(round(d + (s - d) * alpha)) for d, s in zip(dst, src))


def render(size):
    """Return a supersampled RGBA pixel buffer of `size` x `size`."""
    n = size * SS
    col = [[*BG, 1.0] for _ in range(n) for _ in range(n)]

    def fill_rect(x0, y0, x1, y1, border=None, bw=0.0):
        """Fill [x0,x1]x[y0,y1]; if `border` is given paint a stroke of width bw."""
        for y in range(max(0, int(y0)), min(n, math.ceil(y1))):
            cy = y + 0.5
            for x in range(max(0, int(x0)), min(n, math.ceil(x1))):
                cx = x + 0.5
                if border is None:
                    col[y * n + x][0], col[y * n + x][1], col[y * n + x][2] = BG
                    continue
                near_x = min(abs(cx - x0), abs(cx - x1))
                near_y = min(abs(cy - y0), abs(cy - y1))
                if near_x < bw or near_y < bw:
                    col[y * n + x][0], col[y * n + x][1], col[y * n + x][2] = border

    s = n / 512.0
    fill_rect(0, 0, n, n)
    # outer gold square stroke (24 units) at 96..416
    fill_rect(96 * s, 96 * s, 416 * s, 416 * s, GOLD, 24 * s)
    # inner tan square stroke (16 units) at 176..336
    fill_rect(176 * s, 176 * s, 336 * s, 336 * s, TAN, 16 * s)
    # gold dot r=32 at center
    r = 32 * s
    for y in range(max(0, int(256 * s - r)), min(n, int(256 * s + r) + 1)):
        cy = y + 0.5 - 256 * s
        for x in range(max(0, int(256 * s - r)), min(n, int(256 * s + r) + 1)):
            cx = x + 0.5 - 256 * s
            if cx * cx + cy * cy <= r * r:
                col[y * n + x][0], col[y * n + x][1], col[y * n + x][2] = GOLD

    # Downsample SS x SS blocks to the final size.
    out = bytearray()
    for oy in range(size):
        out.append(0)  # PNG filter type 0 for this scanline
        for ox in range(size):
            acc = [0.0, 0.0, 0.0]
            for sy in range(SS):
                base = (oy * SS + sy) * n + ox * SS
                for sx in range(SS):
                    px = col[base + sx]
                    for i in range(3):
                        acc[i] += px[i]
            out.extend(
                bytes(
                    [
                        int(round(acc[0] / (SS * SS))),
                        int(round(acc[1] / (SS * SS))),
                        int(round(acc[2] / (SS * SS))),
                        255,
                    ]
                )
            )
    return bytes(out)


def png(size):
    raw = render(size)

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(png_bytes, size):
    """Minimal ICO wrapping a single PNG image (supported since Vista)."""
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack(
        "<BBBBHHII",
        size if size < 256 else 0,
        size if size < 256 else 0,
        0,
        0,
        1,
        32,
        len(png_bytes),
        6 + 16,
    )
    return header + entry + png_bytes


here = os.path.dirname(os.path.abspath(__file__))
icons = os.path.join(here, "icons")
os.makedirs(icons, exist_ok=True)

for name, size in [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("icon.png", 512),
]:
    with open(os.path.join(icons, name), "wb") as f:
        f.write(png(size))
    print("wrote", name)

with open(os.path.join(icons, "icon.ico"), "wb") as f:
    f.write(ico(png(256), 256))
print("wrote icon.ico")

with open(os.path.join(icons, "tray-32.png"), "wb") as f:
    f.write(png(32))
print("wrote tray-32.png")