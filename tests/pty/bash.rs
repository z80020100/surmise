//! The widget in a real bash.
//!
//! `zsh` covers the zsh widget. These run the bash one in an interactive bash
//! 4.4 or later. The first `bash` on the `PATH` is the one under test, and an
//! older one is a machine this suite cannot speak for and fails rather than
//! skips. macOS ships bash 3.2 and `brew install bash` puts a newer one first.

use crate::term::Term;
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use surmise::fixture::Fixture;

const PROMPT: &str = "»";

/// What the `.bashrc` prints once the widget is really bound.
const LOADED: &str = "LOADED";

const WAIT: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_millis(400);

/// The first `bash` on the `PATH`, once it says it is new enough.
fn bash() -> PathBuf {
    let found = std::env::split_paths(&crate::term::path())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .expect("a bash on the PATH");
    let version = Command::new(&found)
        .args(["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"])
        .output()
        .expect("bash runs");
    let said = String::from_utf8_lossy(&version.stdout);
    let mut parts = said
        .split_whitespace()
        .map(|n| n.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    assert!(
        (major, minor) >= (4, 4),
        "{} is bash {major}.{minor} and the widget needs 4.4 or later. brew install bash",
        found.display()
    );
    found
}

/// A home holding a `.bashrc` that installs the widget the way `CLAUDE.md`
/// says to.
fn home() -> Fixture {
    let f = Fixture::new(&["work", "deep"]);
    let rc = format!(
        "PS1='{PROMPT} '\n\
         eval \"$($SURMISE_BIN init bash)\"\n\
         [[ -n $_surmise_tag ]] && echo {LOADED}\n"
    );
    std::fs::write(f.path().join(".bashrc"), rc).expect("a bashrc");
    f
}

/// An interactive bash in `home` that has drawn its prompt.
fn ready(home: &Path) -> Term {
    let mut cmd = CommandBuilder::new(bash());
    cmd.args(["--noprofile", "--rcfile"]);
    cmd.arg(home.join(".bashrc"));
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

fn screen(t: &Term) -> String {
    t.lines().join("\n")
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
        t.lines()[0].starts_with(&format!("{PROMPT} cd deep/")),
        "{:?}",
        t.lines()
    );
}

#[test]
fn tab_on_a_line_surmise_leaves_alone_is_bash_own_completion() {
    // One word is a command name still being typed and surmise passes it.
    let f = home();
    let mut t = ready(f.path());
    typed(&mut t, "ech\t");
    assert!(
        t.lines()[0].starts_with(&format!("{PROMPT} echo")),
        "{:?}",
        t.lines()
    );
}

#[test]
fn enter_on_the_row_that_runs_the_line_runs_it() {
    let f = home();
    let mut t = ready(f.path());
    // `cd deep` names a folder and the row that runs it leads the menu.
    t.send("cd deep\t");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.pump(SETTLE);
    typed(&mut t, "\r");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    typed(&mut t, "pwd\r");
    assert!(screen(&t).contains("/deep"), "{:?}", t.lines());
}

#[test]
fn the_prompt_hook_records_a_change_of_directory() {
    let f = home();
    let out = Command::new(bash())
        .args([
            "-c",
            "eval \"$($SURMISE_BIN init bash)\" 2>/dev/null\ncd deep\n_surmise_prompt",
        ])
        .env_clear()
        .env("HOME", f.path())
        .env("XDG_DATA_HOME", f.path().join("data"))
        .env("PATH", crate::term::path())
        .env("SURMISE_BIN", env!("CARGO_BIN_EXE_surmise"))
        .current_dir(f.path())
        .output()
        .expect("bash runs");
    assert!(out.status.success(), "{out:?}");
    assert!(
        f.path().join("data/surmise/history.sqlite3").is_file(),
        "{out:?}"
    );
}
