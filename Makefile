TARGET := arm-unknown-linux-gnueabi

.PHONY: build
build:
	cargo build

.PHONY: release
release:
	cargo build --release

.PHONY: cross
cross:
	cross build --target ${TARGET} --release

.PHONY: clean
clean:
	cargo clean

.PHONY: setup
setup:
	@echo "Enter the following command to setup Rust and cross compilation environment"
	@echo "source scripts/envsetup.sh"
