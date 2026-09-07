//! The widget in a real zsh.
//!
//! `pick` runs the command the widget runs. These run the widget itself. zsh
//! holds the line, the key bindings and the plugins. surmise is only what one
//! key reaches. These tests cover the widget and the directory hook that
//! changes the next menu's order. `pick` covers the rest of what the menu draws.

use crate::term::Term;
use portable_pty::CommandBuilder;
use std::path::Path;
use std::time::Duration;
use surmise::fixture::Fixture;

/// The whole prompt the generated `.zshrc` sets. It carries no directory,
/// because the fixture's own name is a temporary one and a macOS `/var` is a
/// symbolic link that `%~` cannot fold back into a `~` anyway.
const PROMPT: &str = "❯";

/// What the `chpwd` hook prints. Nothing else on the screen says it.
const MOVED: &str = "MOVED";

/// What the `.zshrc` prints once the widget is really bound.
///
/// Five of the tests below assert that no menu opened. A widget that never
/// loaded would satisfy every one of them. This is what stops that.
const LOADED: &str = "LOADED";

/// How long the shell gets to start and how long a key gets an answer.
const WAIT: Duration = Duration::from_secs(10);

/// Long enough for a keystroke to reach the shell and the answer to be drawn.
const SETTLE: Duration = Duration::from_millis(400);

/// A home directory with the widget installed in a `.zshrc` of its own.
///
/// `before` goes in ahead of the widget where a person's own `bindkey` would.
/// `after` goes behind it. That is where `CLAUDE.md` puts the line that gives
/// the space key back. The install line between them is the one it names as
/// well. A change to what `init zsh` prints therefore reaches these tests.
fn home(before: &str, after: &str) -> Fixture {
    let f = Fixture::new(&["work", "deep"]);
    // `chpwd` is zsh's own hook for a directory change. It is how a test sees
    // that a `cd` ran rather than only that a line was accepted.
    let rc = format!(
        "PROMPT='{PROMPT} '\n\
         PROMPT_EOL_MARK=''\n\
         autoload -Uz compinit && compinit -u\n\
         chpwd() {{ print \"{MOVED}:${{PWD:t}}\" }}\n\
         {before}\n\
         eval \"$($SURMISE_BIN init zsh)\"\n\
         {after}\n\
         (( $+widgets[surmise-space] )) && print {LOADED}\n"
    );
    std::fs::write(f.path().join(".zshrc"), rc).expect("a zshrc");
    f
}

/// An interactive zsh in `home` that has drawn its prompt.
fn ready(home: &Path) -> Term {
    let mut cmd = CommandBuilder::new("/bin/zsh");
    cmd.arg("-i");
    // `-d` drops the global startup files. `ZDOTDIR` below points zsh at the
    // `.zshrc` above and that alone is what a machine can put in front of the
    // widget. Debian and Ubuntu ship an `/etc/zsh/zshrc` that runs `compinit`
    // with no flag, and on a machine whose completion directories are group
    // writable that call asks the terminal whether to continue. The question
    // arrives before the prompt does and every test here then waits for a
    // prompt that is never drawn.
    cmd.arg("-d");
    // A test machine's own environment is not the one under test. zsh reads
    // `ZDOTDIR` for the `.zshrc` above and the widget reads nothing else.
    cmd.env_clear();
    cmd.env("HOME", home);
    cmd.env("ZDOTDIR", home);
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", "/usr/bin:/bin");
    // The widget defaults to the `surmise` on the PATH. The one under test is
    // the build's own binary and this is the hook the widget documents for it.
    cmd.env("SURMISE_BIN", env!("CARGO_BIN_EXE_surmise"));
    cmd.cwd(home);
    let mut t = Term::new(cmd, 100, 30);
    assert!(t.wait_line(PROMPT, WAIT), "no prompt: {:?}", t.lines());
    assert!(
        t.lines().join("\n").contains(LOADED),
        "the widget never bound: {:?}",
        t.lines()
    );
    // Clear what the `.zshrc` printed so a test reads its own line as the
    // first one. The marker going is what says the clear landed. A wait for
    // the prompt would find the one already on the screen and read nothing.
    t.send("\x0c");
    t.pump(SETTLE);
    assert!(
        !t.lines().join("\n").contains(LOADED),
        "the screen never cleared: {:?}",
        t.lines()
    );
    t
}

