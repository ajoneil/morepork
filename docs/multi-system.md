# Going multi-system

gbtrace was built for one machine. Its sibling project missingno has already grown
past that — the frontend drives Game Boy, Atari 2600, Master System, and NES cores
through family-agnostic seams (`docs/adding-a-system.md` in the missingno repo,
https://github.com/ajoneil/missingno). This document is the equivalent map for the
trace side: how the format, core library, CLI, FFI, and web viewer become
system-agnostic, what stays exactly as it is, and the order of work. Trust the
seams named here, but verify signatures against the source before building on
them.

## Two different axes (same as missingno)

- **A variant within a family, sharing silicon** — DMG↔CGB. Modelled by the
  header's free-form `model` string (`"DMG-B"`, `"CGB-E"`), the
  `systems.{dmg,cgb}` manifest dimension, `SYSTEMS=` build sharding. A variant
  changes field *values* and display encoding (`pix_format`), never the field
  catalogue's shape.
- **A new family entirely** (NES, SMS, VCS, …) — a new field catalogue, ISA,
  frame geometry, flag semantics, clock vocabulary. This axis is what the
  header's `family` field and the core's family registry carry.

## What is generic (do not "fix" these)

The data plane is system-agnostic and must stay that way:

- `entry.rs` — `TraceEntry` is a `BTreeMap<String, serde_json::Value>`; setters
  are name-agnostic.
- `store.rs` / `reader.rs` / `downsample.rs` — the `TraceStore` trait is
  columns-by-name; JSONL reading infers fields.
- `format/` — the container (chunks, Arrow IPC field groups, zstd, footer,
  dictionary encoding) is field-name-driven.
- `comparison.rs` — the diff engine operates on arbitrary columns; family
  specifics enter only through alignment hints.
- `gbtrace-ffi` — the C writer API is column-index + field-name driven (the
  adapter builds the header JSON itself and pushes typed values by column).
  No register structs, no screen dimensions.
- Web shell — trace-table, trace-diff-table, chart, timeline, query, selector,
  file-loader are column-generic and driven by header metadata.

## The architecture

Two principles, in tension-free layers:

### 1. The format is fully self-describing

Readers need **zero family knowledge** for info/query/diff/table/chart. The
header carries, beyond the ordered `fields` list:

- `family: String` — `"gb"`, `"nes"`, … Absent (traces written before the
  field existed) means `"gb"`.
- `field_defs` — ordered typed declarations `{ name, type, subsystem, layer,
  nullable, dictionary }`; the source of truth for resolution. The static GB
  catalogue remains only as the fallback for old traces.
- `field_groups` — the chunk storage layout actually used for this file (each
  group is one Arrow IPC block). Legacy traces re-derive it from the
  wire-frozen `derive_groups`.
- `instruction_addr_field` — names the column that means "address of the
  current instruction" (the `op_addr`-then-`pc` preference is the legacy
  fallback).
- `snapshot_kinds` — tag-indexed kind names. `frame` and `memory` are
  format-level kinds the viewer depends on; system state uses namespaced
  names (`gb.cpu`, …).

`GbtraceWriter::create` enriches the header itself, so every producer (FFI
adapters, missingno, `convert`) writes self-describing traces without changes
on their side.

`pix_format` values: `shade2` (DMG greyscale pix stream), `rgb555` (CGB colour
pix stream), and `indexed8` — the family-agnostic form, one palette index per
pixel, with per-frame dimensions, the frame-end palette, and the display pixel
aspect carried in each `frame` snapshot payload (`snapshot::IndexedFrame`,
mirroring missingno's `IndexedFrame`; VCS frame height is emergent, SMS CRAM
is mutable, so both ride per-frame). GB traces keep their raw frame payloads.

### 2. Family knowledge lives in one registry in the core

`crates/gbtrace/src/family/` — a static registry (like missingno's `FAMILIES`
table), one module per family, GB at `src/family/gb/`. A `Family` provides:

- **Default field catalogue** (`subsystems`) — validates profiles and types
  legacy traces. The GB catalogue lives in `family/gb/catalogue.rs`.
