#!/usr/bin/env python3
"""Convert 160x144 reference PNGs to raw RGB555 reference files.

The Game Boy Color PPU outputs 15-bit colour (RGB555 — 5 bits / 32 levels
per channel). PNGs are RGB888, so we reduce each channel to its top 5 bits
(`>> 3`). Comparing at 555 precision is *expansion-neutral*: emulators that
expand 555->888 (and apply colour correction) differently all collapse back
to the same 5-bit values, so a correct emulator isn't penalised for its
display-expansion curve. It also still distinguishes the four DMG shades
(0xFF/0xAA/0x55/0x00 -> 31/21/10/0).

Output per file: 160*144*3 = 69120 bytes, one byte per channel (value 0-31),
RGB order.

Batch mode (default) walks the given roots (or `test-suites`) once, in a
single interpreter, and (re)generates the sibling `<name>.rgb555` for every
`<name>.png` that is missing or older than its source. This matters in CI:
`.rgb555` files are gitignored, so every fresh-checkout trace shard would
otherwise re-spawn one Python process per PNG (~240 spawns, ~10s) — doing it
in one process drops that to a fraction of a second.

Usage:
  png-to-pix.py [root ...]          # batch: walk roots (default: test-suites)
  png-to-pix.py <input.png> <out>   # single file (e.g. for manual debugging)
"""
import os
import sys

from PIL import Image

# Per-byte ">> 3" as a translation table, so the whole RGB888 buffer collapses
# to 5-bit channels in one C-level pass instead of a Python per-pixel loop.
SHIFT5 = bytes(i >> 3 for i in range(256))


def convert(png_path, out_path):
    img = Image.open(png_path).convert('RGB')
    if img.size != (160, 144):
        print(f'Error: {png_path}: expected 160x144, got '
              f'{img.size[0]}x{img.size[1]}', file=sys.stderr)
        return False
    with open(out_path, 'wb') as f:
        f.write(img.tobytes().translate(SHIFT5))
    return True


def stale(png_path, out_path):
    """True if out is missing or older than the source PNG."""
    try:
        return os.path.getmtime(png_path) > os.path.getmtime(out_path)
    except FileNotFoundError:
        return True


def batch(roots):
    converted = skipped = failed = 0
    for root in roots:
        for dirpath, _, names in os.walk(root):
            for name in names:
                if not name.endswith('.png'):
                    continue
                png = os.path.join(dirpath, name)
                ref = png[:-4] + '.rgb555'
                if not stale(png, ref):
                    skipped += 1
                    continue
                if convert(png, ref):
                    converted += 1
                else:
                    failed += 1
    print(f'  RGB555 refs: {converted} generated, {skipped} up-to-date'
          + (f', {failed} failed' if failed else ''))
    return 1 if failed else 0


def main():
    args = sys.argv[1:]
    # Single-file form: exactly two args and the second is an output path.
    if len(args) == 2 and args[1].endswith('.rgb555'):
        sys.exit(0 if convert(args[0], args[1]) else 1)
    roots = args or ['test-suites']
    sys.exit(batch(roots))


if __name__ == '__main__':
    main()
