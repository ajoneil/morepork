.PHONY: all clean

RELEASE ?= 1
CARGO_FLAGS = $(if $(filter 1,$(RELEASE)),--release,)
TARGET_DIR = $(if $(filter 1,$(RELEASE)),release,debug)

all: morepork-missingno

morepork-missingno: src/main.rs Cargo.toml
	cargo build $(CARGO_FLAGS) --manifest-path Cargo.toml
	cp target/$(TARGET_DIR)/morepork-missingno .

clean:
	cargo clean --manifest-path Cargo.toml
	rm -f morepork-missingno
