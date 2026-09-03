//! The renderer.
//!
//! Everything is positioned relative to the row the input line starts on. The
//! frame is laid out into physical rows before anything is written. The
//! terminal therefore never auto-wraps and the cursor position is always
//! known. Two easier approaches are deliberately absent. Save and restore of
//! the cursor breaks the moment a paint scrolls the screen. A cursor-position
//! query hangs on a terminal that does not answer.

use crate::candidates::{Candidate, Kind};
use crate::line::Line;
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

pub const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const RESET: &str = "\x1b[0m";
/// The panel sits on a ground of its own that is a shade off the terminal's.
const PANEL: &str = "\x1b[48;5;236m";
/// The highlighted row's ground.
const PANEL_CHOSEN: &str = "\x1b[48;5;25m";
const NAME: &str = "\x1b[38;5;252m";
/// The name on the highlighted row's own ground. The glyph in front of it
/// wears a colour of its own and this is what puts the name back.
const NAME_CHOSEN: &str = "\x1b[97m";
const SPECIAL: &str = "\x1b[38;5;179m";
/// Nerd Font `nf-fa-folder`. This is the glyph on a directory row and on the
/// two places every shell can go from anywhere. `RUN_ICON` below is the other
/// one. A terminal without a patched font draws a blank box here.
const ICON: &str = "\u{f07b}";
/// The row that runs the line rather than growing it. This one is an ordinary
/// character, because the row names an action rather than a directory and a
/// terminal with no patched font still draws it.
const RUN_ICON: &str = "\u{21b5}";
const ICON_FG: &str = "\x1b[38;5;75m";
/// The glyph on the row that runs the line. A colour apart from the folder
/// blue is what marks that row out now that it carries no name.
const RUN_ICON_FG: &str = "\x1b[38;5;167m";
/// The same two glyphs on the highlighted row. That row has a ground of its
/// own and both colours above are too close to it to read. A lighter tint of
/// the same hue clears it and still says which sort of row this is.
const ICON_FG_CHOSEN: &str = "\x1b[38;5;153m";
const RUN_ICON_FG_CHOSEN: &str = "\x1b[38;5;217m";
const NAME_MAX: usize = 32;
const MENU_ROWS: usize = 8;

/// The terminal's width. It is never fewer than 24 cells and the panel's
/// layout arithmetic rests on that floor.
pub fn width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
        .max(24)
}

fn height() -> usize {
    crossterm::terminal::size()
        .map(|(_, h)| h as usize)
        .unwrap_or(24)
}

/// Display width in terminal cells. A character can be wider than one cell.
pub fn cells(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Drop every character that would drive the terminal rather than show up in
/// it. A directory name can hold an escape sequence and a paste can carry one.
/// A bidirectional override is not a control character and survives this.
pub fn printable(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// A run of text under one style.
#[derive(Clone)]
pub struct Seg {
    pub style: &'static str,
    pub text: String,
}

pub struct Menu<'a> {
    items: &'a [Candidate],
    selected: usize,
    rows: usize,
}

/// Build the menu for a candidate list and size it to the terminal. `None`
/// means there is nothing to show. The column the panel hangs from is the
/// renderer's to decide and `menu_rows` takes it.
pub fn menu(items: &[Candidate], selected: usize) -> Option<Menu<'_>> {
    menu_in(items, selected, height())
}

fn menu_in(items: &[Candidate], selected: usize, height: usize) -> Option<Menu<'_>> {
    if items.is_empty() {
        return None;
    }
    Some(Menu {
        items,
        selected: selected.min(items.len() - 1),
        // Four rows are left to the input line and to whatever the shell put
        // above it.
        rows: MENU_ROWS
            .min(items.len())
            .min(height.saturating_sub(4).max(1)),
    })
}

