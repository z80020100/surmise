//! `surmise demo` at a real terminal.
//!
//! The demo writes a `.zshrc` of its own under a throwaway directory, sends
//! `$ZDOTDIR` at it and starts zsh on the directory it was run in. Nothing
//! else moves: the config, the directory history and the state file are the
//! ones an installed surmise reads and writes. These tests are what says
//! the parts still meet: a menu that opens on a file beside the shell
//! proves the startup file was laid out, the widget bound and a native
//! reader reached the working directory the command was given. `zsh.rs`
//! covers the widget itself and `pick.rs` covers what the menu draws.
//!
//! `$HOME` is inherited rather than replaced, so every test here gives the
//! demo a home of its own. The test machine's own is not the one under test
//! and a claim about what the demo reads out of a home needs a home the test
//! wrote.

use crate::term::Term;
use portable_pty::CommandBuilder;
use std::path::Path;
use std::time::Duration;
use surmise::fixture::Fixture;

/// How long the demo gets to lay its root out and draw the first prompt. The
/// shell loads `compinit` before that prompt.
const WAIT: Duration = Duration::from_secs(30);

/// Long enough for a keystroke to reach the shell and the answer to land.
const SETTLE: Duration = Duration::from_millis(400);

/// A demo shell in `cwd` with `home` for its `$HOME`, which has drawn its
/// prompt. The prompt the demo's own `.zshrc` sets carries the last part of
/// that directory and that is what says this shell is the one the test
/// started.
fn ready(cwd: &Path, home: &Path) -> Term {
    ready_on(cwd, home, crate::term::path())
}

/// The inherited `PATH` with every directory holding a `surmise` taken out.
///
/// Cargo puts `~/.cargo/bin` in front of the `PATH` of everything it runs, so
/// an installed surmise answers a name typed in the demo and the build under
/// test is never reached. A test that means this build has to say so.
fn path_without_surmise() -> String {
    let path = crate::term::path()
        .split(':')
        .filter(|dir| !Path::new(dir).join("surmise").exists())
        .collect::<Vec<&str>>()
        .join(":");
    // A surmise installed beside zsh goes off this list with the directory
    // that holds it and the demo then fails to start at all. That failure
    // says nothing about the claim under test, so it is named here instead.
    assert!(
        path.split(':')
            .any(|dir| Path::new(dir).join("zsh").exists()),
        "no zsh left to run the demo on: {path:?}"
    );
    path
}

fn ready_on(cwd: &Path, home: &Path, path: String) -> Term {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_surmise"));
    cmd.arg("demo");
    // A test machine's own environment is not the one under test. `PATH` and
    // `$HOME` are what every test here puts back: zsh and the demo's own
    // readers both mean a name that lookup finds, and the demo reads the
    // home it is given rather than one of its own.
    cmd.env_clear();
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", path);
    cmd.env("HOME", home);
    cmd.cwd(cwd);
    let mut t = Term::new(cmd, 100, 30);
    let prompt = format!(
        "demo {}",
        cwd.file_name().expect("a name").to_string_lossy()
    );
    assert!(t.wait_line(&prompt, WAIT), "no prompt: {:?}", t.lines());
    t
}

/// Write `text` at `name` under `home`, making the directories above it.
fn in_home(home: &Path, name: &str, text: &str) {
    let path = home.join(name);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
    std::fs::write(&path, text).expect("a file");
}

