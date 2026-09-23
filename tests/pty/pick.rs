//! surmise drawing on a real terminal.
//!
//! The tests under `src/` reach the picker's state directly. These run the
//! command a shell widget runs — `surmise --pick LINE` — inside a pty and read
//! the screen a person would be looking at.

use crate::term::{Panel, Term};
use portable_pty::CommandBuilder;
use std::path::Path;
use std::time::Duration;
use surmise::fixture::Fixture;
use surmise::pick;

/// The glyph surmise puts on a directory row.
const ICON: char = '▸';

/// The glyph on the home shortcut. A directory row's colour with a shape of
/// its own.
const HOME_ICON: char = '~';

/// The glyph on the row that runs the line.
const RUN_ICON: char = '\u{21b5}';

/// The glyph on a Git subcommand row.
const CMD_ICON: char = '$';

/// The glyph on a file row in the text set.
const FILE_ICON: char = '=';

/// Three glyphs from the set `icons = "nerd"` asks for: a folder, a `.rs`
/// file and a `.md` file. They are named here rather than imported, the same
/// way the five above are, because what this file checks is the screen a
/// person looks at rather than the table that wrote it.
const NF_DIR: char = '\u{f07b}';
const NF_RUST: char = '\u{e7a8}';
const NF_MARKDOWN: char = '\u{e73e}';

/// How long a run gets to draw and how long it gets to exit. Both are far past
/// what the work takes and neither is a measurement.
const WAIT: Duration = Duration::from_secs(5);

/// Long enough for a keystroke to be read and the answer drawn.
const SETTLE: Duration = Duration::from_millis(250);

/// A directory tree with a written-out shell history in it. surmise must never
/// read that history and the tests below say so.
fn fixture() -> Fixture {
    let f = Fixture::new(&[
        "work/alpha",
        "work/beta",
        "deep/nested/target",
        "my docs/inner",
    ]);
    let history = "\
: 1700000000:0;cd work/alpha
: 1700000001:0;cd work/beta
: 1700000002:0;cd deep/nested/target
";
    std::fs::write(f.path().join(".zsh_history"), history).expect("a history");
    f
}

/// The picker over `line`, in `home`, on a terminal of the given size.
fn surmise(home: &Path, line: &str, cols: u16, rows: u16) -> Term {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_surmise"));
    cmd.args(["--pick", line]);
    // A test machine's own environment is not the one under test.
    cmd.env_clear();
    cmd.env("HOME", home);
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", crate::term::path());
    cmd.cwd(home);
    Term::new(cmd, cols, rows)
}

/// Write `text` as the config the picker reads. The harness gives the child a
/// `HOME` of its own and clears `XDG_CONFIG_HOME`, so this is the path
/// `config::path` resolves to.
fn config(home: &Path, text: &str) {
    let dir = home.join(".config").join("surmise");
    std::fs::create_dir_all(&dir).expect("a config directory");
    std::fs::write(dir.join("config.toml"), text).expect("a config file");
}

/// The picker over `line` on a terminal wide enough for anything.
fn opened(home: &Path, line: &str) -> Term {
    let mut t = surmise(home, line, 100, 30);
    assert!(t.wait_panel(WAIT), "nothing was drawn");
    // The first painted cell is not the whole frame. A read can land in the
    // middle of one and the panel would then be short a row.
    t.pump(SETTLE);
    t
}

/// Whether `row` carries a candidate name. The line under the list is what
/// ends the names where the list has one and a blank row is no name either.
/// A rule is what a row opens with rather than what the whole of it holds:
/// the top edge carries the position in the list and the one under the list
/// carries the key that opens the word below. Both readers below skip the top
/// edge rather than tell it apart.
fn is_name(row: &Panel) -> bool {
    let text = row.text.trim();
    !text.is_empty() && !text.starts_with('─')
}

/// The rows the list holds, drawn as they were drawn. The rule that closes
/// the panel's top edge comes first and the first row that is not a name is
/// where the names stop: the rule under the list, or the end of the list's
/// own box where the word sits beside it. This is the one place that knows
/// where the list begins and where it ends.
fn name_rows(t: &Term) -> Vec<String> {
    t.panel()
        .iter()
        .skip(1)
        .take_while(|row| is_name(row))
        .map(|row| row.text.clone())
        .collect()
}

/// The candidate names, without a row glyph or its padding.
///
/// The glyph is read off by position rather than by character: the home
/// shortcut's own name is `~`, the same character its glyph now is, and a
/// blind replace would strip the name along with the glyph that precedes it.
/// One character is what a glyph measures and `every_glyph_is_one_cell_wide`
/// in `src/icons.rs` is what keeps that true.
fn names(t: &Term) -> Vec<String> {
    name_rows(t)
        .iter()
        .map(|text| {
            let mut chars = text.trim_start().chars();
            chars.next();
            chars.as_str().trim().to_string()
        })
        .collect()
}

