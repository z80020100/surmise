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

# The shell gate. The POSIX scripts get shellcheck and shfmt. The zsh widgets
# get a syntax check alone, because neither tool has a zsh dialect. A missing
# tool fails rather than skips. `CLAUDE.md` gives the reasoning.
SH_SCRIPTS := .cargo-husky/hooks/pre-commit
ZSH_WIDGETS := $(wildcard shell/*.zsh)

# `shell/` is a directory. Without .PHONY make calls this target up to date and
# runs nothing.
.PHONY: shell
shell:
	@for t in shellcheck shfmt; do \
	  command -v $$t >/dev/null 2>&1 || \
	    { echo "shell: $$t not found (brew install $$t)" >&2; exit 1; }; \
	done
	shellcheck $(SH_SCRIPTS)
	shfmt -i 2 -d $(SH_SCRIPTS)
	@[ -n "$(ZSH_WIDGETS)" ] || echo "shell: no zsh widgets matched"
	@for f in $(ZSH_WIDGETS); do \
	  echo "zsh -n $$f"; \
	  zsh -n "$$f" || exit 1; \
	done

.PHONY: check
check: fmt-check clippy test shell

.PHONY: install
install:
	cargo install --locked --path .

.PHONY: uninstall
uninstall:
	cargo uninstall surmise

.PHONY: clean
clean:
	cargo clean
