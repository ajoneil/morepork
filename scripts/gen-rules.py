#!/usr/bin/env python3
"""Generate Makefile rules for trace targets.

Outputs Make rules to stdout, one per ROM × emulator × model combination.

Model dimension
---------------
Each suite declares which models (`dmg`, `cgb`) its ROMs run under:

  - root_models: models the ROMs directly under the suite dir run under.
  - cgb_subdir:  if set, ROMs under `<suite>/cgb/` additionally run under cgb
                 (these are the curated CGB-only ROM sets from missingno-gbc).
  - gambatte:    special — each root ROM's models come from its filename tags
                 (`_dmg08` → dmg, `_cgb04c` → cgb, `_blank` → both).

Trace output files are named `<test>_<emu>_<model>_<status>.gbtrace`. The model
is passed to the trace script via the MODEL env var. docboy selects DMG/CGB at
compile time, so (docboy, cgb) resolves to the separate `gbtrace-docboy-cgb`
binary; all other adapters take `--model` at runtime.
"""
import os
import sys


def sanitize(name):
    """Replace non-alphanumeric chars with underscores for Make target names."""
    return ''.join(c if c.isalnum() or c in '-_' else '_' for c in name)


def find_roms(rom_dir, exclude_dirs):
    roms = []
    for dirpath, dirnames, filenames in os.walk(rom_dir):
        dirnames[:] = [d for d in dirnames if os.path.join(dirpath, d) not in exclude_dirs]
        for f in sorted(filenames):
            if f.endswith('.gb') or f.endswith('.gbc'):
                roms.append(os.path.join(dirpath, f))
    return sorted(roms)


def binary_for(emu, model):
    # docboy is compiled per-model; everything else takes --model at runtime.
    if emu == 'docboy' and model == 'cgb':
        return 'adapters/docboy/gbtrace-docboy-cgb'
    return f'adapters/{emu}/gbtrace-{emu}'


def rel_name(rom, name_base):
    rel = os.path.relpath(rom, name_base)
    if rel.endswith('.gbc'):
        return rel[:-4]
    if rel.endswith('.gb'):
        return rel[:-3]
    return rel


def emit(stamps, rom, name_base, model, emu, profile, trace_dir, script, max_frames=None):
    safe = sanitize(rel_name(rom, name_base))
    binary = binary_for(emu, model)
    stamp = f'{trace_dir}/.stamp_{safe}_{model}_{emu}'
    stamps.append(stamp)
    # max_frames is the 6th positional arg, honoured by the screenshot-suite
    # script and harmlessly ignored by the others.
    extra = f' {max_frames}' if max_frames else ''
    print(f"{stamp}: {binary} {profile} | $(CLI)")
    print(f"\t@mkdir -p {trace_dir}")
    print(f"\t@MODEL={model} bash {script} {binary} '{rom}' {profile} {trace_dir} {name_base}{extra} || true")
    print(f"\t@touch $@")
    print()


MODELS = ('dmg', 'cgb')


def model_has_ref(rom, model, cgb_only):
    """Does a reference image for `model` exist next to `rom`?

    Mirrors `scripts/ref-lib.sh`. Used to make screenshot suites *ref-driven*:
    a ROM runs under a model only when an appropriate reference exists — so a
    DMG-only test (no CGB reference) isn't run on CGB (where it would render
    differently and spuriously fail). Checks the committed `.png` sources.
    For a CGB-only suite the unsuffixed base name *is* the CGB reference.
    """
    d = os.path.dirname(rom)
    stem = os.path.basename(rom)
    stem = stem[:-4] if stem.endswith('.gbc') else stem[:-3]
    if model == 'dmg':
        cands = [f'{stem}_dmg08', f'{stem}-dmg', stem]
    else:
        cands = [f'{stem}_cgb04c', f'{stem}_cgb_c', f'{stem}-cgb']
        if cgb_only:
            cands.append(stem)
    return any(os.path.exists(os.path.join(d, f'{c}.png')) for c in cands)


