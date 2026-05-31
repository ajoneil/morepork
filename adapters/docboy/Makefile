# gbtrace-docboy adapter
#
# Uses DocBoy (https://github.com/Docheinstein/docboy) to produce .gbtrace
# files with T-cycle granularity via the debugger tick callback API.
#
# Dependencies are cloned and built automatically on first build:
#   make
#
# Or point at an existing checkout:
#   make DOCBOY_DIR=/path/to/docboy
#
# DocBoy selects DMG vs CGB at COMPILE TIME (-DENABLE_CGB), so we build two
# binaries from the same source: gbtrace-docboy (DMG) and gbtrace-docboy-cgb
# (CGB), each linked against a separately-built core.

DOCBOY_DIR       ?= docboy
DOCBOY_BUILD     ?= $(DOCBOY_DIR)/build
DOCBOY_BUILD_CGB ?= $(DOCBOY_DIR)/build-cgb

CXX      ?= g++
# ENABLE_DEBUGGER: required for tick callback and register access
# ENABLE_AUDIO: required for DocBoy headers to compile (APU struct members)
CXXFLAGS  = -O2 -std=c++17 -Wall -Wextra -DENABLE_DEBUGGER -DENABLE_AUDIO
FFI_DIR  ?= ../../crates/gbtrace-ffi
FFI_LIB  ?= ../../target/release/libgbtrace_ffi.a
INCLUDES  = -I$(DOCBOY_DIR)/src/docboy -I$(DOCBOY_DIR)/src/utils -I$(DOCBOY_DIR)/src/extra -I$(FFI_DIR)

DOCBOY_LIB = $(DOCBOY_BUILD)/src/docboy/libdocboy.a
UTILS_LIB  = $(DOCBOY_BUILD)/src/utils/libutils.a
EXTRA_LIB  = $(DOCBOY_BUILD)/src/extra/libextra.a

DOCBOY_LIB_CGB = $(DOCBOY_BUILD_CGB)/src/docboy/libdocboy.a
UTILS_LIB_CGB  = $(DOCBOY_BUILD_CGB)/src/utils/libutils.a
EXTRA_LIB_CGB  = $(DOCBOY_BUILD_CGB)/src/extra/libextra.a

TARGET     = gbtrace-docboy
TARGET_CGB = gbtrace-docboy-cgb

.PHONY: all clean

all: $(TARGET) $(TARGET_CGB)

# Clone DocBoy if not present
$(DOCBOY_DIR):
	git clone --depth 1 https://github.com/Docheinstein/docboy.git $(DOCBOY_DIR)
	cd $(DOCBOY_DIR) && git submodule update --init --depth 1

# Apply trace API patch (tick callback + register getters)
$(DOCBOY_DIR)/.patched: $(DOCBOY_DIR) docboy-trace-api.patch
	cd $(DOCBOY_DIR) && git checkout -- . && patch -p1 --fuzz=5 < ../docboy-trace-api.patch
	touch $@

# Build DocBoy as a static library via CMake (DMG)
$(DOCBOY_LIB): $(DOCBOY_DIR)/.patched
	cmake -S $(DOCBOY_DIR) -B $(DOCBOY_BUILD) \
		-DBUILD_SDL_FRONTEND=OFF \
		-DBUILD_NOGUI_FRONTEND=OFF \
		-DBUILD_LIBRETRO_CORE=ON `# prevents CMake from auto-enabling SDL frontend` \
		-DENABLE_DEBUGGER=ON \
		-DENABLE_BOOTROM=OFF \
		-DENABLE_AUDIO=ON \
		-DENABLE_CGB=OFF \
		-DENABLE_TWO_PLAYERS_MODE=OFF \
		-DCMAKE_BUILD_TYPE=Release \
		-DSDL_UNIX_CONSOLE_BUILD=ON
	cmake --build $(DOCBOY_BUILD) --target docboy extra -j$$(nproc)

# Build DocBoy as a static library via CMake (CGB)
$(DOCBOY_LIB_CGB): $(DOCBOY_DIR)/.patched
	cmake -S $(DOCBOY_DIR) -B $(DOCBOY_BUILD_CGB) \
		-DBUILD_SDL_FRONTEND=OFF \
		-DBUILD_NOGUI_FRONTEND=OFF \
		-DBUILD_LIBRETRO_CORE=ON `# prevents CMake from auto-enabling SDL frontend` \
		-DENABLE_DEBUGGER=ON \
		-DENABLE_BOOTROM=OFF \
		-DENABLE_AUDIO=ON \
		-DENABLE_CGB=ON \
		-DENABLE_TWO_PLAYERS_MODE=OFF \
		-DCMAKE_BUILD_TYPE=Release \
		-DSDL_UNIX_CONSOLE_BUILD=ON
	cmake --build $(DOCBOY_BUILD_CGB) --target docboy extra -j$$(nproc)

$(TARGET): gbtrace-docboy.cpp $(DOCBOY_LIB) $(FFI_LIB)
	$(CXX) $(CXXFLAGS) $(INCLUDES) -o $@ $< -Wl,--start-group $(DOCBOY_LIB) $(EXTRA_LIB) $(UTILS_LIB) -Wl,--end-group $(FFI_LIB) -lm -ldl -lpthread

$(TARGET_CGB): gbtrace-docboy.cpp $(DOCBOY_LIB_CGB) $(FFI_LIB)
	$(CXX) $(CXXFLAGS) -DENABLE_CGB $(INCLUDES) -o $@ $< -Wl,--start-group $(DOCBOY_LIB_CGB) $(EXTRA_LIB_CGB) $(UTILS_LIB_CGB) -Wl,--end-group $(FFI_LIB) -lm -ldl -lpthread

clean:
	rm -f $(TARGET) $(TARGET_CGB)
