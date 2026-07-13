# morepork-stella

A morepork adapter for the [Stella](https://github.com/stella-emu/stella)
emulator (VCS / Atari 2600 family). Drives Stella's emucore headlessly one CPU
instruction at a time and writes a **native `.morepork`** (via the morepork C FFI)
with per-instruction 6507 registers, TIA beam position, and the test-suite
RESULT convention RAM bytes.

Stella is the reference-accuracy 2600 emulator, so this is the gold-standard
oracle for the VCS test suite — run alongside the Gopher2600 adapter and diff.

## How it works

Stella isn't an embeddable library, so:

1. We build Stella's **libretro core** objects (`src/os/libretro`), which provide
   a headless `OSystem` with no SDL/window.
2. A small **patch** (`stella-trace-api.patch`) exposes the otherwise-private
   6507 register file (the debugger, which the libretro build omits, is normally
   its only friend).
3. The wrapper (`morepork-stella.cxx`) creates the console via `StellaLIBRETRO`,
   then steps `m6502().execute(1)` per instruction, reading registers, the TIA
   beam (`scanlines()`/`clocksThisLine()`), and RIOT RAM. It provides the few
   libretro-frontend glue symbols the core references (a real filesystem `stat`
   VFS so the ROM is read from disk; no-op logger/message hooks).

## Build

```
make
```

Clones Stella into `./src` (gitignored), applies the patch, builds the core
(~5-10 min, first time), and links `./morepork-stella`. Needs the morepork FFI
static lib (built automatically if missing).

## Usage

```
./morepork-stella -rom test.bin -out trace.morepork -spec NTSC -frames 30
```

- `-spec` `NTSC` | `PAL` | `PAL60` | `SECAM` | `AUTO`
- `-swchb` console switches (bit3=colour, bit6=P0 difficulty-A, bit7=P1 difficulty-A); default `0x48`
- stops early once RAM `$80` (RESULT) holds a terminal verdict (`$A5`/`$5A`).

## Captured fields (Tier 1 profile)

`pc a x y s p line clock result code observed expected`, one entry per
instruction. No `timer`/`port_a`/`port_b` yet (reading those safely without the
debugger subsystem is a TODO), and no frame snapshot yet.

## Trace alignment

The Stella and Gopher2600 per-instruction traces align **100% on the instruction
stream** (`pc a x y s p`) and **99.9% on `clock`**, so `morepork diff` between them
is clean. Getting there required matching two conventions (both handled in the
Gopher2600 adapter):

- **One entry per retired instruction.** A WSYNC halt (including WSYNC strobed via
  `CLEAN_START`'s stack writes to TIA mirrors) stalls the CPU for many cycles;
  Stella's `execute(1)` absorbs the halt into one instruction, so the Gopher2600
  adapter skips halt cycles (RDY low) to match.
- **Canonical `clock` origin** 0..227 with 0 = start of HBLANK.

Residual differences are genuine, not noise:
- The first few entries differ in `a/x/y` and the `D` flag — power-on state
  (Stella randomises, Gopher2600 zeros), cleared within a few instructions by
  `CLEAN_START`/`CLD`.
- `line` diverges on compute-only SELF ROMs (no real VSYNC → emulator-specific
  frame detection). It aligns for ROMs that drive proper video.

## TODO / known gaps

- Add `timer`/ports (via a side-effect-free RIOT read) and frame snapshots.
