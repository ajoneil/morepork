#!/usr/bin/env python3
"""Generate manifest.json for a test suite directory.

Usage: manifest.py <trace-dir> <rom-dir>

Scans trace-dir for `<test>_<emu>_<system>_<status>.gbtrace` files and rom-dir
for `*.gb`/`*.gbc` ROMs, then writes manifest.json. DMG and CGB are modelled as
separate but related systems; each test entry carries a per-system coverage map:

    { "name": ..., "rom": ..., "systems": { "dmg": {emu: status}, "cgb": {...} } }

ROMs under a suite's `cgb/` subdir are CGB-only sets; their test name is taken
relative to `cgb/` (matching gen-rules) so it isn't prefixed with `cgb__`.
"""
import json
import os
import sys

EMULATORS = ['missingno', 'docboy', 'gambatte', 'sameboy']
SYSTEMS = ['dmg', 'cgb']
STATUSES = ['pass', 'fail']


def rom_test_name(path, rom_dir):
    """Test name for a ROM: path relative to rom_dir (or to rom_dir/cgb for
    CGB-only ROMs), with subdirs flattened by `__` and the extension dropped."""
    cgb_dir = os.path.join(rom_dir, 'cgb')
    base = cgb_dir if path.startswith(cgb_dir + os.sep) else rom_dir
    rel = os.path.relpath(path, base)
    stem = rel[:-4] if rel.endswith('.gbc') else rel[:-3]
    return stem.replace(os.sep, '__')


def generate_manifest(trace_dir, rom_dir):
    # ROMs → test name + relative path (for the viewer to fetch).
    roms = {}
    for dirpath, _, filenames in sorted(os.walk(rom_dir)):
        for fname in sorted(filenames):
            if not (fname.endswith('.gb') or fname.endswith('.gbc')):
                continue
            path = os.path.join(dirpath, fname)
            roms.setdefault(rom_test_name(path, rom_dir), os.path.relpath(path, rom_dir))

    # Traces → per-test, per-system, per-emulator status.
    traces = {}
    for dirpath, _, filenames in sorted(os.walk(trace_dir)):
        for fname in sorted(filenames):
            if not fname.endswith('.gbtrace'):
                continue
            base = fname[:-len('.gbtrace')]
            parts = base.rsplit('_', 3)  # test, emu, system, status
            if len(parts) != 4:
                continue
            test_name, emu, system, status = parts
            if emu not in EMULATORS or system not in SYSTEMS or status not in STATUSES:
                continue
            traces.setdefault(test_name, {}).setdefault(system, {})[emu] = status

    # Build manifest (union of ROMs and any trace-only test names).
    names = sorted(set(roms) | set(traces))
    manifest = [
        {'name': name, 'rom': roms.get(name), 'systems': traces.get(name, {})}
        for name in names
    ]

    os.makedirs(trace_dir, exist_ok=True)  # robust to a suite with no traces yet
    out_path = os.path.join(trace_dir, 'manifest.json')
    with open(out_path, 'w') as f:
        json.dump(manifest, f)

    total = sum(len(s) for e in manifest for s in e['systems'].values())
    print(f'  {len(manifest)} tests, {total} traces -> {out_path}')


if __name__ == '__main__':
    if len(sys.argv) != 3:
        print(f'Usage: {sys.argv[0]} <trace-dir> <rom-dir>', file=sys.stderr)
        sys.exit(1)
    generate_manifest(sys.argv[1], sys.argv[2])
