# gbtrace-gopher2600

A gbtrace adapter for the [Gopher2600](https://github.com/JetSetIlly/Gopher2600)
emulator (VCS / Atari 2600 family). Drives Gopher2600 headlessly one CPU
instruction at a time and writes a **native `.gbtrace`** file via the gbtrace
C FFI (`libgbtrace_ffi.a`).

Built to give the Atari 2600 test suite an independent, headless reference
oracle — Gopher2600's Go core is designed for exactly this (its REGRESS mode
already records per-CPU-step state and supports NTSC/PAL/PAL60/SECAM), so no
GUI or screenshotting is involved.

## Build

```
make
```

On first build this clones Gopher2600 into `./src` (gitignored) and builds the
gbtrace FFI static lib (`../../target/release/libgbtrace_ffi.a`) if missing.
The tracked adapter source is `main.go`; the Makefile copies it into the cloned
module as a `cmd` so it resolves Gopher2600's packages locally, and links the
FFI via `CGO_CFLAGS`/`CGO_LDFLAGS`.

## Usage

```
./gbtrace-gopher2600 -rom test.bin -out trace.gbtrace -spec NTSC -frames 30
```

- `-rom`    path to a `.bin`/`.a26` cartridge
- `-out`    output `.gbtrace` (native format)
- `-spec`   `NTSC` | `PAL` | `PAL60` | `SECAM` | `AUTO`
- `-frames` frame cap (also stops early once RAM `$80` — the RESULT byte — is non-zero)

Inspect with the gbtrace CLI:

```
gbtrace info  trace.gbtrace
gbtrace query trace.gbtrace -w "pc=0xfc01"
gbtrace query trace.gbtrace -w "timer changes"
```

## Captured fields (MVP / Tier 1 profile)

One entry per instruction (`trigger: instruction`):

| Field | Source |
|---|---|
| `pc a x y s p` | 6507 register file (`vcs.CPU.*`) |
| `line clock` | TIA beam position (`vcs.TV.GetCoords()`) |
| `timer` | RIOT INTIM (`$0284`) |
| `port_a port_b` | SWCHA `$0280` / SWCHB `$0282` |
| `result code observed expected` | test-suite RESULT convention RAM (`$80–$83`) |

The field set + memory addresses are currently **hardcoded** for Tier 1.

## Frame snapshots

With `-frame` (default on) the adapter attaches a `PixelRenderer` to the TV,
captures the final rendered frame (160-wide visible area), and embeds it in the
trace as an `IndexedFrame` (palette + indices) via the FFI's
`gbtrace_writer_mark_frame_indexed`. View it with `gbtrace render trace.gbtrace
-o out/` — the GOLD/visual modality, same path as the Game Boy framebuffer.

## TODO

- Drive the field set from a gbtrace **profile TOML** (like the docboy adapter)
  instead of hardcoding, so higher tiers add fields without a rebuild.
- Emit a frame snapshot **per frame** (not just the last one) for full GOLD diffs.
- Add collision registers (`CXxx`) and full TIA register state once the gbtrace
  VCS family gains those first-class fields (contribute upstream as Tier 3 needs).
- Per-color-clock (`tcycle`) trigger option for the racing-the-beam edge cases.
