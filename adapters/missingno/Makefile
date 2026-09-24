.PHONY: all clean

RELEASE ?= 1
CARGO_FLAGS = $(if $(filter 1,$(RELEASE)),--release,)
TARGET_DIR = $(if $(filter 1,$(RELEASE)),release,debug)

all: morepork-missingno

morepork-missingno: src/main.rs src/vcs.rs src/vcs_palette.rs src/sg1000.rs src/colecovision.rs src/ti_vdp_args.rs Cargo.toml
	cargo build $(CARGO_FLAGS) --manifest-path Cargo.toml
	cp target/$(TARGET_DIR)/morepork-missingno .
	cp target/$(TARGET_DIR)/morepork-missingno-vcs .
	cp target/$(TARGET_DIR)/morepork-missingno-sg1000 .
	cp target/$(TARGET_DIR)/morepork-missingno-colecovision .

clean:
	cargo clean --manifest-path Cargo.toml
	rm -f morepork-missingno morepork-missingno-vcs morepork-missingno-sg1000 morepork-missingno-colecovision
