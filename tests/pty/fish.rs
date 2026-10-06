//! The widget in a real fish.
//!
//! `zsh` and `bash` cover the other two widgets. These run the fish one in an
//! interactive fish. The first `fish` on the `PATH` is the one under test and
//! a machine without one fails rather than skips. `brew install fish` or
//! `apt-get install fish` is where it comes from.

use crate::term::Term;
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use surmise::fixture::Fixture;

const PROMPT: &str = "»";

/// What the `config.fish` prints once the widget is really bound.
const LOADED: &str = "LOADED";

const WAIT: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_millis(400);

/// The first `fish` on the `PATH`.
fn fish() -> PathBuf {
    std::env::split_paths(&crate::term::path())
        .map(|dir| dir.join("fish"))
        .find(|path| path.is_file())
        .expect("a fish on the PATH. brew install fish or apt-get install fish")
}

/// A home holding a `config.fish` that installs the widget the way
/// `CLAUDE.md` says to. The prompt is a plain one and the greeting says
/// nothing, so the first row is the line. An autosuggestion would draw the
/// rest of a word Tab never put there and is off.
fn home() -> Fixture {
    let f = Fixture::new(&["work", "deep", ".config/fish"]);
    let rc = format!(
        "function fish_prompt; echo -n '{PROMPT} '; end\n\
         function fish_greeting; end\n\
         set -g fish_autosuggestion_enabled 0\n\
         $SURMISE_BIN init fish | source\n\
         set -q _surmise_tag; and echo {LOADED}\n"
    );
    std::fs::write(f.path().join(".config/fish/config.fish"), rc).expect("a config.fish");
    f
}

fn ready(home: &Path) -> Term {
    let mut cmd = CommandBuilder::new(fish());
    cmd.arg("-i");
    cmd.env_clear();
    cmd.env("HOME", home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("XDG_DATA_HOME", home.join(".local/share"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", crate::term::path());
    cmd.env("SURMISE_BIN", env!("CARGO_BIN_EXE_surmise"));
    cmd.cwd(home);
    let mut t = Term::new(cmd, 100, 30);
    assert!(t.wait_line(PROMPT, WAIT), "no prompt: {:?}", t.lines());
    assert!(
        t.lines().join("\n").contains(LOADED),
        "the widget never bound: {:?}",
        t.lines()
    );
    t.send("\x0c");
    t.pump(SETTLE);
    t
}

fn typed(t: &mut Term, keys: &str) {
    t.send(keys);
    t.pump(SETTLE);
}

/// The row holding the prompt, wherever the screen put it.
fn prompt_row(t: &Term) -> String {
    t.lines()
        .into_iter()
        .find(|row| row.starts_with(PROMPT))
        .unwrap_or_default()
}

#[test]
fn a_space_after_cd_opens_the_menu_and_enter_takes_a_folder() {
    let f = home();
    let mut t = ready(f.path());
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.pump(SETTLE);
    typed(&mut t, "\r");
    typed(&mut t, "\x1b");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    t.pump(SETTLE);
    assert!(
        prompt_row(&t).starts_with(&format!("{PROMPT} cd deep/")),
        "{:?}",
        t.lines()
    );
}

#[test]
fn tab_on_a_line_surmise_leaves_alone_is_fish_own_completion() {
    // One word is a command name still being typed and surmise passes it.
    let f = home();
    let mut t = ready(f.path());
    typed(&mut t, "ech\t");
    assert!(
        prompt_row(&t).starts_with(&format!("{PROMPT} echo")),
        "{:?}",
        t.lines()
    );
}

#[test]
fn enter_on_the_row_that_runs_the_line_runs_it() {
    let f = home();
    let mut t = ready(f.path());
    t.send("cd deep\t");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.pump(SETTLE);
    typed(&mut t, "\r");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    typed(&mut t, "pwd\r");
    assert!(t.lines().join("\n").contains("/deep"), "{:?}", t.lines());
}

#[test]
fn a_change_of_directory_is_recorded() {
    let f = home();
    let out = Command::new(fish())
        .args([
            "--no-config",
            "-c",
            "$SURMISE_BIN init fish | source 2>/dev/null; cd deep",
        ])
        .env_clear()
        .env("HOME", f.path())
        .env("XDG_DATA_HOME", f.path().join("data"))
        .env("PATH", crate::term::path())
        .env("SURMISE_BIN", env!("CARGO_BIN_EXE_surmise"))
        .current_dir(f.path())
        .output()
        .expect("fish runs");
    assert!(out.status.success(), "{out:?}");
    assert!(
        f.path().join("data/surmise/history.sqlite3").is_file(),
        "{out:?}"
    );
}
