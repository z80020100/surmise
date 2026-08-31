.PHONY: build
build:
	cargo build

.PHONY: release
release:
	cargo build --release

.PHONY: check
check:
	cargo fmt --all -- --check
	cargo clippy --all-targets
	cargo test

.PHONY: clean
clean:
	cargo clean