- **Flag vocabulary** (`flags`) — name → (field, bit), driving the query
  engine's `flag …` conditions and the viewer's flag rendering (exported
  through wasm `flagDefs()`).
- **Semantic query phrases** (`exact_phrases`, `numbered_phrases`) — named
  conditions (`"lcd on"`, `"ppu enters mode N"`) that desugar to the generic
  `Condition` variants; `parse_condition` takes the family whose vocabulary
  it parses.
- **Disassembler** (`disassemble`) — `fn(&[u8], u16) -> (String, u8)`. SM83
  lives in `family/gb/disasm.rs`.
- **Diff alignment hint** (`entry_addrs`) — the address every trace of the
  system reaches at program entry plus the entry's second instruction (GB:
  cartridge entry `0x0100`/`0x0101`); families without a fixed entry use the
  generic first-common-address alignment.
- **Frame reconstruction** — the GB `pix`/`ly` replay and VRAM/tile logic
  (`family/gb/framebuffer.rs`, `family/gb/vram.rs`) are family capabilities,
  not format features. The generic path is `frame` snapshots. Call sites gate
  on the family id; promote to a function-table hook when a second family
  implements reconstruction.
- **Typed snapshot payloads** — `family/gb/snapshot.rs` defines the `gb.*`
  payload layouts (missingno's `from_snapshot` constructors restore console
  state from them). `memory` and `frame` payloads are family-agnostic
  (`src/snapshot.rs`).

What stays *out* of the registry: everything in the "generic" list. The
registry is consulted only for disassembly, rendering, semantic query sugar,
catalogue defaults/validation, and diff alignment hints.

The `profile.rs` free functions (`lookup_field`, `field_group`, …) consult the
GB catalogue only. They exist for traces whose headers predate `field_defs` —
every such trace is a GB trace, so this fallback is permanently GB and
deliberately not family-parameterised.

### Profiles

```toml
[profile]
name = "nes-smoke"
family = "nes"          # absent = "gb"
trigger = "cycle"

[fields]
cpu = ["pc", "a", "x", "y", "s", "p"]
```

`[fields]` keys are validated against the family catalogue (unknown subsystem
keys are an error), resolved in catalogue order. `[fields.memory]` and
`[fields.extensions]` are family-independent.

## Compatibility constraints (hard requirements)

1. **The existing trace corpus (~20 GB on Spaces) must stay readable without
   regeneration.** Traces without `family`/`field_defs` imply `family = "gb"`
   and resolve through the GB catalogue. This fallback is permanent, cheap,
   and tested by the roundtrip tests. The legacy `derive_groups` in
   `format/read.rs` is wire-frozen — pre-`field_groups` traces reconstruct
   their chunk layout from it.
2. **missingno tracks gbtrace's git HEAD with no pin**
   (`missingno-gb/Cargo.toml: gbtrace = { git = ... }`). Breaking the Rust API
   on main breaks missingno's `--features gbtrace` build immediately. Land
   breaking changes together with the matching missingno update, and push
   gbtrace first, then missingno immediately after. The consumer surface:
   - `gbtrace::format::write::GbtraceWriter` — `create(path, &header,
     &groups)`, `set_u8/u16/bool/str/null(col, v)`, `finish_entry`,
     `mark_frame`, `write_snapshot(SnapshotType, &[u8])`, `finish`.
   - `gbtrace::format::read::derive_groups_pub`, `gbtrace::format::SnapshotType`.
   - `gbtrace::header::{TraceHeader (all fields), ExtensionField, PixFormat}`.
   - `gbtrace::profile::{FieldType, field_type, field_nullable}`.
   - `gbtrace::{BootRom, Profile (.trigger/.fields/.extensions/.memory/.name),
     Trigger, Error::Profile}`.
   - `gbtrace::family::gb::snapshot::{CpuSnapshot, PpuSnapshot, ApuSnapshot,
     TimerSnapshot, DmaSnapshot, SerialSnapshot, MbcSnapshot}` and
     `gbtrace::snapshot::{MemoryRegion, build_memory_payload}` — the
     save-state restore path.
3. **Adapter CLI surface is frozen** (`--rom/--profile/--output/--frames/
   --stop-when/--stop-opcode/--reference/--model`): `gen-rules.py` and the
   trace scripts hard-code it. Additions must not disturb existing
   invocations.

