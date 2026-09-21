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

# The Git environment goes before the tests run. A fixture-backed test spawns
# Git for the temporary repository in front of it, and the code under test
# inherits whatever the shell holds. `GIT_DIR` and its companions send that
# Git somewhere else entirely. git sets them for every hook it runs, and in
# the main worktree `GIT_DIR` is the relative `.git`, which resolves to
# nothing from a fixture's own directory and does no harm. In a linked
# worktree it is absolute and the fixture tests then read this repository
# rather than the fixture. A gate that reads the wrong repository is not a
# gate.
.PHONY: test
test:
	env -u GIT_DIR -u GIT_INDEX_FILE -u GIT_WORK_TREE -u GIT_COMMON_DIR \
	    -u GIT_OBJECT_DIRECTORY -u GIT_ALTERNATE_OBJECT_DIRECTORIES \
	    -u GIT_NAMESPACE -u GIT_PREFIX \
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
	  command -v $$t >/dev/null 2>&1 || { \
	    echo "shell: $$t not found (brew or apt-get install $$t)" >&2; \
	    exit 1; \
	  }; \
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

# The spec converter in `tools/spec-convert`. It is a development tool and not
# part of the build: `specs/` is committed and every build reads it as it
# stands. Run this when the corpus moves, read the diff, commit it. CORPUS is
# either the published tarball or an unpacked package directory.
#
#   make specs CORPUS=~/Downloads/autocomplete-2.692.3.tgz
#
# The converter is outside this package. Its manifest carries an empty
# `[workspace]` table, so `cargo build` here never sees it and `cargo install`
# never sees its dependencies. `make check` does not cover it either. Use
# `make specs-check` for that.
CONVERTER := tools/spec-convert/target/release/spec-convert

.PHONY: specs
specs:
	@[ -n "$(CORPUS)" ] || { \
	  echo "specs: set CORPUS to the corpus tarball or an unpacked package" >&2; \
	  exit 1; \
	}
	cargo build --locked --release --manifest-path tools/spec-convert/Cargo.toml
	@case "$(CORPUS)" in \
	  *.tgz|*.tar.gz) \
	    d=$$(mktemp -d) && trap 'rm -rf "$$d"' EXIT && \
	    tar xzf "$(CORPUS)" -C "$$d" && \
	    $(CONVERTER) --corpus "$$d/package" --out specs ;; \
	  *) \
	    $(CONVERTER) --corpus "$(CORPUS)" --out specs ;; \
	esac

.PHONY: specs-check
specs-check:
	cargo fmt --manifest-path tools/spec-convert/Cargo.toml -- --check
	cargo clippy --locked --manifest-path tools/spec-convert/Cargo.toml --all-targets

.PHONY: install
install:
	cargo install --locked --path .

.PHONY: uninstall
uninstall:
	cargo uninstall surmise

.PHONY: clean
clean:
	cargo clean
