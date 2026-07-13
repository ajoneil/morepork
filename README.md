# morepork

**Capture and explore detailed execution traces from emulators — across the Game Boy, Game Boy Color, and Atari VCS.**

morepork records what happens inside an emulated system — every instruction, register value, CPU flag, and IO/video state change — and provides tools to explore, query, and compare that data. Use it to understand how the hardware works, debug emulator behaviour, investigate how software uses specific features, or verify accuracy against a reference implementation.

Originally built for the Game Boy, morepork now spans multiple systems that share one binary trace format and one toolchain.

**[Try the web viewer](https://ajoneil.github.io/morepork/)** — browse pre-captured traces from hundreds of test ROMs across multiple emulators, or upload your own.

## Supported systems

Each system is a **family**: it brings its own field catalogue, disassembler, flag vocabulary, and query phrases. The binary format, CLI, and web viewer are system-agnostic — the trace header records which family it belongs to.

| System | CPU | Captured state |
| --- | --- | --- |
| **Game Boy** (DMG) | Sharp SM83 | CPU registers & flags, PPU (LCDC/STAT/LY…), timer, interrupts, memory watches |
| **Game Boy Color** (CGB) | Sharp SM83 | as Game Boy, plus colour PPU state and double-speed timing |
| **Atari VCS / 2600** | MOS 6507 | 6507 registers & flags, TIA beam position (line/clock), RIOT timer and ports |

The Game Boy and Game Boy Color share the `gb` family (they are modelled as separate but related *systems*); the Atari VCS is the `vcs` family, with NTSC, PAL, and SECAM as models within it.

## Features

- **Detailed execution traces** — capture every register value, CPU flag, and IO/video state change, per-instruction or per-T-cycle
- **Per-CPU disassembly** in the web viewer — SM83 for the Game Boy, MOS 6502/6507 for the Atari VCS — shown inline with register state
- **Field value charts** with drag-to-zoom to visualise how registers and IO change over time
- **Side-by-side trace comparison** with per-field and per-flag diff highlighting, even across different emulators of the same system
- **Pre-captured reference traces** from multiple independent emulators across 600+ test ROMs
- **CLI query engine** — search traces by condition (e.g. `pc=0150`, `flag z becomes set`, `a changes`)
- **Open, system-agnostic trace format** — any emulator, on any supported system, can produce compatible traces

## Trace format

morepork uses a compact binary format for efficient storage and querying. There are two ways to produce traces:

**Native format** — use the `morepork` Rust library (or its C FFI bindings) to write `.morepork` files directly.

**JSONL format** — for quick integration, emit `.morepork.jsonl` files (one JSON object per line). Both the CLI and web viewer can work with JSONL files directly, but you can convert them for smaller file sizes and faster loading:

```bash
morepork convert trace.morepork.jsonl -o trace.morepork
```

### JSONL format

The first line is a header describing the trace. It declares the `family` (which system the trace belongs to) and the `fields` captured:

```json
{"_header":true,"format_version":"0.1.0","family":"gb","emulator":"my-emulator","emulator_version":"1.0","rom_sha256":"...","model":"DMG-B","boot_rom":"skip","profile":"gbmicrotest","fields":["pc","sp","a","f","b","c","d","e","h","l","lcdc","stat","ly"],"trigger":"instruction"}
```

Each subsequent line is a trace entry with the fields listed in the header:

```json
{"pc":256,"sp":65534,"a":1,"f":176,"b":0,"c":19,"d":0,"e":216,"h":1,"l":77,"lcdc":145,"stat":128,"ly":153}
```

The `fields` array defines what's captured, and the valid field names depend on the `family`. Common Game Boy configurations:

**CPU only:**
```json
"fields": ["pc", "sp", "a", "f", "b", "c", "d", "e", "h", "l"]
```

**CPU + PPU + interrupts + timer:**
```json
"fields": ["pc", "sp", "a", "f", "b", "c", "d", "e", "h", "l", "lcdc", "stat", "ly", "lyc", "scy", "scx", "if_", "ie", "ime", "div", "tima", "tma", "tac"]
```

Other systems declare their own family and field set — an Atari VCS trace (`"family":"vcs"`) exposes the 6507 registers, the TIA beam position (`line`, `clock`), and the RIOT timer and ports.

Values should be numeric (not hex strings). 8-bit fields use 0-255, 16-bit fields (pc, sp) use 0-65535, booleans (ime) use `true`/`false`.

The `trigger` field indicates granularity: `"instruction"` for one entry per instruction, `"mcycle"` for one entry per M-cycle, or `"tcycle"` for one entry per T-cycle. Traces at different granularities can be compared — the viewer automatically downsamples higher-granularity traces to match.

Capture profiles define which fields to record, but you don't need to provide all of them — include whatever level of detail your emulator can supply.

## Web viewer

The [web viewer](https://ajoneil.github.io/morepork/) provides:

- **Test ROM browser** — pre-captured traces from hundreds of test ROMs across multiple emulators, with pass/fail indicators and a DMG/CGB system toggle
- **Trace viewer** — virtual-scrolling table with inline disassembly, field value charts, and search/filter
- **Comparison mode** — side-by-side diff with synced scrolling, per-field and per-flag highlighting, and match percentage statistics
- **Drag-to-zoom charts** — visualise any field over the trace timeline, with dual-trace overlay in comparison mode
- **Upload your own traces** — drop a `.morepork` or `.morepork.jsonl` (or gzipped `.morepork.jsonl.gz`) file to view or compare

The hosted browser currently carries the Game Boy / Game Boy Color test suites; Atari VCS traces can be captured with the VCS adapters and viewed by upload.

### Included traces

The pre-captured Game Boy / Game Boy Color traces come from several emulators:

- **[Missingno](https://github.com/ajoneil/missingno)** — the author's emulator, with full support for all morepork trace features (per-T-cycle capture, all subsystem fields).
- Traces from several well-regarded community emulators — [SameBoy](https://github.com/LIJI32/SameBoy), [gambatte](https://github.com/pokemon-speedrunning/gambatte-speedrun), and [docboy](https://github.com/Docheinstein/docboy) — are also included.

For the Atari VCS, morepork ships adapters for three independent-lineage emulators — [Gopher2600](https://github.com/JetSetIlly/Gopher2600), [MAME](https://www.mamedev.org/)'s `a2600` driver, and [Stella](https://github.com/stella-emu/stella) — so the same trace can be cross-checked against emulators that don't share a codebase.

## CLI

The `morepork` tool provides offline trace inspection:

```bash
# Show trace metadata
morepork info trace.morepork

# Find entries matching a condition
morepork query trace.morepork -w "pc=0x0150"
morepork query trace.morepork -w "a changes"

# Compare two traces (e.g. two emulators of the same system)
morepork diff missingno.morepork gambatte.morepork --fields pc,a,f

# Convert JSONL to native format
morepork convert trace.morepork.jsonl -o trace.morepork
```

Run `morepork --help` for a full list of commands.

## Building

morepork is a Rust workspace. The crates are not yet published to crates.io — install from git:

```bash
# CLI
cargo install --git https://github.com/ajoneil/morepork --features cli morepork

# Local web viewer (requires wasm-pack)
make serve
```
