# gbtrace-stella

A gbtrace adapter for the [Stella](https://github.com/stella-emu/stella)
emulator (VCS / Atari 2600 family). Drives Stella's emucore headlessly one CPU
instruction at a time and writes a **native `.gbtrace`** (via the gbtrace C FFI)
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
3. The wrapper (`gbtrace-stella.cxx`) creates the console via `StellaLIBRETRO`,
   then steps `m6502().execute(1)` per instruction, reading registers, the TIA
   beam (`scanlines()`/`clocksThisLine()`), and RIOT RAM. It provides the few
   libretro-frontend glue symbols the core references (a real filesystem `stat`
   VFS so the ROM is read from disk; no-op logger/message hooks).

## Build

```
make
```

Clones Stella into `./src` (gitignored), applies the patch, builds the core
(~5-10 min, first time), and links `./gbtrace-stella`. Needs the gbtrace FFI
static lib (built automatically if missing).

## Usage

```
./gbtrace-stella -rom test.bin -out trace.gbtrace -spec NTSC -frames 30
```

- `-spec` `NTSC` | `PAL` | `PAL60` | `SECAM` | `AUTO`
- stops early once RAM `$80` (RESULT) holds a terminal verdict (`$A5`/`$5A`).

## Captured fields (Tier 1 profile)

`pc a x y s p line clock result code observed expected`, one entry per
instruction. No `timer`/`port_a`/`port_b` yet (reading those safely without the
debugger subsystem is a TODO), and no frame snapshot yet.

## TODO / known gaps

- **Trace alignment for diffing:** the Stella and Gopher2600 traces are not yet
  instruction-aligned (slightly different reset/startup lengths), so a naive
  `gbtrace diff` shows offset-driven noise. Needs a common start anchor.
- **`p` normalization:** the 6502 status byte's bit 4 (B) / bit 5 (unused)
  convention differs between emulators; normalize before diffing.
- Add `timer`/ports (via a side-effect-free RIOT read) and frame snapshots.
