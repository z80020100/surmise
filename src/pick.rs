//! The shell picker.
//!
//! A zsh widget hands over the current line, surmise draws the menu on
//! /dev/tty and the chosen line comes back on stdout. The shell keeps its own
//! line editor, its own key bindings and its own plugins. surmise never wraps
//! a widget and never binds a key of its own beyond the one that starts it.
//!
//! The exit status is the rest of that contract. It tells the widget which of
//! the four outcomes below happened and whether stdout holds a line.

use crate::app::App;
use crate::histfile;
use crate::history::History;
use crate::keys;
use crate::state::State;
use crate::tty;
use crate::ui;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;

/// Take the line back and leave it on the shell's editor. It is on stdout.
pub const ACCEPTED: u8 = 0;
/// Undo. The shell keeps the line the person started with.
pub const CANCELLED: u8 = 1;
/// Not a line surmise completes. The shell runs its own completion instead.
pub const PASS: u8 = 2;
/// Take the line back and run it. It is on stdout.
pub const RUN: u8 = 3;

/// Cells the line wants to the right of the column it starts on. A line that
/// starts closer than this to the right edge gets a row of its own instead.
/// The panel asks for nothing here. It slides left of the cursor rather than
/// run off the edge.
const MIN_ROOM: usize = 8;

/// The column the shell's line starts on, worked back from where it left the
/// cursor. `None` when the line has wrapped or when too little of the
/// terminal is left to draw in.
fn anchor_col(cursor_col: usize, seed: &str, width: usize) -> Option<usize> {
    // A cursor left of where the line would have to start means the line
    // wrapped and the column reported belongs to its last row.
    let start = cursor_col.checked_sub(ui::cells(seed))?;
    (start + MIN_ROOM < width).then_some(start)
}

/// The state for `seed`, with `aliases`, `history` and `cmd_history` in
/// place before the one refresh that builds the first candidate list.
/// `App::over` cannot be used here: it calls `refresh` on construction and
/// any of the three set afterward would answer a menu already drawn without
/// it, which is what would force a second refresh to fix. `None` when
/// surmise has nothing to offer and the key therefore belongs to the shell.
fn seeded(
    seed: &str,
    cwd: &Path,
    aliases: HashMap<String, String>,
    history: History,
    cmd_history: histfile::Counts,
) -> Option<App> {
    let mut app = App::new(cwd.to_path_buf());
    app.aliases = aliases;
    app.line.insert(seed);
    app.seed_history(history, cmd_history);
    app.refresh();
    (!app.items.is_empty()).then_some(app)
}

/// What the widget put on stdin: `RBUFFER`, `$HISTFILE` and the shell's
/// alias table.
#[derive(Default)]
struct Input {
    rbuffer: String,
    /// `$HISTFILE`, from the same record. Empty when the shell has none set,
    /// which `crate::histfile::read` reads as no history to count.
    histfile: String,
    aliases: HashMap<String, String>,
}

/// Stdin as the widget's own record, or the empty record when there was
/// none to read.
///
/// `run` reads this once, ahead of `seeded`, and answers `PASS` on it alone
/// when the cursor sits mid-word. Otherwise its fields land on the `App`
/// `seeded` builds, the aliases before that build's own first refresh and
/// the rest once one exists.
fn read_input() -> io::Result<Input> {
    // A terminal on stdin means nobody piped a record in. Reading it would
    // wait on a key that never comes, and a hand run of `surmise --pick` puts
    // exactly that on stdin.
    if tty::stdin_is_terminal() {
        return Ok(Input::default());
    }
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes)?;
    Ok(parse_input(&bytes))
}

/// The tag the widget leads its record with. A record without it came from
/// a widget older than the `$HISTFILE` field, sourced into a shell that was
/// already running when the binary was replaced. That shell keeps its own
/// copy of the widget for as long as it lives, so this binary has to read
/// both shapes.
///
/// The tag can be read where none was meant, and only one way round: an
/// older widget whose `RBUFFER` is this exact text writes it into the field
/// the tag now sits in, and the record is then read as the newer shape it
/// is not. That menu takes the first alias's own name for `RBUFFER` and its
/// value for the path, loses that one pair and keeps every pair behind it.
/// A newer widget cannot collide at all, because it writes the tag itself
/// and whatever `RBUFFER` holds goes in the field behind it. One menu, on
/// a line whose text to the right of the cursor is exactly this tag, is
/// the whole of what the tag costs, against every menu of an upgraded
/// binary reading an older shell's aliases one field out.
const RECORD_TAG: &str = "surmise-record-3";