/// Pad `s` out to `w` cells. Cut it down and mark the cut when it is wider.
fn fit(s: &str, w: usize) -> String {
    let have = cells(s);
    if have <= w {
        return format!("{s}{}", " ".repeat(w - have));
    }
    if w == 0 {
        return String::new();
    }
    // One cell is held back for the ellipsis. Each character is measured
    // against the string it makes rather than added to a running total. A
    // character can join the one in front of it and the pair then takes more
    // cells together than the two apart.
    let mut out = String::new();
    for c in s.chars() {
        out.push(c);
        if cells(&out) > w - 1 {
            out.pop();
            break;
        }
    }
    out.push('…');
    format!("{out}{}", " ".repeat(w.saturating_sub(cells(&out))))
}

/// Break styled segments into physical rows of at most `w` cells. The active
/// style is re-opened at the start of each row. `indent` is what the first row
/// has already given away to whatever sits in front of it.
///
/// The result always holds at least one row.
fn wrap(segs: &[Seg], w: usize, indent: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    // What shows on the row being built. It is measured whole for the same
    // reason `fit` measures its prefix whole.
    let mut visible = String::new();
    let mut room = w.saturating_sub(indent);
    for seg in segs {
        row.push_str(seg.style);
        let text = printable(&seg.text);
        for c in text.chars() {
            visible.push(c);
            // A character that will not fit moves whole to the next row and
            // leaves the cell behind it empty.
            if cells(&visible) > room {
                if !seg.style.is_empty() {
                    row.push_str(RESET);
                }
                rows.push(std::mem::take(&mut row));
                row.push_str(seg.style);
                visible.clear();
                visible.push(c);
                room = w;
            }
            row.push(c);
        }
        if !seg.style.is_empty() {
            row.push_str(RESET);
        }
    }
    rows.push(row);
    rows
}

/// Display width of a rendered row. The escape sequences do not count.
fn cells_of_row(row: &str) -> usize {
    let mut visible = String::new();
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Run to the sequence's final byte. It is the only letter in one.
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            visible.push(c);
        }
    }
    cells(&visible)
}

fn window_start(selected: usize, rows: usize, total: usize) -> usize {
    if total <= rows {
        return 0;
    }
    selected.saturating_sub(rows / 2).min(total - rows)
}

fn colour(k: Kind) -> &'static str {
    match k {
        // The row that runs the line carries no name and only its glyph is
        // coloured. `glyph` below is where that happens.
        Kind::Dir | Kind::Run => NAME,
        Kind::Special => SPECIAL,
    }
}

/// The glyph in front of a row and the colour it wears, on the highlighted row
/// or off it.
fn glyph(k: Kind, chosen: bool) -> (&'static str, &'static str) {
    // Every variant is named rather than swept into a catch-all. A new one
    // then fails the build here the way it already does in `colour` above.
    match (k, chosen) {
        (Kind::Run, false) => (RUN_ICON, RUN_ICON_FG),
        (Kind::Run, true) => (RUN_ICON, RUN_ICON_FG_CHOSEN),
        (Kind::Dir | Kind::Special, false) => (ICON, ICON_FG),
        (Kind::Dir | Kind::Special, true) => (ICON, ICON_FG_CHOSEN),
    }
}

