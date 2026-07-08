# gbtrace-sameboy adapter
#
# SameBoy is used as a library with a single small patch (sameboy-tcycle.patch)
# that adds a per-T-cycle callback, enabling T-cycle granularity traces. Set it
# up once:
#   git clone https://github.com/LIJI32/SameBoy.git
#   make lib            # applies the patch + builds libsameboy.a/.so
#
# Then build the adapter:
#   make
#
# `make lib` applies sameboy-tcycle.patch (idempotently, guarded by a stamp file
# inside SameBoy/) and builds only the static + shared libs — it deliberately
# skips SameBoy's public-header generation step (which needs the `cppp` tool and
# is unused here, since we include Core/gb.h directly with GB_INTERNAL).

SAMEBOY_DIR ?= SameBoy

CXX ?= g++
CXXFLAGS = -O2 -std=c++17 -Wall -Wextra
FFI_DIR  ?= ../../crates/gbtrace-ffi
FFI_LIB  ?= ../../target/release/libgbtrace_ffi.a
# Include Core/ directly (we define GB_INTERNAL for full struct access)
INCLUDES = -I$(SAMEBOY_DIR)/Core -I$(FFI_DIR)
LDFLAGS = -L$(SAMEBOY_DIR)/build/lib
LDLIBS = -lsameboy -lm -ldl -lpthread

TARGET = gbtrace-sameboy

PATCH = sameboy-tcycle.patch
PATCH_STAMP = $(SAMEBOY_DIR)/.gbtrace-tcycle-patched

.PHONY: all clean patch lib

all: $(TARGET)

$(TARGET): gbtrace-sameboy.cpp $(FFI_LIB)
	$(CXX) $(CXXFLAGS) $(INCLUDES) -o $@ $< $(FFI_LIB) $(LDFLAGS) $(LDLIBS)

# Apply the per-T-cycle patch, idempotently. The stamp lives inside SameBoy/, so
# a fresh clone re-applies cleanly. The apply is guarded by a reverse-dry-run:
# if the patch already reverses cleanly it is already applied, so we skip rather
# than fail. This matters on CI, where SameBoy/ is restored from cache (already
# patched) but the fresh git checkout gives $(PATCH) a newer mtime than the
# cached stamp — so make re-runs this recipe over an already-patched tree.
$(PATCH_STAMP): $(PATCH)
	@if patch -p1 -d $(SAMEBOY_DIR) -R --dry-run --force < $(PATCH) >/dev/null 2>&1; then \
		echo "$(PATCH) already applied to $(SAMEBOY_DIR)/Core — skipping"; \
	else \
		echo "Applying $(PATCH) to $(SAMEBOY_DIR)/Core ..."; \
		patch -p1 -d $(SAMEBOY_DIR) --forward < $(PATCH); \
	fi
	@touch $@

patch: $(PATCH_STAMP)

# Build the SameBoy static + shared libs (skips the cppp-based header target).
lib: $(PATCH_STAMP)
	$(MAKE) -C $(SAMEBOY_DIR) CONF=release \
		build/lib/libsameboy.a build/lib/libsameboy.so

clean:
	rm -f $(TARGET)
