# morepork-mame adapter
#
# Standalone Go program (stdlib + cgo). Links the morepork FFI to write native
# .morepork, and drives the system `mame` binary headlessly over its gdbstub.
#
#   make          # build ./morepork-mame
#   make clean

FFI_DIR ?= ../../crates/morepork-ffi
FFI_LIB ?= ../../target/release/libmorepork_ffi.a
TARGET   = morepork-mame

.PHONY: all clean

all: $(TARGET)

$(FFI_LIB):
	cd ../.. && cargo build --release -p morepork-ffi

$(TARGET): $(wildcard *.go) go.mod $(FFI_LIB)
	CGO_ENABLED=1 \
	  CGO_CFLAGS="-I$(abspath $(FFI_DIR))" \
	  CGO_LDFLAGS="$(abspath $(FFI_LIB)) -lm -ldl -lpthread" \
	  go build -o $(TARGET) .

clean:
	rm -f $(TARGET)
