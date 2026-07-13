# morepork - Build adapters, generate traces, assemble site
#
# Usage:
#   make adapters            - Build all adapter binaries
#   make cli                 - Build morepork CLI
#   make wasm                - Build WASM module
#   make traces              - Generate all traces (use -jN for parallelism)
#   make traces-gbmicrotest  - Generate gbmicrotest traces only
#   make traces-blargg       - Generate blargg traces only
#   make site                - Assemble deployable site in build/
#   make serve               - Serve locally for development
#   make clean               - Remove build artifacts
#
# Parallel trace generation:
#   make traces -j$(nproc)
#
# Override emulators:
#   make traces EMUS=gambatte,mgba

SHELL := /bin/bash
.SHELLFLAGS := -euo pipefail -c

PROJECT_DIR := $(shell pwd)
CLI := $(PROJECT_DIR)/target/release/morepork
BUILD_DIR := $(PROJECT_DIR)/build
PAGES_URL ?= https://ajoneil.github.io/morepork

# Adapters
ADAPTERS := gambatte sameboy missingno docboy
ADAPTER_BINS := $(foreach a,$(ADAPTERS),adapters/$(a)/morepork-$(a))

# Emulators to run (comma-separated, override with EMUS=gambatte,missingno)
EMUS ?= gambatte,sameboy,missingno,docboy

# Systems to trace (DMG / CGB are modelled as separate but related systems).
# Override to shard, e.g. SYSTEMS=cgb to generate only Game Boy Color traces.
SYSTEMS ?= dmg,cgb

# Whether trace stamps may build a missing adapter binary. On (default) for
# local one-shot `make traces`; set BUILD_ADAPTERS=0 in CI, where adapters are
# downloaded artifacts and a missing one should fail fast, not silently recompile.
BUILD_ADAPTERS ?= 1

# Trace output dirs
GBMICROTEST_TRACE_DIR := $(BUILD_DIR)/traces/gbmicrotest
BLARGG_TRACE_DIR := $(BUILD_DIR)/traces/blargg
MOONEYE_TRACE_DIR := $(BUILD_DIR)/traces/mooneye
GAMBATTE_TESTS_TRACE_DIR := $(BUILD_DIR)/traces/gambatte-tests
MEALYBUG_TEAROOM_TRACE_DIR := $(BUILD_DIR)/traces/mealybug-tearoom
AGE_TRACE_DIR := $(BUILD_DIR)/traces/age
MOONEYE_WILBERTPOL_TRACE_DIR := $(BUILD_DIR)/traces/mooneye-wilbertpol
SAMESUITE_TRACE_DIR := $(BUILD_DIR)/traces/samesuite
SCRIBBLTESTS_TRACE_DIR := $(BUILD_DIR)/traces/scribbltests
BULLY_TRACE_DIR := $(BUILD_DIR)/traces/bully
MBC3_TESTER_TRACE_DIR := $(BUILD_DIR)/traces/mbc3-tester
STRIKETHROUGH_TRACE_DIR := $(BUILD_DIR)/traces/strikethrough
TURTLE_TESTS_TRACE_DIR := $(BUILD_DIR)/traces/turtle-tests
DMG_ACID2_TRACE_DIR := $(BUILD_DIR)/traces/dmg-acid2
CGB_ACID2_TRACE_DIR := $(BUILD_DIR)/traces/cgb-acid2
CGB_ACID_HELL_TRACE_DIR := $(BUILD_DIR)/traces/cgb-acid-hell

export LD_LIBRARY_PATH := $(PROJECT_DIR)/adapters/sameboy/SameBoy/build/lib:$(LD_LIBRARY_PATH)
export CLI

# ── Generated rules ──────────────────────────────────────────────────
# gen-rules.py produces per-ROM×emulator stamp targets and the
# GBMICROTEST_STAMPS / BLARGG_STAMPS variable definitions.

RULES_MK := $(BUILD_DIR)/rules.mk

$(RULES_MK): scripts/gen-rules.py
	@mkdir -p $(BUILD_DIR)
	@MOREPORK_BUILD_ADAPTERS=$(BUILD_ADAPTERS) python3 scripts/gen-rules.py $(EMUS) $(SYSTEMS) > $@

-include $(RULES_MK)

# ── Top-level targets ────────────────────────────────────────────────