/// The row the shell is editing on.
fn line(t: &Term) -> String {
    t.lines().first().cloned().unwrap_or_default()
}

/// Type `keys` and let the answer land.
fn typed(t: &mut Term, keys: &str) {
    t.send(keys);
    t.pump(SETTLE);
}

/// A shell with the menu open on a bare `cd `. That is the widget's own way in
/// and most of the claims below start from it.
fn opened(home: &Path) -> Term {
    let mut t = ready(home);
    t.send("cd ");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.pump(SETTLE);
    t
}

/// Wait for the menu to go and for the shell to have its row back.
fn closed(t: &mut Term) {
    assert!(t.wait_bare(WAIT), "the menu stayed: {:?}", t.lines());
    t.pump(SETTLE);
}

#[test]
fn typing_a_bare_cd_opens_the_menu() {
    let f = home("", "");
    let t = opened(f.path());
    // The shell's own prompt row carries the line and the menu starts below
    // it. A row of surmise's own would put the line on screen twice.
    assert!(line(&t).starts_with("❯ cd"), "{:?}", t.lines());
    assert_eq!(t.panel()[0].row, 1, "{:?}", t.lines());
}

#[test]
fn git_subcommands_return_to_editing_without_running() {
    for key in ["\r", "\t", "\x1b[C"] {
        let f = home("git() { print -r -- GIT-RAN }", "");
        let mut t = ready(f.path());
        t.send("git ");
        assert!(t.wait_panel(WAIT), "no Git menu: {:?}", t.lines());
        typed(&mut t, "stat");
        assert!(t.panel().iter().any(|row| row.text.contains("status")));
        t.send(key);
        closed(&mut t);
        assert!(line(&t).starts_with("❯ git status"), "{:?}", t.lines());
        assert!(!t.lines().join("\n").contains("GIT-RAN"));
        typed(&mut t, "--short");
        assert!(
            line(&t).starts_with("❯ git status --short"),
            "{:?}",
            t.lines()
        );
        t.send("\r");
        assert!(t.wait_line("GIT-RAN", WAIT), "{:?}", t.lines());
    }
}

#[test]
fn a_whole_subcommand_gives_the_keys_back_to_the_shell() {
    // Tab and Right accept a subcommand outside the arm Enter breaks out of.
    // Only the end of the run puts the keys back and nothing on the screen
    // says which side of that the line is on. The shell's own Tab says it:
    // surmise answers PASS on a finished word and the widget hands the key to
    // whatever held it. surmise's own Tab on a closed menu rings the bell and
    // leaves the line alone.
    for key in ["\t", "\x1b[C"] {
        let f = home(
            "sample-complete() { LBUFFER+=SAMPLE }\nzle -N sample-complete\nbindkey '^I' sample-complete",
            "",
        );
        let mut t = ready(f.path());
        t.send("git ");
        assert!(t.wait_panel(WAIT), "no Git menu: {:?}", t.lines());
        // `statu` reaches `status` and nothing else. Tab therefore takes the
        // whole of one name rather than a prefix two of them share.
        typed(&mut t, "statu");
        t.send(key);
        closed(&mut t);
        assert_eq!(line(&t).trim(), "\u{276f} git status", "{:?}", t.lines());
        typed(&mut t, "\t");
        assert_eq!(
            line(&t).trim(),
            "\u{276f} git status SAMPLE",
            "the keys never went back: {:?}",
            t.lines()
        );
    }
}

#[test]
fn git_tab_opens_the_menu_and_cancel_restores_the_seed() {
    let f = home("", "bindkey ' ' $_surmise_space");
    let mut t = ready(f.path());
    t.send("git stat\t");
    assert!(t.wait_panel(WAIT), "no Git menu: {:?}", t.lines());
    typed(&mut t, "u");
    t.send("\x03");
    closed(&mut t);
    assert_eq!(line(&t).trim(), "❯ git stat");
}

