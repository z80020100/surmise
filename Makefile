.PHONY: build
build:
	cargo build --locked

.PHONY: release
release:
	cargo build --locked --release

.PHONY: check
check:
	cargo fmt --all -- --check
	cargo clippy --locked --all-targets
	cargo test --locked

.PHONY: install
install:
	cargo install --locked --path .

.PHONY: uninstall
uninstall:
	cargo uninstall surmise

.PHONY: clean
clean:
	cargo clean
