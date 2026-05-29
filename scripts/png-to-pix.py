#!/usr/bin/env python3
"""Convert a 160x144 reference PNG to a raw RGB555 reference file.

The Game Boy Color PPU outputs 15-bit colour (RGB555 — 5 bits / 32 levels
per channel). PNGs are RGB888, so we reduce each channel to its top 5 bits
(`>> 3`). Comparing at 555 precision is *expansion-neutral*: emulators that
expand 555→888 (and apply colour correction) differently all collapse back
to the same 5-bit values, so a correct emulator isn't penalised for its
display-expansion curve. It also still distinguishes the four DMG shades
(0xFF/0xAA/0x55/0x00 → 31/21/10/0).

Output: 160*144*3 = 69120 bytes, one byte per channel (value 0-31), RGB order.

Usage: png-to-pix.py <input.png> <output.rgb555>
"""
import sys
from PIL import Image


def main():
    if len(sys.argv) != 3:
        print(f'Usage: {sys.argv[0]} <input.png> <output.rgb555>', file=sys.stderr)
        sys.exit(1)

    img = Image.open(sys.argv[1]).convert('RGB')
    if img.size != (160, 144):
        print(f'Error: expected 160x144, got {img.size[0]}x{img.size[1]}', file=sys.stderr)
        sys.exit(1)

    out = bytearray(160 * 144 * 3)
    i = 0
    for r, g, b in img.getdata():
        out[i] = r >> 3
        out[i + 1] = g >> 3
        out[i + 2] = b >> 3
        i += 3

    with open(sys.argv[2], 'wb') as f:
        f.write(out)
    print(f'  {sys.argv[1]} -> {sys.argv[2]} ({len(out)} bytes, RGB555)')


if __name__ == '__main__':
    main()