def gambatte_models(rom):
    """Models a root Gambatte ROM runs under, from its filename tags / refs."""
    stem = os.path.basename(rom)
    stem = stem[:-4] if stem.endswith('.gbc') else stem[:-3]
    if '_blank' in stem:
        return ['dmg', 'cgb']
    d = os.path.dirname(rom)
    has_dmg = '_dmg08' in stem or os.path.exists(os.path.join(d, f'{stem}_dmg08.png'))
    has_cgb = '_cgb04c' in stem or os.path.exists(os.path.join(d, f'{stem}_cgb04c.png'))
    models = []
    if has_dmg:
        models.append('dmg')
    if has_cgb:
        models.append('cgb')
    return models or ['dmg']  # untagged (screenshot) ROMs default to dmg


def gen_suite(stamps, rom_dir, profile, trace_dir, emus, script, policy,
              name_base=None, exclude_dirs=None):
    name_base = name_base or rom_dir
    exclude = set(exclude_dirs or [])
    cgb_dir = os.path.join(rom_dir, 'cgb')

    max_frames = policy.get('max_frames')
    ref_driven = policy.get('ref_driven')
    cgb_only = policy.get('root_models') == ['cgb']

    # Root ROMs (the suite's cgb/ subdir is handled separately below).
    for rom in find_roms(rom_dir, exclude | {cgb_dir}):
        if policy.get('gambatte'):
            models = gambatte_models(rom)
        elif ref_driven:
            # Screenshot suite: run a model only when its reference exists.
            models = [m for m in policy['root_models'] if model_has_ref(rom, m, cgb_only)]
        else:
            models = policy['root_models']
        for model in models:
            for emu in emus:
                emit(stamps, rom, name_base, model, emu, profile, trace_dir, script, max_frames)

    # CGB-only ROM set (curated, from missingno-gbc) — names relative to cgb/.
    if policy.get('cgb_subdir') and os.path.isdir(cgb_dir):
        for rom in find_roms(cgb_dir, set()):
            if ref_driven and not model_has_ref(rom, 'cgb', cgb_only=True):
                continue
            for emu in emus:
                emit(stamps, rom, cgb_dir, 'cgb', emu, profile, trace_dir, script, max_frames)


