## Project overview

gbtrace captures detailed execution traces from Game Boy emulators and provides tooling
(CLI, WASM-powered web viewer) to inspect and compare them. The repository hosts:

- A Rust core library (`crates/gbtrace`) that defines the trace format, profile schema,
  query engine, disassembler, snapshots, and downsampling.
- WASM (`crates/gbtrace-wasm`) and C FFI (`crates/gbtrace-ffi`) bindings on top of that core.
- Per-emulator **adapters** (`adapters/<emu>/`) that drive each emulator and emit traces by
  linking against the Rust core via the C FFI (or, for `missingno`, native Rust).
- Pre-captured **test-suite traces** rendered into a static web app (`web/`).

## Common commands

All build orchestration is in the top-level `Makefile`. From the repo root:

```bash
make cli                        # build target/release/gbtrace
make wasm                       # build WASM module into web/pkg/
make ffi                        # build target/release/libgbtrace_ffi.a + header
make adapters                   # build every adapter binary in adapters/<emu>/gbtrace-<emu>
make traces                     # generate every trace (run with -j$(nproc))
make traces-gbmicrotest         # one suite (similar targets exist per suite)
make traces EMUS=gambatte,mgba  # restrict which emulators run
make site                       # assemble deployable site in build/site/
make serve                      # local dev server (uses scripts/devserver.py)
make clean                      # rm -rf build/
```

The Makefile dynamically generates per-(suite × ROM × emulator) rules via
`scripts/gen-rules.py`, written to `build/rules.mk` and `-include`d. To regenerate after
changing the script or ROM lists, `make clean` (or just delete `build/rules.mk`).

### Rust workspace

- `cargo build --release --features cli` — same as `make cli`.
- `cargo test -p gbtrace` — run library tests (integration + roundtrip in
  `crates/gbtrace/tests/`).
- `cargo check` — type check across the workspace.
- The workspace is defined in the root `Cargo.toml`. `adapters/missingno` is **excluded**
  from the workspace and builds independently with its own `cargo build --release`
  inside its `Makefile` (because it is a vendored emulator with its own dep tree).

### Running the CLI

`gbtrace` (the binary) provides `info`, `convert`, `query`, `frames`, `render`, `diff`.
Examples:

```bash
target/release/gbtrace info trace.gbtrace
target/release/gbtrace query trace.gbtrace -w "pc=0x0150" --context 2
target/release/gbtrace diff a.gbtrace b.gbtrace --fields pc,a,f --sync pc
target/release/gbtrace convert trace.gbtrace.jsonl -o trace.gbtrace
```

## Architecture

### Trace format (`crates/gbtrace/src/format/`)

- Native binary format (`.gbtrace`): magic `GBTR`, current `VERSION = 2`. Layout is
  `[header zstd-JSON] [snapshots/chunks interleaved] [footer]`. Each chunk holds up to
  `DEFAULT_CHUNK_SIZE` (65536) entries with field groups compressed independently using
  Arrow IPC + zstd. See the doc comment at the top of `format/mod.rs` for the layout.
- Snapshot records (tag `SNAP`) carry typed bulk state at specific entry indices —
  used for frame boundaries (with optional 160×144 screen pixels) and initial state
  (memory, APU).
- JSONL format (`.gbtrace.jsonl`): first line is a header with `_header: true`, every
  subsequent line is one `TraceEntry` keyed by field name. Convenient for emulators that
  cannot link against the Rust core; can be converted via `gbtrace convert`.

### Profiles (`crates/gbtrace/src/profile.rs`)

A trace profile (TOML) declares the trigger granularity (`instruction` / `mcycle` /
`tcycle`) and which subsystem-layer fields are captured:

```toml
[profile]
name = "gbmicrotest"
trigger = "tcycle"

[fields]
cpu = ["pc", "sp", "a", "f", "b", "c", "d", "e", "h", "l"]
ppu = ["lcdc", "stat", "ly", "lyc", "scy", "scx"]

[fields.memory]
test_pass = "FF82"        # arbitrary memory watch fields
```

Field metadata (type, dictionary-encoded, nullable) is fixed in code per subsystem layer
(`Layer::Registers | Internal | Writes | Output | Timing`).

### Query engine (`query.rs`, `comparison.rs`)

The `--where` flag in `gbtrace query` accepts conditions like `pc=0x0150`, `a changes`,
`flag z becomes set`, `pc&0xFF00=0xC000`. `gbtrace diff` uses a sync condition (default
`pc`) to align two traces before reporting per-field divergence and match percentages.

### Adapters

Each adapter is a stand-alone CLI named `gbtrace-<emu>` placed at
`adapters/<emu>/gbtrace-<emu>` (this exact path is hard-coded in `gen-rules.py` and the
trace shell scripts). All adapters expose the same surface:

```
--rom <path> --profile <profile.toml> --output <trace.gbtrace>
[--frames N] [--stop-when ADDR=VAL] [--stop-opcode HEX] [--reference <ref.pix>]
```

C/C++ adapters (`gambatte`, `sameboy`, `mgba`, `gateboy`, `docboy`, `bgb`) link against
`libgbtrace_ffi.a` (header at `crates/gbtrace-ffi/gbtrace.h`). The Rust adapter
(`missingno`) uses the core crate directly. Per-adapter build details live in
`adapters/<emu>/Makefile` and may invoke nested cmake/scons builds against vendored
emulator sources (which are gitignored — see `.gitignore`).

