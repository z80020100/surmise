# surmise

Completion for the directory argument of a `cd`.

> This file also provides guidance to [Claude Code](https://claude.ai/code) when
> working with code in this repository. `README.md`, `AGENTS.md` and `GEMINI.md`
> are symbolic links to it.

**surmise installs, runs and has its reference.** The licence is settled. The
platform is not.

## Prerequisites

- Rust 1.98.0. `rust-toolchain.toml` pins it and rustup installs it on demand
- Target platforms: macOS for now. That is provisional rather than a decision.
  It is simply the only platform anything has been built on

## Install

```sh
cargo install --git https://github.com/z80020100/surmise
```

Then in `~/.zshrc`, after zsh-autosuggestions and zsh-syntax-highlighting and
after `bindkey -e` or `bindkey -v`:

```sh
eval "$(surmise init zsh)"
```

`cargo install` places a binary and has no mechanism for anything beside it.
`shell/surmise.zsh` is therefore compiled into that binary and `init` prints
it. The two cannot fall out of step, because they are one artifact. The
`make shell` gate reads the same bytes the command emits.

## Use

Two ways in. Tab asks surmise about the line you already have and falls through
to the shell's own completion when surmise has nothing to offer. The paragraphs
under the table say what narrows that. Typing a bare `cd ` opens surmise on
its own. A menu of directories opens below the line. The mark on the first
name sits under the cursor and the menu follows the cursor along the line. The
menu is one width whatever it holds. Near the right edge it keeps that width
and gives the alignment up. A terminal too narrow for that width is the one
thing that shrinks it. Seven rows show at a time and a shorter list or a
shorter terminal shows fewer. The menu holds still while the highlight has
somewhere to go inside it and follows the highlight one row at a time past
that. Up on the first name and Down on the last wrap to the other end of the
list and the menu follows in one step.

| Key | Inside the menu |
| --- | --- |
| Up and Down | Move the highlight |
| Shift-Tab | Move the highlight back |
| Tab | Take the prefix the directories in the menu share |
| Right | Take what the highlighted directory adds. At the end of the line |
| Enter | Go into the highlighted directory or run the line |
| Esc | Leave the menu and keep what you typed |
| Ctrl-C and Ctrl-G | Leave and restore the line you started with |

A line that already names a directory gets a row of its own at the top of the
menu. That row runs the line rather than growing it. A `↵` in a colour of its
own is the whole row: the line is on the screen already and a name there would
be that same text a second time. The word under the list is what says what the
row does. Enter on any other row takes that directory and leaves the menu open
on what is inside it. Two presses therefore go one level down and run the line.
Enter also runs the line when there is nothing left to take. That is what
keeps the key working once the cursor has moved off the argument.

That row is the one the highlight starts on. Any line whose argument already
names a directory therefore opens a menu rather than falling through to the
shell's own completion. A bare `cd ..` or `cd .` is such a line and the row
that runs it arrives with whatever else the dots in it match. A dot in front
of the last part of the argument also turns the hidden names on. That is the
trade the row costs.

Tab reads the directories in the menu rather than the row under the highlight
and where the highlight sits therefore does not change what it offers. That
includes the row that runs the line: Tab looks past it to the directories
below. One directory that leads with what you typed goes in whole, inside
quotes as well as outside them. Past that it takes the prefix they all share.
It leaves the line alone when that prefix adds nothing, when the shell would
not read it as a single literal word, when what they share is the whole of
one of the names, when they spell that shared part differently, when the
argument is inside quotes and when the argument ends in a space. The quote is
because half a name cannot carry the one that closes it. The space is you
saying the word is finished. Right leaves it alone for that same reason and
Enter runs the line, because a finished word leaves nothing to take.

Tab rings the terminal's bell whenever it leaves the line alone. The line is
the one that was already there and nothing on the screen would say the key
had been read at all. Whether that bell is a sound or a flash or nothing is
the terminal's own setting rather than surmise's.

A match need not lead with what you typed and Tab ignores the rows that do
not. `cd wk` reaching `work/` is a match Enter takes. Right leaves it: that
key wants a name leading with what you typed, whatever case either is in. It
is not a match Tab can build a prefix from either.

Every name marks the characters what you typed reached. The `w` and the `k`
carry a ground of their own in that row and the `or` between them does not.
That is what says how a name got into the menu.

What Tab would add is underlined in every row it would add it to. The rows
Tab passes over carry no underline and neither does a line Tab would leave
alone. The key and the underline read one answer and the menu therefore
cannot promise what the key will not do.

Everything else is ordinary line editing. Left, Home, End, Backspace, Delete
and Ctrl-A, Ctrl-E, Ctrl-U and Ctrl-K all do what they do in the shell. Ctrl-W
takes back one path segment where the shell would take the whole path.
Emptying the line leaves the menu as well and keeps the empty line.

`bindkey ' ' $_surmise_space` after the `eval` gives the space key back and
keeps the Tab route.

`SURMISE_BIN` names the binary. It defaults to the `surmise` on the PATH.
That is the one that printed the widget when `init` was run from there.

## Build, test and lint

```sh
make build                  # cargo build --locked
make release                # cargo build --locked --release
make check                  # the gate: fmt-check, clippy, test, then shell
make install                # cargo install --locked --path .
make uninstall              # cargo uninstall surmise
make clean                  # cargo clean
```

`make check` is the gate and the pre-commit hook runs it. CI calls the same
Makefile targets as separate steps so each one gets its own result in the
GitHub interface and the commands keep a single definition. CI then runs
`make release` as a fifth step. The gate does not cover that step and a green
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

The crate has a library target as well as a binary target. `src/lib.rs` holds
every module. There are two reasons. The first is the tests: `cargo test`
reaches a library and a module under a binary is testable only from inside
itself. The second is the gate. `dead_code` is a warning and `pub` does not
exempt an item in a binary crate, because nothing outside that crate can reach
it. A module that lands before its caller therefore turns the gate red. A
library target makes the same items reachable.

The library target also puts the library's `//!` and `///` code fences into the
gate. `cargo test` compiles each one as Rust unless the fence is tagged `text`
or `ignore`. An example that does not build therefore turns the gate red. A
fence in `src/main.rs` is not collected, because doc-tests come from the
library alone.

`tests/` holds the integration tests. They run inside a pty and read the
screen the program draws rather than the bytes it wrote. A byte stream can
carry a box-drawing character and still render as garbage. `vt100` does the
rendering and `portable-pty` opens the device. Both are dev-dependencies and
neither reaches the installed binary.

The program runs inside that pty rather than beside it. crossterm resolves
`/dev/tty` for raw mode rather than reading stdin. A child spawned any other
way would therefore put the terminal the suite was started from into raw mode.

`tests/pty/main.rs` is the only test binary and its siblings are its modules.
`term` is the harness, `pick` runs `surmise --pick LINE` and `zsh` runs the
widget in a real `zsh -i`. cargo makes a target of `tests/<name>/main.rs` as
well as of a file directly under `tests/`. The second form would compile
`term` again for each one. `dead_code` counts the methods a binary never calls
and turns the gate red. One binary sees every caller the harness has and the
harness's own `#[cfg(test)]` tests still run in it.

`pick` covers what surmise draws. `zsh` covers the widget that reaches it.
That widget has no other gate: `make shell` reads its syntax alone. The `zsh`
tests write a `.zshrc` of their own and install the widget through
`eval "$($SURMISE_BIN init zsh)"`. A change to what `init zsh` prints therefore
reaches them. `SURMISE_BIN` points the widget at the build's own binary and
`/bin/zsh` is the shell they run. macOS ships that.

That `.zshrc` prints a marker once `surmise-space` is bound and every `zsh`
test asserts it. Three of them claim that no menu opened and a widget that
never loaded would satisfy all three. It also sets a `chpwd` hook, because the
claim that Enter *ran* the line needs something the shell says on a directory
change rather than on an accepted line.

Two of the prototype's zsh cases are not here. The first loaded
zsh-autosuggestions and zsh-syntax-highlighting. The widget's own comment names
that pair as the reason it hangs the trigger off the space key rather than off
`self-insert`. Neither plugin ships with macOS and a test that skips itself
when they are absent checks nothing. The second measured how long the menu
takes to open. That is a number rather than a claim and the gate has no place
for it.

`src/fixture.rs` is `pub` rather than `#[cfg(test)]`. An integration test links
the library as an ordinary crate and a `#[cfg(test)]` module is not compiled
into that build.

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

The shell files have a gate of their own and `make shell` runs it as the last
step of `make check`. The one POSIX script gets `shellcheck` and
`shfmt -i 2 -d`. The indentation flag matches what that script already uses.
The zsh widgets get `zsh -n` and nothing else. Neither shellcheck nor shfmt
has a zsh dialect. `# shellcheck shell=zsh` is SC1103 and shellcheck then
guesses bash. It reports SC2296 and SC2298 against a nested parameter
expansion that is ordinary zsh. It reports SC2086 and SC2076 against a shell
that splits no unquoted expansion and reads a quoted `=~` as a regex anyway.
shfmt parses a widget and then asks for the bash `case` layout it does not
use. Neither tool therefore says anything true about a widget and the gate
reads its syntax alone. `zsh -n` takes only its first file argument and each widget therefore
gets a run of its own.

A missing `shellcheck` or `shfmt` fails that gate rather than skipping it.
`brew install shellcheck shfmt` is the fix and CI installs them the same way.
zsh ships with macOS and needs no install. A gate that quietly checks nothing
is worse than no gate and `make shell` therefore says so when the widget glob
matches nothing.

`shellcheck` and `shfmt` float rather than pin. A new release of either can
turn CI red with no change to the tree and that is the failure
`rust-toolchain.toml` exists to prevent. Homebrew has no clean way to pin a
formula and the shell surface here is one widget and one hook. Answer the new
finding when it lands rather than pin against it.

`README.md`, `AGENTS.md` and `GEMINI.md` are symbolic links to this file. Every
tool and every reader therefore gets the same document. GitHub follows the link
and renders this file as the repository README.

## Licence

MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