/// Parse the record: [`RECORD_TAG`], then `RBUFFER`, then `$HISTFILE`, then
/// a NUL-separated name and value for every shell alias. A record with no
/// tag is read as the shape before the `$HISTFILE` field: `RBUFFER` first
/// and the aliases straight after it. Empty bytes parse to the empty
/// record, which is what an absent or closed stdin already reads as.
fn parse_input(bytes: &[u8]) -> Input {
    let mut fields = bytes.split(|&b| b == 0);
    let text = |f: &[u8]| String::from_utf8_lossy(f).into_owned();
    let first = fields.next().map(&text).unwrap_or_default();
    // Without the tag the field just read is `RBUFFER` and there is no
    // `$HISTFILE` behind it to read.
    let tagged = first == RECORD_TAG;
    let rbuffer = if tagged {
        fields.next().map(&text).unwrap_or_default()
    } else {
        first
    };
    let histfile = if tagged {
        fields.next().map(&text).unwrap_or_default()
    } else {
        String::new()
    };

    // The widget writes a NUL after every field including the last. That
    // leaves one empty field behind rather than a name with no value, and it
    // is dropped before the fields are paired up.
    let mut rest: Vec<&[u8]> = fields.collect();
    if rest.last().is_some_and(|f| f.is_empty()) {
        rest.pop();
    }

    let mut aliases = HashMap::new();
    let mut rest = rest.into_iter();
    while let Some(name) = rest.next() {
        // A name with nothing left to pair it with is a malformed record.
        // Nothing here guesses at the value and the name is dropped.
        let Some(value) = rest.next() else { break };
        aliases.insert(text(name), text(value));
    }
    Input {
        rbuffer,
        histfile,
        aliases,
    }
}

/// The prompt for a row of surmise's own. It is dim so that the shell's own
/// prompt above it stays the brighter one.
fn head() -> Vec<ui::Seg> {
    vec![ui::Seg {
        style: ui::DIM,
        text: "▸ ".into(),
    }]
}

