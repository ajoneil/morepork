# Going multi-system

morepork was built for one machine and has since grown past it, the same way its
sibling project missingno did — that frontend drives Game Boy, Atari 2600, Master
System, and NES cores through system-agnostic seams (`docs/adding-a-system.md` in
the missingno repo, https://github.com/ajoneil/missingno). This document is the
equivalent map for the trace side: how the format, core library, CLI, and FFI
stay system-agnostic, where system knowledge lives (the registry
currently knows the `dmg`, `cgb`, `vcs`, `sg1000`, `colecovision`, and `msx1`
systems, on the `sm83`, `6502`, and `z80` ISAs), and what adding a
system involves. Trust the
seams named here, but verify signatures against the source before building on
them.

## Two orthogonal axes

The header carries two small strings that, with the existing `pix_format`,
replace the old monolithic `family` tag:

- **`isa`** (`"sm83"`, `"6502"`, `"z80"`) — the instruction-set architecture.
  Names the decoder (disassembly is driven by the shared `missingno_core`
  instruction-set vocabulary, keyed on this id) and the flag vocabulary.
  Systems that share silicon share an ISA: the Game Boy's DMG and CGB are both
  `sm83`; the NES's 2A03 and the VCS's 6507 are both `6502`.
- **`system`** (`"dmg"`, `"cgb"`, `"vcs"`, `"sg1000"`, `"colecovision"`,
  `"msx1"`) — the machine identity: missingno's state-schema id. Selects the
  column vocabulary (at write time, through the registry) and the semantic
  query phrases. Distinct from `model` (the free-form hardware revision,
  `"DMG-B"`/`"CGB-C"`).

Frame reconstruction keys off `pix_format` (`shade2`/`rgb555` → the GB pixel
replay, `indexed8` → the system-agnostic indexed-frame path), not the system.

**DMG↔CGB** are two `system`s on the shared `sm83` ISA + GB render: same
disassembler, flags, phrases, and reconstruction; the CGB adds a `cgb`
subsystem (colour palettes, KEY1 double-speed, VRAM/WRAM banks, HDMA).
**A new machine** (NES, SMS, …) adds a `system` (and an `isa` if its CPU is
new): a state schema on the missingno side, its registry entry here, frame
geometry, and — when the ISA is new — decode table and flag semantics.

## The vocabulary is missingno's schema

A column's name, type, subsystem and layer come from missingno: each
system's `SystemStateSchema` (its fields; the observable tier is the
`registers` layer, the boundary tier `internal`), the trace observations
every corpus-driven producer carries (`missingno_trace::TRACE_OBSERVATIONS`:
`cycles`, `result`, `code`, `observed`, `expected`, `ram_write_addr`,
`ram_write_data`), and the system's own capture-bridge observations (the
Game Boy's `op_addr`/`pix`/`pix_x`, the VCS's `cycles`/`line`). morepork keeps
no second spelling of any system's state. Every adapter emits those names,
decomposing composite bytes the schema splits (the TI VDP status byte is
`vdp_frame_flag`, `vdp_fifth_sprite_flag`, `vdp_coincidence_flag` and
`vdp_fifth_sprite_index`); a column outside the vocabulary fails when the
writer is created.

## What is generic (do not "fix" these)

The data plane is system-agnostic and must stay that way:

- `entry.rs` — `TraceEntry` is a `BTreeMap<String, serde_json::Value>`; setters
  are name-agnostic.
- `store.rs` / `reader.rs` / `downsample.rs` — the `TraceStore` trait is
  columns-by-name; JSONL reading infers fields.
- `format/` — the container (chunks, Arrow IPC field groups, zstd, footer,
  dictionary encoding) is field-name-driven.
- `comparison.rs` — the diff engine operates on arbitrary columns; system
  specifics enter only through alignment hints.
- `morepork-ffi` — the C writer API is column-index + field-name driven (the
  adapter builds the header JSON itself and pushes typed values by column;
  the registry types the columns). No register structs, no screen
  dimensions.

## The architecture

Two principles, in tension-free layers:

### 1. The format is fully self-describing

Readers need **zero system-specific knowledge** for info/query/diff/table work, and
self-description is *required*: the reader rejects a header without field
metadata or with an ISA morepork has no flag table for. The header carries,
beyond the ordered `fields` list:

- `system: String` — the schema id.
- `isa: String` — `"sm83"`, `"6502"`, `"z80"`.
- `entry_addrs` — the diff-alignment hint: the program-entry address every
  trace of the system reaches and the address after it (GB:
  `0x0100`/`0x0101`); absent for systems without a fixed entry.
- `field_defs` — ordered typed declarations `{ name, type, subsystem, layer,
  nullable, dictionary }`; the source of truth for resolution.
- `field_groups` — the chunk storage layout actually used for this file (each
  group is one Arrow IPC block).
