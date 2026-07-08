# gbtrace-stella adapter
#
# Builds a native-.gbtrace, per-instruction adapter for the Stella emulator
# (VCS family). Stella isn't an embeddable library, so we build its libretro
# core objects (which give a headless OSystem) and link a small wrapper that
# drives the emucore one instruction at a time. A tiny patch exposes the 6507
# register file (otherwise private to the debugger, which the libretro build
# omits).
#
#   make            # clone + patch Stella, build core, build ./gbtrace-stella
#   make clean

STELLA_DIR ?= src
STELLA_URL ?= https://github.com/stella-emu/stella.git
S           = $(STELLA_DIR)/src
LIBRETRO    = $(S)/os/libretro
FFI_DIR    ?= ../../crates/gbtrace-ffi
FFI_LIB    ?= ../../target/release/libgbtrace_ffi.a
TARGET      = gbtrace-stella
NPROC      := $(shell nproc)

INC = -I$(LIBRETRO) -I$(S) -I$(S)/emucore -I$(S)/emucore/tia -I$(S)/emucore/elf \
      -I$(S)/common -I$(S)/common/audio -I$(S)/common/tv_filters \
      -I$(S)/common/repository/sqlite -I$(S)/lib -I$(S)/lib/sqlite -I$(FFI_DIR)
CXXFLAGS = -std=c++23 -fno-rtti -O2 -DSOUND_SUPPORT -D__LIB_RETRO__ -DHAVE_STDINT_H

.PHONY: all clean

all: $(TARGET)

# Clone Stella if not present
$(STELLA_DIR):
	git clone --depth 1 $(STELLA_URL) $(STELLA_DIR)

# Apply the trace-api patch (exposes the M6502 register file)
$(STELLA_DIR)/.patched: stella-trace-api.patch | $(STELLA_DIR)
	cd $(STELLA_DIR) && git apply ../stella-trace-api.patch
	touch $@

$(FFI_LIB):
	cd ../.. && cargo build --release -p gbtrace-ffi

# Build all Stella core objects via its own libretro Makefile
.PHONY: stella-objs
stella-objs: $(STELLA_DIR)/.patched
	$(MAKE) -C $(LIBRETRO) -j$(NPROC)

# Compile the wrapper and link it against every Stella object except the
# libretro C-API translation unit (whose frontend glue we stub ourselves).
$(TARGET): gbtrace-stella.cxx $(FFI_LIB) stella-objs
	g++ $(CXXFLAGS) $(INC) -c gbtrace-stella.cxx -o gbtrace-stella.o
	OBJS=$$(find $(S) -name '*.o' ! -name 'libretro.o' ! -name 'gbtrace-stella.o'); \
	g++ -flto -O2 -o $(TARGET) gbtrace-stella.o $$OBJS $(abspath $(FFI_LIB)) \
	  -lpthread -static-libgcc -static-libstdc++ -lm -ldl

clean:
	rm -f $(TARGET) gbtrace-stella.o