/// The panel for `m` in a terminal `w` cells wide. `col` is where the cursor
/// sits and the first name lands there. The panel therefore follows the
/// cursor.
fn menu_rows(m: &Menu, w: usize, col: usize) -> Vec<String> {
    let Some(current) = m.items.get(m.selected) else {
        return Vec::new();
    };
    let first = window_start(m.selected, m.rows, m.items.len());
    let last = (first + m.rows).min(m.items.len());
    let shown = &m.items[first..last];
    let foot_r = format!("{}/{}", m.selected + 1, m.items.len());

    // The panel is as wide as its widest row and no wider. Past that it gives
    // way to the terminal.
    let icon_w = cells(ICON).max(cells(RUN_ICON)) + 1;
    let widest = shown
        .iter()
        .map(|c| icon_w + cells(&c.display))
        .chain(std::iter::once(cells(current.label) + cells(&foot_r) + 2))
        .max()
        .unwrap_or(10);
    // The width comes first. It is the panel's own and the terminal is the
    // only thing that takes it back.
    let inner = widest.clamp(8, NAME_MAX).min(w.saturating_sub(2));
    // The panel opens with a space of its own. Starting one column early puts
    // the first name under the cursor rather than past it. It keeps the width
    // above and slides left of the cursor when the right edge is nearer than
    // that.
    let indent = col.saturating_sub(1).min(w.saturating_sub(inner + 2));
    // A terminal too narrow for the icon and a name gets no panel at all.
    let Some(text_w) = inner.checked_sub(icon_w) else {
        return Vec::new();
    };
    let pad = " ".repeat(indent);

    let mut rows: Vec<String> = shown
        .iter()
        .enumerate()
        .map(|(r, c)| {
            let name = fit(&printable(&c.display), text_w);
            let chosen = first + r == m.selected;
            let ground = if chosen { PANEL_CHOSEN } else { PANEL };
            // The glyph's own colour replaces whatever the name wanted. The
            // name therefore names its colour again behind it.
            let name_fg = if chosen { NAME_CHOSEN } else { colour(c.kind) };
            let (icon, icon_fg) = glyph(c.kind, chosen);
            format!("{pad}{ground} {icon_fg}{icon} {name_fg}{name} {RESET}")
        })
        .collect();

    // The count goes in only if it fits beside the label.
    let mut foot = current.label.to_string();
    let room = inner.saturating_sub(cells(&foot) + cells(&foot_r));
    if room > 0 {
        foot.push_str(&" ".repeat(room));
        foot.push_str(&foot_r);
    }
    rows.push(format!(
        "{pad}{PANEL}{DIM}{ITALIC} {} {RESET}",
        fit(&foot, inner)
    ));
    rows
}

pub struct Ui<W> {
    /// Where the frame is written. The picker writes to /dev/tty, because its
    /// stdout carries the result back to the shell. A test writes to a buffer
    /// and reads the bytes back out of it.
    out: W,
    /// Row of the cursor relative to the first input row. `None` when the next
    /// frame should start where the cursor already is.
    cursor_row: Option<usize>,
    /// Column the frame's first row starts on. The picker draws that row over
    /// the shell's own prompt and everything to the left of this column
    /// belongs to the shell. It is never written to and never cleared. Every
    /// row below starts at the left edge and is the frame's own to use.
    anchor: usize,
}

impl<W: Write> Ui<W> {
    pub fn new(out: W, anchor: usize) -> Ui<W> {
        Ui {
            out,
            cursor_row: None,
            anchor,
        }
    }

    /// Forget the painted frame. Use after writing ordinary output.
    pub fn detach(&mut self) {
        self.cursor_row = None;
    }

    /// The absolute column the end of row `r` sits on. Only the first row
    /// starts past the left edge.
    fn end_col(&self, rows: &[String], r: usize) -> usize {
        cells_of_row(&rows[r]) + if r == 0 { self.anchor } else { 0 }
    }

    pub fn render(
        &mut self,
        prompt: &[Seg],
        line: &Line,
        ghost: &str,
        menu: Option<Menu>,
    ) -> io::Result<()> {
        self.render_at(prompt, line, ghost, menu, width())
    }

