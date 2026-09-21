//! `surmise demo` at a real terminal.
//!
//! The demo builds its own home, writes its own `.zshrc` and starts zsh on
//! them. These tests are what says the three still meet: a menu that opens
//! on the demo's own files proves the tree was laid out, the widget bound
//! and a native reader found what the banner promises. `zsh.rs` covers the
//! widget itself and `pick.rs` covers what the menu draws.

use crate::term::Term;
use portable_pty::CommandBuilder;
use std::time::Duration;

/// How long the demo gets to build its home and draw the first prompt. It
/// runs Git half a dozen times and the shell then loads `compinit`.
const WAIT: Duration = Duration::from_secs(30);

/// Long enough for a keystroke to reach the shell and the answer to land.
const SETTLE: Duration = Duration::from_millis(400);

/// The prompt the demo's own `.zshrc` sets, in the directory it starts in.
const PROMPT: &str = "demo repo";

/// A demo shell that has drawn its prompt.
fn ready() -> Term {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_surmise"));
    cmd.arg("demo");
    // A test machine's own environment is not the one under test and the
    // demo replaces the variables it cares about anyway. `PATH` is the
    // exception every test here makes: zsh, Git and the demo's own reader
    // all mean a name this lookup finds.
    cmd.env_clear();
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", crate::term::path());
    let mut t = Term::new(cmd, 100, 30);
    assert!(t.wait_line(PROMPT, WAIT), "no prompt: {:?}", t.lines());
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
fn the_demo_offers_the_targets_of_the_makefile_it_wrote() {
    let mut t = ready();
    t.send("make ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    let rows = t.panel();
    assert!(
        rows.iter().any(|row| row.text.contains("package")),
        "{:?}",
        t.lines()
    );
    leave(&mut t);
}

#[test]
fn the_demo_offers_the_hosts_of_the_ssh_configuration_it_wrote() {
    let mut t = ready();
    t.send("ssh ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    let rows = t.panel();
    assert!(
        rows.iter().any(|row| row.text.contains("build-north")),
        "{:?}",
        t.lines()
    );
    leave(&mut t);
}
