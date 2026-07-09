# gbtrace-mame adapter
#
# Standalone Go program (stdlib + cgo). Links the gbtrace FFI to write native
# .gbtrace, and drives the system `mame` binary headlessly over its gdbstub.
#
#   make          # build ./gbtrace-mame
#   make clean

FFI_DIR ?= ../../crates/gbtrace-ffi
FFI_LIB ?= ../../target/release/libgbtrace_ffi.a
TARGET   = gbtrace-mame

.PHONY: all clean

all: $(TARGET)

$(FFI_LIB):
	cd ../.. && cargo build --release -p gbtrace-ffi

$(TARGET): $(wildcard *.go) go.mod $(FFI_LIB)
	CGO_ENABLED=1 \
	  CGO_CFLAGS="-I$(abspath $(FFI_DIR))" \
	  CGO_LDFLAGS="$(abspath $(FFI_LIB)) -lm -ldl -lpthread" \
	  go build -o $(TARGET) .

clean:
	rm -f $(TARGET)
