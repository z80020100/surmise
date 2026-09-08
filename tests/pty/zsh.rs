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
    cmd.env("PATH", crate::term::path());
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
fn the_line_surmise_writes_gets_a_suggestion_of_its_own() {
    // zsh-autosuggestions asks for a suggestion after a widget it wrapped
    // runs. A surmise it never wrapped has to ask itself and these two stand
    // in for the widgets its README names. What `autosuggest-fetch` reads is
    // the line surmise wrote. A call before the write would show the old one.
    //
    // The clear leaves a word of its own and the fetch adds to what it finds.
    // Both calls are therefore in the answer and so is the order they ran in.
    // A fetch that ran alone would read the same line and say nothing of the
    // suggestion left on the screen while an asynchronous one is still out.
    let f = home(
        "autosuggest-clear() { POSTDISPLAY=CLEARED }\nautosuggest-fetch() { POSTDISPLAY=\"$POSTDISPLAY<$BUFFER>\" }\nzle -N autosuggest-clear\nzle -N autosuggest-fetch",
        "",
    );
    let mut t = ready(f.path());
    t.send("git ");
    assert!(t.wait_panel(WAIT), "no Git menu: {:?}", t.lines());
    typed(&mut t, "stat");
    t.send("\r");
    closed(&mut t);
    assert_eq!(
        line(&t).trim(),
        "❯ git status CLEARED<git status >",
        "{:?}",
        t.lines()
    );
}

