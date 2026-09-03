//! The renderer.
//!
//! Everything is positioned relative to the row the input line starts on. The
//! frame is laid out into physical rows before anything is written. The
//! terminal therefore never auto-wraps and the cursor position is always
//! known. Two easier approaches are deliberately absent. Save and restore of
//! the cursor breaks the moment a paint scrolls the screen. A cursor-position
//! query hangs on a terminal that does not answer.

use crate::candidates::{Candidate, Kind};
use crate::fuzzy;
use crate::line::Line;
use std::io::{self, Write};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

pub const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const RESET: &str = "\x1b[0m";
/// The panel sits on a ground of its own that is a shade off the terminal's.
const PANEL: &str = "\x1b[48;5;236m";
/// The highlighted row's ground.
const PANEL_CHOSEN: &str = "\x1b[48;5;26m";
const NAME: &str = "\x1b[38;5;249m";
/// The name on the highlighted row's own ground. The glyph in front of it
/// wears a colour of its own and this is what puts the name back.
const NAME_CHOSEN: &str = "\x1b[97m";
const SPECIAL: &str = "\x1b[38;5;179m";
/// The word under the list. It reads at the weight a name does and italic
/// alone is what sets it apart. What the row says is worth reading rather
/// than worth fading out.
const FOOT: &str = NAME;
/// The ground under a character what was typed reached. A dark olive rather
/// than a tint of the panel's own grey. A mark then reads as a mark rather
/// than as another row.
const MARK: &str = "\x1b[48;5;58m";
/// The same on the highlighted row. That row's ground is a blue the olive
/// disappears into and a lighter tint of that blue takes over.
const MARK_CHOSEN: &str = "\x1b[48;5;68m";
/// A marked character's own name. A brighter tint of the colour its row
/// already wears: the ground says which characters what was typed reached and
/// the brighter name is what makes them read first. `bright` below is what
/// pairs one with a row. The highlighted row has none of its own, because
/// `NAME_CHOSEN` is already as bright as a name gets.
const NAME_MARKED: &str = "\x1b[38;5;188m";
const SPECIAL_MARKED: &str = "\x1b[38;5;222m";
/// The run Tab would add to the argument. An underline rather than a ground
/// of its own: the characters what was typed reached already carry one and a
/// second ground beside it would read as another row.
/// The line between the list and the word under it. A shade off the panel's
/// own ground rather than a name's colour: it separates two things rather
/// than saying anything of its own.
const BORDER: &str = "\x1b[38;5;238m";
/// What that line is drawn with.
const RULE: char = '\u{2500}';
const UNDER: &str = "\x1b[4m";
const UNDER_OFF: &str = "\x1b[24m";
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
/// Cells inside the panel's own two edge spaces. It is fixed rather than
/// measured from what the panel holds. Every panel is therefore the same
/// width and a name keeps the column the eye last found it in. A terminal
/// with no room for all of it takes some back.
const PANEL_INNER: usize = 40;
const MENU_ROWS: usize = 6;

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
    /// What the rows were matched against. The characters it reached in a name
    /// carry a ground and a brighter name of their own.
    typed: &'a str,
    /// How many characters of a name Tab would leave on the line. The run of
    /// that past what was typed is underlined.
    reach: usize,
}

/// Build the menu for a candidate list and size it to the terminal. `None`
/// means there is nothing to show. The column the panel hangs from and the
/// item it opens on are the renderer's to decide and `menu_rows` takes both.
/// `typed` and `reach` are not: the caller is what knows what the rows
/// answered and what a key would take from them.
pub fn menu<'a>(
    items: &'a [Candidate],
    selected: usize,
    typed: &'a str,
    reach: usize,
) -> Option<Menu<'a>> {
    menu_in(items, selected, height(), typed, reach)
}