## What each future family brings

| | NES | VCS | SMS |
|---|---|---|---|
| CPU state | 6502: `a,x,y,s,p,pc` (+rdy) | same 6502 core (6507) | Z80: full main+shadow set, `ix,iy,sp,pc,wz,i,r,im,iff1/2` |
| Stepping | `step_cycle` / `step_instruction` / `step_frame` | same + own core-side `Debugger` | `Cpu::step` returns T-states |
| Frame | 256×240 fixed, 6-bit colour indices | `Vec<[u8; VISIBLE_CLOCKS]>`, **emergent height**, TIA indices | 256×192, CRAM-indexed + per-frame 32-byte CRAM |
| Disassembler | ✓ `missingno_6502::disasm` | ✓ (same) | ✗ none exists |
| Trace hooks in missingno | none yet | none yet | none yet (its `bus_trace()` is test-only) |

Recommended second family: **NES** — fixed geometry, cycle-granular stepping,
public PPU/APU/OAM state, an existing disassembler to check a gbtrace 6502
decoder against, and it exercises every seam (catalogue, flags, disasm,
indexed frames) without VCS's emergent-height wrinkle. VCS third, as the
stress test of the per-frame-dimensions model. SMS waits for a Z80
disassembler or ships with hex-dump disassembly.

On the missingno side the only trace seam is
`SystemDebugger::capture_trace(&mut self, path) -> Option<ScreenDisplay>`
(GB overrides it; others inherit the `None` default) plus the
`missingno trace` CLI subcommand — a per-family tracer there is missingno
work, but the family contract in this document is what it implements.

## Web viewer notes

Field display is metadata-driven: the wasm store exposes `fieldDefs()` and
`flagDefs()`, and `web/src/lib/format.js` keeps its GB tables only as
defaults for legacy traces. Two deliberate GB-shaped remainders:

- The ASM column anchors at the visible `pc` column. Every surveyed family
  names its program counter `pc`, while `instruction_addr_field` is typically
  the hidden `op_addr` — anchoring there would remove the column from the
  default GB view.
- `trace-query.js`'s `SEMANTIC_CONDITIONS` chip list is GB-worded but gated
  on field presence; it follows the family vocabulary once the registry
  exports labelled semantic phrases.

GB-specific panels (sprite table, APU, FIFO, VRAM, pixel display) are gated
on field presence, which non-GB traces won't have; a per-family panel
registry keyed on `header.family` becomes worthwhile when a second family
ships panels.

## Naming

The rename ("emutrace"?) is mechanical but wide: crate names, `gbtrace.h` /
`gbtrace_*` C symbols, the `GBTR` magic, binary name, repo name, CI, Pages
URL, Spaces paths, missingno's git dependency URL, and the `.gbtrace`
extension. Nothing in the architecture depends on it, so: build everything
under the current names and rename in one commit once a name is chosen.
Format note for that day: keep accepting `GBTR` magic forever; a new magic
(if any) only for traces that require `field_defs`.

## Order of work

Each step leaves the GB pipeline green (`cargo test -p gbtrace`, spot-check
`make traces-<suite>`):

1. **Self-describing format** — header metadata + reader preference +
   catalogue fallback (see "The format is fully self-describing").
2. **Family registry** — `src/family/`, GB moved behind it, `family` header
   field, `Indexed8`/`IndexedFrame`.
3. **Second family (NES)** — `family/nes/` catalogue, flags, 6502
   disassembler (verify against `missingno_6502::disasm`); then a NES
   profile, a missingno-side tracer, and a captured trace that opens in the
   CLI and web viewer.
4. **Web panels for non-GB families** — per-family panel registry, labelled
   semantic-phrase export for the query chips.
5. **VCS** (frame-model stress test), then SMS behind a Z80 disassembler.
6. **Rename** — blocked on the name decision; deliberately last.

The manifest's `systems.{dmg,cgb}` map and the test picker stay GB-only until
a non-GB test suite exists; they need a family level then
(`scripts/manifest.py`, `web/src/components/test-picker.js`).