- `instruction_addr_field` — names the column that means "address of the
  current instruction" (the writer prefers `op_addr`, which is stable across
  an instruction's T-cycles, over `pc`).
- `snapshot_kinds` — tag-indexed kind names. `frame` (tag 0) and `memory`
  (tag 1) are the only kinds anything writes or decodes; the writer stamps
  those two, and a reader resolves any higher tag by name from this list.

The producer states all of this: missingno's bridges from the schema, every
other producer through the registry (`morepork_systems::describe`, which the
FFI's `morepork_writer_new` calls). `MoreporkWriter::create` only completes
what is not vocabulary — defs for declared `extension_fields`, the
instruction-address column when none is named, the `frame`/`memory`
snapshot-kind names, and storage groups from the defs — and rejects a column
nothing declares.

`pix_format` values: `shade2` (DMG greyscale pix stream), `rgb555` (CGB colour
pix stream), and `indexed8` — the system-agnostic form, one palette index per
pixel, with per-frame dimensions, the frame-end palette, and the display pixel
aspect carried in each `frame` snapshot payload (`snapshot::IndexedFrame`,
mirroring missingno's `IndexedFrame`; VCS frame height is emergent, SMS CRAM
is mutable, so both ride per-frame). GB traces keep their raw frame payloads.

### 2. The library reads systems from headers; the registry sits above it

missingno's cores depend on the `morepork` library for the writer, so the
crate that depends on those cores cannot be the library. The workspace is
therefore split:

- `crates/morepork` — the library: format, writer, reader, query,
  comparison, snapshots, render, disassembly. Vocabulary-free:
  `TraceHeader::system_def()` builds a `SystemView` from the header (id, ISA,
  entry hint) plus morepork's own trace-level vocabulary — the flag bits of
  each ISA (`system/isa.rs`, driving `flag …` queries) and each system's
  semantic query phrases (`system/phrases.rs`, keyed by system id:
  `"lcd on"`, `"ppu enters mode N"`, `"vblank starts"`). GB frame
  reconstruction (`system/gb/framebuffer.rs`, `vram.rs`) keys on
  `pix_format`.
- `crates/morepork-systems` — the registry: depends on `missingno-core`,
  `missingno-trace` and the system crates (by git, `morepork` feature).
  `schema(id)`, `trace_observations()`, `bridge_observations(id)`,
  `column_defs(id, names)` (an unknown name is an error naming the system
  and column), `describe(&mut header)`, and profile expansion. The MSX1 has
  no missingno system; its vocabulary is composed from the Z80 and TI VDP
  chip crates' own fields.
- `crates/morepork-cli` (the `morepork` binary) and `crates/morepork-ffi` —
  both on the library plus the registry. `convert` types a legacy JSONL
  header through the registry; a legacy header with no `system` is a Game Boy
  (`dmg`) trace there, and nowhere else.

Snapshot payloads — `frame` and `memory` are the only snapshot kinds, both
system-agnostic (`src/snapshot.rs`: `IndexedFrame`, `MemoryRegion`).

### Profiles

```toml
[profile]
name = "sg1000-smoke"
system = "sg1000"       # absent = "dmg"
trigger = "instruction"

[fields]
cpu = "registers"
```

The library parses a profile; the registry expands it
(`morepork_systems::load_profile`, which `morepork_profile_load` calls).
`[fields]` keys are the system's subsystems (unknown keys are an error); a
layer is a schema tier (`registers`, `internal`) or an observation's own
layer (`timing`, `output`). `[fields.memory]` and `[fields.extensions]` are
system-independent. (The trace header's `system` is set by the adapter from
`--model`, so a shared `dmg` profile can be captured as `cgb`.)

## Compatibility constraints

1. **Trace-file backward compatibility is NOT required.** There is no
   external userbase and captured traces are regenerable, so the format may
   evolve freely; prefer deleting legacy fallbacks over freezing them. The
   reader rejects headers without field metadata with a clear "regenerate"
   error. After a format change, regenerate any captured trace sets.
2. **missingno tracks morepork's git HEAD with no pin**
   (`missingno-{gb,gbc,nes,vcs}/Cargo.toml: morepork = { git = ... }`).
   Breaking the Rust API on main breaks missingno's `--features morepork`
   build immediately. Land breaking changes together with the matching
   missingno update, and push morepork first, then missingno immediately
   after. The consumer surface:
   - `morepork::format::write::MoreporkWriter` — `create(path, &header,
     &groups)` (usually `&[]`: the writer groups by the header's field
     defs), `set_u8/u16/bool/str/null(col, v)`, `finish_entry`,
     `mark_frame`, `write_snapshot(tag, &[u8])`, `finish`.
   - `morepork::format::{TAG_FRAME, TAG_MEMORY}` — the only snapshot tags.
   - `morepork::header::{TraceHeader (all fields), HeaderFieldDef,
     ExtensionField, PixFormat}`.
   - `morepork::profile::FieldType`.
   - `morepork::{BootRom, Profile (.trigger/.name, parsed unexpanded),
     Trigger, Error::Profile}`.
   - `morepork::snapshot::{IndexedFrame, MemoryRegion, build_memory_payload}` —
     the system-agnostic frame/memory payloads (the NES and VCS tracers write
     `IndexedFrame` frames). Console state is restored on the missingno side
     from its own `missingno_core` state vocabulary, not from morepork-side
     `gb.*` snapshot structs (those were removed).
3. **Adapter CLI surface is frozen** (`--rom/--profile/--output/--frames/
   --stop-when/--stop-opcode/--reference/--model`): downstream tooling
   hard-codes it. Additions must not disturb existing invocations.

## What each system brings

| | NES | VCS | SMS |
|---|---|---|---|
| CPU state | 6502: `a,x,y,s,p,pc` (+rdy) | same 6502 core (6507) | Z80: full main+shadow set, `ix,iy,sp,pc,wz,i,r,im,iff1/2` |
| Stepping | `step_cycle` / `step_instruction` / `step_frame` | same + own core-side `Debugger` | `Cpu::step` returns T-states |
| Frame | 256×240 fixed, 6-bit colour indices | `Vec<[u8; VISIBLE_CLOCKS]>`, **emergent height**, TIA indices | 256×192, CRAM-indexed + per-frame 32-byte CRAM |
| Disassembler | ✓ shared 6502 core + iNES map | ✓ shared core + 6507 cartridge map | ✗ none exists |
| Trace hooks in missingno | ✓ `missingno-nes/src/trace.rs` | ✓ `missingno-vcs/src/trace.rs` | none (its `bus_trace()` is test-only) |

NES went second because it exercises every seam (fields, flags, disasm,
indexed frames) with fixed geometry; VCS third as the stress test of the
per-frame-dimensions model (its emergent height is why `IndexedFrame`
carries dimensions per frame). SMS waits for a Z80 disassembler or ships
with hex-dump disassembly.

**SG-1000** (`sg1000`, the first `z80` system) entered ahead of SMS as the
host for TI VDP (TMS9918A) verification work: Z80 + TMS9918A + SN76489, fixed
256×192 `indexed8` frames, cartridge at 0x0000 with no BIOS. The SC-3000
(same envelope plus a keyboard) captures as `sg1000` with the machine in
`model`. **ColecoVision** (`colecovision`) and **MSX1** (`msx1`) carry the
identical Z80 + TMS9918A pair — different machine wrappers (BIOS ownership,
cart windows, test-RAM/port addresses) that live in the adapters. missingno
has SG-1000 and ColecoVision cores (driven here by
`morepork-missingno-sg1000` and `morepork-missingno-colecovision`) but no
MSX1; disassembly shares SMS's blocker (the missingno Z80 crate's
`InstructionSet` impl).