pub fn run(seed: &str) -> io::Result<u8> {
    // The widget's own record has to be read before anything opens the terminal.
    // `tty::claim` below replaces stdin outright and there is no reading it
    // back afterwards.
    let input = read_input()?;

    // The file is read once. `enabled = false` keeps surmise out of the way
    // entirely: every key answers `PASS`, the same status a menu with nothing
    // to offer already gives back, and the shell's own completion runs in its
    // place. The glyph set below is the other thing this run takes from it.
    let config = crate::config::Config::load();
    if !config.enabled {
        return Ok(PASS);
    }

    // A character right of the cursor that is not blank means the cursor
    // sits inside a word. Completing there would split it, so the key goes
    // back to the shell before surmise looks at `seed` at all. Q refuses the
    // same way.
    if input.rbuffer.starts_with(|c: char| c != ' ' && c != '\t') {
        return Ok(PASS);
    }

    // Without a current directory there is nothing to complete against. The
    // shell's own completion is the honest answer.
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(PASS);
    };

    // Decide what this line needs before `seeded` builds the menu, rather
    // than refreshing again for each thing learned once it already has.
    // `left_of_cursor` and `right_of_cursor` are `seed` and the empty string
    // here: the cursor sits at the end of what the widget handed over and
    // `RBUFFER`, read above, is what would sit to its right.
    let completes_git = crate::git::parse(seed).is_some();
    let completes_spec = crate::spec_menu::parse(seed, "", &input.aliases).is_some();
    // Both are true of a line Git's own menu claims, since the spec menu
    // reads a `git` line as well and `App::reader` is what holds it to the
    // ones Git's own declined. The first is therefore what answers below:
    // Git's own candidates never read the directory history, and a `git`
    // line the walk answers wants that history the way `ls ` does. A run
    // that opens on Git's own menu never reaches a row the walk weighs,
    // because it ends where that menu does. Nothing but these two menus
    // reads the command history, so each line pays for the one it will
    // actually be ordered by and not for the other.
    let history = if completes_git {
        History::default()
    } else {
        History::load(&cwd)
    };
    let cmd_history = if completes_git || completes_spec {
        histfile::read(&input.histfile, &input.aliases)
    } else {
        histfile::Counts::default()
    };
    // Nothing to offer. Give the key back without touching the terminal.
    let Some(mut app) = seeded(seed, &cwd, input.aliases, history, cmd_history) else {
        return Ok(PASS);
    };
    app.rbuffer = input.rbuffer;
    // Whatever the last menu was left set to. The key below is the only
    // thing that writes it and a person who asked for the whole word once
    // is asking for it again.
    app.whole_word = State::load().whole_word;

    let mut term = tty::claim()?;
    let _raw = tty::Raw::on(term.try_clone()?)?;

    // Draw on the shell's own prompt row in place of the line it already
    // shows. Nothing then appears twice. That needs the column the shell left
    // the cursor on.
    let anchor = tty::column(&mut term).and_then(|c| anchor_col(c, seed, ui::width()));
    // A terminal that will not say gets the row below and a prompt of
    // surmise's own to sit behind.
    let head = match anchor {
        Some(_) => Vec::new(),
        None => {
            term.write_all(b"\r\n")?;
            term.flush()?;
            head()
        }
    };
    let frame_col = anchor.unwrap_or(0);

    // The frame goes out through a handle of its own. `term` therefore stays
    // open for the writes that follow the loop.
    let mut ui = ui::Ui::new(term.try_clone()?, frame_col);

    let outcome = loop {
        // `App` decides whether the menu shows. `ui` only sizes it.
        let typed = app.typed();
        let menu = app
            .menu_open()
            .then(|| {
                ui::menu(
                    &app.items,
                    app.selected,
                    &typed,
                    app.reach(),
                    app.whole_word,
                    config.icons,
                )
            })
            .flatten();
        ui.render(&head, &app.line, &app.ghost(), menu)?;

        let completing_git = app.completes_git();
        match event::read()? {
            Event::Paste(s) => {
                app.line.insert(&keys::pasted(&s));
                app.edited();
            }
            Event::Key(k) if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    // Escape hands the line back as it stands. Anything typed
                    // in here is the person's work and must survive.
                    KeyCode::Esc => break ACCEPTED,
                    KeyCode::Char('c' | 'g') if ctrl => break CANCELLED,
                    // The word under the list takes one row on its own and
                    // a sentence longer than that loses its tail. This
                    // opens the whole of it and closes it again. It opens
                    // beside the list where the terminal has room for both.
                    // Q binds the same thing to Ctrl-K and surmise keeps
                    // Ctrl-K as the shell's own kill-line.
                    //
                    // The answer outlives this menu. Every later one opens
                    // the way this key last left it. How the menu it was
                    // pressed in ended makes no difference.
                    //
                    // The key answers for a menu and there has to be one to
                    // answer for. A line with no rows draws no panel, no
                    // word under it and no badge naming this key, and a
                    // press there would set every later menu from a screen
                    // that showed none of it. `keys::edit` leaves a ctrl
                    // character alone, so nothing else takes the key.
                    KeyCode::Char('o') if ctrl && app.menu_open() => {
                        app.whole_word = !app.whole_word;
                        State {
                            whole_word: app.whole_word,
                        }
                        .save();
                    }
                    // Nothing to take is an answer of its own and the line
                    // cannot show it. The bell is what says it instead.
                    KeyCode::Tab => {
                        if !app.accept_common() {
                            ui.bell()?;
                        } else if app.menu_repeats || !app.menu_open() {
                            // A whole name went in and left nothing behind to
                            // answer the next press with. The Enter arm below
                            // ends on the same two conditions and says why.
                            // A prefix is not a whole name and never lands on
                            // the first of them: the rows it came from are the
                            // rows that still match it.
                            break ACCEPTED;
                        }
                    }
                    // A directory row is one to go into and the menu stays
                    // open on what is inside it. The row that runs the line
                    // ends the run and so does a row with nothing left to
                    // take. The second of those is what keeps Enter working
                    // once the cursor has moved off the argument the menu
                    // answers for.
                    KeyCode::Enter => {
                        // Every exit takes the row first. The line the shell is
                        // handed has to name the directory surmise resolved and
                        // a bare `it's` or `~root` names something else.
                        if app.runs_the_line() {
                            app.accept();
                            break if completing_git { ACCEPTED } else { RUN };
                        }
                        if !app.accept() {
                            break if completing_git { ACCEPTED } else { RUN };
                        }
                        // The descent landed somewhere with nothing to show
                        // and nothing to go on into. A directory nobody may
                        // read does that. Hand the line to the shell's own
                        // editor rather than hold a frame with no menu on it.
                        //
                        // A menu that came back with the list it already had
                        // is handed back for the same reason: the row took
                        // nothing out of it and the next press would put the
                        // same name on the line a second time. The shell has
                        // the finished word and the press after this one runs
                        // it. `App::menu_repeats` is where that is written down.
                        if app.menu_repeats || !app.menu_open() {
                            break ACCEPTED;
                        }
                    }
                    _ => {
                        keys::edit(&mut app, k);
                        // An empty line is the plainest way to say "not this".
                        // Leave it empty and give the terminal back.
                        if app.line.is_empty() {
                            break ACCEPTED;
                        }
                    }
                }
            }
            // A resize needs no answer of its own. The next frame measures the
            // terminal again.
            _ => {}
        }
        // A run Git's own menu answered ends where that menu does. An accepted
        // subcommand or branch hands the next word to the walk and the run
        // stops there rather than going on under it. Enter would take the
        // walk's first row, and a second press that used to run `git status`
        // or switch branches would write an option or a file onto the line
        // instead. The next Tab is what opens the walk.
        if completing_git && !(app.menu_open() && app.completes_git()) {
            break ACCEPTED;
        }
    };

    ui.erase()?;
    // Leave the cursor at the start of the shell's own line where it stood
    // when the widget called. A frame drawn below that line has a newline to
    // undo first. This holds even when the frame scrolled the screen, because
    // the shell's line scrolled with it.
    term.write_all(if anchor.is_some() { b"\r" } else { b"\x1b[A\r" })?;
    term.flush()?;

    if outcome == ACCEPTED || outcome == RUN {
        print!("{}", app.line.text());
        io::stdout().flush()?;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    /// `seeded` for a test with no history of either kind to give it. Every
    /// case below is about which line opens a menu rather than about what
    /// orders the rows in one.
    fn seeded_bare(seed: &str, cwd: &Path, aliases: HashMap<String, String>) -> Option<App> {
        seeded(
            seed,
            cwd,
            aliases,
            History::default(),
            histfile::Counts::default(),
        )
    }

    #[test]
    fn the_anchor_is_the_column_the_line_started_on() {
        assert_eq!(anchor_col(10, "cd wo", 80), Some(5));
    }

    #[test]
    fn a_line_that_starts_at_the_left_edge_still_anchors() {
        // Column zero is an answer. It is not the absence of one.
        assert_eq!(anchor_col(5, "cd wo", 80), Some(0));
    }

    #[test]
    fn a_wide_character_in_the_seed_counts_as_two_cells() {
        assert_eq!(anchor_col(10, "cd 日", 80), Some(5));
    }

    #[test]
    fn a_cursor_left_of_its_own_line_means_the_line_wrapped() {
        assert_eq!(anchor_col(2, "cd wo", 80), None);
    }

    #[test]
    fn a_line_that_leaves_too_little_room_gets_no_anchor() {
        assert_eq!(anchor_col(71, "", 80), Some(71));
        assert_eq!(anchor_col(72, "", 80), None);
    }

    #[test]
    fn a_line_that_is_not_a_cd_is_left_to_the_shell() {
        // `ls` completes its own argument now that `spec_menu` answers a
        // `filepaths` template; `zzz` is what still matches nothing there,
        // in the fixture or among `ls`'s own options.
        let f = Fixture::new(&["work"]);
        assert!(seeded_bare("ls zzz", f.path(), HashMap::new()).is_none());
    }

    #[test]
    fn the_alias_map_is_in_place_before_the_first_refresh_reads_it() {
        // `App::over` would call `refresh` before `aliases` was ever set,
        // the bug this build fixes: an alias resolved only afterward leaves
        // the first screen with nothing behind it. `d ` has no specification
        // of its own, so items here can only have come from `docker`'s.
        let f = Fixture::new(&[]);
        let mut aliases = HashMap::new();
        aliases.insert("d".to_string(), "docker".to_string());
        let app =
            seeded_bare("d ", f.path(), aliases).expect("the alias reaches the first refresh");
        assert!(!app.items.is_empty());
    }

    #[test]
    fn a_cd_with_nothing_to_offer_is_left_to_the_shell() {
        let f = Fixture::new(&["work"]);
        assert!(seeded_bare("cd zzz", f.path(), HashMap::new()).is_none());
    }

    #[test]
    fn a_finished_word_is_left_to_the_shell() {
        // The space says the word is done. Nothing here would grow it and the
        // key therefore belongs to whatever the shell completes next.
        let f = Fixture::new(&["work"]);
        assert!(seeded_bare("cd work ", f.path(), HashMap::new()).is_none());
        assert!(seeded_bare("cd wo ", f.path(), HashMap::new()).is_none());
    }

    #[test]
    fn a_cd_opens_on_the_line_the_shell_handed_over() {
        let f = Fixture::new(&["work", "other"]);
        let app = seeded_bare("cd wo", f.path(), HashMap::new()).expect("a picker");
        assert_eq!(app.line.text(), "cd wo");
        assert!(app.line.at_end());
        assert!(app.menu_open());
        assert_eq!(app.items[0].insert, "work/");
    }

    #[test]
    fn a_bare_cd_has_something_to_offer() {
        let f = Fixture::new(&["work"]);
        assert!(seeded_bare("cd ", f.path(), HashMap::new()).is_some());
    }

    #[test]
    fn a_cd_that_already_names_a_directory_opens_on_the_row_that_runs_it() {
        // `work` holds nothing. The row that runs the line is the whole menu
        // and the shell would otherwise never see this line at all.
        let f = Fixture::new(&["work"]);
        assert!(
            seeded_bare("cd work/", f.path(), HashMap::new())
                .expect("a picker")
                .runs_the_line()
        );
    }

    #[test]
    fn empty_bytes_parse_to_the_empty_record() {
        let input = parse_input(b"");
        assert_eq!(input.rbuffer, "");
        assert_eq!(input.histfile, "");
        assert!(input.aliases.is_empty());
    }

    #[test]
    fn rbuffer_alone_needs_no_nul_to_follow_it() {
        let input = parse_input(b"tail");
        assert_eq!(input.rbuffer, "tail");
        assert_eq!(input.histfile, "");
        assert!(input.aliases.is_empty());
    }

    #[test]
    fn a_record_with_no_tag_is_read_as_the_shape_before_the_histfile_field() {
        // What a shell still holding the older widget sends. Its second
        // field is the first alias's own name rather than a path, and
        // reading it as one would shift every pair behind it.
        let input = parse_input(b"tail\0g\0git\0ll\0ls -la\0");
        assert_eq!(input.rbuffer, "tail");
        assert_eq!(input.histfile, "");
        assert_eq!(input.aliases.get("g").map(String::as_str), Some("git"));
        assert_eq!(input.aliases.get("ll").map(String::as_str), Some("ls -la"));
        assert_eq!(input.aliases.len(), 2);
    }

    #[test]
    fn an_older_record_whose_rbuffer_is_the_tag_loses_its_first_pair() {
        // The one way round the tag can be read where none was meant. Every
        // pair behind the first still lands, which is what holds this to
        // the one menu.
        let input = parse_input(b"surmise-record-3\0g\0git\0ll\0ls -la\0");
        assert_eq!(input.rbuffer, "g");
        assert_eq!(input.histfile, "git");
        assert_eq!(input.aliases.get("ll").map(String::as_str), Some("ls -la"));
        assert_eq!(input.aliases.len(), 1);
    }

    #[test]
    fn the_second_field_becomes_the_histfile_path() {
        let input = parse_input(b"surmise-record-3\0tail\0/sample/histfile\0");
        assert_eq!(input.rbuffer, "tail");
        assert_eq!(input.histfile, "/sample/histfile");
        assert!(input.aliases.is_empty());
    }

    #[test]
    fn pairs_after_the_histfile_field_become_the_alias_map() {
        let input = parse_input(b"surmise-record-3\0tail\0/sample/histfile\0g\0git\0ll\0ls -la\0");
        assert_eq!(input.rbuffer, "tail");
        assert_eq!(input.histfile, "/sample/histfile");
        assert_eq!(input.aliases.get("g").map(String::as_str), Some("git"));
        assert_eq!(input.aliases.get("ll").map(String::as_str), Some("ls -la"));
        assert_eq!(input.aliases.len(), 2);
    }

    #[test]
    fn a_value_keeps_a_space_and_a_newline() {
        let input = parse_input(b"surmise-record-3\0\0\0sample\0ls -la\nreally\0");
        assert_eq!(
            input.aliases.get("sample").map(String::as_str),
            Some("ls -la\nreally")
        );
    }

    #[test]
    fn a_trailing_nul_and_a_missing_one_read_the_same_pairs() {
        let trailing = parse_input(b"surmise-record-3\0\0\0g\0git\0");
        let missing = parse_input(b"surmise-record-3\0\0\0g\0git");
        assert_eq!(trailing.aliases, missing.aliases);
        assert_eq!(trailing.aliases.get("g").map(String::as_str), Some("git"));
    }

    #[test]
    fn a_malformed_record_drops_the_name_with_no_value() {
        // Three fields after `HISTFILE` cannot pair evenly. The first two
        // still make a pair and the third is dropped rather than guessed at.
        let input = parse_input(b"surmise-record-3\0\0\0g\0git\0extra");
        assert_eq!(input.aliases.get("g").map(String::as_str), Some("git"));
        assert_eq!(input.aliases.len(), 1);
    }
}