#[test]
fn git_arguments_keep_the_shells_completion() {
    let f = home(
        "sample-complete() { LBUFFER+=SAMPLE }\nzle -N sample-complete\nbindkey '^I' sample-complete",
        "bindkey ' ' $_surmise_space",
    );
    let mut t = ready(f.path());
    typed(&mut t, "git switch \t");
    assert_eq!(line(&t).trim(), "❯ git switch SAMPLE");
    assert!(t.panel().is_empty(), "a menu opened: {:?}", t.lines());
}

#[test]
fn a_word_no_subcommand_matches_keeps_the_shells_completion() {
    let f = home(
        "sample-complete() { LBUFFER+=SAMPLE }\nzle -N sample-complete\nbindkey '^I' sample-complete",
        "bindkey ' ' $_surmise_space",
    );
    let mut t = ready(f.path());
    // No subcommand holds four of one letter and surmise therefore answers
    // PASS. Tab is the shell's own again and the menu never opens.
    typed(&mut t, "git zzzz\t");
    assert_eq!(line(&t).trim(), "❯ git zzzzSAMPLE");
    assert!(t.panel().is_empty(), "a menu opened: {:?}", t.lines());
}

#[test]
fn git_alias_names_are_completed_without_running_the_alias() {
    let f = home("", "");
    std::fs::write(
        f.path().join(".gitconfig"),
        "[alias]\n sample-alias = !touch ALIAS-RAN\n",
    )
    .unwrap();
    let mut t = ready(f.path());
    t.send("git ");
    assert!(t.wait_panel(WAIT), "no Git menu: {:?}", t.lines());
    typed(&mut t, "sample-al");
    assert!(
        t.panel()
            .iter()
            .any(|row| row.text.contains("sample-alias"))
    );
    t.send("\r");
    closed(&mut t);
    assert_eq!(line(&t).trim(), "❯ git sample-alias");
    assert!(!f.path().join("ALIAS-RAN").exists());
}

#[test]
fn the_menu_narrows_as_the_line_grows() {
    let f = home("", "");
    let mut t = opened(f.path());
    typed(&mut t, "de");
    let rows = t.panel();
    let text: String = rows.iter().map(|r| r.text.as_str()).collect();
    assert!(text.contains("deep"), "{text:?}");
    assert!(!text.contains("work"), "{text:?}");
}

#[test]
fn enter_takes_the_directory_and_a_second_enter_runs_the_line() {
    let f = home("", "");
    let mut t = opened(f.path());
    typed(&mut t, "de");
    // The first press takes `deep/` and leaves the menu open on it. Asserting
    // that nothing has moved yet is what pins the two presses: a single-press
    // Enter would already have run the line here and the second press would
    // land on a fresh prompt and this test would never see it.
    typed(&mut t, "\r");
    assert!(
        !t.lines().join("\n").contains(MOVED),
        "the first press ran the line: {:?}",
        t.lines()
    );
    assert!(!t.panel().is_empty(), "the menu closed: {:?}", t.lines());
    assert!(
        !f.path()
            .join(".local/share/surmise/history.sqlite3")
            .exists()
    );
    t.send("\r");
    // The hook fires on a directory change alone. Naming `deep` is the whole
    // of the claim: the line was taken and it was run.
    assert!(
        t.wait_line(&format!("{MOVED}:deep"), WAIT),
        "the cd never ran: {:?}",
        t.lines()
    );
    t.pump(SETTLE);
    assert!(
        f.path()
            .join(".local/share/surmise/history.sqlite3")
            .exists()
    );
}

#[test]
fn escape_leaves_the_menu_and_keeps_what_was_typed() {
    let f = home("", "");
    let mut t = opened(f.path());
    typed(&mut t, "de");
    t.send("\x1b");
    closed(&mut t);
    assert!(line(&t).starts_with("❯ cd de"), "{:?}", t.lines());
    assert!(
        !f.path()
            .join(".local/share/surmise/history.sqlite3")
            .exists()
    );
}