    /// The frame for a terminal `w` cells wide.
    fn render_at(
        &mut self,
        prompt: &[Seg],
        line: &Line,
        ghost: &str,
        menu: Option<Menu>,
        w: usize,
    ) -> io::Result<()> {
        // The cursor is placed by laying out what sits in front of it rather
        // than by dividing a cell count by the width. A character that will
        // not fit leaves a gap at the end of a row and arithmetic misses it.
        let mut segs = prompt.to_vec();
        let left = line.left_of_cursor();
        segs.push(Seg {
            style: "",
            text: left.to_string(),
        });
        let placed = wrap(&segs, w, self.anchor);
        let mut cursor_row = placed.len() - 1;
        let mut cursor_col = self.end_col(&placed, cursor_row);
        // A cursor at the right edge belongs at the start of the next row.
        // `wrap` only opens that row once a character needs it.
        if cursor_col >= w {
            cursor_row += 1;
            cursor_col = 0;
        }

        // The cursor offset is the length of what is behind it.
        segs.push(Seg {
            style: "",
            text: line.text()[left.len()..].to_string(),
        });
        if !ghost.is_empty() {
            segs.push(Seg {
                style: DIM,
                text: ghost.to_string(),
            });
        }

        let mut rows = wrap(&segs, w, self.anchor);
        while rows.len() <= cursor_row {
            rows.push(String::new());
        }
        if let Some(m) = menu {
            rows.extend(menu_rows(&m, w, cursor_col));
        }
        self.paint(&rows, cursor_row, cursor_col)
    }

    /// Leave the input line on screen and park the cursor on a fresh row below
    /// it so ordinary output can follow.
    pub fn close(&mut self, prompt: &[Seg], line: &Line) -> io::Result<()> {
        let mut segs = prompt.to_vec();
        segs.push(Seg {
            style: "",
            text: line.text().to_string(),
        });
        let rows = wrap(&segs, width(), self.anchor);
        let last = rows.len() - 1;
        let col = self.end_col(&rows, last);
        self.paint(&rows, last, col)?;
        self.out.write_all(b"\r\n")?;
        self.out.flush()?;
        self.cursor_row = None;
        Ok(())
    }

    /// Erase everything this Ui painted and leave the cursor where the frame
    /// started. The picker uses it so the shell's own line survives untouched.
    pub fn erase(&mut self) -> io::Result<()> {
        let up = self.cursor_row.take().unwrap_or(0);
        let mut buf = String::new();
        if up > 0 {
            buf.push_str(&format!("\x1b[{up}A"));
        }
        buf.push('\r');
        if self.anchor > 0 {
            buf.push_str(&format!("\x1b[{}C", self.anchor));
        }
        buf.push_str("\x1b[J");
        self.out.write_all(buf.as_bytes())?;
        self.out.flush()
    }

    fn paint(&mut self, rows: &[String], cursor_row: usize, cursor_col: usize) -> io::Result<()> {
        let mut buf = String::from("\x1b[?25l"); // hide the cursor while painting
        if let Some(up) = self.cursor_row
            && up > 0
        {
            buf.push_str(&format!("\x1b[{up}A"));
        }
        buf.push('\r');
        if self.anchor > 0 {
            buf.push_str(&format!("\x1b[{}C", self.anchor));
        }
        buf.push_str("\x1b[J"); // clear from here to the end of the screen
        buf.push_str(&rows.join("\r\n"));

        let back = (rows.len() - 1).saturating_sub(cursor_row);
        if back > 0 {
            buf.push_str(&format!("\x1b[{back}A"));
        }
        buf.push('\r');
        if cursor_col > 0 {
            buf.push_str(&format!("\x1b[{cursor_col}C"));
        }
        buf.push_str("\x1b[?25h");

        self.out.write_all(buf.as_bytes())?;
        self.out.flush()?;
        self.cursor_row = Some(cursor_row);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::{folder, run_row};

    fn dir(display: &str) -> Candidate {
        folder(display.to_string(), display.to_string(), 0)
    }

    fn dirs(n: usize) -> Vec<Candidate> {
        (0..n).map(|i| dir(&format!("d{i}"))).collect()
    }

    fn seg(text: &str) -> Seg {
        Seg {
            style: "",
            text: text.to_string(),
        }
    }

    /// One frame written to a sink a test can read back.
    fn drawn(anchor: usize, w: usize, text: &str, ghost: &str) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert(text);
        Ui::new(&mut buf, anchor)
            .render_at(&[], &line, ghost, None, w)
            .expect("a Vec always takes a write");
        String::from_utf8(buf).expect("the frame is text")
    }

