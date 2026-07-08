# gbtrace-mame

A gbtrace adapter for [MAME](https://www.mamedev.org/)'s Atari 2600 driver
(`a2600`), used as a third, **independent-lineage** behavioural oracle for the
VCS test suite alongside Stella and Gopher2600. (Stella and Gopher2600 both
descend from shared TIA-core work; MAME's driver is its own, so it's a genuine
third vote — not just a third copy.)

## Approach (differs from the Stella/Gopher2600 adapters)

MAME is far too large to link the way the Stella and Gopher2600 adapters embed
their emulators. But the output must still be a **native `.gbtrace` file written
through the gbtrace FFI** — no JSONL. MAME is driven for per-instruction state
via its scripting/debugger, and that state is fed to the FFI to write native.

Two candidate mechanisms (finalise against installed MAME):

1. **Lua ⇄ FFI binding (one-step, preferred).** Build a small Lua C module
   (`gbtrace_lua.so`) that wraps `libgbtrace_ffi.a` (writer_new / set_u8/u16 /
   finish_entry / mark_frame_indexed / close). A MAME `-autoboot_script`
   `require`s it, steps the CPU (`devices[":maincpu"].debug:step()`), reads
   `state[...]` + `spaces["program"]:read_u8()`, and writes native gbtrace
   directly. Risk: MAME's Lua sandbox may restrict `require` of C modules.

2. **Debugger trace-log → FFI converter (two-step fallback).** A `-debugscript`
   emits a per-instruction text log (`tracelog "%04X %02X ...",pc,a,...`); a
   small C/Go converter (linking the FFI, like the other adapters) parses it and
   writes native gbtrace. The intermediate is MAME's own trace format, not gbtrace
   JSONL.

Either way the output is native gbtrace. Fields match the other adapters:
`pc a x y s p line clock` + the RESULT convention RAM bytes (`$80–$83`).

## How it works (implemented)

`gbtrace-mame` (Go + cgo/FFI) launches `mame a2600 -debug -debugger gdbstub`
headless and drives it over the **GDB remote protocol**: after the handshake
(`qSupported` + fetch `target.xml` — MAME's gdbstub only answers `g` afterwards),
it loops `s` (step one instruction) → `g` (read the 6507 register file) →
`m80,4` (read the RESULT bytes), writing one native `.gbtrace` entry per
instruction via the FFI. Stops at the RESULT verdict. Fields:
`pc a x y s p result code observed expected`.

Verified: **100% agreement with the Stella and Gopher2600 adapters** on the
instruction stream (synced to the harness anchor), and matching verdicts.

### Notes / limitations
- **No frame snapshot** — gdbstub exposes registers + memory only. A frame would
  need a parallel Lua screen capture (for GOLD tests later); not implemented.
- **No `line`/`clock`** — the TIA beam isn't exposed over gdbstub.
- **Console switches are best-effort.** An autoboot Lua sets the `:SWB` switch
  fields (`-swchb`), but under gdbstub the switch test reads SWCHB within the
  first frame, before MAME's input re-poll applies the setting reliably — so the
  switch test (t06) is not dependable on MAME. Input tests like this are rarely
  covered by ROM suites anyway; the register/verdict oracle is the point.
- **Slow** — ~19s/ROM (3 GDB round-trips per instruction). Fine for cross-checks.

## Speeding it up (WIP: ~70× faster path proven)

The slowness is entirely gdbstub's per-instruction round-trips. MAME's own
debugger `trace` command logs every instruction at *full emulation speed*, and
it can be driven headlessly over gdbstub via the GDB **`monitor`** (`qRcmd`)
command — no per-instruction round-trips. Proven so far:

- `monitor trace <file>,maincpu,,{tracelog "...",pc,a,...}` installs a
  full-speed per-instruction trace with a custom register format.
- `monitor wpset 0x80,1,w,{(wpdata==0xa5)||(wpdata==0x5a)}` + `c` runs to the
  RESULT verdict and stops cleanly (**270ms** vs 19s); `m80,4` then reads the
  correct RESULT. `monitor trace off` flushes the file.

**Open issue:** the `tracelog` register format lands inconsistently — it worked
in a plain `trace`+`c` run (lines like `R F001 00 80`), but produced only the
default disassembly in the watchpoint flow. Adding a memory read (`b@0x80`) to
the format also broke it. Needs nailing down (escaping / arg count / flow), then
a small log→gbtrace FFI converter (filter the `R` lines; RESULT from the
watchpoint) replaces the per-instruction gdbstub stepping. This is the intended
fast adapter; the stepping version above stays as the correctness reference.
- **`read-tap` was a dead end** — reading `cpu.state[...]` inside a memory-tap
  callback core-dumps MAME; gdbstub is the working path.

## Notes

- MAME's `a2600` cartridge slot autodetects the bankswitch type from the `.bin`.
- TV standard via the `a2600`/`a2600p` machine or a slot option (NTSC vs PAL).
- The gbtrace family is `vcs`; emit a JSONL header with `"family":"vcs"` and the
  same field set, then diff against Stella/Gopher2600 via `scripts/compare.sh`.
