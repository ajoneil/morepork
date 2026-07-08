# gbtrace-gopher2600 adapter
#
# Builds a native-.gbtrace adapter for the Gopher2600 emulator (VCS family).
# Gopher2600 source is cloned into ./src on first build; the tracked adapter
# source (main.go) is copied into the cloned module as a cmd and built there,
# so it resolves Gopher2600's packages locally. The gbtrace FFI static lib is
# linked via cgo, with paths passed through CGO_CFLAGS/CGO_LDFLAGS.
#
#   make                 # clone deps if needed, build ./gbtrace-gopher2600
#   make GOPHER_REF=v0.56.0
#   make clean

GOPHER_DIR ?= src
GOPHER_URL ?= https://github.com/JetSetIlly/Gopher2600.git
FFI_DIR    ?= ../../crates/gbtrace-ffi
FFI_LIB    ?= ../../target/release/libgbtrace_ffi.a
CMD_DIR     = $(GOPHER_DIR)/cmd/gbtrace-gopher2600
TARGET      = gbtrace-gopher2600

.PHONY: all clean

all: $(TARGET)

# Clone Gopher2600 if not present
$(GOPHER_DIR):
	git clone --depth 1 $(GOPHER_URL) $(GOPHER_DIR)

# Build the gbtrace FFI static lib if missing
$(FFI_LIB):
	cd ../.. && cargo build --release -p gbtrace-ffi

$(TARGET): main.go | $(GOPHER_DIR) $(FFI_LIB)
	mkdir -p $(CMD_DIR)
	cp main.go $(CMD_DIR)/main.go
	cd $(GOPHER_DIR) && CGO_ENABLED=1 \
	  CGO_CFLAGS="-I$(abspath $(FFI_DIR))" \
	  CGO_LDFLAGS="$(abspath $(FFI_LIB)) -lm -ldl -lpthread" \
	  go build -o $(abspath $(TARGET)) ./cmd/gbtrace-gopher2600

clean:
	rm -f $(TARGET)
	rm -rf $(CMD_DIR)