Adapters honour the profile's `trigger`: `instruction` emits one entry per opcode;
`tcycle` emits one entry per T-cycle (`docboy`, `missingno`, and `sameboy` support
this). The **sameboy** adapter reaches T-cycle granularity via a small checked-in
patch, `adapters/sameboy/sameboy-tcycle.patch` (adds `GB_set_tcycle_callback`,
firing once per T-cycle inside `GB_advance_cycles`); `make lib` in that dir applies
it before building `libsameboy`. With no callback the patch is a no-op, so
instruction-mode behaviour is unchanged. (Before this, sameboy hardcoded
`trigger:"instruction"` regardless of the profile — all suite profiles are `tcycle`.)

The **bgb** adapter is experimental and excluded from CI/site (see
`adapters/bgb/README.md`). The **mgba** adapter has been removed from the trace pipeline
but the directory still builds.

### Test suites (`test-suites/`)

Each suite directory contains the ROMs (`*.gb`/`*.gbc`) plus a `profile.toml`. Trace
generation goes through one of the per-suite shell scripts (`scripts/trace-<suite>.sh`)
which invoke the adapter, then use the CLI to determine pass/fail (typically by querying a
"magic" memory address) and rename the output to `<rom>_<emu>_<system>_<status>.gbtrace`.

**System dimension (DMG / CGB).** DMG and CGB are modelled as *separate but related
systems* (not two configs of one system) — the intended direction as gbtrace grows toward
more systems. Which systems a ROM runs under is decided in `scripts/gen-rules.py` by a
per-suite policy: `root_models` (`dmg`/`cgb`/both), an optional `cgb/` subdir holding a
curated CGB-only ROM set (from `missingno-gbc`), `gambatte` mode (system from filename tags
`_dmg08`/`_cgb04c`/`_blank`), and `ref_driven` (screenshot suites run a system only when a
matching reference exists). A build can be sharded to one system with `SYSTEMS=dmg|cgb`
(gen-rules' 2nd arg / the Makefile var); CI runs one job per `(suite × emulator × system)`.
The system is passed to the trace scripts via the `MODEL` env var (→ the adapter `--model`,
which selects the hardware revision: dmg→DMG-B, cgb→CGB-C). docboy selects DMG/CGB at
compile time, so `(docboy, cgb)` resolves to the separate `gbtrace-docboy-cgb` binary.

**Screenshot references** are raw **RGB555** (`.rgb555`, 160×144×3 bytes, 5-bit/channel),
generated from a checked-in `.png` by `make pix-refs` (`scripts/png-to-pix.py`). Comparing
at the CGB's native 5-bit precision is expansion-neutral. `scripts/ref-lib.sh`'s
`find_ref` picks the model-appropriate reference (CGB-specific `_cgb04c`/`_cgb_c`/`-cgb`,
falling back to the DMG/base ref). Gambatte `_out<hex>`/`_blank`/`_outaudio` tests need no
reference image — the expected value is decoded from the filename (`check-gambatte-hex.py`,
adapter `--report-audio`).

`scripts/manifest.py` writes a `manifest.json` per suite trace dir, keyed per test with a
`systems: { dmg: {emu: status}, cgb: {emu: status} }` map. The emulator list is hard-coded
near the top — keep it in sync with the `EMUS`/`ADAPTERS` Makefile vars. The per-suite
system map in `traces.yml`'s matrix-setup also mirrors the gen-rules policy.

### Web viewer (`web/`)

Lit-based static site (no bundler). `web/index.html` imports `lit` from a CDN via an
import map. Components in `web/src/components/` use the WASM bridge to read traces from the
same `gbtrace` core that powers the CLI. The test picker has a **DMG/CGB toggle** and reads
trace blobs from a configurable base URL (`window.GBTRACE_TRACE_BASE`); manifests + ROMs
are same-origin. `make site` assembles `build/site/` for local serving.

**Trace hosting.** The full trace set (~20 GB) far exceeds the 1 GB GitHub Pages limit, so
**Pages hosts only the app + manifests + ROMs + profiles**, and the `.gbtrace` blobs live
on **DigitalOcean Spaces** (S3-compatible). `traces.yml` `aws s3 sync`s the blobs up;
`deploy.yml` injects the Spaces CDN URL as `window.GBTRACE_TRACE_BASE`. Required repo
secrets/vars: `SPACES_KEY`/`SPACES_SECRET` (secrets), `SPACES_BUCKET`/`SPACES_REGION`/
`TRACE_BASE` (vars).

### CI workflows (`.github/workflows/`)

- `build.yml` — builds CLI/FFI/WASM + adapters, uploads artifacts (docboy ships two
  binaries: `gbtrace-docboy` + `gbtrace-docboy-cgb`).
- `traces.yml` — generates traces per `(suite × emulator)` via `make traces-<suite>`
  (which encodes the model policy), uploads blobs to Spaces + tiny filename-list artifacts.
- `deploy.yml` — rebuilds manifests from the filename lists, assembles + deploys the Pages
  site (no blobs).

`gateboy` and `mgba` are no longer in automated collection (their adapter dirs still
build). When adding/removing an emulator or test suite, update **all of**: the Makefile
(`ADAPTERS`, `EMUS`, suite vars + trace dirs + `traces-<suite>` target), `scripts/gen-rules.py`
(default emus, `SUITES` table), the relevant `scripts/trace-<suite>.sh`,
`scripts/manifest.py` (`EMULATORS`), `web/src/components/test-picker.js` (`EMULATORS`,
`TEST_SUITES`), and the workflow YAMLs.