/// The word under the list, joined back into the sentence it is. The rule
/// under the list is where the names stop and the key on that rule can give
/// the word every row below it. This reads every row past the rule rather
/// than the panel's last one. A row broken at a space joins back to exactly
/// what went in. A highlighted name too wide for its own row draws above
/// the word and no test that reads this has one.
fn footer(t: &Term) -> String {
    let panel = t.panel();
    let rule = panel
        .iter()
        .skip(1)
        .position(|row| !is_name(row))
        .expect("the rule under the list");
    panel[rule + 2..]
        .iter()
        .map(|row| row.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The word the key put beside the list, joined back into the sentence it
/// is. It has a box of its own there and the first row of that box is the
/// edge carrying the key. A row broken at a space joins back to exactly what
/// went in.
fn beside(t: &Term) -> String {
    t.detail()
        .iter()
        .skip(1)
        .map(|row| row.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The panel's top edge. It carries the position in the list.
fn edge(t: &Term) -> String {
    t.panel().first().expect("a top edge").text.clone()
}

/// The whole screen as one string. The picker's own line is in there.
fn shown(t: &Term) -> String {
    t.lines().join("\n")
}

/// Whether the panel is one closed box inside the terminal.
///
/// Row length alone proves nothing. The emulator wraps an over-wide row onto
/// the next one. Every row then still measures within the terminal while the
/// panel itself is torn in half.
fn intact(panel: &[Panel], cols: u16) -> Result<(), String> {
    if panel.len() < 2 {
        return Err(format!("{} panel rows", panel.len()));
    }
    let lefts: Vec<u16> = dedup(panel.iter().map(|r| r.lo));
    let rights: Vec<u16> = dedup(panel.iter().map(|r| r.hi));
    if lefts.len() != 1 {
        return Err(format!("rows disagree on the left edge: {lefts:?}"));
    }
    if rights.len() != 1 {
        return Err(format!("rows disagree on the right edge: {rights:?}"));
    }
    let rows: Vec<u16> = panel.iter().map(|r| r.row).collect();
    if rows.windows(2).any(|w| w[1] != w[0] + 1) {
        return Err(format!("panel rows are not adjacent: {rows:?}"));
    }
    if rights[0] >= cols {
        return Err(format!("panel reaches column {} of {cols}", rights[0]));
    }
    Ok(())
}

fn dedup(values: impl Iterator<Item = u16>) -> Vec<u16> {
    let mut out: Vec<u16> = values.collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[test]
fn history_keeps_a_literal_tilde_child_separate_from_the_home_directory() {
    // The first two rows are the halves of the bug. A visit to the child named
    // `~` has to reach that child and a visit to the home directory must not
    // reach it. The third is the control: `cd ~/` does mean home and the fix
    // has to leave that expansion alone. The empty name in it is the row that
    // runs the line.
    for (target, line, expected) in [
        ("source/~", "cd ", vec!["~/", "alpha/", "../", "~"]),
        ("home", "cd ", vec!["alpha/", "~/", "../", "~"]),
        ("home/beta", "cd ~/", vec!["", "beta/", "alpha/", "../"]),
    ] {
        let f = Fixture::new(&["source/alpha", "source/~", "home/alpha", "home/beta"]);
        let source = f.path().join("source");
        let home = f.path().join("home");
        let data = f.path().join("data");
        let recorded = std::process::Command::new(env!("CARGO_BIN_EXE_surmise"))
            .arg("--record")
            .arg(&source)
            .arg(f.path().join(target))
            .env_clear()
            .env("HOME", &home)
            .env("XDG_DATA_HOME", &data)
            .output()
            .unwrap();
        assert!(recorded.status.success());

        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_surmise"));
        cmd.args(["--pick", line]);
        cmd.env_clear();
        cmd.env("HOME", &home);
        cmd.env("XDG_DATA_HOME", &data);
        cmd.env("TERM", "xterm-256color");
        cmd.cwd(&source);
        let mut t = Term::new(cmd, 100, 30);
        assert!(t.wait_panel(WAIT), "nothing was drawn");
        t.pump(SETTLE);
        assert_eq!(names(&t), expected, "visit to {target} with {line}");
    }
}

#[test]
fn the_current_directory_is_the_whole_list() {
    let f = fixture();
    let t = opened(f.path(), "cd ");
    assert_eq!(names(&t), ["deep/", "my docs/", "work/", "../", "~"]);
}

#[test]
fn the_menu_holds_still_to_its_edge_and_follows_the_highlight_past_it() {
    // Nine directories and the two rows every bare `cd` gets are more than
    // the menu shows at once. The window therefore has somewhere to move and
    // this test is where it does.
    let f = Fixture::new(&["d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9"]);
    let head = ["d1/", "d2/", "d3/", "d4/", "d5/", "d6/"];
    let mut t = opened(f.path(), "cd ");
    assert_eq!(names(&t), head);
    // Five presses put the highlight on the last row already on the screen.
    t.send(&"\x1b[B".repeat(5));
    t.pump(SETTLE);
    // The names alone cannot say the presses landed. The window has not moved
    // and nothing here reads which row the highlight is on. The count on the
    // panel's top edge is what says it and the claim rests on both halves.
    assert_eq!(names(&t), head, "{:?}", t.lines());
    assert!(edge(&t).contains("6/11"), "{:?}", t.lines());
    // One more and the window follows it by a single row.
    t.send("\x1b[B");
    t.pump(SETTLE);
    assert_eq!(
        names(&t),
        ["d2/", "d3/", "d4/", "d5/", "d6/", "d7/"],
        "{:?}",
        t.lines()
    );
    assert!(edge(&t).contains("7/11"), "{:?}", t.lines());
    // Four more take the highlight to the end of the list and the window
    // with it. Up from there lands on a row the window already holds and the
    // window therefore holds still. A window read off the highlight alone
    // would step back a row here.
    t.send(&"\x1b[B".repeat(4));
    t.pump(SETTLE);
    let tail = ["d6/", "d7/", "d8/", "d9/", "../", "~"];
    assert_eq!(names(&t), tail, "{:?}", t.lines());
    assert!(edge(&t).contains("11/11"), "{:?}", t.lines());
    t.send("\x1b[A");
    t.pump(SETTLE);
    assert_eq!(names(&t), tail, "{:?}", t.lines());
    assert!(edge(&t).contains("10/11"), "{:?}", t.lines());
}

#[test]
fn a_directory_only_the_history_knows_about_does_not_get_in() {
    let f = fixture();
    // `target` is three levels down and the fixture's history visited it.
    let mut t = surmise(f.path(), "cd targ", 100, 30);
    assert_eq!(t.status(WAIT), Some(pick::PASS));
}

#[test]
fn a_bare_cd_puts_a_folder_glyph_on_every_row() {
    let f = fixture();
    let t = opened(f.path(), "cd ");
    // A `take_while` over an empty list would claim this with no row read.
    assert!(!names(&t).is_empty());
    // The line under the list is where the names stop. What is below it
    // carries no glyph and no name.
    let rows = name_rows(&t);
    for text in &rows {
        assert!(text.contains(ICON) || text.contains(HOME_ICON), "{text:?}");
    }
    // One of them is the home shortcut and its own shape is what says so.
    assert_eq!(
        rows.iter().filter(|text| text.contains(HOME_ICON)).count(),
        1,
        "{rows:?}"
    );
}

/// The drawn row holding `name`. `ls ` answers with the folders and files of
/// one directory and then with its own options, so a test that reads a glyph
/// off a named row says which row it read.
fn row_holding(rows: &[String], name: &str) -> String {
    rows.iter()
        .find(|text| text.contains(name))
        .unwrap_or_else(|| panic!("no row for {name}: {rows:?}"))
        .clone()
}

#[test]
fn a_config_that_asks_for_the_nerd_set_draws_it() {
    // Three rows out of one menu: a folder, a file whose extension the table
    // answers for and a second such file. The shapes differ from each other
    // and none of them is the shape the same line draws without the config.
    let f = Fixture::new(&["assets/inner", "build.rs*", "notes.md*"]);
    config(f.path(), "icons = \"nerd\"\n");
    let t = opened(f.path(), "ls ");
    let rows = name_rows(&t);
    assert!(row_holding(&rows, "assets/").contains(NF_DIR), "{rows:?}");
    assert!(row_holding(&rows, "build.rs").contains(NF_RUST), "{rows:?}");
    assert!(
        row_holding(&rows, "notes.md").contains(NF_MARKDOWN),
        "{rows:?}"
    );
    // The text set's own folder shape is gone with it.
    for text in &rows {
        assert!(!text.contains(ICON), "{text:?}");
    }
}

#[test]
fn the_text_set_is_what_the_same_line_draws_with_no_config_at_all() {
    // The test above is what the config buys. Every other test in this file
    // reads the text set and would fail on a default that had moved, but none
    // of them says that is the default rather than an accident of the
    // machine's own home. This is also the one place that says a file row and
    // a folder row differ without the nerd set's own tables.
    let f = Fixture::new(&["assets/inner", "build.rs*"]);
    let t = opened(f.path(), "ls ");
    let rows = name_rows(&t);
    assert!(row_holding(&rows, "assets/").contains(ICON), "{rows:?}");
    assert!(
        row_holding(&rows, "build.rs").contains(FILE_ICON),
        "{rows:?}"
    );
    for text in &rows {
        assert!(!text.contains(NF_DIR), "{text:?}");
        assert!(!text.contains(NF_RUST), "{text:?}");
    }
}

#[test]
fn a_bare_git_puts_a_subcommand_glyph_on_every_row() {
    let f = fixture();
    let t = opened(f.path(), "git ");
    // A Git row is read here as it was drawn rather than through `names`,
    // which takes the glyph off.
    let rows = name_rows(&t);
    // A `take_while` over an empty list would claim the rest with no row read.
    assert!(!rows.is_empty());
    // Every row above that line names a subcommand and none of them names a
    // directory. The shape is what says which of the two the menu is holding.
    for text in &rows {
        assert!(text.contains(CMD_ICON), "{text:?}");
        assert!(!text.contains(ICON), "{text:?}");
    }
}

/// The glyph test above reads rows the installed Git named, so it can claim
/// nothing about any one of them. `add` is a name every Git has and an exact
/// match leads the list, so the highlight opens on the row whose description
/// the committed specification carries.
#[test]
fn a_git_subcommand_row_shows_the_description_its_specification_carries() {
    let f = fixture();
    let t = opened(f.path(), "git add");
    assert!(
        footer(&t).contains("Add file contents to the index"),
        "{:?}",
        footer(&t)
    );
}

/// The longest description git's own 38 subcommands carry. Three of the
/// panel's rows hold it and the one row under the list does not.
const LONG_DESCRIPTION: &str = "Create new commit that undoes all of the changes made in <commit>, then apply it to the current branch";

/// Ctrl-O, the key the rule under the list names.
const WHOLE_WORD: &str = "\x0f";

#[test]
fn a_key_opens_the_whole_of_a_description_the_one_row_cut() {
    let f = fixture();
    let mut t = opened(f.path(), "git revert");
    assert!(footer(&t).contains('…'), "{:?}", footer(&t));
    t.send(WHOLE_WORD);
    t.pump(SETTLE);
    assert_eq!(beside(&t), LONG_DESCRIPTION, "{:?}", t.lines());
    t.send(WHOLE_WORD);
    t.pump(SETTLE);
    assert!(footer(&t).contains('…'), "{:?}", footer(&t));
    assert!(t.detail().is_empty(), "{:?}", t.lines());
}

#[test]
fn the_rule_under_the_list_names_the_key_that_opens_the_word() {
    let f = fixture();
    let t = opened(f.path(), "git revert");
    let panel = t.panel();
    let rule = panel
        .iter()
        .skip(1)
        .position(|row| !is_name(row))
        .expect("the rule under the list");
    let rule = panel[rule + 1].text.trim();
    // A rule with the badge let into its right end, rather than the badge
    // anywhere on any row. The row the key names is the one above the word.
    assert!(rule.starts_with('─'), "{rule:?}");
    assert!(rule.ends_with("^O ─"), "{rule:?}");
}

#[test]
fn the_key_puts_the_word_beside_the_list_and_the_list_shows_one_name_more() {
    let f = fixture();
    let mut t = opened(f.path(), "docker ");
    assert_eq!(names(&t).len(), 6, "{:?}", t.lines());
    assert!(t.detail().is_empty(), "{:?}", t.lines());
    t.send(WHOLE_WORD);
    t.pump(SETTLE);
    // The word has a box of its own beside the list. Its edge opens on the
    // list's own row, past the list's right edge, and carries the key. The
    // list's own edge carries the count and nothing else.
    let panel = t.panel();
    let detail = t.detail();
    let rule = detail.first().expect("the word's own edge").text.trim();
    assert!(rule.ends_with("^O ─"), "{rule:?}");
    assert_eq!(detail[0].row, panel[0].row, "{:?}", t.lines());
    assert!(detail[0].lo > panel[0].hi, "{:?}", t.lines());
    assert!(!panel[0].text.contains("^O"), "{:?}", panel[0].text);
    assert_eq!(intact(&panel, 100), Ok(()));
    assert_eq!(intact(&detail, 100), Ok(()));
    // The line under the list and the word's row under that are gone and
    // the list spends one of them on a seventh name.
    assert_eq!(names(&t).len(), 7, "{:?}", t.lines());
    assert!(
        beside(&t).starts_with("Attach local standard input"),
        "{:?}",
        beside(&t)
    );
}

#[test]
fn the_next_menu_opens_the_way_the_key_last_left_it() {
    let f = fixture();
    let mut t = opened(f.path(), "git revert");
    t.send(WHOLE_WORD);
    t.pump(SETTLE);
    assert_eq!(beside(&t), LONG_DESCRIPTION, "{:?}", t.lines());
    // The menu it was pressed in is over and the line is back with the
    // shell. The answer is not.
    t.send("\x1b");
    assert_eq!(t.status(WAIT), Some(pick::ACCEPTED));
    let mut t = opened(f.path(), "git revert");
    assert_eq!(beside(&t), LONG_DESCRIPTION, "{:?}", t.lines());
    t.send(WHOLE_WORD);
    t.pump(SETTLE);
    t.send("\x1b");
    assert_eq!(t.status(WAIT), Some(pick::ACCEPTED));
    let t = opened(f.path(), "git revert");
    assert!(footer(&t).contains('…'), "{:?}", footer(&t));
}

#[test]
fn a_git_subcommand_argument_draws_the_menu_its_specification_asks_for() {
    // `git blame ` is a line Git's own menu declines outright: the subcommand
    // is finished and the word behind it is neither a branch nor an `add`
    // path. The committed specification says that word is a file, and this is
    // the screen that answers with one.
    let f = Fixture::new(&["assets/inner", "notes.md*"]);
    let t = opened(f.path(), "git blame ");
    let rows = name_rows(&t);
    assert!(
        row_holding(&rows, "notes.md").contains(FILE_ICON),
        "{rows:?}"
    );
    assert!(row_holding(&rows, "assets/").contains(ICON), "{rows:?}");
}

#[test]
fn a_bare_docker_draws_a_menu_of_subcommands_with_their_descriptions() {
    let f = fixture();
    let t = opened(f.path(), "docker ");
    let rows = name_rows(&t);
    assert!(!rows.is_empty());
    for text in &rows {
        assert!(text.contains(CMD_ICON), "{text:?}");
    }
    // Nothing narrows an empty search term, so the rows sort alphabetically
    // and the highlight opens on the first of them.
    assert!(rows[0].contains("attach"), "{rows:?}");
    assert!(
        footer(&t).contains("Attach local standard input"),
        "{:?}",
        footer(&t)
    );
}

#[test]
fn a_subcommand_row_shows_its_argument_hint() {
    let f = fixture();
    let t = opened(f.path(), "git status");
    let rows = name_rows(&t);
    assert!(rows[0].contains("status"), "{rows:?}");
    assert!(rows[0].contains("[pathspec...]"), "{rows:?}");
}

#[test]
fn tab_accepts_a_spec_subcommand_and_grows_the_line() {
    let f = fixture();
    let mut t = opened(f.path(), "docker contai");
    t.send("\t");
    t.pump(SETTLE);
    assert!(shown(&t).contains("docker container "), "{:?}", t.lines());
}

#[test]
fn the_menu_marks_what_the_argument_reached() {
    let f = fixture();
    let t = opened(f.path(), "cd wk");
    // `work/` is a match that does not lead with what was typed. The marks
    // are what say how it got into the menu.
    assert_eq!(names(&t), ["work/"]);
    assert_eq!(t.marks(1), "wk");
    // The footer is a row of its own and carries nothing to mark. It is the
    // last painted row, under the rule that ends the list.
    assert_eq!(t.marks(t.panel().len() - 1), "");
}

#[test]
fn the_menu_underlines_what_tab_would_add() {
    let f = fixture();
    let t = opened(f.path(), "cd wo");
    // One row leads with what was typed and Tab would take the whole of it.
    // The marks say what was typed and the underline says what the key adds.
    assert_eq!(names(&t), ["work/"]);
    assert_eq!(t.marks(1), "wo");
    assert_eq!(t.underlined(1), "rk/");
}

#[test]
fn the_menu_makes_its_own_room_at_the_bottom_of_the_screen() {
    // The shell has filled the screen and left its prompt on the last row.
    // Nothing is left under the line and the menu wants eight rows. The
    // terminal scrolls to make them rather than the menu shrinking or moving
    // above the line: what is above the line is the shell's own output and
    // surmise cannot read it back to put it there again.
    let f = Fixture::new(&["d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7"]);
    let mut t = surmise(f.path(), "cd d", 40, 12);
    t.shell_drew(&"\r\n".repeat(14));
    t.shell_drew("~ > cd d");
    assert!(t.wait_panel(WAIT), "nothing was drawn");
    t.pump(SETTLE);
    // Every row the menu asked for is on the screen.
    assert_eq!(names(&t).len(), 6);
    assert!(edge(&t).contains("1/8"), "{:?}", edge(&t));
    assert_eq!(intact(&t.panel(), 40), Ok(()));
    // The line the menu answers for scrolled with it and the shell's own
    // prompt is still in front of it.
    assert!(shown(&t).contains("~ > cd d"), "{:?}", t.lines());
}

#[test]
fn the_menu_is_a_closed_box_at_every_width() {
    for cols in [100, 40, 30] {
        let f = fixture();
        let mut t = surmise(f.path(), "cd ", cols, 30);
        assert!(t.wait_panel(WAIT), "nothing was drawn at {cols} columns");
        t.pump(SETTLE);
        assert_eq!(intact(&t.panel(), cols), Ok(()), "at {cols} columns");
    }
}

#[test]
fn a_long_selected_name_is_readable_below_the_list() {
    let prefix = "example-directory-with-a-shared-prefix-";
    let alpha = format!("{prefix}alpha");
    let beta = format!("{prefix}beta");
    let f = Fixture::new(&[&alpha, &beta, "short"]);
    let mut t = opened(f.path(), "cd ");
    let detail = |t: &Term| -> String {
        let panel = t.panel();
        panel[..panel.len() - 1]
            .iter()
            .skip(1) // the rule that closes the panel's top edge
            .skip_while(|row| is_name(row))
            .skip(1) // the rule under the list
            .map(|row| row.text.trim())
            .collect()
    };
    assert_eq!(detail(&t), format!("{alpha}/"));
    assert_eq!(names(&t)[0], names(&t)[1]);
    assert_eq!(intact(&t.panel(), 100), Ok(()));
    t.send("\x1b[B");
    t.pump(SETTLE);
    assert_eq!(detail(&t), format!("{beta}/"));
    t.send("\x1b[B");
    t.pump(SETTLE);
    assert_eq!(detail(&t), "");
    assert!(!shown(&t).contains("beta/"));
    assert_eq!(intact(&t.panel(), 100), Ok(()));
}

#[test]
fn a_wide_name_leaves_the_panel_one_box() {
    // The terminal clears the second cell of a wide character to its own
    // ground. The row is still one run of the panel's and the name reads whole.
    let f = Fixture::new(&["目錄"]);
    let t = opened(f.path(), "cd ");
    assert_eq!(names(&t)[0], "目錄/", "{:?}", t.lines());
    assert_eq!(intact(&t.panel(), 100), Ok(()));
}

#[test]
fn a_long_detail_keeps_the_prompt_visible_in_a_short_terminal() {
    let name = format!("example-{}-tail", "x".repeat(170));
    let f = Fixture::new(&[&name]);
    let mut t = surmise(f.path(), "cd xt", 24, 10);
    t.shell_drew(&"\r\n".repeat(12));
    t.shell_drew("~ > cd xt");
    assert!(t.wait_panel(WAIT), "nothing was drawn");
    t.pump(SETTLE);
    let panel = t.panel();
    assert_eq!(panel.len(), 7);
    assert!(panel[panel.len() - 2].text.contains('…'));
    assert!(shown(&t).contains("~ > cd xt"));
    assert_eq!(intact(&panel, 24), Ok(()));
    t.send("\x1b");
    assert!(t.wait_bare(WAIT));
    assert!(!shown(&t).contains("example-"));
}

#[test]
fn the_menu_follows_the_cursor_along_the_line() {
    let f = fixture();
    let mut t = opened(f.path(), "cd wo");
    let under_the_argument = t.panel()[0].lo;
    // Ctrl-A takes the cursor to the start of the line. The menu is open on
    // the same argument and only the column it hangs from has moved.
    t.send("\x01");
    t.pump(SETTLE);
    let panel = t.panel();
    assert!(!panel.is_empty(), "the menu closed: {:?}", t.lines());
    let under_the_prompt = panel[0].lo;
    assert!(
        under_the_prompt < under_the_argument,
        "the panel stayed at column {under_the_argument}: {:?}",
        t.lines()
    );
}

#[test]
fn the_menu_slides_in_from_the_right_edge() {
    // The panel hangs from the cursor and this cursor sits too far right for
    // the panel to fit under it. The width is what it keeps and the alignment
    // is what it gives up.
    let cols = 40;
    let f = Fixture::new(&["one/two/three/four/five/target"]);
    let mut t = surmise(f.path(), "cd one/two/three/four/five/", cols, 30);
    assert!(t.wait_panel(WAIT), "nothing was drawn");
    t.pump(SETTLE);
    let panel = t.panel();
    assert_eq!(intact(&panel, cols), Ok(()));
    assert_eq!(panel[0].hi, cols - 1, "the panel is not against the edge");
}

#[test]
fn the_menu_survives_the_bottom_of_the_screen() {
    let f = fixture();
    let mut t = surmise(f.path(), "cd ", 76, 10);
    // The shell has filled the screen. The picker has to scroll for its rows
    // and the box has to stay whole across that scroll.
    t.shell_drew(&"\r\n".repeat(12));
    t.shell_drew("~ ❯ cd ");
    assert!(t.wait_panel(WAIT), "nothing was drawn");
    t.pump(SETTLE);
    assert_eq!(intact(&t.panel(), 76), Ok(()));
}

#[test]
fn the_picker_draws_on_the_row_the_shell_left_the_cursor_on() {
    let f = fixture();
    let mut t = surmise(f.path(), "cd ", 100, 30);
    t.shell_drew("~ ❯ cd ");
    assert!(t.wait_panel(WAIT), "nothing was drawn");
    t.pump(SETTLE);
    // The shell's own prompt is still there and the picker put its line on
    // that same row rather than on one of its own below it.
    assert!(shown(&t).starts_with("~ ❯ cd "), "{:?}", t.lines());
    assert_eq!(t.panel()[0].row, 1, "{:?}", t.lines());
}

#[test]
fn a_cursor_left_of_its_own_line_gets_a_row_of_its_own() {
    let f = fixture();
    // Nothing has been drawn. The cursor therefore stands left of where the
    // line would have to start. That is the wrapped case and the picker takes
    // the row below with a prompt of its own to sit behind.
    let t = opened(f.path(), "cd ");
    assert!(
        t.lines().iter().any(|l| l.starts_with("▸ cd ")),
        "{:?}",
        t.lines()
    );
}

#[test]
fn accepting_inserts_the_directory() {
    let f = fixture();
    let mut t = opened(f.path(), "cd wo");
    t.send("\t");
    t.pump(SETTLE);
    assert!(shown(&t).contains("cd work/"), "{:?}", t.lines());
}

#[test]
fn tab_takes_the_prefix_two_rows_share_rather_than_either_row() {
    // `work` and `worse` agree on `wor` and no further and both are therefore
    // still on offer afterwards. Tab taking the highlighted row whole would
    // have left the menu on what is inside `work` instead. The screen cannot
    // tell the two apart on its own, because the ghost draws the rest of the
    // highlighted name either way.
    let f = Fixture::new(&["work", "worse"]);
    let mut t = opened(f.path(), "cd wo");
    t.send("\t");
    t.pump(SETTLE);
    assert_eq!(names(&t), ["work/", "worse/"]);
    // A key that took something has nothing to say about it.
    assert_eq!(t.bells(), 0);
}

#[test]
fn tab_with_nothing_to_take_rings_the_bell() {
    let f = fixture();
    let mut t = opened(f.path(), "cd work/");
    // The two names inside `work/` share nothing the line does not already
    // hold. The line is therefore the same line afterwards and the bell is
    // the only answer there is.
    assert_eq!(t.bells(), 0);
    t.send("\t");
    t.pump(SETTLE);
    assert_eq!(t.bells(), 1);
    assert!(shown(&t).contains("cd work/"), "{:?}", t.lines());
}

#[test]
fn a_name_with_a_space_is_quoted_and_completion_carries_on_inside_it() {
    let f = fixture();
    let mut t = opened(f.path(), "cd my");
    t.send("\t");
    t.pump(SETTLE);
    assert!(shown(&t).contains("cd 'my docs/'"), "{:?}", t.lines());
    // Tab again, with the quote already on the line. The one directory row
    // under the row that runs it is a whole name and the quote closes behind
    // it. Enter on that row reaches the same place and the two are checked
    // separately, because only Tab looks past the highlight.
    t.send("\t");
    t.pump(SETTLE);
    assert!(shown(&t).contains("cd 'my docs/inner/'"), "{:?}", t.lines());
    let mut t = opened(f.path(), "cd my");
    t.send("\t");
    t.pump(SETTLE);
    t.send("\x1b[B\r");
    t.pump(SETTLE);
    assert!(shown(&t).contains("cd 'my docs/inner/'"), "{:?}", t.lines());
}

#[test]
fn a_path_lists_only_what_that_directory_holds() {
    let f = fixture();
    let t = opened(f.path(), "cd ~/work/");
    // The first row runs the line and carries no name of its own.
    assert_eq!(names(&t), ["", "alpha/", "beta/", "../"]);
}

#[test]
fn the_row_that_runs_the_line_carries_a_glyph_of_its_own() {
    let f = fixture();
    let t = opened(f.path(), "cd work/");
    let run_row = t.panel().get(1).expect("a row").text.clone();
    assert!(run_row.contains(RUN_ICON), "{run_row:?}");
    // The glyph is the whole row. The line is on the screen above it and the
    // folder glyph is not on it either.
    assert_eq!(run_row.replace(RUN_ICON, "").trim(), "", "{run_row:?}");
}

#[test]
fn escape_hands_the_line_back() {
    let f = fixture();
    let mut t = opened(f.path(), "cd wo");
    t.send("\x1b");
    assert_eq!(t.status(WAIT), Some(pick::ACCEPTED));
}

#[test]
fn ctrl_c_gives_the_shell_its_own_line_back() {
    let f = fixture();
    let mut t = opened(f.path(), "cd wo");
    t.send("\x03");
    assert_eq!(t.status(WAIT), Some(pick::CANCELLED));
}

#[test]
fn enter_takes_the_directory_and_a_second_enter_asks_for_the_line_to_be_run() {
    let f = fixture();
    let mut t = opened(f.path(), "cd wo");
    t.send("\r");
    t.pump(SETTLE);
    // The first press took the directory rather than the line. The menu is
    // still on the screen and that is what says the run has not ended. Reading
    // the line alone would not: the picker prints the accepted line to stdout
    // and in a pty that is this same screen.
    assert!(!t.panel().is_empty(), "the menu closed: {:?}", t.lines());
    assert!(shown(&t).contains("cd work/"), "{:?}", t.lines());
    t.send("\r");
    assert_eq!(t.status(WAIT), Some(pick::RUN));
}

#[test]
fn enter_on_a_file_a_specification_offers_hands_the_line_back() {
    // `ls` takes as many names as it is given and reads them out of the one
    // directory. The menu this acceptance would reopen is therefore the menu
    // that was already on the screen. A second press there would put `one` on
    // the line again. The run ends instead and the shell gets the finished
    // word.
    let f = Fixture::new(&["one*", "two*"]);
    let mut t = opened(f.path(), "ls ");
    assert_eq!(names(&t)[0], "one");
    t.send("\r");
    assert_eq!(t.status(WAIT), Some(pick::ACCEPTED));
    assert!(shown(&t).contains("ls one"), "{:?}", t.lines());
}

#[test]
fn a_path_argument_a_specification_fills_gets_the_row_that_runs_the_line() {
    // `cd`'s own row, on an argument a specification reads off the
    // filesystem. Without it this line descends for as long as there are
    // directories under it.
    let f = Fixture::new(&["assets/inner"]);
    let mut t = opened(f.path(), "ls assets/");
    let run_row = t.panel().get(1).expect("a row").text.clone();
    assert!(run_row.contains(RUN_ICON), "{run_row:?}");
    assert_eq!(run_row.replace(RUN_ICON, "").trim(), "", "{run_row:?}");
    t.send("\r");
    assert_eq!(t.status(WAIT), Some(pick::RUN));
}

#[test]
fn the_menu_is_a_closed_box_with_the_row_that_runs_in_it() {
    // The other two `intact` cases open on a bare `cd ` and that line never
    // gets the row. This one does and the glyph and the row's empty name
    // therefore go through the panel's own arithmetic under a check.
    for cols in [100, 40, 30] {
        let f = fixture();
        let mut t = surmise(f.path(), "cd work/", cols, 30);
        assert!(t.wait_panel(WAIT), "nothing was drawn at {cols} columns");
        t.pump(SETTLE);
        assert!(
            t.panel().iter().any(|r| r.text.contains(RUN_ICON)),
            "no row that runs the line at {cols} columns: {:?}",
            t.lines()
        );
        assert_eq!(intact(&t.panel(), cols), Ok(()), "at {cols} columns");
    }
}

#[test]
fn enter_runs_the_line_once_the_cursor_leaves_the_argument() {
    // Home puts the cursor left of the argument the menu answers for. The
    // menu is still open and the highlight is still on a directory row. Enter
    // therefore has nothing to take and the line as it stands is the answer.
    let f = fixture();
    let mut t = opened(f.path(), "cd wo");
    t.send("\x01");
    t.pump(SETTLE);
    assert!(!t.panel().is_empty(), "the menu closed: {:?}", t.lines());
    t.send("\r");
    assert_eq!(t.status(WAIT), Some(pick::RUN));
}

#[test]
fn a_line_that_is_not_a_cd_never_reaches_the_terminal() {
    // `ls` completes its own argument now that `spec_menu` answers a
    // `filepaths` template; `zzz` is what still matches nothing in this
    // fixture or among `ls`'s own options.
    let f = fixture();
    let mut t = surmise(f.path(), "ls zzz", 100, 30);
    assert_eq!(t.status(WAIT), Some(pick::PASS));
    assert!(t.lines().is_empty(), "{:?}", t.lines());
}

#[test]
fn a_parent_can_be_browsed_repeatedly_with_tab_and_enter() {
    let f = Fixture::new(&["base/level/inner", "base/sibling", "other"]);
    let mut t = opened(f.path(), "cd base/level/..");
    assert_eq!(names(&t), ["", "../"]);
    t.send("\t");
    t.pump(SETTLE);
    assert_eq!(t.bells(), 1);
    assert_eq!(t.underlined(2), "");
    t.send("\x1b[B");
    t.pump(SETTLE);
    assert_eq!(t.underlined(2), "/");
    t.send("\t");
    t.pump(SETTLE);
    assert!(shown(&t).contains("cd base/level/../"));
    assert_eq!(names(&t), ["", "level/", "sibling/", "../"]);
    t.send(&"\x1b[B".repeat(3));
    t.pump(SETTLE);
    assert_eq!(t.underlined(2), "");
    assert_eq!(t.underlined(3), "");
    assert_eq!(t.underlined(4), "../");
    t.send("\r");
    t.pump(SETTLE);
    assert!(shown(&t).contains("cd base/level/../../"));
    assert_eq!(names(&t), ["", "base/", "other/", "../"]);
    t.send("\r");
    assert_eq!(t.status(WAIT), Some(pick::RUN));
}

#[test]
fn enter_on_the_initial_two_dot_row_runs_the_line() {
    let f = Fixture::new(&["base/level"]);
    let mut t = opened(f.path(), "cd base/level/..");
    t.send("\r");
    assert_eq!(t.status(WAIT), Some(pick::RUN));
}
