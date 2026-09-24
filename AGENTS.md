## Project overview

morepork captures detailed execution traces from emulators and provides tooling to
inspect and compare them. The repository hosts:

- A Rust core library (`crates/morepork`) that defines the trace format, profile parsing,
  query engine, disassembler, snapshots, and downsampling.
- The system registry (`crates/morepork-systems`): each system's column vocabulary,
  taken from missingno's state schemas.
- The CLI (`crates/morepork-cli`, binary `morepork`) and C FFI bindings
  (`crates/morepork-ffi`), both on the library plus the registry.
- Per-emulator **adapters** (`adapters/<emu>/`) that drive each emulator and emit traces by
  linking against the Rust core via the C FFI (or, for the Rust adapters, the core crate
  directly).

morepork began as **gbtrace**, a Game Boy trace tool. The Game Boy systems and adapters
are still fully supported, but the in-repo test-ROM suites (`test-suites/`), the
trace-generation pipeline (`scripts/`, Makefile trace targets, `traces.yml`), and the
WASM web viewer (`web/`, `crates/morepork-wasm`, `deploy.yml`) were removed once they
stopped being maintained — they live on in git history (see the README's Origins
section). morepork no longer runs full-suite trace generation itself; how outside
projects drive the adapters is up to them.

## Common commands

```bash
make cli        # build target/release/morepork
make ffi        # build target/release/libmorepork_ffi.a + header
make adapters   # build the adapter binaries in adapters/<emu>/morepork-<emu>
make clean      # rm -rf build/
```

### Rust workspace