#[test]
fn a_manual_visit_changes_the_order_in_the_next_picker() {
    let f = home("", "bindkey ' ' $_surmise_space");
    let mut t = ready(f.path());
    // The order before any visit is the alphabetical one. Without this the
    // claim below would hold just as well for a menu that never changed.
    t.send("cd \t");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.pump(SETTLE);
    assert!(t.panel()[0].text.contains("deep"), "{:?}", t.lines());
    t.send("\x15");
    closed(&mut t);
    typed(&mut t, "cd work\r");
    assert!(t.lines().join("\n").contains("MOVED:work"));
    typed(&mut t, "cd ..\r");
    typed(&mut t, "\x0c");
    t.send("cd \t");
    assert!(t.wait_panel(WAIT), "no menu: {:?}", t.lines());
    t.pump(SETTLE);
    assert!(t.panel()[0].text.contains("work"), "{:?}", t.lines());
}

#[test]
fn ctrl_c_gives_back_the_line_that_was_there() {
    let f = home("", "");
    let mut t = opened(f.path());
    typed(&mut t, "de");
    t.send("\x03");
    closed(&mut t);
    // What was typed inside the menu is gone and the line the shell had when
    // the widget was called is back.
    assert_eq!(line(&t), "❯ cd", "{:?}", t.lines());
}

#[test]
fn clearing_the_line_closes_the_menu_and_leaves_the_shell_working() {
    let f = home("", "");
    let mut t = opened(f.path());
    t.send("\x15");
    closed(&mut t);
    assert_eq!(line(&t), "❯", "{:?}", t.lines());
    // The line typed must not itself hold what the answer is checked for.
    // `echo back` would put `back` on the screen whether it ran or not.
    t.send("echo $((6*7))\r");
    assert!(
        t.wait_line("42", WAIT),
        "the shell never ran it: {:?}",
        t.lines()
    );
}

#[test]
fn a_space_that_is_not_a_bare_cd_only_inserts_a_space() {
    let f = home("", "");
    let mut t = ready(f.path());
    typed(&mut t, "echo hello world");
    assert!(t.panel().is_empty(), "a menu opened: {:?}", t.lines());
    assert_eq!(line(&t), "❯ echo hello world", "{:?}", t.lines());
}

#[test]
fn a_cd_behind_another_command_does_not_open_the_menu() {
    let f = home("", "");
    let mut t = ready(f.path());
    typed(&mut t, "echo x && cd ");
    assert!(t.panel().is_empty(), "a menu opened: {:?}", t.lines());
    assert_eq!(line(&t), "❯ echo x && cd", "{:?}", t.lines());
}

#[test]
fn tab_on_a_line_surmise_passes_on_reaches_the_shells_own_completion() {
    let f = home("", "");
    let mut t = ready(f.path());
    typed(&mut t, "ls .zsh");
    typed(&mut t, "\t");
    // `.zshrc` is the only name in the fixture that starts that way and zsh
    // completes it outright. `compinit` leaves a `.zcompdump` beside it and
    // that one starts `.zc`. surmise answered PASS and gave the key back.
    assert!(line(&t).starts_with("❯ ls .zshrc"), "{:?}", t.lines());
}

#[test]
fn vi_command_mode_still_edits_after_the_menu() {
    let f = home("bindkey -v", "");
    let mut t = opened(f.path());
    // The first Escape leaves the menu. The second is zsh's own and puts the
    // line editor into command mode, where `dd` deletes the line.
    t.send("\x1b");
    closed(&mut t);
    typed(&mut t, "\x1bdd");
    assert_eq!(line(&t), "❯", "{:?}", t.lines());
}

#[test]
fn tab_asks_surmise_about_a_line_already_typed() {
    // The widget's own comment names `bindkey ' ' $_surmise_space` as the way
    // to give the space key back and keep the Tab route. Tab is then the only
    // way in and a line can reach `cd wo` without a bare `cd ` on the way.
    let f = home("", "bindkey ' ' $_surmise_space");
    let mut t = ready(f.path());
    typed(&mut t, "cd wo");
    assert!(
        t.panel().is_empty(),
        "the space opened it anyway: {:?}",
        t.lines()
    );
    t.send("\t");
    assert!(t.wait_panel(WAIT), "Tab opened nothing: {:?}", t.lines());
    t.pump(SETTLE);
    assert!(line(&t).starts_with("❯ cd wo"), "{:?}", t.lines());
}