fn menu_in<'a>(
    items: &'a [Candidate],
    selected: usize,
    height: usize,
    typed: &'a str,
    reach: usize,
) -> Option<Menu<'a>> {
    if items.is_empty() {
        return None;
    }
    Some(Menu {
        items,
        typed,
        reach,
        selected: selected.min(items.len() - 1),
        // Five rows are left to the input line, to the line and the word
        // under the list and to whatever the shell put above it.
        rows: MENU_ROWS
            .min(items.len())
            .min(height.saturating_sub(5).max(1)),
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

/// The first item the panel shows. `top` is what the last frame showed and
/// the window moves as little as it can from there. The highlight therefore
/// stays on the row the eye last found it on and the list moves one row for
/// one press once the highlight reaches an edge. The last clamp answers a
/// `top` with no full window left under it. A list that got shorter or a
/// panel that got taller leaves one. `rows` is never 0. `menu_in` floors it
/// at one and turns an empty list away.
fn window_start(top: usize, selected: usize, rows: usize, total: usize) -> usize {
    if total <= rows {
        return 0;
    }
    top.min(selected)
        .max(selected.saturating_sub(rows - 1))
        .min(total - rows)
}

fn colour(k: Kind) -> &'static str {
    match k {
        // The row that runs the line carries no name and only its glyph is
        // coloured. `glyph` below is where that happens.
        Kind::Dir | Kind::Run => NAME,
        Kind::Special => SPECIAL,
    }
}

/// The name a marked character wears. It is a brighter tint of what `colour`
/// gives the same row rather than one colour for every row. One colour would
/// take away what tells a directory from one of the two places every shell
/// can go from anywhere.
fn bright(k: Kind) -> &'static str {
    match k {
        Kind::Dir | Kind::Run => NAME_MARKED,
        Kind::Special => SPECIAL_MARKED,
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

/// Whether Tab would grow this row's name. `common` reads the menu the same
/// way and the underline therefore covers what the key would take.
fn tab_grows(k: Kind, at: &[usize]) -> bool {
    match k {
        // The row that runs the line grows nothing and the two places every
        // shell can go from anywhere are not names Tab reads.
        Kind::Run | Kind::Special => false,
        // A match is a subsequence. Only a name whose match is the front of it
        // can take a prefix the menu agreed on.
        Kind::Dir => at.iter().enumerate().all(|(i, &j)| i == j),
    }
}

/// `name` with the characters at `at` under `on` and the run at `under`
/// underlined. `off` is what the row had and a mark hands it back, because a
/// code ends the run it opened rather than the row. `on` names its ground
/// last and what follows a mark is therefore the character itself.
fn marked(name: &str, at: &[usize], under: &Range<usize>, off: &str, on: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        let on_a_mark = at.contains(&i);
        let underlined = under.contains(&i);
        if on_a_mark {
            out.push_str(on);
        }
        if underlined {
            out.push_str(UNDER);
        }
        out.push(c);
        if underlined {
            out.push_str(UNDER_OFF);
        }
        if on_a_mark {
            out.push_str(off);
        }
    }
    out
}

/// The panel for `m` in a terminal `w` cells wide. `col` is where the cursor
/// sits and the first name lands there. The panel therefore follows the
/// cursor. `first` is the item the panel opens on and `window_start` is
/// where it comes from. A `first` from anywhere else can draw a panel with
/// the highlight outside it.
fn menu_rows(m: &Menu, w: usize, col: usize, first: usize) -> Vec<String> {
    let Some(current) = m.items.get(m.selected) else {
        return Vec::new();
    };
    let last = (first + m.rows).min(m.items.len());
    let shown = &m.items[first..last];
    let foot_r = format!("{}/{}", m.selected + 1, m.items.len());

    // The width comes first. Nothing below holds a floor under it and `width`
    // is what keeps one.
    let inner = PANEL_INNER.min(w.saturating_sub(2));
    // The panel opens with a space of its own. Starting one column early puts
    // the first row's glyph under the cursor rather than past it. It keeps the
    // width above and slides left of the cursor when the right edge is nearer
    // than that.
    let indent = col.saturating_sub(1).min(w.saturating_sub(inner + 2));
    let icon_w = cells(ICON).max(cells(RUN_ICON)) + 1;
    // A terminal too narrow for the icon and a name gets no panel at all.
    let Some(text_w) = inner.checked_sub(icon_w) else {
        return Vec::new();
    };
    let pad = " ".repeat(indent);

    let mut rows: Vec<String> = shown
        .iter()
        .enumerate()
        .map(|(r, c)| {
            let text = printable(&c.display);
            let chosen = first + r == m.selected;
            let ground = if chosen { PANEL_CHOSEN } else { PANEL };
            // The glyph's own colour replaces whatever the name wanted. The
            // name therefore names its colour again behind it.
            let name_fg = if chosen { NAME_CHOSEN } else { colour(c.kind) };
            // A mark carries a ground and a name colour together.
            let (mark, mark_fg) = if chosen {
                (MARK_CHOSEN, NAME_CHOSEN)
            } else {
                (MARK, bright(c.kind))
            };
            // The name is fitted first. A cut one ends in an ellipsis rather
            // than in its own last character and padding is what follows a
            // short one. `kept` is how far the row still shows the name
            // itself and a mark or an underline past that would land on
            // something the name does not own.
            let fitted = fit(&text, text_w);
            let shown: Vec<char> = fitted.chars().collect();
            let source: Vec<char> = text.chars().collect();
            let kept = (0..source.len())
                .take_while(|&i| shown.get(i) == source.get(i))
                .count();
            let all = fuzzy::matched(m.typed, &text);
            // What Tab would add. An empty range is what a row Tab passes
            // over gets and so is a reach no further than what was typed.
            // Whether Tab reads this row at all is asked of the whole match
            // rather than of the part the row shows. Dropping a match past
            // the cut first would leave a leading run behind and underline a
            // row the key passes over.
            let under = if tab_grows(c.kind, &all) {
                all.len()..m.reach.min(kept)
            } else {
                0..0
            };
            let at: Vec<usize> = all.into_iter().filter(|&i| i < kept).collect();
            let off = format!("{ground}{name_fg}");
            let on = format!("{mark_fg}{mark}");
            let name = marked(&fitted, &at, &under, &off, &on);
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
        "{pad}{PANEL}{BORDER}{}{RESET}",
        String::from(RULE).repeat(inner + 2)
    ));
    rows.push(format!(
        "{pad}{PANEL}{FOOT}{ITALIC} {} {RESET}",
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
    /// First item the last menu showed. A menu that opens again starts on the
    /// first item and the selection alone therefore pulls this back to 0.
    top: usize,
}

impl<W: Write> Ui<W> {
    pub fn new(out: W, anchor: usize) -> Ui<W> {
        Ui {
            out,
            cursor_row: None,
            anchor,
            top: 0,
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
            self.top = window_start(self.top, m.selected, m.rows, m.items.len());
            rows.extend(menu_rows(&m, w, cursor_col, self.top));
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

    /// Ring the terminal's own bell. A key that had nothing to do says so
    /// with this and nothing on the screen would say it: the line and the menu
    /// are the ones that were already there.
    ///
    /// Whether it is a sound or a flash or nothing at all is the terminal's to
    /// decide. The person has already set that where they wanted it.
    pub fn bell(&mut self) -> io::Result<()> {
        self.out.write_all(b"\x07")?;
        self.out.flush()
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
    fn the_bell_is_one_byte_and_nothing_else() {
        let mut buf: Vec<u8> = Vec::new();
        Ui::new(&mut buf, 0)
            .bell()
            .expect("a Vec always takes a write");
        assert_eq!(buf, b"\x07");
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

    // window_start(top, selected, rows, total)

    #[test]
    fn a_list_that_fits_starts_at_the_top() {
        assert_eq!(window_start(0, 3, 8, 5), 0);
        // A whole list on the screen goes back to the top rather than staying
        // wherever the last window left off.
        assert_eq!(window_start(9, 3, 8, 5), 0);
    }

    #[test]
    fn the_window_holds_still_while_the_selection_shows() {
        assert_eq!(window_start(4, 5, 4, 40), 4);
    }

    #[test]
    fn the_window_follows_the_selection_one_row_at_a_time() {
        assert_eq!(window_start(4, 8, 4, 40), 5);
        assert_eq!(window_start(4, 3, 4, 40), 3);
    }

    #[test]
    fn a_wrap_to_the_far_end_takes_the_window_with_it() {
        assert_eq!(window_start(0, 39, 4, 40), 36);
        assert_eq!(window_start(36, 0, 4, 40), 0);
    }

    #[test]
    fn the_window_stops_at_the_end_of_the_list() {
        assert_eq!(window_start(36, 39, 4, 40), 36);
        assert_eq!(window_start(99, 39, 4, 40), 36);
    }

    #[test]
    fn an_empty_list_has_no_menu() {
        assert!(menu_in(&[], 0, 24, "", 0).is_none());
    }

    #[test]
    fn a_selection_past_the_end_is_pulled_back() {
        let items = dirs(3);
        assert_eq!(menu_in(&items, 99, 24, "", 0).expect("a menu").selected, 2);
    }

    #[test]
    fn the_menu_never_grows_past_its_own_cap() {
        let items = dirs(40);
        assert_eq!(
            menu_in(&items, 0, 40, "", 0).expect("a menu").rows,
            MENU_ROWS
        );
    }

    #[test]
    fn a_short_terminal_shortens_the_menu() {
        let items = dirs(40);
        assert_eq!(menu_in(&items, 0, 7, "", 0).expect("a menu").rows, 2);
        assert_eq!(menu_in(&items, 0, 1, "", 0).expect("a menu").rows, 1);
    }

    /// Cells of empty space in front of a panel row.
    fn pad_of(row: &str) -> usize {
        row.chars().take_while(|&c| c == ' ').count()
    }

    /// Which rows the panel drew as the chosen one.
    fn chosen_rows(rows: &[String]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.contains(PANEL_CHOSEN))
            .map(|(i, _)| i)
            .collect()
    }

    /// The characters of `row` that carry `mark` as their ground. Each one is
    /// a mark of its own and the row's own ground follows it.
    fn marks(row: &str, mark: &str) -> String {
        row.split(mark)
            .skip(1)
            .filter_map(|part| part.chars().next())
            .collect()
    }

    /// The underlined characters of `row`.
    fn underlined(row: &str) -> String {
        marks(row, UNDER)
    }

    /// One of the two places every shell can go from anywhere.
    fn special(display: &str) -> Candidate {
        Candidate {
            display: display.to_string(),
            insert: format!("{display}/"),
            label: "parent",
            kind: Kind::Special,
            score: 0,
        }
    }

    #[test]
    fn the_panel_starts_one_column_before_the_cursor() {
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        assert_eq!(pad_of(&menu_rows(&m, 80, 10, 0)[0]), 9);
        assert_eq!(pad_of(&menu_rows(&m, 80, 0, 0)[0]), 0);
    }

    #[test]
    fn the_panel_slides_left_of_a_cursor_near_the_edge() {
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let width = cells_of_row(&menu_rows(&m, 80, 0, 0)[0]);
        // The cursor is far enough right that the panel would run off the end.
        // The terminal is measured from the panel rather than named, because a
        // change to `PANEL_INNER` moves what counts as far enough.
        let w = width + 10;
        let rows = menu_rows(&m, w, w - 2, 0);
        assert_eq!(pad_of(&rows[0]) + width, w);
        // It keeps the width it had against the left edge.
        assert_eq!(cells_of_row(&rows[0]) - pad_of(&rows[0]), width);
    }

    #[test]
    fn the_menu_draws_a_row_for_each_entry_and_two_under_them() {
        let items = dirs(3);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        assert_eq!(menu_rows(&m, 80, 1, 0).len(), 5);
    }

    #[test]
    fn a_line_separates_the_list_from_the_word_under_it() {
        // It spends the panel's whole width and it is the row above the word
        // rather than the last one.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let rule = &rows[rows.len() - 2];
        assert_eq!(rule.matches(RULE).count(), PANEL_INNER + 2, "{rule:?}");
        assert!(
            rows.last().expect("a footer").contains("folder"),
            "{rows:?}"
        );
    }

    #[test]
    fn the_panel_opens_on_the_window_rather_than_the_list() {
        let items = dirs(40);
        let m = menu_in(&items, 17, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 12);
        // A window opening on d12 draws six names from there and puts the
        // eighteenth under the mark. The names are read as well as the mark.
        // A panel taking the mark from the window and the names from the list
        // would agree on the mark alone.
        assert_eq!(rows.len(), 8);
        assert!(rows[0].contains("d12"), "{rows:?}");
        assert_eq!(chosen_rows(&rows), vec![5]);
    }

    #[test]
    fn the_footer_carries_the_position_in_the_list() {
        let items = dirs(3);
        let m = menu_in(&items, 1, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(rows.last().expect("a footer").contains("2/3"));
    }

    #[test]
    fn the_footer_is_not_dimmed() {
        // Italic alone is what sets it apart from the names above it. It reads
        // at the weight they do.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let foot = rows.last().expect("a footer");
        assert!(!foot.contains(DIM), "{foot:?}");
        assert!(foot.contains(ITALIC), "{foot:?}");
    }

    #[test]
    fn one_row_is_drawn_as_chosen_and_it_is_the_selected_one() {
        let items = dirs(3);
        let m = menu_in(&items, 1, 24, "", 0).expect("a menu");
        assert_eq!(chosen_rows(&menu_rows(&m, 80, 1, 0)), vec![1]);
    }

    #[test]
    fn the_characters_what_was_typed_reached_are_marked() {
        let items = vec![dir("work/")];
        let m = menu_in(&items, 0, 24, "wk", 0).expect("a menu");
        assert_eq!(marks(&menu_rows(&m, 80, 1, 0)[0], MARK_CHOSEN), "wk");
    }

    #[test]
    fn a_row_off_the_highlight_wears_the_other_mark() {
        // The mark has to read against the ground under it and the highlighted
        // row has a ground of its own. Neither name leads with what was typed
        // in the second row's case.
        let items = vec![dir("alpha/"), dir("beta/")];
        let m = menu_in(&items, 0, 24, "a", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(marks(&rows[0], MARK_CHOSEN), "a");
        assert_eq!(marks(&rows[1], MARK), "a");
    }

    #[test]
    fn a_marked_character_wears_a_name_of_its_own() {
        // The ground is not the whole mark. The character on it carries a
        // brighter name than the rest of the row and the row's own ground and
        // name follow it. The highlighted row has nothing brighter than the
        // name it already wears.
        let items = vec![dir("alpha/"), dir("beta/")];
        let m = menu_in(&items, 0, 24, "a", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(
            rows[0].contains(&format!(
                "{NAME_CHOSEN}{MARK_CHOSEN}a{PANEL_CHOSEN}{NAME_CHOSEN}"
            )),
            "{rows:?}"
        );
        assert!(
            rows[1].contains(&format!("{NAME_MARKED}{MARK}a{PANEL}{NAME}")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_mark_keeps_a_special_row_apart_from_a_directory() {
        // The two places every shell can go from anywhere wear a colour of
        // their own and a mark brightens that rather than replaces it. A mark
        // that named one colour for every row would take the difference away
        // on the one character the eye is drawn to.
        let items = vec![dir("dot/"), special("..")];
        let m = menu_in(&items, 0, 24, ".", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(
            rows[1].contains(&format!("{SPECIAL_MARKED}{MARK}.{PANEL}{SPECIAL}")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_mark_on_the_cut_itself_is_not_drawn() {
        // The character at the ellipsis is the one `fit` put there rather
        // than the one that matched. `a_mark_past_the_cut_is_not_drawn` above
        // covers a match well past the cut and this is the boundary: the `z`
        // sits on the very cell the ellipsis takes.
        let cut = PANEL_INNER - 3;
        let items = vec![dir(&format!("{}z{}/", "a".repeat(cut), "a".repeat(5)))];
        let m = menu_in(&items, 0, 24, "z", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[0];
        assert!(row.contains('\u{2026}'), "the name was not cut: {row:?}");
        assert!(!row.contains(MARK_CHOSEN), "{row:?}");
    }

    #[test]
    fn a_row_tab_passes_over_carries_no_underline_past_the_cut() {
        // The `z` is what this name is in the menu for and the cut took it
        // away. Tab reads the whole match rather than the part the row shows,
        // so the leading `a` left behind does not turn the underline on.
        let items = vec![dir(&format!("a{}z/", "b".repeat(40))), dir("az/")];
        let m = menu_in(&items, 1, 24, "az", 2).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(
            rows[0].contains('\u{2026}'),
            "the name was not cut: {rows:?}"
        );
        assert_eq!(underlined(&rows[0]), "");
    }

    #[test]
    fn an_underline_stops_at_the_cut() {
        // Tab would reach past what the row shows. The run it underlines ends
        // where the name the row holds does.
        let items = vec![dir(&format!("{}/", "a".repeat(60)))];
        let m = menu_in(&items, 0, 24, "a", 60).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[0];
        assert_eq!(cells_of_row(row) - pad_of(row), PANEL_INNER + 2);
        assert!(!underlined(row).contains('\u{2026}'), "{row:?}");
    }

    #[test]
    fn nothing_typed_marks_nothing() {
        let items = dirs(2);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        for row in menu_rows(&m, 80, 1, 0) {
            assert!(!row.contains(MARK), "{row:?}");
            assert!(!row.contains(MARK_CHOSEN), "{row:?}");
        }
    }

    #[test]
    fn a_row_with_no_name_is_not_marked() {
        // The row that runs the line carries no name. Nothing there answered
        // what was typed and nothing there is marked.
        let items = vec![run_row("work".into()), dir("work/")];
        let m = menu_in(&items, 1, 24, "wo", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(!rows[0].contains(MARK), "{rows:?}");
        assert_eq!(marks(&rows[1], MARK_CHOSEN), "wo");
    }

    #[test]
    fn a_mark_past_the_cut_is_not_drawn() {
        // `fit` cut this name down. A mark on a character the row no longer
        // holds would land on whatever took its place.
        let items = vec![dir(&format!("{}z/", "a".repeat(60)))];
        let m = menu_in(&items, 0, 24, "z", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[0];
        assert!(!row.contains(MARK_CHOSEN), "{row:?}");
    }

    #[test]
    fn the_run_tab_would_add_is_underlined() {
        // Two characters are typed and the three the rows share reach one
        // past them.
        let items = vec![dir("work/"), dir("worse/")];
        let m = menu_in(&items, 0, 24, "wo", 3).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[0]), "r");
        assert_eq!(underlined(&rows[1]), "r");
    }

    #[test]
    fn a_row_tab_passes_over_carries_no_underline() {
        // `beta/` holds an `a` and is in the menu for it. It does not lead
        // with what was typed and Tab reads the rows that do.
        let items = vec![dir("alpha/"), dir("beta/")];
        let m = menu_in(&items, 0, 24, "a", 2).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[0]), "l");
        assert_eq!(underlined(&rows[1]), "");
    }

    #[test]
    fn a_row_that_is_not_a_name_carries_no_underline() {
        // Nothing is typed and every name leads with that. The row that runs
        // the line and the two places every shell can go from anywhere are
        // still not names Tab reads.
        let items = vec![run_row("x".into()), dir("work/"), special("..")];
        let m = menu_in(&items, 1, 24, "", 1).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[0]), "");
        assert_eq!(underlined(&rows[1]), "w");
        assert_eq!(underlined(&rows[2]), "");
    }

    #[test]
    fn a_reach_no_further_than_what_was_typed_underlines_nothing() {
        // Tab has nothing to add here. The run would start past its own end
        // and an empty one is what that has to draw.
        let items = vec![dir("work/")];
        let m = menu_in(&items, 0, 24, "work/", 5).expect("a menu");
        assert_eq!(underlined(&menu_rows(&m, 80, 1, 0)[0]), "");
    }

    #[test]
    fn a_mark_and_an_underline_leave_the_width_alone() {
        let items = vec![dir("work/")];
        let plain = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let picked = menu_in(&items, 0, 24, "wo", 5).expect("a menu");
        assert_eq!(
            cells_of_row(&menu_rows(&plain, 80, 1, 0)[0]),
            cells_of_row(&menu_rows(&picked, 80, 1, 0)[0])
        );
    }

    #[test]
    fn the_panel_is_the_same_width_whatever_it_holds() {
        // A short name does not shrink the panel and a long one does not grow
        // it. The test below is where the terminal takes the width back.
        for items in [vec![dir("a")], vec![dir(&"n".repeat(60))]] {
            let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
            let row = &menu_rows(&m, 80, 5, 0)[0];
            assert_eq!(cells_of_row(row) - pad_of(row), PANEL_INNER + 2);
        }
    }

    #[test]
    fn a_narrow_terminal_takes_the_width_back() {
        // The one thing the constant gives way to. The panel then spends the
        // whole terminal and still has a name column.
        let w = 24;
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = &menu_rows(&m, w, 1, 0)[0];
        assert_eq!(cells_of_row(row), w);
        assert!(row.contains("d0"), "{row:?}");
    }

    #[test]
    fn every_menu_row_is_the_same_width() {
        let items = vec![dir("short"), dir("a-much-longer-directory-name")];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let widths: Vec<usize> = menu_rows(&m, 80, 5, 0)
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
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
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
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = menu_rows(&m, 80, 1, 0).first().expect("a row").clone();
        // The glyph carries its own colour and the name names the ground's
        // bright one again behind it.
        assert_ne!(ICON_FG_CHOSEN, NAME_CHOSEN);
        let want = format!("{ICON_FG_CHOSEN}{ICON} {NAME_CHOSEN}");
        assert!(row.contains(&want), "{row:?}");
    }

    #[test]
    fn the_row_that_runs_the_line_holds_no_name() {
        let items = vec![run_row("work/".to_string()), dir("alpha/")];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let row = rows.first().expect("a row");
        assert!(row.contains(RUN_ICON), "{row:?}");
        // `insert` is the only text this row could have drawn.
        assert!(!row.contains("work"), "{row:?}");
    }

    #[test]
    fn the_two_row_glyphs_are_the_same_width() {
        // `menu_rows` takes the glyph column out of the panel's inner width
        // once and measures it from the widest glyph. Names line up only while
        // the two agree, because each row spends `cells(glyph) + 1` on its own
        // before the name starts.
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
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        assert!(menu_rows(&m, 3, 1, 0).is_empty());
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
            .render_at(&[], &line, "", menu_in(&items, 0, 24, "", 0), 40)
            .expect("a Vec always takes a write");
        let out = String::from_utf8(buf).expect("the frame is text");
        // One input row, two entries, the line under them and the footer.
        assert!(out.contains("\x1b[4A"), "{out:?}");
        assert!(out.contains("d0") && out.contains("d1"), "{out:?}");
    }

    #[test]
    fn the_window_the_last_frame_showed_is_the_one_the_next_moves_from() {
        let items = dirs(11);
        let mut buf: Vec<u8> = Vec::new();
        let mut line = Line::new();
        line.insert("cd d");
        let mut ui = Ui::new(&mut buf, 0);
        // The highlight on the last row the window shows moves nothing.
        ui.render_at(&[], &line, "", menu_in(&items, 5, 30, "", 0), 40)
            .expect("a Vec always takes a write");
        assert_eq!(ui.top, 0);
        // One past it and the window follows by a single row.
        ui.render_at(&[], &line, "", menu_in(&items, 6, 30, "", 0), 40)
            .expect("a Vec always takes a write");
        assert_eq!(ui.top, 1);
        // The end of the list takes the window with it.
        ui.render_at(&[], &line, "", menu_in(&items, 10, 30, "", 0), 40)
            .expect("a Vec always takes a write");
        assert_eq!(ui.top, 5);
        // Back up to a row the window already holds. This is the frame the
        // field exists for: a window read off the highlight alone would jump
        // to 0 here rather than stay.
        ui.render_at(&[], &line, "", menu_in(&items, 6, 30, "", 0), 40)
            .expect("a Vec always takes a write");
        assert_eq!(ui.top, 5);
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
