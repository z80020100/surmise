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
use crate::keys;
use crate::tty;
use crate::ui;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io::{self, Write};
use std::path::Path;

/// Take the line back and leave it on the shell's editor. It is on stdout.
pub const ACCEPTED: u8 = 0;
/// Undo. The shell keeps the line the person started with.
pub const CANCELLED: u8 = 1;
/// Not a line surmise completes. The shell runs its own completion instead.
pub const PASS: u8 = 2;
/// Take the line back and run it. It is on stdout.
pub const RUN: u8 = 3;

/// Cells the frame wants to the right of the column it starts on. A line
/// that starts closer than this to the right edge gets a row of its own
/// instead.
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

/// The state for `seed`. `None` when surmise has nothing to offer and the key
/// therefore belongs to the shell.
fn seeded(seed: &str, cwd: &Path) -> Option<App> {
    let app = App::over(cwd, seed);
    (!app.items.is_empty()).then_some(app)
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
    // Without a current directory there is nothing to complete against. The
    // shell's own completion is the honest answer.
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(PASS);
    };
    // Nothing to offer. Give the key back without touching the terminal.
    let Some(mut app) = seeded(seed, &cwd) else {
        return Ok(PASS);
    };

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
    let base = frame_col + ui::cells_of(&head);

    // The frame goes out through a handle of its own. `term` therefore stays
    // open for the writes that follow the loop.
    let mut ui = ui::Ui::new(term.try_clone()?, frame_col);

    let outcome = loop {
        // `App` decides whether the menu shows. `ui` only sizes it.
        let menu = app
            .menu_open()
            .then(|| ui::menu(&app.items, app.selected, base + app.arg_col()))
            .flatten();
        ui.render(&head, &app.line, &app.ghost(), menu)?;

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
                    KeyCode::Tab => {
                        app.accept();
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
                            break RUN;
                        }
                        if !app.accept() {
                            break RUN;
                        }
                        // The descent landed somewhere with nothing to show
                        // and nothing to go on into. A directory nobody may
                        // read does that. Hand the line to the shell's own
                        // editor rather than hold a frame with no menu on it.
                        if !app.menu_open() {
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
        let f = Fixture::new(&["work"]);
        assert!(seeded("ls wo", f.path()).is_none());
    }

    #[test]
    fn a_cd_with_nothing_to_offer_is_left_to_the_shell() {
        let f = Fixture::new(&["work"]);
        assert!(seeded("cd zzz", f.path()).is_none());
    }

    #[test]
    fn a_cd_opens_on_the_line_the_shell_handed_over() {
        let f = Fixture::new(&["work", "other"]);
        let app = seeded("cd wo", f.path()).expect("a picker");
        assert_eq!(app.line.text(), "cd wo");
        assert!(app.line.at_end());
        assert!(app.menu_open());
        assert_eq!(app.items[0].insert, "work/");
    }

    #[test]
    fn a_bare_cd_has_something_to_offer() {
        let f = Fixture::new(&["work"]);
        assert!(seeded("cd ", f.path()).is_some());
    }

    #[test]
    fn a_cd_that_already_names_a_directory_opens_on_the_row_that_runs_it() {
        // `work` holds nothing. The row that runs the line is the whole menu
        // and the shell would otherwise never see this line at all.
        let f = Fixture::new(&["work"]);
        assert!(
            seeded("cd work/", f.path())
                .expect("a picker")
                .runs_the_line()
        );
    }
}
