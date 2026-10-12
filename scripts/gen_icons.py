#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Regenerate the PNG/ICO icon set for Guild Foundry AI.

The binary icons are not committed to git; run this after cloning:
    python3 scripts/gen_icons.py
Requires: Pillow (pip install pillow)
"""
from PIL import Image, ImageDraw
import os

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src-tauri", "icons")
os.makedirs(OUT, exist_ok=True)


def draw_icon(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (10, 10, 10, 255))
    d = ImageDraw.Draw(img)
    m = size // 16
    w = max(2, size // 20)
    d.rectangle([m * 3, m * 3, size - m * 3, size - m * 3],
                outline=(201, 162, 39, 255), width=w)
    d.rectangle([m * 5, m * 5, size - m * 5, size - m * 5],
                outline=(210, 180, 140, 255), width=max(1, w // 2))
    r = size // 16
    d.ellipse([size // 2 - r, size // 2 - r, size // 2 + r, size // 2 + r],
              fill=(201, 162, 39, 255))
    return img


draw_icon(32).save(os.path.join(OUT, "32x32.png"))
draw_icon(128).save(os.path.join(OUT, "128x128.png"))
draw_icon(256).save(os.path.join(OUT, "128x128@2x.png"))
draw_icon(32).save(os.path.join(OUT, "tray-32.png"))
draw_icon(256).save(os.path.join(OUT, "icon.png"))
draw_icon(256).save(os.path.join(OUT, "icon.ico"), format="ICO",
                    sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("icons written to", OUT)
