# surmise

TBD.

> This file also provides guidance to [Claude Code](https://claude.ai/code) when
> working with code in this repository. `README.md`, `AGENTS.md` and `GEMINI.md`
> are symbolic links to it.

**Nothing is here yet but the scaffolding.** This repository holds the package,
the licence, the lint gate and CI. `src/main.rs` is a placeholder and
`[dependencies]` is empty. The licence is settled. Everything else below that is
marked TBD is not.

## Prerequisites

- Rust 1.98.0. `rust-toolchain.toml` pins it and rustup installs it on demand
- Target platforms: macOS for now. That is provisional rather than a decision.
  It is simply the only platform anything has been built on

## Build, test and lint

```sh
make build                  # cargo build --locked
make release                # cargo build --locked --release
make check                  # the gate: fmt-check, then clippy, then test
make install                # cargo install --locked --path .
make uninstall              # cargo uninstall surmise
make clean                  # cargo clean
```

`make check` is the gate and the pre-commit hook runs it. CI calls the same
Makefile targets as separate steps so each one gets its own result in the
GitHub interface and the commands keep a single definition. CI then runs
`make release` as a fourth step. The gate does not cover that step and a green
hook therefore does not promise a green CI run.

CI runs on macOS only, by choice. No other platform is built and no other
platform is checked.

## Repository conventions

`.cargo/config.toml`, `.vscode/`, the cargo-husky hook and the three symbolic
links come from the template this repository started from. Everything else in
this section was decided here.

`.cargo/config.toml` sets `rustflags = ["-Dwarnings"]`. Every warning is an
error in local builds, in the pre-commit hook and in CI. A new clippy lint
therefore breaks the build and has to be answered rather than ignored.
Restructure the code where a lint is wrong rather than reach for `#[allow]`.
Note that a `RUSTFLAGS` environment variable replaces this setting rather than
adds to it. An empty one is enough. A shell that sets one turns the gate off
and `make check` then passes on code that CI rejects.

`rust-toolchain.toml` pins the toolchain for the same reason. A floating stable
plus `-Dwarnings` means a new lint can turn CI red with no change to the code.
Bump the pin deliberately and answer the new lints in the same commit. Note that
a `RUSTUP_TOOLCHAIN` environment variable overrides the file. A local shell
that sets one is not testing the pinned toolchain.

`rust-version` in `Cargo.toml` names the pinned toolchain rather than a lower
bound, because the pinned one is the only toolchain CI builds. Edition 2024
needs 1.85 at the least. Lower the declaration once a CI job proves that bound.

Every cargo command in the Makefile passes `--locked`. `Cargo.lock` is tracked
and a command that quietly re-resolves it would build something other than what
the lockfile describes. `cargo install` ignores the lockfile without that flag.

`publish = false` is set on purpose. This is scaffolding rather than a release.
Note also that `cargo package` collects every file git does not ignore. Add an
`exclude` list before you take that line out.

## The pre-commit hook

The hook runs `make check` against the **staged** content rather than against
the working tree. git runs a hook in the working tree and a gate that reads the
working tree can pass a commit it never saw. The hook checks the index out into
`.git/precommit` and runs there. Your working tree is never touched and a
failing gate therefore leaves nothing to clean up.

**cargo-husky** copies `.cargo-husky/hooks/pre-commit` into `.git/hooks` from a
build script. That build script runs only when the dev-dependencies compile.
`cargo test`, `cargo clippy --all-targets` and `cargo check --all-targets`
install the hook and plain `cargo build` does not. A fresh clone is therefore
ungated until one of those runs. `make check` installs it at the clippy step.

**cargo-husky does not reinstall a hook that already exists.** Editing
`.cargo-husky/hooks/pre-commit` and running `cargo test` leaves the old hook in
place. The running gate then differs from the committed one without saying so.
To install a changed hook:

```sh
cargo clean -p cargo-husky && cargo test
```

Then confirm it with `diff .git/hooks/pre-commit .cargo-husky/hooks/pre-commit`.
The installed copy carries two extra banner lines from cargo-husky.

Three states turn the gate off with no message. `git config core.hooksPath`
makes git ignore `.git/hooks` altogether. The `diff` above still reports a match
in that state and cannot detect it. `CARGO_HUSKY_DONT_INSTALL_HOOKS` in the
environment skips the install and cargo hides the warning it prints unless you
pass `-vv`. A `pre-commit` hook that some other tool wrote first also wins,
because cargo-husky leaves a foreign hook alone.

`README.md`, `AGENTS.md` and `GEMINI.md` are symbolic links to this file. Every
tool and every reader therefore gets the same document. GitHub follows the link
and renders this file as the repository README.

## Licence

MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
