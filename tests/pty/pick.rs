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
const ICON: char = '\u{f07b}';

/// The glyph on the row that runs the line.
const RUN_ICON: char = '\u{21b5}';

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
    cmd.cwd(home);
    Term::new(cmd, cols, rows)
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

/// The candidate names, without a row glyph or its padding. The last panel row
/// is the footer rather than a candidate.
fn names(t: &Term) -> Vec<String> {
    let panel = t.panel();
    let Some((_footer, rows)) = panel.split_last() else {
        return Vec::new();
    };
    rows.iter()
        .map(|row| row.text.replace([ICON, RUN_ICON], "").trim().to_string())
        .collect()
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
fn the_current_directory_is_the_whole_list() {
    let f = fixture();
    let t = opened(f.path(), "cd ");
    assert_eq!(names(&t), ["deep/", "my docs/", "work/", "..", "~"]);
}

#[test]
fn a_directory_only_the_history_knows_about_does_not_get_in() {
    let f = fixture();
    // `target` is three levels down and the fixture's history visited it.
    let mut t = surmise(f.path(), "cd targ", 100, 30);
    assert_eq!(t.status(WAIT), Some(pick::PASS));
}

#[test]
fn a_bare_cd_puts_the_folder_glyph_on_every_row() {
    let f = fixture();
    let t = opened(f.path(), "cd ");
    let panel = t.panel();
    for row in &panel[..panel.len() - 1] {
        assert!(row.text.contains(ICON), "{:?}", row.text);
    }
}

#[test]
fn the_menu_is_a_closed_box_at_every_width() {
    for cols in [100, 72, 40] {
        let f = fixture();
        let mut t = surmise(f.path(), "cd ", cols, 30);
        assert!(t.wait_panel(WAIT), "nothing was drawn at {cols} columns");
        t.pump(SETTLE);
        assert_eq!(intact(&t.panel(), cols), Ok(()), "at {cols} columns");
    }
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
    let cols = 32;
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
    assert_eq!(names(&t), ["", "alpha/", "beta/"]);
}

#[test]
fn the_row_that_runs_the_line_carries_a_glyph_of_its_own() {
    let f = fixture();
    let t = opened(f.path(), "cd work/");
    let first = t.panel().first().expect("a row").text.clone();
    assert!(first.contains(RUN_ICON), "{first:?}");
    // The glyph is the whole row. The line is on the screen above it and the
    // folder glyph is not on it either.
    assert_eq!(first.replace(RUN_ICON, "").trim(), "", "{first:?}");
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
fn the_menu_is_a_closed_box_with_the_row_that_runs_in_it() {
    // The other two `intact` cases open on a bare `cd ` and that line never
    // gets the row. This one does and the glyph and the row's empty name
    // therefore go through the panel's own arithmetic under a check.
    for cols in [100, 72, 40] {
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
    let f = fixture();
    let mut t = surmise(f.path(), "ls wo", 100, 30);
    assert_eq!(t.status(WAIT), Some(pick::PASS));
    assert!(t.lines().is_empty(), "{:?}", t.lines());
}
