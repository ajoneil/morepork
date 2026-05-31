# SameBoy Adapter

Produces `.gbtrace` files using [SameBoy](https://github.com/LIJI32/SameBoy) as a
library. SameBoy is used essentially unmodified — a single small patch
(`sameboy-tcycle.patch`, ~40 lines across `Core/gb.h`, `Core/gb.c`,
`Core/timing.c`) adds a public `GB_set_tcycle_callback` hook so traces can be
captured at **T-cycle granularity**, not just per instruction.

## How it works

The adapter captures the full CPU/IO state via `GB_get_registers` and
`GB_safe_read_memory`, at a granularity chosen by the profile's `trigger`:

- **`trigger = "instruction"`** — uses SameBoy's stock `GB_set_execution_callback`,
  which fires once per CPU instruction with the opcode address.
- **`trigger = "tcycle"`** — uses the patched `GB_set_tcycle_callback`, which fires
  once per emulated T-cycle. This exposes sub-instruction state: the PPU advancing
  dot-by-dot (LY/STAT/mode), the DIV/timer counters, and the live (mid-instruction)
  program counter. `pc` then carries the live PC while `op_addr` carries the stable
  address of the in-flight instruction.

The patch only *splits* SameBoy's existing per-M-cycle `GB_advance_cycles` into
single-T-cycle steps when the callback is installed — every timing subsystem in
SameBoy already accumulates a cycle budget and carries its remainder, so stepping
`1+1+1+1` is identical to stepping `4` at once (verified: DIV increments every 256
T-cycles, LY every 456). With no callback installed the code path is byte-identical
to upstream.

The adapter:

1. Loads a ROM via `libsameboy`
2. Registers the instruction or T-cycle callback per the profile trigger
3. Runs the emulator for N frames (rendering disabled unless pixels are needed)
4. Produces a `.gbtrace` file matching the spec

## Prerequisites & build

```bash
git clone https://github.com/LIJI32/SameBoy.git
make lib    # applies sameboy-tcycle.patch + builds libsameboy.a/.so
make        # builds the adapter
```

`make lib` applies the patch once (idempotently, via a stamp file inside
`SameBoy/`) and builds only the static + shared libraries — it skips SameBoy's
public-header generation step, which needs the `cppp` tool and is unused here (the
adapter includes `Core/gb.h` directly with `GB_INTERNAL`). The adapter links
`libsameboy.so`, so run it with `SameBoy/build/lib` on `LD_LIBRARY_PATH` (the trace
scripts handle this).

## Usage

```bash
./gbtrace-sameboy --rom cpu_instrs.gb --profile ../../profiles/cpu_basic.toml --output trace.gbtrace --frames 3000
```

Options:
- `--rom <path>` — ROM file (required)
- `--profile <path>` — Capture profile TOML file (required)
- `--output <path>` — output file (default: stdout)
- `--frames <n>` — stop after N frames (default: 3000)
- `--model dmg|cgb` — hardware model (default: dmg)

## Cycle counting

SameBoy internally counts in 8MHz ticks (`GB_advance_cycles` receives 4MHz
T-cycles; one M-cycle = 4 T-cycles). In T-cycle mode the per-T-cycle callback
fires once per CPU T-cycle regardless of speed: at normal speed one CPU T-cycle is
one PPU dot, and under CGB double speed the CPU runs two T-cycles per dot — so the
PPU still advances correctly while the CPU is sampled at its own (doubled) rate.

## Differences from gambatte adapter

- **IME field**: SameBoy exposes the IME register, so the `ime` field works correctly (gambatte currently hardcodes `false`)
- **Rendering**: Uses `GB_set_rendering_disabled` and `GB_set_turbo_mode` for faster trace generation