# Screenshot test reference files: .png (checked in) → .rgb555 (generated).
# One Python process walks test-suites and (re)generates every stale/missing
# ref. Doing this in a single interpreter — rather than spawning python per PNG
# — turns a ~10s, ~240-spawn step into a fraction of a second, which matters
# because every fresh-checkout CI trace shard runs it (.rgb555 is gitignored).
.PHONY: pix-refs
pix-refs: scripts/png-to-pix.py
	@python3 scripts/png-to-pix.py test-suites

# pix-refs writes the .rgb555 files that the screenshot-suite trace stamps read
# (via find_ref). They are sibling prerequisites of the traces-<suite> targets,
# so under `make -j` pix-refs and the stamps run UNORDERED: a stamp can call
# find_ref before its reference has been generated, in which case the adapter
# runs with no --reference, never reports "Reference match", and the trace is
# mislabelled `fail`. (This consistently bit the first ~nproc stamps of each CI
# shard — e.g. blargg cpu_instrs/01-special/02-interrupts on DMG.) An order-only
# prerequisite forces pix-refs to finish first, without letting the phony target
# invalidate the stamps. Guarded with ifneq so the first parse (before rules.mk
# is generated and the *_STAMPS vars are populated) doesn't see an empty target.
SCREENSHOT_STAMPS := $(BLARGG_STAMPS) $(GAMBATTE_TESTS_STAMPS) $(MEALYBUG_TEAROOM_STAMPS) \
	$(DMG_ACID2_STAMPS) $(AGE_STAMPS) $(SCRIBBLTESTS_STAMPS) $(BULLY_STAMPS) \
	$(MBC3_TESTER_STAMPS) $(STRIKETHROUGH_STAMPS) $(TURTLE_TESTS_STAMPS) \
	$(CGB_ACID2_STAMPS) $(CGB_ACID_HELL_STAMPS)
ifneq ($(strip $(SCREENSHOT_STAMPS)),)
$(SCREENSHOT_STAMPS): | pix-refs
endif

DMG_ACID2_REF := test-suites/dmg-acid2/reference.rgb555

.PHONY: all adapters cli wasm traces traces-gbmicrotest traces-blargg \
        traces-mooneye traces-gambatte-tests traces-mealybug-tearoom traces-dmg-acid2 \
        traces-age traces-mooneye-wilbertpol traces-samesuite traces-scribbltests \
        traces-bully traces-mbc3-tester traces-strikethrough \
        traces-turtle-tests traces-cgb-acid2 traces-cgb-acid-hell \
        manifests site serve clean

all: site

adapters: $(ADAPTER_BINS)

cli: $(CLI)

traces: traces-gbmicrotest traces-blargg traces-mooneye traces-gambatte-tests traces-mealybug-tearoom traces-dmg-acid2 \
        traces-age traces-mooneye-wilbertpol traces-samesuite traces-scribbltests \
        traces-bully traces-mbc3-tester traces-strikethrough \
        traces-turtle-tests traces-cgb-acid2 traces-cgb-acid-hell

traces-gbmicrotest: $(RULES_MK) $(GBMICROTEST_STAMPS)
	@echo "Generating gbmicrotest manifest..."
	@python3 scripts/manifest.py "$(GBMICROTEST_TRACE_DIR)" "test-suites/gbmicrotest"
	@echo "=== gbmicrotest complete ==="

traces-blargg: $(RULES_MK) pix-refs $(BLARGG_STAMPS)
	@echo "Generating blargg manifest..."
	@python3 scripts/manifest.py "$(BLARGG_TRACE_DIR)" "test-suites/blargg"
	@echo "=== blargg complete ==="

traces-mooneye: $(RULES_MK) $(MOONEYE_STAMPS)
	@echo "Generating mooneye manifest..."
	@python3 scripts/manifest.py "$(MOONEYE_TRACE_DIR)" "test-suites/mooneye"
	@echo "=== mooneye complete ==="

traces-gambatte-tests: $(RULES_MK) pix-refs $(GAMBATTE_TESTS_STAMPS)
	@echo "Generating gambatte-tests manifest..."
	@python3 scripts/manifest.py "$(GAMBATTE_TESTS_TRACE_DIR)" "test-suites/gambatte"
	@echo "=== gambatte-tests complete ==="