#[test]
fn the_line_that_runs_also_gets_its_suggestion_asked_for() {
    // The arm above accepts the line. This one hands it to the shell and the
    // suggestion goes with the prompt it was drawn on. A file is what outlasts
    // that. `cd ..` opens on the row that runs the line and Enter there is the
    // whole of the arm. Without this a lost call on that arm keeps the suite
    // green. The row it would have drawn on is already gone by then.
    let f = home(
        "autosuggest-clear() { : }\nautosuggest-fetch() { print -r -- $BUFFER >> $HOME/FETCHED }\nzle -N autosuggest-clear\nzle -N autosuggest-fetch",
        "",
    );
    let mut t = opened(f.path());
    typed(&mut t, "..");
    t.send("\r");
    assert!(
        t.wait_line(MOVED, WAIT),
        "the cd never ran: {:?}",
        t.lines()
    );
    t.pump(SETTLE);
    let got = std::fs::read_to_string(f.path().join("FETCHED")).expect("a fetch");
    assert_eq!(got.trim(), "cd ..", "{got:?}");
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
fn git_branch_arguments_outside_a_repository_keep_the_shells_completion() {
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
fn git_branch_acceptance_returns_to_editing_for_enter_tab_and_right() {
    for subcommand in ["switch", "checkout"] {
        for key in ["\r", "\t", "\x1b[C"] {
            let f = home(
                "git() { print -r -- GIT-RAN }\nsample-complete() { LBUFFER+=SAMPLE }\nzle -N sample-complete\nbindkey '^I' sample-complete",
                "bindkey ' ' $_surmise_space",
            );
            f.init_git(&["sample/topic"]);
            let mut t = ready(f.path());
            t.send(&format!("git {subcommand} sample/t\t"));
            assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
            t.pump(SETTLE);
            assert!(
                t.panel()
                    .iter()
                    .any(|row| row.text.contains("sample/topic"))
            );
            t.send(key);
            closed(&mut t);
            assert_eq!(line(&t).trim(), format!("❯ git {subcommand} sample/topic"));
            assert!(!t.lines().join("\n").contains("GIT-RAN"));
            typed(&mut t, "\t");
            assert_eq!(
                line(&t).trim(),
                format!("❯ git {subcommand} sample/topic SAMPLE")
            );
        }
    }
}

#[test]
fn accepting_switch_or_checkout_opens_the_branch_menu() {
    for (prefix, key) in [("swit", "\r"), ("checko", "\t"), ("swit", "\x1b[C")] {
        let f = home("git() { print -r -- GIT-RAN }", "");
        f.init_git(&["sample/topic"]);
        let mut t = ready(f.path());
        t.send("git ");
        assert!(t.wait_panel(WAIT), "no command menu: {:?}", t.lines());
        typed(&mut t, prefix);
        typed(&mut t, key);
        assert!(
            t.panel()
                .iter()
                .any(|row| row.text.contains("sample/topic")),
            "no branch menu: {:?}",
            t.lines()
        );
        assert!(!t.lines().join("\n").contains("GIT-RAN"));
    }
}

#[test]
fn a_space_after_switch_or_checkout_opens_branches() {
    for subcommand in ["switch", "checkout"] {
        let f = home("", "");
        f.init_git(&["sample/topic"]);
        let mut t = ready(f.path());
        t.send("git ");
        assert!(t.wait_panel(WAIT), "no command menu: {:?}", t.lines());
        t.send("\x1b");
        closed(&mut t);
        t.send(&format!("{subcommand} "));
        assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
        t.pump(SETTLE);
        assert!(
            t.panel()
                .iter()
                .any(|row| row.text.contains("sample/topic"))
        );
    }
}

#[test]
fn git_branch_cancel_restores_the_seed_and_escape_keeps_edits() {
    for (key, expected) in [
        ("\x03", "sample/"),
        ("\x07", "sample/"),
        ("\x1b", "sample/t"),
    ] {
        let f = home("", "bindkey ' ' $_surmise_space");
        f.init_git(&["sample/topic"]);
        let mut t = ready(f.path());
        t.send("git switch sample/\t");
        assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
        typed(&mut t, "t");
        t.send(key);
        closed(&mut t);
        assert_eq!(line(&t).trim(), format!("❯ git switch {expected}"));
    }
}

#[test]
fn unsupported_git_arguments_still_use_shell_completion_in_a_repository() {
    let f = home(
        "sample-complete() { LBUFFER+=SAMPLE }\nzle -N sample-complete\nbindkey '^I' sample-complete",
        "bindkey ' ' $_surmise_space",
    );
    f.init_git(&["sample/topic"]);
    let mut t = ready(f.path());
    for seed in [
        "git switch -c ",
        "git checkout -- ",
        "git switch zzzz",
        "git checkout sample/topic ",
        "git status ",
        "git -C work switch ",
    ] {
        typed(&mut t, &format!("{seed}\t"));
        assert_eq!(line(&t).trim(), format!("❯ {seed}SAMPLE"));
        assert!(t.panel().is_empty(), "a menu opened: {:?}", t.lines());
        typed(&mut t, "\x15");
    }
}

#[test]
fn git_remote_branch_candidates_follow_guess_and_default_remote_settings() {
    let f = home("", "bindkey ' ' $_surmise_space");
    f.init_git(&[]);
    for remote in ["sample-a", "sample-b"] {
        f.git(&[
            "config",
            &format!("remote.{remote}.fetch"),
            &format!("+refs/heads/*:refs/remotes/{remote}/*"),
        ]);
        f.git(&[
            "update-ref",
            &format!("refs/remotes/{remote}/sample/shared"),
            "HEAD",
        ]);
    }
    f.git(&["update-ref", "refs/remotes/sample-a/sample/unique", "HEAD"]);
    f.git(&[
        "symbolic-ref",
        "refs/remotes/sample-a/HEAD",
        "refs/remotes/sample-a/sample/unique",
    ]);
    let mut t = ready(f.path());
    for (guess, preferred, shared, unique) in [
        ("true", "missing", false, true),
        ("true", "sample-b", true, true),
        ("false", "sample-b", false, false),
    ] {
        f.git(&["config", "checkout.guess", guess]);
        f.git(&["config", "checkout.defaultRemote", preferred]);
        t.send("git switch \t");
        assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
        t.pump(SETTLE);
        let rows: String = t.panel().iter().map(|row| row.text.as_str()).collect();
        assert!(rows.contains("sample-main"), "{rows:?}");
        assert_eq!(rows.contains("sample/shared"), shared, "{rows:?}");
        assert_eq!(rows.contains("sample/unique"), unique, "{rows:?}");
        assert!(!rows.contains("HEAD"), "{rows:?}");
        t.send("\x1b");
        closed(&mut t);
        typed(&mut t, "\x15");
    }
}

#[test]
fn git_branch_names_reach_the_shell_as_one_literal_argument() {
    let f = home("", "bindkey ' ' $_surmise_space");
    f.init_git(&["sample$(false)'suffix"]);
    let mut t = ready(f.path());
    t.send("git switch suffix\t");
    assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
    t.send("\r");
    closed(&mut t);
    assert_eq!(line(&t).trim(), "❯ git switch 'sample$(false)'\\''suffix'");
    assert_eq!(f.git(&["symbolic-ref", "--short", "HEAD"]), "sample-main");
    t.send("\r");
    t.pump(SETTLE);
    assert_eq!(
        f.git(&["symbolic-ref", "--short", "HEAD"]),
        "sample$(false)'suffix"
    );
}

#[test]
fn git_branch_lists_stay_fixed_until_the_next_menu() {
    let f = home("", "bindkey ' ' $_surmise_space");
    f.init_git(&["sample/topic"]);
    let mut t = ready(f.path());
    t.send("git switch \t");
    assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
    f.git(&["branch", "sample/new"]);
    typed(&mut t, "sample/");
    assert!(
        t.panel()
            .iter()
            .any(|row| row.text.contains("sample/topic"))
    );
    assert!(!t.panel().iter().any(|row| row.text.contains("sample/new")));
    t.send("\x1b");
    closed(&mut t);
    t.send("\t");
    assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
    t.pump(SETTLE);
    assert!(t.panel().iter().any(|row| row.text.contains("sample/new")));
}

#[test]
fn enter_after_leaving_the_branch_argument_does_not_run_or_replace_the_command() {
    let f = home(
        "git() { print -r -- GIT-RAN }",
        "bindkey ' ' $_surmise_space",
    );
    f.init_git(&["sample/topic"]);
    let mut t = ready(f.path());
    t.send("git switch sample/t\t");
    assert!(t.wait_panel(WAIT), "no branch menu: {:?}", t.lines());
    typed(&mut t, &"\x1b[D".repeat(" sample/t".len()));
    t.send("\r");
    closed(&mut t);
    assert_eq!(line(&t).trim(), "❯ git switch sample/t");
    assert!(!t.lines().join("\n").contains("GIT-RAN"));
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