/// Leave the menu, drop the line and end the demo. What the demo laid out
/// goes the way it would for a person who typed the same three keys.
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
    let home = Fixture::new(&[]);
    std::fs::write(
        f.path().join("Makefile"),
        "package:\n\t@echo package\n\nclean:\n\t@echo clean\n",
    )
    .expect("a makefile");
    let mut t = ready(f.path(), home.path());
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
fn the_demo_offers_the_hosts_of_the_persons_own_ssh_configuration() {
    // `$HOME` is the person's, so this reader answers out of the
    // configuration they keep rather than out of one the demo wrote for
    // them. The names are the test's own for the same reason every name in
    // this repository is: `.invalid` resolves nowhere.
    let f = Fixture::new(&[]);
    let home = Fixture::new(&[]);
    in_home(
        home.path(),
        ".ssh/config",
        "Host build-north\n  HostName build-north.example.invalid\n",
    );
    let mut t = ready(f.path(), home.path());
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
fn the_history_file_a_person_keeps_is_the_one_the_demo_reads() {
    // A bare `git ` matches every subcommand equally well, so the `$HISTFILE`
    // tie-break is the whole of the order. The demo sets no history file of
    // its own now and this is what says it found theirs.
    let f = Fixture::new(&[]);
    let home = Fixture::new(&[]);
    in_home(home.path(), ".zsh_history", &"git revert\n".repeat(20));
    let mut t = ready(f.path(), home.path());
    t.send("git ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    // `panel()[0]` is the line that closes the panel above the list;
    // `panel()[1]` is the first row in it, the way `zsh.rs` reads its own.
    assert!(t.panel()[1].text.contains("revert"), "{:?}", t.lines());
    leave(&mut t);
}

#[test]
fn a_folder_beside_the_shell_reaches_the_menu() {
    // The working directory is the demo's whole answer about a person's own
    // files. This is the claim that it is inherited rather than replaced.
    let f = Fixture::new(&["reachable"]);
    let home = Fixture::new(&[]);
    let mut t = ready(f.path(), home.path());
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    assert!(
        t.panel().iter().any(|row| row.text.contains("reachable")),
        "{:?}",
        t.lines()
    );
    leave(&mut t);
}

/// The nerd folder glyph and the text one.
///
/// Named here rather than imported, the way `pick.rs` names its own, because
/// what these tests read is the screen rather than the table behind it.
const NF_DIR: char = '\u{f07b}';
const TEXT_DIR: char = '▸';

#[test]
fn the_config_a_person_keeps_is_what_the_demo_opens_on() {
    // Nothing detects the font the nerd set asks for, so a person who has
    // answered that question once has answered it. A demo that opened on the
    // defaults would be showing them somebody else's surmise.
    let f = Fixture::new(&["reachable"]);
    let home = Fixture::new(&[]);
    in_home(
        home.path(),
        ".config/surmise/config.toml",
        "icons = \"nerd\"\n",
    );
    let mut t = ready(f.path(), home.path());
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    assert!(
        t.panel().iter().any(|row| row.text.contains(NF_DIR)),
        "the demo did not open on the config it was given: {:?}",
        t.lines()
    );
    leave(&mut t);
}

#[test]
fn what_the_demo_writes_reaches_the_files_the_person_keeps() {
    // The other half of the promise, over all three files the demo used to
    // copy. A `settings set` typed here is the setting they keep afterwards,
    // a `cd` run here is a visit their next menu ranks by and the answer
    // `Ctrl-O` leaves is the one their next menu opens on. A demo answering
    // any of the three out of a copy would answer nothing about it.
    let f = Fixture::new(&["reachable"]);
    let home = Fixture::new(&[]);
    in_home(
        home.path(),
        ".config/surmise/config.toml",
        "icons = \"nerd\"\n",
    );
    in_home(home.path(), ".zsh_history", "git status\n");
    // No surmise on the PATH, so the name typed below reaches this build
    // through the demo's own `.zshrc` or it reaches nothing.
    let mut t = ready_on(f.path(), home.path(), path_without_surmise());
    // The space opens the menu and Esc hands the line back with `cd ` still
    // on it. A `cd` sent in one piece would race the menu for the keys
    // behind that space, and what this test needs is a `cd` that ran rather
    // than a particular way of reaching one. `zsh.rs` covers the keys.
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.send("\x1b");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    t.pump(SETTLE);
    t.send("reachable\r");
    t.pump(SETTLE);
    t.send("surmise settings set icons text\r");
    t.pump(SETTLE);
    // `Ctrl-O` is the one key whose answer outlives the menu it was given
    // in. A subcommand menu is where it has a word to open, and `git` needs
    // no repository for one.
    t.send("git ");
    assert!(t.wait_panel(WAIT), "no subcommand menu: {:?}", t.lines());
    t.send("\x0f");
    t.pump(SETTLE);
    t.send("\x1b");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    t.pump(SETTLE);
    t.send("\x15");
    t.pump(SETTLE);
    t.send("exit\r");
    assert_eq!(t.status(WAIT), Some(0), "the demo stayed: {:?}", t.lines());

    assert_eq!(
        std::fs::read_to_string(home.path().join(".config/surmise/config.toml")).expect("it back"),
        "icons = \"text\"\n",
        "the demo did not reach the config a person keeps: {:?}",
        t.lines()
    );
    let database = home.path().join(".local/share/surmise/history.sqlite3");
    assert!(
        database.exists(),
        "the cd did not reach {}",
        database.display()
    );
    let state = home.path().join(".local/share/surmise/state.toml");
    assert!(state.exists(), "Ctrl-O did not reach {}", state.display());
    // The one file read and never written. A line typed in a demo is worth
    // nothing to the ranking afterwards and `SAVEHIST=0` is what keeps it
    // out of the history a person keeps.
    assert_eq!(
        std::fs::read_to_string(home.path().join(".zsh_history")).expect("it back"),
        "git status\n",
        "the demo wrote the history a person keeps"
    );
}

#[test]
fn the_glyph_set_the_demo_offers_reaches_the_demos_own_menu() {
    // The opening text offers that set to a person whose config does not ask
    // for it yet, so the command it names has to land where this demo reads
    // and reach the next menu out of it.
    let f = Fixture::new(&["reachable"]);
    let home = Fixture::new(&[]);
    // No surmise on the PATH, so the name the opening text offers reaches
    // this build through the demo's own `.zshrc` or it reaches nothing.
    let mut t = ready_on(f.path(), home.path(), path_without_surmise());
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    assert!(
        t.panel().iter().any(|row| row.text.contains(TEXT_DIR)),
        "the text set is not what a demo with no config opens on: {:?}",
        t.lines()
    );
    t.send("\x1b");
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    t.pump(SETTLE);
    t.send("\x15");
    t.pump(SETTLE);

    t.send("surmise settings set icons nerd\r");
    t.pump(SETTLE);
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no second menu: {:?}", t.lines());
    let after = t.panel();
    assert!(
        after.iter().any(|row| row.text.contains(NF_DIR)),
        "the write did not reach the menu: {:?}",
        t.lines()
    );
    assert!(
        !after.iter().any(|row| row.text.contains(TEXT_DIR)),
        "the text set is still there: {:?}",
        t.lines()
    );
    leave(&mut t);
}