# suite var name -> (rom_dir, profile, trace_dir make-var, script, policy, kwargs)
SUITES = [
    ('GBMICROTEST_STAMPS', 'test-suites/gbmicrotest', 'test-suites/gbmicrotest/profile.toml',
     '$(GBMICROTEST_TRACE_DIR)', 'scripts/trace-gbmicrotest.sh', {'root_models': ['dmg']}, {}),
    ('BLARGG_STAMPS', 'test-suites/blargg', 'test-suites/blargg/profile.toml',
     '$(BLARGG_TRACE_DIR)', 'scripts/trace-blargg.sh',
     {'root_models': ['dmg'], 'cgb_subdir': True}, {'exclude_dirs': {'test-suites/blargg/dmg_sound'}}),
    ('MOONEYE_STAMPS', 'test-suites/mooneye', 'test-suites/mooneye/profile.toml',
     '$(MOONEYE_TRACE_DIR)', 'scripts/trace-mooneye.sh', {'root_models': ['dmg']}, {}),
    ('GAMBATTE_TESTS_STAMPS', 'test-suites/gambatte', 'test-suites/gambatte/profile.toml',
     '$(GAMBATTE_TESTS_TRACE_DIR)', 'scripts/trace-gambatte-tests.sh',
     {'gambatte': True, 'cgb_subdir': True}, {}),
    # mealybug: shared root ROMs run under BOTH models (DMG ref `<stem>.png`,
    # CGB ref `<stem>_cgb_c.png`); the cgb/ subdir holds 7 CGB-only variants.
    ('MEALYBUG_TEAROOM_STAMPS', 'test-suites/mealybug-tearoom', 'test-suites/mealybug-tearoom/profile.toml',
     '$(MEALYBUG_TEAROOM_TRACE_DIR)', 'scripts/trace-mealybug-tearoom.sh',
     {'root_models': ['dmg', 'cgb'], 'cgb_subdir': True, 'ref_driven': True}, {}),
    ('AGE_STAMPS', 'test-suites/age', 'test-suites/age/profile.toml',
     '$(AGE_TRACE_DIR)', 'scripts/trace-age.sh',
     {'root_models': ['dmg'], 'cgb_subdir': True}, {}),
    ('MOONEYE_WILBERTPOL_STAMPS', 'test-suites/mooneye-wilbertpol', 'test-suites/mooneye-wilbertpol/profile.toml',
     '$(MOONEYE_WILBERTPOL_TRACE_DIR)', 'scripts/trace-mooneye-wilbertpol.sh',
     {'root_models': ['dmg'], 'cgb_subdir': True}, {}),
    ('SAMESUITE_STAMPS', 'test-suites/samesuite', 'test-suites/samesuite/profile.toml',
     '$(SAMESUITE_TRACE_DIR)', 'scripts/trace-samesuite.sh',
     {'root_models': ['dmg'], 'cgb_subdir': True}, {}),
    ('SCRIBBLTESTS_STAMPS', 'test-suites/scribbltests', 'test-suites/scribbltests/profile.toml',
     '$(SCRIBBLTESTS_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['dmg', 'cgb'], 'ref_driven': True}, {}),
    ('BULLY_STAMPS', 'test-suites/bully', 'test-suites/bully/profile.toml',
     '$(BULLY_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['dmg', 'cgb'], 'ref_driven': True}, {}),
    ('MBC3_TESTER_STAMPS', 'test-suites/mbc3-tester', 'test-suites/mbc3-tester/profile.toml',
     '$(MBC3_TESTER_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['dmg', 'cgb'], 'ref_driven': True}, {}),
    ('STRIKETHROUGH_STAMPS', 'test-suites/strikethrough', 'test-suites/strikethrough/profile.toml',
     '$(STRIKETHROUGH_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['dmg', 'cgb'], 'ref_driven': True}, {}),
    ('TURTLE_TESTS_STAMPS', 'test-suites/turtle-tests', 'test-suites/turtle-tests/profile.toml',
     '$(TURTLE_TESTS_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['dmg', 'cgb'], 'ref_driven': True}, {}),
    # dmg-acid2: shared ROM, separate DMG (`-dmg`) and CGB (`-cgb`) references.
    # acid tests settle within a few frames; cap so a non-matching emulator
    # (e.g. greyscale-only on a colour test) doesn't run to a huge trace.
    ('DMG_ACID2_STAMPS', 'test-suites/dmg-acid2', 'test-suites/dmg-acid2/profile.toml',
     '$(DMG_ACID2_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['dmg', 'cgb'], 'max_frames': 300, 'ref_driven': True}, {}),
    # CGB-only suites
    ('CGB_ACID2_STAMPS', 'test-suites/cgb-acid2', 'test-suites/cgb-acid2/profile.toml',
     '$(CGB_ACID2_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['cgb'], 'max_frames': 300, 'ref_driven': True}, {}),
    ('CGB_ACID_HELL_STAMPS', 'test-suites/cgb-acid-hell', 'test-suites/cgb-acid-hell/profile.toml',
     '$(CGB_ACID_HELL_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['cgb'], 'max_frames': 300, 'ref_driven': True}, {}),
    ('RTC3TEST_STAMPS', 'test-suites/rtc3test', 'test-suites/rtc3test/profile.toml',
     '$(RTC3TEST_TRACE_DIR)', 'scripts/trace-screenshot-suite.sh', {'root_models': ['cgb'], 'max_frames': 60, 'ref_driven': True}, {}),
]


def main():
    emus = sys.argv[1].split(',') if len(sys.argv) > 1 else ['gambatte', 'sameboy', 'missingno', 'docboy']

    var_stamps = {}
    for var, rom_dir, profile, trace_dir, script, policy, kwargs in SUITES:
        stamps = []
        gen_suite(stamps, rom_dir, profile, trace_dir, emus, script, policy, **kwargs)
        var_stamps.setdefault(var, []).extend(stamps)

    # dmg_sound: blargg sub-suite, DMG-only, separate profile/script. Appends to BLARGG_STAMPS.
    dmg_sound = []
    gen_suite(dmg_sound, 'test-suites/blargg/dmg_sound', 'test-suites/blargg/dmg_sound/profile.toml',
              '$(BLARGG_TRACE_DIR)', emus, 'scripts/trace-dmg-sound.sh',
              {'root_models': ['dmg']}, name_base='test-suites/blargg')
    var_stamps['BLARGG_STAMPS'].extend(dmg_sound)

    for var, _, _, _, _, _, _ in SUITES:
        print(f"{var} := {' '.join(var_stamps.get(var, []))}")
        print()


if __name__ == '__main__':
    main()
