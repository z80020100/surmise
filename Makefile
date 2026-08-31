.PHONY: build
build:
	cargo build --locked

.PHONY: release
release:
	cargo build --locked --release

# The gate. Each command is its own target so CI can report it as its own step
# without restating the command.
.PHONY: fmt-check
fmt-check:
	cargo fmt --all -- --check

.PHONY: clippy
clippy:
	cargo clippy --locked --all-targets

.PHONY: test
test:
	cargo test --locked

.PHONY: check
check: fmt-check clippy test

.PHONY: install
install:
	cargo install --locked --path .

.PHONY: uninstall
uninstall:
	cargo uninstall surmise

.PHONY: clean
clean:
	cargo clean
