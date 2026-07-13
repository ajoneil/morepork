# morepork-bgb adapter
#
# BGB is a closed-source Windows emulator.  This adapter runs it under
# Wine in headless mode, capturing per-instruction state via BGB's
# debug-message breakpoint feature and writing .morepork via the FFI.
#
# Build dependencies: curl, unzip (to download BGB)
# Runtime dependencies: wine, xvfb-run

CC ?= cc
CFLAGS = -O2 -std=c11 -Wall -Wextra
FFI_DIR  ?= ../../crates/morepork-ffi
FFI_LIB  ?= ../../target/release/libmorepork_ffi.a

INCLUDES = -I$(FFI_DIR)
LDLIBS = -lm -ldl -lpthread

TARGET = morepork-bgb
BGB_URL = https://bgb.bircd.org/bgb.zip

.PHONY: all clean

all: $(TARGET) bgb.exe

$(TARGET): morepork-bgb.c $(FFI_LIB)
	$(CC) $(CFLAGS) $(INCLUDES) -o $@ $< $(FFI_LIB) $(LDLIBS)

bgb.exe:
	curl -fsSL --retry 3 -o bgb.zip '$(BGB_URL)'
	unzip -oq bgb.zip bgb.exe bgb.ini
	rm -f bgb.zip
	sed -i 's/DebugMsgFile=0/DebugMsgFile=1/' bgb.ini

clean:
	rm -f $(TARGET)