On the missingno side each family's tracer is a `trace` module in its core
crate behind a `morepork` feature (a `Tracer` with per-field emitters,
`mark_frame` writing self-contained `IndexedFrame` payloads), routed from
the `missingno trace` CLI subcommand by ROM detection — a per-family tracer
there is missingno work, but the family contract in this document is what
it implements.

## Naming

The rename ("emutrace"?) is mechanical but wide: crate names, `morepork.h` /
`morepork_*` C symbols, the `MPRK` magic, binary name, repo name, CI,
missingno's git dependency URL, and the `.morepork` extension. Nothing in
the architecture depends on it, so: build everything
under the current names and rename in one commit once a name is chosen.
Format note for that day: with back-compat waived, the magic can simply
change with the name; regenerate traces after.

## Order of work

The generalization landed in this order, each step leaving
`cargo test -p morepork` green: self-describing format → system registry
(GB moved behind it, `Indexed8`/`IndexedFrame`) → NES (fields, flags,
6502 disassembler, missingno tracer) → VCS (the emergent-height stress
test, on the shared `mos6502` core) → SG-1000 (the `z80` ISA and shared
TI VDP) → ColecoVision and MSX1 as thin siblings → the vocabulary
handed to missingno's schemas (the registry crate; the NES, whose missingno
core has no schema yet, left the registry). The pre-captured GB trace corpus, the in-repo GB test suites and
trace-generation pipeline, and the web viewer were retired along the way —
they remain in git history. What remains:

1. **Z80 disassembly** — the `z80` ISA and flag vocabulary are registered,
   but decode needs an `InstructionSet` impl in missingno's Z80
   crate (its decode table exists; only the display mapping is missing).
   Until it lands, `z80` traces disassemble as hex. Unblocks SMS too.
2. **SMS** — beyond the disassembler, its missingno core has no trace
   hooks yet, and its VDP (11 registers, CRAM, counter ports) gets its
   own schema distinct from the TI VDP's.
3. **Rename** — blocked on the name decision; deliberately last.
