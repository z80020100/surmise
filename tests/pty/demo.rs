//! `surmise demo` at a real terminal.
//!
//! The demo builds a home of its own, writes its own `.zshrc` and starts
//! zsh on the directory it was run in. These tests are what says the three
//! still meet: a menu that opens on a file beside the shell proves the home
//! was laid out, the widget bound and a native reader reached the working
//! directory the command was given. `zsh.rs` covers the widget itself and
//! `pick.rs` covers what the menu draws.

use crate::term::Term;
use portable_pty::CommandBuilder;
use std::path::Path;
use std::time::Duration;
use surmise::fixture::Fixture;

/// How long the demo gets to write its home and draw the first prompt. The
/// shell loads `compinit` before that prompt.
const WAIT: Duration = Duration::from_secs(30);

/// Long enough for a keystroke to reach the shell and the answer to land.
const SETTLE: Duration = Duration::from_millis(400);

/// A demo shell in `cwd` that has drawn its prompt. The prompt the demo's
/// own `.zshrc` sets carries the last part of that directory and that is
/// what says this shell is the one the test started.
fn ready(cwd: &Path) -> Term {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_surmise"));
    cmd.arg("demo");
    // A test machine's own environment is not the one under test and the
    // demo replaces the variables it cares about anyway. `PATH` is the
    // exception every test here makes: zsh and the demo's own readers both
    // mean a name this lookup finds.
    cmd.env_clear();
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", crate::term::path());
    cmd.cwd(cwd);
    let mut t = Term::new(cmd, 100, 30);
    let prompt = format!(
        "demo {}",
        cwd.file_name().expect("a name").to_string_lossy()
    );
    assert!(t.wait_line(&prompt, WAIT), "no prompt: {:?}", t.lines());
    t
}

/// Leave the menu, drop the line and end the demo. The home it made goes
/// the way it would for a person who typed the same three keys.
fn leave(t: &mut Term) {
    // The picker reads the terminal itself and the shell has the keys back
    // only once it has erased its frame. A key sent in between lands in
    // neither.
    t.send("\x1b");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    t.pump(SETTLE);
    t.send("\x15");
    t.pump(SETTLE);
    t.send("exit\r");
    assert_eq!(t.status(WAIT), Some(0), "the demo stayed: {:?}", t.lines());
}

#[test]
fn the_demo_reads_the_makefile_of_the_directory_it_was_run_in() {
    let f = Fixture::new(&[]);
    std::fs::write(
        f.path().join("Makefile"),
        "package:\n\t@echo package\n\nclean:\n\t@echo clean\n",
    )
    .expect("a makefile");
    let mut t = ready(f.path());
    t.send("make ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    assert!(
        t.panel().iter().any(|row| row.text.contains("package")),
        "{:?}",
        t.lines()
    );
    leave(&mut t);
}

#[test]
fn the_demo_offers_the_hosts_of_the_ssh_configuration_it_wrote() {
    // `$HOME` moves with the demo. This therefore answers out of the home
    // it made rather than out of the one the test machine has.
    let f = Fixture::new(&[]);
    let mut t = ready(f.path());
    t.send("ssh ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    assert!(
        t.panel().iter().any(|row| row.text.contains("build-north")),
        "{:?}",
        t.lines()
    );
    leave(&mut t);
}

#[test]
fn a_folder_beside_the_shell_reaches_the_menu() {
    // The working directory is the demo's whole answer about a person's own
    // files. This is the claim that it is inherited rather than replaced.
    let f = Fixture::new(&["reachable"]);
    let mut t = ready(f.path());
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    assert!(
        t.panel().iter().any(|row| row.text.contains("reachable")),
        "{:?}",
        t.lines()
    );
    leave(&mut t);
}