traces-mealybug-tearoom: $(RULES_MK) pix-refs $(MEALYBUG_TEAROOM_STAMPS)
	@echo "Generating mealybug-tearoom manifest..."
	@python3 scripts/manifest.py "$(MEALYBUG_TEAROOM_TRACE_DIR)" "test-suites/mealybug-tearoom"
	@echo "=== mealybug-tearoom complete ==="


traces-dmg-acid2: $(RULES_MK) pix-refs $(DMG_ACID2_STAMPS)
	@echo "Generating dmg-acid2 manifest..."
	@python3 scripts/manifest.py "$(DMG_ACID2_TRACE_DIR)" "test-suites/dmg-acid2"
	@echo "=== dmg-acid2 complete ==="

traces-age: $(RULES_MK) pix-refs $(AGE_STAMPS)
	@echo "Generating age manifest..."
	@python3 scripts/manifest.py "$(AGE_TRACE_DIR)" "test-suites/age"
	@echo "=== age complete ==="

traces-mooneye-wilbertpol: $(RULES_MK) $(MOONEYE_WILBERTPOL_STAMPS)
	@echo "Generating mooneye-wilbertpol manifest..."
	@python3 scripts/manifest.py "$(MOONEYE_WILBERTPOL_TRACE_DIR)" "test-suites/mooneye-wilbertpol"
	@echo "=== mooneye-wilbertpol complete ==="

traces-samesuite: $(RULES_MK) $(SAMESUITE_STAMPS)
	@echo "Generating samesuite manifest..."
	@python3 scripts/manifest.py "$(SAMESUITE_TRACE_DIR)" "test-suites/samesuite"
	@echo "=== samesuite complete ==="

traces-scribbltests: $(RULES_MK) pix-refs $(SCRIBBLTESTS_STAMPS)
	@echo "Generating scribbltests manifest..."
	@python3 scripts/manifest.py "$(SCRIBBLTESTS_TRACE_DIR)" "test-suites/scribbltests"
	@echo "=== scribbltests complete ==="

traces-bully: $(RULES_MK) pix-refs $(BULLY_STAMPS)
	@echo "Generating bully manifest..."
	@python3 scripts/manifest.py "$(BULLY_TRACE_DIR)" "test-suites/bully"
	@echo "=== bully complete ==="

traces-mbc3-tester: $(RULES_MK) pix-refs $(MBC3_TESTER_STAMPS)
	@echo "Generating mbc3-tester manifest..."
	@python3 scripts/manifest.py "$(MBC3_TESTER_TRACE_DIR)" "test-suites/mbc3-tester"
	@echo "=== mbc3-tester complete ==="

traces-strikethrough: $(RULES_MK) pix-refs $(STRIKETHROUGH_STAMPS)
	@echo "Generating strikethrough manifest..."
	@python3 scripts/manifest.py "$(STRIKETHROUGH_TRACE_DIR)" "test-suites/strikethrough"
	@echo "=== strikethrough complete ==="

traces-turtle-tests: $(RULES_MK) pix-refs $(TURTLE_TESTS_STAMPS)
	@echo "Generating turtle-tests manifest..."
	@python3 scripts/manifest.py "$(TURTLE_TESTS_TRACE_DIR)" "test-suites/turtle-tests"
	@echo "=== turtle-tests complete ==="

traces-cgb-acid2: $(RULES_MK) pix-refs $(CGB_ACID2_STAMPS)
	@echo "Generating cgb-acid2 manifest..."
	@python3 scripts/manifest.py "$(CGB_ACID2_TRACE_DIR)" "test-suites/cgb-acid2"
	@echo "=== cgb-acid2 complete ==="

traces-cgb-acid-hell: $(RULES_MK) pix-refs $(CGB_ACID_HELL_STAMPS)
	@echo "Generating cgb-acid-hell manifest..."
	@python3 scripts/manifest.py "$(CGB_ACID_HELL_TRACE_DIR)" "test-suites/cgb-acid-hell"
	@echo "=== cgb-acid-hell complete ==="

comma := ,