    #[test]
    fn a_wide_character_counts_as_two_cells() {
        assert_eq!(cells("日本"), 4);
    }

    #[test]
    fn fit_pads_a_short_name_out_to_the_width() {
        assert_eq!(fit("ab", 5), "ab   ");
        assert_eq!(fit("", 3), "   ");
    }

    #[test]
    fn fit_cuts_a_long_name_and_marks_the_cut() {
        assert_eq!(fit("abcdef", 4), "abc…");
    }

    #[test]
    fn a_character_that_joins_the_one_in_front_of_it_is_measured_with_it() {
        // The width of a string is not the sum of its characters' widths. An
        // emoji and the selector behind it take two cells together and one
        // apart.
        let pair = "\u{2764}\u{FE0F}";
        assert_eq!(cells(pair), 2);
        assert_eq!(cells_of_row(&format!("{DIM}{pair}{RESET}")), 2);
    }

    #[test]
    fn fit_cuts_to_the_width_a_terminal_will_draw() {
        // Summing the characters leaves this one cell over the width. The row
        // then wraps and tears the panel in half.
        let name = format!("\u{2764}\u{FE0F}{}", "a".repeat(40));
        assert_eq!(cells(&fit(&name, 30)), 30);
    }

    #[test]
    fn wrap_breaks_at_the_width_a_terminal_will_draw() {
        let rows = wrap(&[seg("\u{2764}\u{FE0F}aaa")], 4, 0);
        assert!(rows.iter().all(|r| cells_of_row(r) <= 4), "{rows:?}");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn fit_never_splits_a_wide_character() {
        assert_eq!(fit("日本語", 3), "日…");
        assert_eq!(cells(&fit("日本語", 4)), 4);
    }

    #[test]
    fn fit_into_nothing_gives_nothing() {
        assert_eq!(fit("abc", 0), "");
    }

    #[test]
    fn wrap_leaves_a_short_line_on_one_row() {
        assert_eq!(wrap(&[seg("hello")], 20, 0), vec!["hello"]);
    }

    #[test]
    fn wrap_breaks_at_the_width() {
        assert_eq!(wrap(&[seg("abcdef")], 4, 0), vec!["abcd", "ef"]);
    }

    #[test]
    fn wrap_counts_the_indent_against_the_first_row_only() {
        assert_eq!(wrap(&[seg("abcdef")], 4, 2), vec!["ab", "cdef"]);
    }

    #[test]
    fn wrap_moves_a_wide_character_whole_and_leaves_the_gap() {
        // The second cell of 日 will not fit in four. The character therefore
        // starts the next row and the fourth column stays empty.
        assert_eq!(wrap(&[seg("abc日")], 4, 0), vec!["abc", "日"]);
    }

    #[test]
    fn wrap_reopens_the_style_on_every_row() {
        let styled = Seg {
            style: DIM,
            text: "abcdef".to_string(),
        };
        assert_eq!(
            wrap(&[styled], 4, 0),
            vec![format!("{DIM}abcd{RESET}"), format!("{DIM}ef{RESET}")]
        );
    }

    #[test]
    fn wrap_always_gives_back_a_row() {
        assert_eq!(wrap(&[], 10, 0), vec![String::new()]);
    }

    #[test]
    fn a_rendered_row_measures_only_what_shows() {
        assert_eq!(cells_of_row(&format!("{DIM}ab{RESET}")), 2);
        assert_eq!(cells_of_row(&format!("{PANEL} 日 {RESET}")), 4);
    }

    #[test]
    fn a_list_that_fits_starts_at_the_top() {
        assert_eq!(window_start(3, 8, 5), 0);
    }

    #[test]
    fn a_longer_list_keeps_the_selection_in_the_middle() {
        assert_eq!(window_start(10, 4, 40), 8);
    }

    #[test]
    fn the_window_stops_at_the_end_of_the_list() {
        assert_eq!(window_start(39, 4, 40), 36);
    }

    #[test]
    fn an_empty_list_has_no_menu() {
        assert!(menu_in(&[], 0, 24).is_none());
    }

    #[test]
    fn a_selection_past_the_end_is_pulled_back() {
        let items = dirs(3);
        assert_eq!(menu_in(&items, 99, 24).expect("a menu").selected, 2);
    }

    #[test]
    fn the_menu_never_grows_past_its_own_cap() {
        let items = dirs(40);
        assert_eq!(menu_in(&items, 0, 40).expect("a menu").rows, MENU_ROWS);
    }

    #[test]
    fn a_short_terminal_shortens_the_menu() {
        let items = dirs(40);
        assert_eq!(menu_in(&items, 0, 7).expect("a menu").rows, 3);
        assert_eq!(menu_in(&items, 0, 1).expect("a menu").rows, 1);
    }

    /// Cells of empty space in front of a panel row.
    fn pad_of(row: &str) -> usize {
        row.chars().take_while(|&c| c == ' ').count()
    }

    #[test]
    fn the_panel_starts_one_column_before_the_cursor() {
        let items = dirs(1);
        let m = menu_in(&items, 0, 24).expect("a menu");
        assert_eq!(pad_of(&menu_rows(&m, 80, 10)[0]), 9);
        assert_eq!(pad_of(&menu_rows(&m, 80, 0)[0]), 0);
    }

    #[test]
    fn the_panel_slides_left_of_a_cursor_near_the_edge() {
        let items = dirs(1);
        let m = menu_in(&items, 0, 24).expect("a menu");
        let width = cells_of_row(&menu_rows(&m, 80, 0)[0]);
        // The cursor is far enough right that the panel would run off the end.
        let rows = menu_rows(&m, 40, 38);
        assert_eq!(pad_of(&rows[0]) + width, 40);
        // It keeps the width it had against the left edge.
        assert_eq!(cells_of_row(&rows[0]) - pad_of(&rows[0]), width);
    }

    #[test]
    fn the_menu_draws_a_row_for_each_entry_and_one_footer() {
        let items = dirs(3);
        let m = menu_in(&items, 0, 24).expect("a menu");
        assert_eq!(menu_rows(&m, 80, 1).len(), 4);
    }

    #[test]
    fn the_footer_carries_the_position_in_the_list() {
        let items = dirs(3);
        let m = menu_in(&items, 1, 24).expect("a menu");
        let rows = menu_rows(&m, 80, 1);
        assert!(rows.last().expect("a footer").contains("2/3"));
    }

    #[test]
    fn one_row_is_drawn_as_chosen_and_it_is_the_selected_one() {
        let items = dirs(3);
        let m = menu_in(&items, 1, 24).expect("a menu");
        let chosen: Vec<usize> = menu_rows(&m, 80, 1)
            .iter()
            .enumerate()
            .filter(|(_, r)| r.contains(PANEL_CHOSEN))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(chosen, vec![1]);
    }

    #[test]
    fn every_menu_row_is_the_same_width() {
        let items = vec![dir("short"), dir("a-much-longer-directory-name")];
        let m = menu_in(&items, 0, 24).expect("a menu");
        let widths: Vec<usize> = menu_rows(&m, 80, 5)
            .iter()
            .map(|r| cells_of_row(r))
            .collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
    }

    #[test]
    fn a_control_character_never_reaches_the_terminal() {
        assert_eq!(printable("\x1b[31mred"), "[31mred");
        assert_eq!(printable("a\x07b\x7fc"), "abc");
        assert_eq!(wrap(&[seg("a\x1bb")], 20, 0), vec!["ab"]);
    }

    #[test]
    fn an_escape_in_a_directory_name_is_dropped_from_the_panel() {
        let items = vec![dir("\x1b[31mred")];
        let m = menu_in(&items, 0, 24).expect("a menu");
        let rows = menu_rows(&m, 80, 1);
        let row = rows.first().expect("a row");
        assert!(row.contains("[31mred"), "{row:?}");
        // The only escapes left are the ones this module wrote itself: the
        // ground, the glyph's colour, the name's and the reset.
        let ours = [PANEL_CHOSEN, NAME_CHOSEN, ICON_FG_CHOSEN, RESET]
            .iter()
            .map(|s| s.matches('\x1b').count())
            .sum();
        assert_eq!(row.matches('\x1b').count(), ours, "{row:?}");
    }

    #[test]
    fn a_special_row_is_coloured_apart_from_a_directory() {
        assert_eq!(colour(Kind::Dir), NAME);
        assert_eq!(colour(Kind::Special), SPECIAL);
    }

    #[test]
    fn the_row_that_runs_the_line_is_told_apart_by_its_glyph() {
        // The row carries no name and the glyph is therefore the only thing
        // left to tell it apart from a directory. That holds on the
        // highlighted row as well. White there would read as a directory and
        // would take the row's sort with it.
        for chosen in [false, true] {
            let (dir_icon, dir_fg) = glyph(Kind::Dir, chosen);
            let (run_icon, run_fg) = glyph(Kind::Run, chosen);
            assert_eq!(glyph(Kind::Special, chosen), (dir_icon, dir_fg));
            assert_ne!(run_icon, dir_icon);
            assert_ne!(run_fg, dir_fg);
        }
    }

    #[test]
    fn the_glyph_on_the_highlighted_row_is_not_the_name_colour() {
        let items = vec![dir("alpha/")];
        let m = menu_in(&items, 0, 24).expect("a menu");
        let row = menu_rows(&m, 80, 1).first().expect("a row").clone();
        // The glyph carries its own colour and the name names the ground's
        // bright one again behind it.
        assert_ne!(ICON_FG_CHOSEN, NAME_CHOSEN);
        let want = format!("{ICON_FG_CHOSEN}{ICON} {NAME_CHOSEN}");
        assert!(row.contains(&want), "{row:?}");
    }

    #[test]
    fn the_row_that_runs_the_line_holds_no_name() {
        let items = vec![run_row("work/".to_string()), dir("alpha/")];
        let m = menu_in(&items, 0, 24).expect("a menu");
        let rows = menu_rows(&m, 80, 1);
        let row = rows.first().expect("a row");
        assert!(row.contains(RUN_ICON), "{row:?}");
        // `insert` is the only text this row could have drawn.
        assert!(!row.contains("work"), "{row:?}");
    }

    #[test]
    fn the_two_row_glyphs_are_the_same_width() {
        // `menu_rows` measures the glyph column once from the widest glyph and
        // the panel is therefore wide enough whatever they measure. Names line
        // up only while the two agree, because each row spends
        // `cells(glyph) + 1` on its own before the name starts.
        assert_eq!(cells(ICON), cells(RUN_ICON));
    }

    #[test]
    fn a_prompt_sits_in_front_of_the_line_and_moves_the_cursor() {
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("cd");
        let prompt = vec![Seg {
            style: DIM,
            text: "\u{25b8} ".to_string(),
        }];
        Ui::new(&mut buf, 0)
            .render_at(&prompt, &line, "", None, 40)
            .expect("a Vec always takes a write");
        let out = String::from_utf8(buf).expect("the frame is text");
        assert!(out.contains(&format!("{DIM}\u{25b8} {RESET}cd")), "{out:?}");
        // Two cells of prompt and two of line.
        assert!(out.ends_with("\r\x1b[4C\x1b[?25h"), "{out:?}");
    }

    #[test]
    fn a_terminal_too_narrow_for_a_panel_gets_none() {
        let items = dirs(3);
        let m = menu_in(&items, 0, 24).expect("a menu");
        assert!(menu_rows(&m, 3, 1).is_empty());
    }

    #[test]
    fn a_frame_hides_the_cursor_while_it_paints() {
        let out = drawn(0, 40, "cd src", "");
        assert!(out.starts_with("\x1b[?25l"), "{out:?}");
        assert!(out.ends_with("\x1b[?25h"), "{out:?}");
    }

    #[test]
    fn a_frame_holds_the_line_it_was_given() {
        assert!(drawn(0, 40, "cd src", "").contains("cd src"));
    }

    #[test]
    fn the_ghost_follows_the_line_dimmed() {
        let out = drawn(0, 40, "cd s", "rc");
        assert!(out.contains(&format!("cd s{DIM}rc{RESET}")), "{out:?}");
    }

    #[test]
    fn the_anchor_counts_toward_the_cursor_on_the_first_row() {
        let out = drawn(4, 40, "cd src", "");
        assert!(out.ends_with("\r\x1b[10C\x1b[?25h"), "{out:?}");
    }

    #[test]
    fn a_line_that_outruns_the_terminal_wraps_onto_the_next_row() {
        let out = drawn(0, 6, "cd abcdefgh", "");
        assert!(out.contains("cd abc\r\ndefgh"), "{out:?}");
    }

    #[test]
    fn a_cursor_at_the_right_edge_moves_to_the_next_row() {
        // Six cells fill the row exactly. The cursor belongs on a row of its
        // own rather than one past the edge of this one.
        let out = drawn(0, 6, "abcdef", "");
        assert!(out.ends_with("abcdef\r\n\r\x1b[?25h"), "{out:?}");
    }

    #[test]
    fn the_cursor_walks_back_up_over_the_menu() {
        let items = dirs(2);
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("cd d");
        Ui::new(&mut buf, 0)
            .render_at(&[], &line, "", menu_in(&items, 0, 24), 40)
            .expect("a Vec always takes a write");
        let out = String::from_utf8(buf).expect("the frame is text");
        // One input row, two entries and a footer.
        assert!(out.contains("\x1b[3A"), "{out:?}");
        assert!(out.contains("d0") && out.contains("d1"), "{out:?}");
    }

    #[test]
    fn a_second_frame_walks_back_up_to_the_first_row() {
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("abcdefg");
        {
            let mut ui = Ui::new(&mut buf, 0);
            ui.render_at(&[], &line, "", None, 4).expect("write");
            ui.render_at(&[], &line, "", None, 4).expect("write");
        }
        let out = String::from_utf8(buf).expect("the frame is text");
        assert!(out.contains("\x1b[?25h\x1b[?25l\x1b[1A"), "{out:?}");
    }

    #[test]
    fn detach_makes_the_next_frame_start_where_the_cursor_already_is() {
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("abcdefg");
        {
            let mut ui = Ui::new(&mut buf, 0);
            ui.render_at(&[], &line, "", None, 4).expect("write");
            ui.detach();
            ui.render_at(&[], &line, "", None, 4).expect("write");
        }
        let out = String::from_utf8(buf).expect("the frame is text");
        assert!(!out.contains("\x1b[1A"), "{out:?}");
    }

    #[test]
    fn erase_walks_back_to_where_the_frame_started() {
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("cd src");
        {
            let mut ui = Ui::new(&mut buf, 4);
            ui.render_at(&[], &line, "", None, 40).expect("write");
            ui.erase().expect("write");
        }
        let out = String::from_utf8(buf).expect("the frame is text");
        assert!(out.ends_with("\r\x1b[4C\x1b[J"), "{out:?}");
    }

    #[test]
    fn close_leaves_the_line_and_a_fresh_row_below_it() {
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("cd src");
        Ui::new(&mut buf, 0)
            .close(&[], &line)
            .expect("a Vec always takes a write");
        let out = String::from_utf8(buf).expect("the frame is text");
        assert!(out.contains("cd src"), "{out:?}");
        assert!(out.ends_with("\r\n"), "{out:?}");
    }
}