- `cargo build --release -p morepork-cli` — same as `make cli`.
- `cargo test --workspace` — library tests (`crates/morepork/tests/`), registry tests
  (`crates/morepork-systems/tests/`, including every adapter's column list), FFI tests.
- `cargo check` — type check across the workspace.
- The workspace is defined in the root `Cargo.toml` (library, registry, CLI, FFI, and the
  `mame`/`openmsx` adapters). `adapters/missingno` is **excluded** from the workspace and
  builds independently (its `missingno-vcs` dependency is a path dep on a sibling
  `~/Projects/missingno` checkout).
- The registry depends on missingno's crates by git (`branch = "main"`); iterating against
  an unpushed missingno checkout takes a local `[patch."https://github.com/ajoneil/missingno"]`
  block pointing at it, never committed. The committed
  `[patch."https://github.com/ajoneil/morepork"]` block makes missingno's cores use this
  checkout's library, so the registry's column defs and the writer are one crate.

### Running the CLI

`morepork` (the binary) provides `info`, `convert`, `query`, `frames`, `render`,
`downsample`, `diff`. Examples:

```bash
target/release/morepork info trace.morepork
target/release/morepork query trace.morepork -w "pc=0x0150" --context 2
target/release/morepork diff a.morepork b.morepork --fields pc,a,f --sync pc
target/release/morepork convert trace.morepork.jsonl -o trace.morepork
```

## Architecture

**Multi-system design:** the architecture, constraints, and order of work live in
`docs/multi-system.md` — read it before touching `profile.rs`, `header.rs`, `query.rs`,
the `system/` module, or `morepork-systems`. Long-running efforts keep their live status
in `receipts/<effort>/ROADMAP.md` (receipts/ is gitignored — never reference specific
receipt paths from committed files).

### Trace format (`crates/morepork/src/format/`)

- Native binary format (`.morepork`): magic `MPRK`. Layout is
  `[header zstd-JSON] [snapshots/chunks interleaved] [footer]`. Each chunk holds up to
  `DEFAULT_CHUNK_SIZE` (65536) entries with field groups compressed independently using
  Arrow IPC + zstd. See the doc comment at the top of `format/mod.rs` for the layout.
- Snapshot records (tag `SNAP`) carry bulk state at specific entry indices — `frame`
  (raw GB pixel bytes, or `snapshot::IndexedFrame` for `indexed8` systems) and `memory`
  (`snapshot::MemoryRegion`).
- JSONL format (`.morepork.jsonl`): first line is a header with `_header: true`, every
  subsequent line is one `TraceEntry` keyed by field name. Convenient for emulators that
  cannot link against the Rust core; can be converted via `morepork convert`.
- Trace-file backward compatibility is **not** required — regenerate traces freely
  after format changes.

### Systems (`crates/morepork-systems/`, `crates/morepork/src/system/`)

The machine-state vocabulary is missingno's: a column is a field of the system's
`SystemStateSchema`, a shared trace observation (`missingno_trace::TRACE_OBSERVATIONS`),
or one of the system's capture-bridge observations. The registry (`morepork-systems`)
knows `dmg`, `cgb`, `vcs`, `sg1000`, `colecovision` and `msx1` (the last composed from
the Z80 and TI VDP chip crates) and types columns by name (`column_defs`, `describe`);
an unknown column is an error. The library holds no field tables: a trace's system comes
from its header (`system`, `isa`, `entry_addrs`, `field_defs`), plus morepork's own ISA
flag tables (`system/isa.rs`) and query phrases keyed by system id
(`system/phrases.rs`). GB frame reconstruction (`system/gb/framebuffer.rs`, `vram.rs`)
keys on `pix_format`.

### Profiles (`crates/morepork/src/profile.rs`)

A trace profile (TOML) declares the target `system` (absent ⇒ `dmg`), the trigger
granularity (`instruction` / `cycle` / `mcycle` / `tcycle`), and which subsystem-layer
fields are captured:

```toml
[profile]
name = "smoke"
system = "sg1000"
trigger = "instruction"

[fields]
cpu = "registers"
vdp = ["registers", "internal"]

[fields.memory]
test_result = "C000"      # arbitrary memory watch fields
```

The library parses the TOML; `morepork_systems::load_profile` expands the selections over
the system's vocabulary (a layer is a schema tier — `registers` observable, `internal`
boundary — or an observation's own layer). Types come from the schema.

### Query engine (`query.rs`, `comparison.rs`)

The `--where` flag in `morepork query` accepts conditions like `pc=0x0150`, `a changes`,
`flag z becomes set`, `pc&0xFF00=0xC000`, plus per-system semantic phrases (`lcd on`,
`vblank starts`). `morepork diff` uses a sync condition (default `auto`) to align two
traces before reporting per-field divergence and match percentages.

### Adapters

Each adapter is a stand-alone CLI named `morepork-<emu>` placed at
`adapters/<emu>/morepork-<emu>`. All adapters expose the same frozen surface
(downstream tooling relies on it):

```
--rom <path> --profile <profile.toml> --output <trace.morepork>
[--frames N] [--stop-when ADDR=VAL] [--stop-opcode HEX] [--reference <ref>] [--model M]
```

Current adapters and their systems:

- **gambatte** (C++, FFI) — GB/CGB
- **sameboy** (C++, FFI) — GB/CGB (T-cycle via checked-in `sameboy-tcycle.patch`)
- **docboy** (C++, FFI) — GB + CGB (two binaries, compile-time split)
- **mgba** (C, FFI) — GB
- **gateboy** (C++, FFI) — GB (gate-level)
- **bgb** (C, FFI) — GB/CGB (experimental, Wine)
- **missingno** (Rust, workspace-excluded) — GB/CGB (`morepork-missingno`), VCS
  (`morepork-missingno-vcs`; needs the sibling missingno checkout), SG-1000
  (`morepork-missingno-sg1000`) and ColecoVision (`morepork-missingno-colecovision`; BIOS
  via `-bios` or `COLECOVISION_BIOS`)
- **stella** (C++, FFI) — VCS
- **gopher2600** (Go/cgo, FFI) — VCS
- **mame** (Rust, core crate, workspace member) — VCS + SG-1000/SC-3000/ColecoVision
- **openmsx** (Rust, core crate, workspace member) — MSX1
- **ares** (C++, FFI) — ColecoVision/SG-1000/SC-3000/MSX1
- **gearsystem** (C++, FFI) — SG-1000
- **gearcoleco** (C++, FFI) — ColecoVision

Every adapter writes its system's schema names (the TI VDP adapters take `-system
colecovision`, with `coleco` accepted); `crates/morepork-systems/tests/registry.rs` lists
each adapter's columns and must follow any rename. C/C++/Go adapters link
`libmorepork_ffi.a` (header at `crates/morepork-ffi/morepork.h`).
Per-adapter build details live in `adapters/<emu>/Makefile` and may invoke nested
cmake/scons builds against vendored emulator sources (which are gitignored). Some
adapters carry checked-in patches against their upstream (`sameboy-tcycle.patch`,
`stella-trace-api.patch`, `gearsystem-trace-api.patch`). `adapters/genpalette.py`
generates the canonical VCS NTSC/PAL/SECAM palette tables shared by the VCS adapters.

### CI (`.github/workflows/`)

- `build.yml` — builds the CLI + FFI library, runs `cargo test --workspace`, then builds
  the GB adapter matrix (gambatte, sameboy, missingno, docboy) against freshly cloned
  upstreams, and uploads artifacts. The other adapters are not built in CI.