site: wasm traces
	@echo "Assembling site in $(BUILD_DIR)/site..."
	@rm -rf $(BUILD_DIR)/site
	@mkdir -p $(BUILD_DIR)/site/pkg $(BUILD_DIR)/site/tests
	@cp web/index.html $(BUILD_DIR)/site/
	@cp -r web/src $(BUILD_DIR)/site/
	@cp web/pkg/morepork_wasm.js web/pkg/morepork_wasm_bg.wasm $(BUILD_DIR)/site/pkg/
	@# Copy traces, ROMs, and profiles for each suite
	@for suite_dir in $(BUILD_DIR)/traces/*/; do \
		[ -d "$$suite_dir" ] || continue; \
		suite=$$(basename "$$suite_dir"); \
		cp -r "$$suite_dir" "$(BUILD_DIR)/site/tests/$$suite"; \
		rom_dir="test-suites/$$suite"; \
		if [ "$$suite" = "gambatte-tests" ]; then rom_dir="test-suites/gambatte"; fi; \
		if [ -d "$$rom_dir" ]; then \
			cd "$$rom_dir" && find . \( -name '*.gb' -o -name '*.gbc' \) -exec sh -c \
				'mkdir -p "$(BUILD_DIR)/site/tests/'"$$suite"'/$$(dirname "{}")" && cp "{}" "$(BUILD_DIR)/site/tests/'"$$suite"'/{}"' \; && cd $(PROJECT_DIR); \
			if [ -f "$$rom_dir/profile.toml" ]; then \
				cp "$$rom_dir/profile.toml" "$(BUILD_DIR)/site/tests/$$suite/"; \
			fi; \
		fi; \
	done
	@echo "Site ready: $(BUILD_DIR)/site/"

serve: wasm
	@python3 $(PROJECT_DIR)/scripts/devserver.py $(PROJECT_DIR) $(PAGES_URL)

clean:
	rm -rf $(BUILD_DIR)

# ── Adapter builds ───────────────────────────────────────────────────

FFI_LIB := $(PROJECT_DIR)/target/release/libmorepork_ffi.a
FFI_HEADER := $(PROJECT_DIR)/crates/morepork-ffi/morepork.h

# C/C++ adapters statically link the FFI, so they must relink when it
# changes — otherwise they silently ship a stale trace writer.
adapters/gambatte/morepork-gambatte: adapters/gambatte/morepork-gambatte.cpp $(FFI_LIB) $(FFI_HEADER)
	@echo "Building gambatte adapter..."
	@$(MAKE) -C adapters/gambatte -j$$(nproc)

adapters/sameboy/morepork-sameboy: adapters/sameboy/morepork-sameboy.cpp $(FFI_LIB) $(FFI_HEADER)
	@echo "Building sameboy adapter..."
	@$(MAKE) -C adapters/sameboy -j$$(nproc)

adapters/mgba/morepork-mgba: adapters/mgba/morepork-mgba.c $(FFI_LIB) $(FFI_HEADER)
	@echo "Building mgba adapter..."
	@$(MAKE) -C adapters/mgba -j$$(nproc)

adapters/missingno/morepork-missingno:
	@echo "Building missingno adapter..."
	@cd adapters/missingno && cargo build --release && cp target/release/morepork-missingno .

adapters/docboy/morepork-docboy adapters/docboy/morepork-docboy-cgb: adapters/docboy/morepork-docboy.cpp $(FFI_LIB) $(FFI_HEADER)
	@echo "Building docboy adapters (DMG + CGB)..."
	@$(MAKE) -C adapters/docboy

$(CLI): $(wildcard crates/morepork/src/*.rs crates/morepork/src/**/*.rs)
	@echo "Building morepork..."
	@cargo build --release --features cli 2>&1 | tail -1

$(FFI_LIB): $(wildcard crates/morepork-ffi/src/*.rs crates/morepork/src/*.rs crates/morepork/src/**/*.rs)
	@echo "Building morepork-ffi..."
	@cargo build --release -p morepork-ffi 2>&1 | tail -1

ffi: $(FFI_LIB)

wasm: web/pkg/morepork_wasm_bg.wasm

web/pkg/morepork_wasm_bg.wasm: $(wildcard crates/morepork-wasm/src/*.rs crates/morepork/src/*.rs)
	@echo "Building WASM module..."
	@wasm-pack build crates/morepork-wasm --target web --out-dir $(PROJECT_DIR)/web/pkg --no-typescript
	@rm -f web/pkg/.gitignore web/pkg/package.json
