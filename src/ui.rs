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
use crate::icons::{self, Set};
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
const PANEL_CHOSEN: &str = "\x1b[48;5;25m";
const NAME: &str = "\x1b[38;5;249m";
/// The name on the highlighted row's own ground. The glyph in front of it
/// wears a colour of its own and this is what puts the name back.
const NAME_CHOSEN: &str = "\x1b[97m";
/// The word that says what the highlighted row is. It reads at the weight a
/// name does and italic alone is what sets it apart. What the row says is
/// worth reading rather than worth fading out.
const FOOT: &str = NAME;
/// The ground under a character what was typed reached. A dark olive rather
/// than a tint of the panel's own grey. A mark then reads as a mark rather
/// than as another row.
const MARK: &str = "\x1b[48;5;58m";
/// The same on the highlighted row. That row's ground is a blue the olive
/// disappears into and a lighter tint of that blue takes over.
const MARK_CHOSEN: &str = "\x1b[48;5;67m";
/// A marked character's own name. A brighter tint of `NAME`: the ground says
/// which characters what was typed reached and the brighter name is what makes
/// them read first. Every row wears the same one. What a row is is the glyph's
/// to say and a name that changed colour with it would say it twice. The
/// highlighted row has none of its own, because `NAME_CHOSEN` is already as
/// bright as a name gets.
const NAME_MARKED: &str = "\x1b[38;5;188m";
/// The run Tab would add to the argument. An underline rather than a ground
/// of its own: the characters what was typed reached already carry one and a
/// second ground beside it would read as another row.
/// The line that closes a panel and the one that separates the list from
/// what the panel puts under it. A shade off the panel's own ground rather
/// than a name's colour: it separates two things rather than saying anything
/// of its own.
const BORDER: &str = "\x1b[38;5;238m";
/// What that line is drawn with.
const RULE: char = '\u{2500}';
/// What an edge carries: the position in the list on the list's own top edge
/// and the key for the word on the edge above the word. A grey above
/// the edge's own and below a name's. The edge must not swallow either one
/// and neither of them is a name.
const EDGE_FG: &str = "\x1b[38;5;244m";
/// What the rule above the word says the key for it is. Q puts the badge for
/// its own key in the corner of the popout it draws the description in. A
/// rule is where a panel with no box of its own has the room for one. Beside
/// the list the word has a box of its own and the badge sits on its top edge.
/// This is the drawn text alone and `pick` is where the key itself is read.
///
/// `^O` rather than `⌃O`. U+2303 is one more shape to ask of a terminal font
/// and the panel already asks for as few as it can.
const WHOLE_WORD_BADGE: &str = "^O";
const UNDER: &str = "\x1b[4m";
const UNDER_OFF: &str = "\x1b[24m";
/// Cells inside the panel's own two edge spaces. It is fixed rather than
/// measured from what the panel holds. Every panel is therefore the same
/// width and a name keeps the column the eye last found it in. A terminal
/// with no room for all of it takes some back.
const PANEL_INNER: usize = 40;
/// Cells inside the word's own two edge spaces, where the key has put the
/// word beside the list rather than under it. It is the narrower of the two
/// columns and this is the number the layout is bought with. 80 cells is what
/// a terminal has before anybody widens one. A pair spending all 80 of them
/// fits such a terminal and then has nowhere to slide. The menu would stand at
/// the left edge there and stop following the cursor at the commonest width
/// there is. The five cells that buy that movement back come off the sentence
/// rather than off the list, because a name is what the keys act on.
const DETAIL_INNER: usize = 30;
/// Cells of the terminal's own ground between the two panels. One is enough
/// to read them as two panels rather than as one torn in half. A gap drawn
/// on the panel's own ground would be the panel.
const PANEL_GAP: usize = 1;
/// What the pair spends together. 75, which leaves an 80-cell terminal five
/// cells for the panel to follow the cursor in. A terminal with fewer cells
/// than this opens the word under the list instead. There is no third layout
/// between the two: the same sentence in a third shape is one the eye has to
/// find again every time a terminal changes size.
const PAIR_WIDTH: usize = PANEL_INNER + 2 + PANEL_GAP + DETAIL_INNER + 2;
const MENU_ROWS: usize = 6;
/// The names the list shows beside the word. The word there gives up the
/// line under the list and its own row under that. One of the two is a name
/// more and the other is the terminal's to spare.
const MENU_ROWS_BESIDE: usize = MENU_ROWS + 1;
/// Rows the menu never takes. They are left to the input line, to the rule
/// above the list, to the line and the word under it and to whatever the
/// shell put above it. Two budgets read it and both ask the same question of
/// it: how many rows the frame spends before it holds anything. `menu_in`
/// spends what is left on the list and `menu_rows` spends what is left after
/// that on the detail.
///
/// The pair draws neither the line under the list nor the row the word takes
/// there. [`Menu::list_rows`] gives the list what it takes of the two and
/// `menu_rows` hands the rest back to what the terminal spares. The one
/// budget therefore answers for both layouts and the key can open the word
/// down to the same row in either.
///
/// The constructor is the free function `menu_in` rather than a method.
const RESERVED_ROWS: usize = 6;
/// The rows the word under the list takes unasked. One. Every row under the
/// list comes out of what the list and a wrapped name have left and a
/// sentence about a row is worth less than the rows it is about. This one
/// is not among them. [`RESERVED_ROWS`] counts it before `menu_in` sizes
/// the list. It was never a row the list could have taken and `spare` is
/// only ever what is left over beyond it.
///
/// 33.2% of the corpus's descriptions fit that row whole and [`clause`]
/// takes that to 46.3%. [`WHOLE_WORD_BADGE`] opens the rest.
///
/// Beside the list the word takes no row off anything and spends the ones
/// the list is already spending. This row then goes to the list first and
/// to what the terminal spares after that.
const FOOT_ROWS: usize = 1;

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
    /// The names the list shows with the word under it.
    /// [`Menu::list_rows`] is the count for either layout.
    rows: usize,
    /// Rows the menu may take. It opens as the terminal's height and the
    /// renderer takes back whatever a wrapped input line spends.
    height: usize,
    /// What the rows were matched against. The characters it reached in a name
    /// carry a ground and a brighter name of their own.
    typed: &'a str,
    /// How many characters of a name Tab would leave on the line. The run of
    /// that past what was typed is underlined.
    reach: usize,
    /// Whether the key has asked for the whole of the word. A terminal with
    /// room for the pair puts it beside the list, on the list's own rows and
    /// every row the terminal spared under them. One without that room opens
    /// it under the list to every row the terminal spared rather than the one
    /// [`FOOT_ROWS`] allows it. `menu_in` opens a menu with it off and the
    /// picker's own key is what sets it.
    whole_word: bool,
    /// Which glyphs the rows wear. `menu_in` opens a menu on the default set
    /// and the config is what names another.
    icons: Set,
}

impl Menu<'_> {
    /// Whether the word sits beside the list in a terminal `w` cells wide. It
    /// does where the key asked for the whole of it and the pair fits whole.
    /// It stays under the list everywhere else.
    fn beside(&self, w: usize) -> bool {
        self.whole_word && w >= PAIR_WIDTH
    }

    /// The names the list shows in a terminal `w` cells wide. Beside the word
    /// the list takes the two rows the word under it no longer spends, as far
    /// as [`MENU_ROWS_BESIDE`]. The list there never runs further down than
    /// the word under it would have.
    fn list_rows(&self, w: usize) -> usize {
        if self.beside(w) {
            (self.rows + 1 + FOOT_ROWS)
                .min(MENU_ROWS_BESIDE)
                .min(self.items.len())
        } else {
            self.rows
        }
    }
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
    whole_word: bool,
    icons: Set,
) -> Option<Menu<'a>> {
    let mut m = menu_in(items, selected, height(), typed, reach)?;
    m.whole_word = whole_word;
    m.icons = icons;
    Some(m)
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
        height,
        selected: selected.min(items.len() - 1),
        rows: MENU_ROWS
            .min(items.len())
            .min(height.saturating_sub(RESERVED_ROWS).max(1)),
        whole_word: false,
        icons: Set::default(),
    })
}

/// Cut `s` down to `w` cells and mark the cut. A short `s` comes back
/// unpadded, which is what lets `menu_rows` measure the room a hint would
/// still have beside it.
fn cut(s: &str, w: usize) -> String {
    let have = cells(s);
    if have <= w {
        return s.to_string();
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
    out
}

/// The first clause of a description, for a word whose rows have no room for
/// the whole of it. What a parenthetical or a second sentence adds usually
/// qualifies the first clause rather than says anything the first clause
/// does not. `Use TCP/IP device (error if multiple TCP/IP devices are
/// available)` is 66 cells and `Use TCP/IP device` is 17.
///
/// 66.8% of the 371 943 descriptions the corpus carries are wider than the
/// panel's 40 cells and 53.7% still are once this has run. The 13.1% in
/// between come out whole rather than cut short.
///
/// A clause that is the whole of `s` comes back as `s`. So does one this
/// would leave empty: a description that opens with its own parenthetical
/// has nothing in front of it to show.
fn clause(s: &str) -> &str {
    let bytes = s.as_bytes();
    // A full stop closes a sentence only where two letters or digits run
    // into it. `e.g. ` and an initial each end in one of them behind
    // another stop or a space.
    let stop = s.match_indices(". ").find(|(i, _)| {
        *i >= 2 && bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 2].is_ascii_alphanumeric()
    });
    let end = [s.find('('), stop.map(|(i, _)| i)]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(s.len());
    let out = s[..end].trim_end_matches([' ', '.']);
    if out.is_empty() { s } else { out }
}

/// Pad `s` out to `w` cells. Cut it down and mark the cut when it is wider.
fn fit(s: &str, w: usize) -> String {
    let cut = cut(s, w);
    let have = cells(&cut);
    format!("{cut}{}", " ".repeat(w.saturating_sub(have)))
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

/// Break `s` into at most `rows` rows of `w` cells, at a space wherever
/// the row holds one. [`wrap`] breaks wherever the cells run out. That is
/// right for a path and wrong for a sentence: a word split over two rows
/// has to be read twice.
///
/// The last row keeps everything still left rather than its own `w` cells.
/// The caller's own `fit` is what cuts that row and marks the cut. The
/// result always holds at least one row.
fn wrap_words(s: &str, w: usize, rows: usize) -> Vec<String> {
    // A row of no cells takes no character and the walk below would never
    // reach the end of `s`.
    let w = w.max(1);
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        if cells(rest) <= w || out.len() + 1 >= rows {
            out.push(rest.to_string());
            break;
        }
        // Every character that still fits, and then back to the last
        // space in the run where the row ended inside a word.
        let mut head = String::new();
        for c in rest.chars() {
            head.push(c);
            if cells(&head) > w {
                head.pop();
                break;
            }
        }
        let cut = match rest[head.len()..].chars().next() {
            // The row ends where a word does and nothing has to move
            // down with it.
            Some(' ') | None => head.len(),
            // It ends inside one. A word wider than the whole row has no
            // space of its own to go back to and breaks where the cells
            // ran out.
            _ => head.rfind(' ').filter(|i| *i > 0).unwrap_or(head.len()),
        };
        out.push(rest[..cut].trim_end().to_string());
        rest = rest[cut..].trim_start();
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// A rendered row's own text, with the escape sequences taken out.
fn visible(row: &str) -> String {
    let mut out = String::new();
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
            out.push(c);
        }
    }
    out
}

/// Display width of a rendered row. The escape sequences do not count.
fn cells_of_row(row: &str) -> usize {
    cells(&visible(row))
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

/// Whether Tab would grow this row's name. `common` reads the menu the same
/// way and the underline therefore covers what the key would take.
/// `highlighted` is the kind the highlight sits on, because that is what says
/// which rows the key reads.
fn tab_grows(k: Kind, highlighted: Kind, at: &[usize]) -> bool {
    // A match is a subsequence. Only a name whose match is the front of it
    // can take a prefix the menu agreed on.
    if !at.iter().enumerate().all(|(i, &j)| i == j) {
        return false;
    }
    match k {
        // The row that runs the line grows nothing. Tab skips the home shortcut.
        Kind::Run | Kind::Special => false,
        // Tab takes a highlighted parent row whole and reads the child
        // directories under every other highlight.
        Kind::Parent => highlighted == Kind::Parent,
        Kind::Dir => highlighted != Kind::Parent,
        Kind::Command | Kind::Branch | Kind::File | Kind::Option | Kind::Path => k == highlighted,
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

/// The word for the highlighted row, in at most `rows` rows of `w` cells.
///
/// The whole of it where those rows hold it and its first clause where they
/// do not. Wrapping it one row further is what asks the question: a sentence
/// that comes back longer than the rows on offer is one the panel cannot
/// hold. An ellipsis says a sentence was cut short and nothing says what was
/// cut. A clause that ends where the writer ended it reads as a sentence.
///
/// `opened` is the key having asked for the whole of it. The last row is cut
/// and marked then rather than trimmed: an ellipsis there says the terminal
/// is what is short. A clause would answer the question the key had just
/// asked the other way. The caller's own `fit` is what cuts that row.
fn word(label: &str, w: usize, rows: usize, opened: bool) -> Vec<String> {
    let whole = wrap_words(label, w, rows + 1);
    if whole.len() <= rows {
        whole
    } else if opened {
        wrap_words(label, w, rows)
    } else {
        wrap_words(clause(label), w, rows)
    }
}

/// The panel for `m` in a terminal `w` cells wide. `col` is where the cursor
/// sits and the first name lands there. The panel therefore follows the
/// cursor. `first` is the item the panel opens on and `window_start` is
/// where it comes from. A `first` from anywhere else can draw a panel with
/// the highlight outside it.
///
/// One row of the result is one row of the terminal. Where the word sits
/// beside the list that row carries both panels: the list, a reset, the gap
/// on the terminal's own ground and then the word's own panel. A row past the
/// end of either panel carries the other alone. Each panel keeps its own
/// width on every row it reaches.
fn menu_rows(m: &Menu, w: usize, col: usize, first: usize) -> Vec<String> {
    let Some(current) = m.items.get(m.selected) else {
        return Vec::new();
    };
    let beside = m.beside(w);
    let rows = m.list_rows(w);
    let last = (first + rows).min(m.items.len());
    let shown = &m.items[first..last];
    let count = format!("{}/{}", m.selected + 1, m.items.len());

    // The width comes first. Nothing below holds a floor under it and `width`
    // is what keeps one.
    let inner = PANEL_INNER.min(w.saturating_sub(2));
    // A terminal wide enough for the pair is wide enough for the list's own
    // full width as well. This column therefore never shrinks beside the word.
    let pair = if beside { PAIR_WIDTH } else { inner + 2 };
    // The panel opens with a space of its own. Starting one column early puts
    // the first row's glyph under the cursor rather than past it. It keeps the
    // width above and slides left of the cursor when the right edge is nearer
    // than that. A terminal spending every cell it has on the pair has
    // nowhere to slide. The panel stands at the left edge there whatever the
    // cursor is doing.
    let indent = col.saturating_sub(1).min(w.saturating_sub(pair));
    let icon_w = icons::widest(m.icons) + 1;
    // A terminal too narrow for the icon and a name gets no panel at all.
    let Some(text_w) = inner.checked_sub(icon_w) else {
        return Vec::new();
    };
    let pad = " ".repeat(indent);
    // An edge of a panel `across` cells wide inside its own two spaces, with
    // `label` let into its right end. An edge closes a panel above its first
    // row or separates the list from what the panel puts under it. What a
    // label carries belongs on an edge rather than beside the word. The count
    // says where the highlight sits and the key says how to open the word.
    // Neither is the sentence itself and beside it they would take cells from
    // a sentence that is usually too long for the panel already.
    //
    // Two spaces hold the label off the rule and one rule character closes
    // the right end. A panel with no room for all of that keeps the plain
    // edge. So does an edge with nothing to say.
    let edge = |across: usize, label: &str| {
        let lead = (across + 2).saturating_sub(cells(label) + 3);
        if label.is_empty() || lead == 0 {
            return format!(
                "{PANEL}{BORDER}{}{RESET}",
                String::from(RULE).repeat(across + 2)
            );
        }
        format!(
            "{PANEL}{BORDER}{} {EDGE_FG}{label}{BORDER} {RULE}{RESET}",
            String::from(RULE).repeat(lead)
        )
    };

    let mut list: Vec<String> = vec![edge(inner, &count)];
    list.extend(shown.iter().enumerate().map(|(r, c)| {
        let text = printable(&c.display);
        let chosen = first + r == m.selected;
        let ground = if chosen { PANEL_CHOSEN } else { PANEL };
        // The glyph's own colour replaces whatever the name wanted. The
        // name therefore names its colour again behind it.
        let name_fg = if chosen { NAME_CHOSEN } else { NAME };
        // A mark carries a ground and a name colour together.
        let (mark, mark_fg) = if chosen {
            (MARK_CHOSEN, NAME_CHOSEN)
        } else {
            (MARK, NAME_MARKED)
        };
        // The name is cut first, unpadded, so a hint beside it can still see
        // what room the cut left. A cut name ends in an ellipsis rather than
        // in its own last character. `kept` is how far the row still shows
        // the name itself and a mark or an underline past that would land on
        // something the name does not own.
        let clipped = cut(&text, text_w);
        let shown: Vec<char> = clipped.chars().collect();
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
        let under = if tab_grows(c.kind, current.kind, &all) {
            all.len()..m.reach.min(kept)
        } else {
            0..0
        };
        let at: Vec<usize> = all.into_iter().filter(|&i| i < kept).collect();
        let off = format!("{ground}{name_fg}");
        let on = format!("{mark_fg}{mark}");
        let marked_name = marked(&clipped, &at, &under, &off, &on);
        // The hint is not part of what the name owns: no mark and no
        // underline ever reaches it. Its own arguments show in order, one
        // space ahead of each, for as long as the cells the name left over
        // still hold the next one whole. The first that does not fit ends
        // the hint there rather than skipping it for a shorter one further
        // along or cutting it down to an ellipsis of itself.
        let mut room = text_w.saturating_sub(cells(&clipped));
        let mut hint = String::new();
        for arg in &c.hint {
            let arg = printable(arg);
            let needed = cells(&arg) + 1;
            if needed > room {
                break;
            }
            room -= needed;
            // One dim run holds every argument. The space in front of the
            // first is the name's own ground, the way the row drew it.
            let first = hint.is_empty();
            hint.push(' ');
            if first {
                hint.push_str(DIM);
            }
            hint.push_str(&arg);
        }
        let name = format!("{marked_name}{hint}{}", " ".repeat(room));
        let (icon, icon_fg) = icons::of(m.icons, c, chosen);
        let icon_pad = " ".repeat(icon_w - cells(icon));
        format!("{ground} {icon_fg}{icon}{icon_pad}{name_fg}{name} {RESET}")
    }));

    // A highlighted name its own row could not hold, wrapped to the list's
    // own width. It keeps that column in both layouts: the row it came from
    // is up there and the cut it took is this width's doing.
    let text = printable(&current.display);
    // Beside the list the frame draws neither the line under it nor the row
    // the word takes there. [`RESERVED_ROWS`] counted both and what the list
    // did not take of them is the terminal's to spare here, less the line a
    // wrapped name draws above it.
    let freed = if beside { 1 + FOOT_ROWS } else { 0 };
    let mut spare = (m.height + freed).saturating_sub(RESERVED_ROWS + rows);
    let own_rule = usize::from(beside);
    let mut wrapped: Vec<String> = Vec::new();
    if cells(&text) > text_w && spare > own_rule {
        spare -= own_rule;
        wrapped = wrap(&[Seg { style: "", text }], inner, 0);
        if wrapped.len() > spare {
            wrapped.truncate(spare);
            // `fit` below is what cuts this back to the panel's width.
            if let Some(last) = wrapped.last_mut() {
                last.push('…');
            }
        }
        spare -= wrapped.len();
    }
    let wrapped: Vec<String> = wrapped
        .iter()
        .map(|row| format!("{PANEL}{NAME} {} {RESET}", fit(row, inner)))
        .collect();
    // The name has first claim on what the terminal spared. It says which row
    // the keys would act on and the sentence only says what that row does.
    //
    // The key gives that sentence every row the name left. A terminal with
    // none left has none to give and the word keeps the rows it has, down to
    // the clause they show. Trading a trimmed clause for the same rows of the
    // raw sentence is not what the key was pressed for.
    let opened = m.whole_word && spare > 0;
    // The word, drawn the way either layout draws it. The two differ in the
    // width they give it and the rows they have to spend, and in nothing
    // else. One writer is what keeps the sentence reading the same in both.
    let said = |across: usize, rows: usize| {
        word(&current.label, across, rows, opened)
            .into_iter()
            .map(move |row| format!("{PANEL}{FOOT}{ITALIC} {} {RESET}", fit(&row, across)))
    };

    if !beside {
        // The word under the list takes one row unasked and 53.7% of the
        // corpus still has more to say than one row holds once the clause
        // trim has run. The rule above it is what carries the key for the
        // rest.
        list.push(edge(inner, WHOLE_WORD_BADGE));
        list.extend(wrapped);
        let foot_rows = if opened { 1 + spare } else { FOOT_ROWS };
        list.extend(said(inner, foot_rows));
        return list.into_iter().map(|row| format!("{pad}{row}")).collect();
    }

    // The wrapped name keeps a rule above it here too, because it is still
    // a second thing in the list's own column. The badge is not on that rule:
    // the word has an edge of its own here and that is where the key for it
    // belongs.
    if !wrapped.is_empty() {
        list.push(edge(inner, ""));
        list.extend(wrapped);
    }
    // Beside the list the word takes no row off anything. It takes a column,
    // and the rows it has are the rows the list is already spending and every
    // row the terminal spared under them. A short list is where those spare
    // rows count: one match is one row of 30 cells on its own.
    let body = list.len() - 1;
    let word_rows = if opened { body + spare } else { body };
    let mut aside: Vec<String> = vec![edge(DETAIL_INNER, WHOLE_WORD_BADGE)];
    aside.extend(said(DETAIL_INNER, word_rows));
    // Each panel is as tall as what it holds. They open on one row and the
    // shorter of the two ends where its own last row does rather than
    // carrying ground it has nothing to put on. A row past the end of the
    // list is the word's alone and opens with the empty cells the list's
    // column would have taken, because the word's own column is where the
    // eye is already reading.
    let gap = " ".repeat(PANEL_GAP);
    let column = " ".repeat(inner + 2);
    (0..list.len().max(aside.len()))
        .map(|r| {
            let names = list.get(r).map_or(column.as_str(), String::as_str);
            match aside.get(r) {
                Some(row) => format!("{pad}{names}{gap}{row}"),
                None => format!("{pad}{names}"),
            }
        })
        .collect()
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
        if let Some(mut m) = menu {
            m.height = m.height.saturating_sub(rows.len().saturating_sub(1));
            self.top = window_start(self.top, m.selected, m.list_rows(w), m.items.len());
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
    use crate::candidates::{DEFAULT_PRIORITY, folder, run_row};
    use std::borrow::Cow;

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
        assert_eq!(menu_in(&items, 0, 7, "", 0).expect("a menu").rows, 1);
        assert_eq!(menu_in(&items, 0, 1, "", 0).expect("a menu").rows, 1);
    }

    /// A terminal with room for the list and the word beside it, and room
    /// left over for the panel to follow the cursor in.
    const WIDE: usize = 100;

    /// The widest terminal that still draws the word under the list. Every
    /// claim about that layout is measured against this one.
    const NARROW: usize = PAIR_WIDTH - 1;

    /// Cells of empty space in front of a panel row.
    fn pad_of(row: &str) -> usize {
        row.chars().take_while(|&c| c == ' ').count()
    }

    /// The two panels of row `r`: the list's, with the padding in front of
    /// it, and the word's without the gap in front of it. Either comes back
    /// empty where that panel does not reach the row.
    ///
    /// A panel row carries exactly one reset and it is the last thing on it,
    /// so the first reset is where the list ends. A row the list does not
    /// reach opens with the empty cells its column would have taken and is
    /// therefore padded further than row 0, which every panel reaches.
    fn panels(rows: &[String], r: usize) -> (String, String) {
        if pad_of(&rows[r]) > pad_of(&rows[0]) {
            return (String::new(), rows[r].trim_start().to_string());
        }
        let mut parts = rows[r].split(RESET);
        let list = parts.next().unwrap_or_default().to_string();
        let word = parts.next().unwrap_or_default().trim_start().to_string();
        (list, word)
    }

    /// The list's own panel out of row `r`.
    fn left_of(rows: &[String], r: usize) -> String {
        panels(rows, r).0
    }

    /// The word's own panel out of row `r`.
    fn right_of(rows: &[String], r: usize) -> String {
        panels(rows, r).1
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

    /// The home shortcut. It is the one row left that is neither a directory
    /// nor an action.
    fn home() -> Candidate {
        Candidate {
            display: "~".into(),
            insert: "~".into(),
            label: "home".into(),
            hint: Vec::new(),
            kind: Kind::Special,
            score: 0,
            priority: DEFAULT_PRIORITY,
        }
    }

    /// A Git subcommand. Tab reads one the way it reads a directory.
    fn command(name: &str) -> Candidate {
        Candidate {
            display: name.into(),
            insert: name.into(),
            label: "command".into(),
            hint: Vec::new(),
            kind: Kind::Command,
            score: 0,
            priority: DEFAULT_PRIORITY,
        }
    }

    /// The row that goes up. It reads as a directory rather than as a
    /// shortcut and the highlight is what decides whether Tab reads it.
    fn parent(insert: &str) -> Candidate {
        Candidate {
            display: "../".into(),
            insert: insert.to_string(),
            label: "parent".into(),
            hint: Vec::new(),
            kind: Kind::Parent,
            score: 0,
            priority: DEFAULT_PRIORITY,
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
    fn the_menu_draws_a_row_for_each_entry_a_line_above_and_two_below_them() {
        let items = dirs(3);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        assert_eq!(menu_rows(&m, 80, 1, 0).len(), 6);
    }

    #[test]
    fn the_word_beside_the_list_costs_the_menu_no_row_of_its_own() {
        // The same three entries with the key pressed. The word goes into a
        // panel of its own. The line above the entries is all that is left
        // over them and the two rows the word took below are gone.
        let items = dirs(3);
        let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        m.whole_word = true;
        assert_eq!(menu_rows(&m, WIDE, 1, 0).len(), 4);
    }

    #[test]
    fn the_list_beside_the_word_shows_one_name_more() {
        // Under the list the word takes the line above it and a row of its
        // own. Beside the list it takes neither and the list spends one of
        // the two on a seventh name.
        let items = dirs(40);
        let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let under = menu_rows(&m, WIDE, 1, 0);
        assert!(visible(&under[6]).contains("d5"), "{under:?}");
        assert!(
            !under.iter().any(|row| visible(row).contains("d6")),
            "{under:?}"
        );
        m.whole_word = true;
        let beside = menu_rows(&m, WIDE, 1, 0);
        assert!(visible(&left_of(&beside, 7)).contains("d6"), "{beside:?}");
        assert!(
            !beside.iter().any(|row| visible(row).contains("d7")),
            "{beside:?}"
        );
        // A short terminal gives the list both of those rows and the frame
        // ends where the one under the list did.
        let mut m = menu_in(&items, 0, 10, "", 0).expect("a menu");
        let under = menu_rows(&m, WIDE, 1, 0).len();
        m.whole_word = true;
        assert_eq!(m.list_rows(WIDE), m.rows + 2);
        assert_eq!(menu_rows(&m, WIDE, 1, 0).len(), under);
    }

    #[test]
    fn the_two_panels_sit_a_gap_apart_and_each_keeps_its_own_width() {
        // Three names and a one-row word. The list reaches four rows and the
        // word two. The rows below the word are the list's alone.
        let items = dirs(3);
        let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        m.whole_word = true;
        let rows = menu_rows(&m, WIDE, 1, 0);
        assert_eq!(rows.len(), 4);
        for r in 0..rows.len() {
            let list = cells_of_row(&left_of(&rows, r)) - pad_of(&rows[r]);
            assert_eq!(list, PANEL_INNER + 2, "{rows:?}");
        }
        for r in 0..2 {
            let word = cells_of_row(&right_of(&rows, r));
            assert_eq!(word, DETAIL_INNER + 2, "{rows:?}");
            // The gap is the terminal's own ground. The reset in front of it
            // is what hands the ground back.
            let pair = cells_of_row(&rows[r]) - pad_of(&rows[r]);
            assert_eq!(pair, PAIR_WIDTH, "{rows:?}");
        }
        for r in 2..rows.len() {
            assert_eq!(right_of(&rows, r), "", "{rows:?}");
        }
    }

    #[test]
    fn the_word_goes_back_under_the_list_where_the_pair_will_not_fit() {
        // One cell short of the pair. The key asked for the whole of the word
        // and it opens under the list instead. Nothing in between: the word
        // is beside the list or under it and there is no third shape for it.
        let items = dirs(3);
        let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        m.whole_word = true;
        let rows = menu_rows(&m, NARROW, 1, 0);
        for r in 0..rows.len() {
            assert_eq!(right_of(&rows, r), "", "{rows:?}");
            let list = cells_of_row(&rows[r]) - pad_of(&rows[r]);
            assert_eq!(list, PANEL_INNER + 2, "{rows:?}");
        }
        let foot = rows.last().expect("a footer");
        assert!(foot.contains("folder"), "{foot:?}");
    }

    #[test]
    fn the_detail_preserves_wide_characters_and_removes_controls() {
        // The name holds no space. `trim_end` below is there for the padding
        // and it would take a space off a row that ended on one.
        let name = format!("{}\n\x1b-suffix/", "日本語".repeat(12));
        let items = vec![dir(&name)];
        for width in [24, 40, 80] {
            let m = menu_in(&items, 0, 30, "", 0).expect("a menu");
            let rows = menu_rows(&m, width, 1, 0);
            let detail: String = rows[3..rows.len() - 1]
                .iter()
                .map(|row| {
                    row.strip_prefix(&format!("{PANEL}{NAME} "))
                        .unwrap()
                        .strip_suffix(&format!(" {RESET}"))
                        .unwrap()
                        .trim_end()
                })
                .collect();
            assert_eq!(detail, printable(&name));
            assert!(rows.iter().all(|row| cells_of_row(row) <= width));
        }
    }

    #[test]
    fn a_short_terminal_bounds_the_detail_and_marks_missing_text() {
        let items = vec![dir(&"x".repeat(200))];
        let m = menu_in(&items, 0, 8, "", 0).expect("a menu");
        let rows = menu_rows(&m, 24, 1, 0);
        assert_eq!(rows.len(), 5);
        assert!(rows[3].contains('…'));
        assert!(rows.last().unwrap().contains("folder"));
        let m = menu_in(&items, 0, 6, "", 0).expect("a menu");
        assert_eq!(menu_rows(&m, 24, 1, 0).len(), 4);
    }

    #[test]
    fn wrap_words_breaks_at_a_space_and_keeps_the_rest_on_the_last_row() {
        assert_eq!(wrap_words("one two three", 7, 3), ["one two", "three"]);
        // The last row on offer keeps everything still left. `fit` is what
        // cuts it and marks the cut.
        assert_eq!(wrap_words("one two three", 7, 2), ["one two", "three"]);
        assert_eq!(wrap_words("one two three", 7, 1), ["one two three"]);
    }

    #[test]
    fn wrap_words_breaks_a_word_with_no_space_in_it() {
        // A word wider than the row has nowhere to break and the cells
        // running out is the only answer left.
        assert_eq!(wrap_words("abcdefgh", 4, 3), ["abcd", "efgh"]);
        assert_eq!(wrap_words("", 4, 2), [""]);
    }

    #[test]
    fn a_line_separates_the_list_from_the_word_under_it() {
        // It spends the panel's whole width and it is the row above the word
        // rather than the last one.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let rule = &rows[rule_row(&rows)];
        assert_eq!(cells_of_row(rule), PANEL_INNER + 2, "{rule:?}");
        assert!(
            rows.last().expect("a footer").contains("folder"),
            "{rows:?}"
        );
    }

    #[test]
    fn a_line_closes_the_panel_above_the_first_row() {
        // Nothing used to draw above the list and the panel read as cut off
        // against the shell's own line. The rule below closes the bottom and
        // this is the same rule, the same width, above the top. An edge with
        // a label let into it spends every cell the plain rule spent.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let top = &rows[0];
        let bottom = &rows[rule_row(&rows)];
        assert_eq!(
            top.matches(RULE).count() + cells("1/1") + 2,
            PANEL_INNER + 2,
            "{top:?}"
        );
        assert_eq!(
            bottom.matches(RULE).count() + cells(WHOLE_WORD_BADGE) + 2,
            PANEL_INNER + 2,
            "{bottom:?}"
        );
        assert_eq!(cells_of_row(top), cells_of_row(bottom));
    }

    #[test]
    fn an_edge_with_no_room_for_its_label_keeps_a_plain_rule() {
        // A label asks for two spaces and a rule character of its own
        // beside it. An edge that narrow draws the rule it always drew
        // rather than half a label.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 5, 0, 0);
        let plain = String::from(RULE).repeat(5);
        assert_eq!(visible(&rows[0]), plain, "{rows:?}");
        assert_eq!(visible(&rows[rule_row(&rows)]), plain, "{rows:?}");
    }

    #[test]
    fn the_narrowest_terminal_there_is_still_carries_both_labels() {
        // `width` floors at 24 and the two tests below drive the panel
        // under that floor to reach the fallback at all. This is the
        // narrowest panel a person can actually be looking at.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 24, 0, 0);
        assert!(rows[0].contains("1/1"), "{rows:?}");
        assert!(rows[rule_row(&rows)].contains(WHOLE_WORD_BADGE), "{rows:?}");
        assert_eq!(cells_of_row(&rows[0]), 24, "{rows:?}");
        assert_eq!(cells_of_row(&rows[rule_row(&rows)]), 24, "{rows:?}");
    }

    #[test]
    fn the_count_gives_its_room_up_before_the_badge_does() {
        // Each edge answers for its own label. The count is the longer of
        // the two and an edge one cell wider than the badge needs is one
        // the count has already given up on.
        let items = dirs(1);
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 6, 0, 0);
        assert_eq!(visible(&rows[0]), String::from(RULE).repeat(6), "{rows:?}");
        assert!(rows[rule_row(&rows)].contains(WHOLE_WORD_BADGE), "{rows:?}");
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
        assert_eq!(rows.len(), 9);
        assert!(rows[1].contains("d12"), "{rows:?}");
        assert_eq!(chosen_rows(&rows), vec![6]);
    }

    #[test]
    fn the_top_edge_carries_the_position_in_the_list() {
        let items = dirs(3);
        let m = menu_in(&items, 1, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(rows[0].contains("2/3"), "{:?}", rows[0]);
        // The word under the list keeps every cell it has. The count used to
        // take its own off the end of that word and a description long
        // enough left it none. Neither one was sure of its place.
        let foot = rows.last().expect("a footer");
        assert!(!foot.contains("2/3"), "{foot:?}");
        assert!(foot.contains("folder"), "{foot:?}");
    }

    #[test]
    fn a_label_as_wide_as_the_panel_is_not_cut_for_the_count() {
        let mut item = dir("work");
        item.label = Cow::Owned("x".repeat(PANEL_INNER));
        let items = [item];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let foot = rows.last().expect("a footer");
        assert_eq!(foot.matches('x').count(), PANEL_INNER, "{foot:?}");
        assert!(!foot.contains('…'), "{foot:?}");
    }

    /// Where the rule under the list sits. What comes after it is the
    /// wrapped name and the word under the list, and how many rows those
    /// take depends on what they hold. No test counts back from the end
    /// of the panel for that reason.
    ///
    /// A rule is what a row opens with rather than what the whole of it
    /// holds: both of the panel's own carry a label let into the right
    /// end. The top one is skipped rather than told apart.
    fn rule_row(rows: &[String]) -> usize {
        rows.iter()
            .skip(1)
            .position(|row| visible(row).trim_start().starts_with(RULE))
            .expect("a rule under the list")
            + 1
    }

    /// The rows the word under the list took, drawn as they were drawn,
    /// for one row that says nothing else. The terminal is one cell short of
    /// the pair and the key therefore opens the word under the list rather
    /// than beside it.
    fn footer_rows_for(label: &str, height: usize, whole_word: bool) -> Vec<String> {
        let mut item = dir("work");
        item.label = Cow::Owned(label.to_string());
        let items = [item];
        let mut m = menu_in(&items, 0, height, "", 0).expect("a menu");
        m.whole_word = whole_word;
        let rows = menu_rows(&m, NARROW, 1, 0);
        rows[rule_row(&rows) + 1..].to_vec()
    }

    /// The longest description git's own 38 subcommands carry. Three of
    /// the panel's rows hold it and the one row under the list does not.
    const LONG_DESCRIPTION: &str = "Create new commit that undoes all of the changes made in <commit>, then apply it to the current branch";

    /// Rows joined back into the sentence they are. A row broken at a
    /// space joins back to exactly what went in.
    fn joined(rows: &[String]) -> String {
        rows.iter()
            .map(|row| visible(row).trim().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The word under the list on a terminal of the given height.
    fn footer_in(label: &str, height: usize) -> String {
        joined(&footer_rows_for(label, height, false))
    }

    /// The same on a terminal with room for every row the panel wants.
    fn footer_for(label: &str) -> String {
        footer_in(label, 24)
    }

    /// The same again, with the key that opens the word pressed.
    fn footer_open(label: &str) -> String {
        joined(&footer_rows_for(label, 24, true))
    }

    #[test]
    fn a_description_the_panel_has_room_for_keeps_every_word() {
        // The trim runs on a sentence the rows on offer cannot hold and
        // on no other. A parenthetical that fits is a parenthetical the
        // reader gets.
        let whole = "Stage a file (or a folder)";
        assert_eq!(footer_for(whole), whole);
        assert_eq!(footer_in(whole, 7), whole);
    }

    #[test]
    fn a_description_too_wide_falls_back_to_its_first_clause() {
        // 66 cells against the one row the word takes unasked. The clause
        // in front of the parenthetical is 17 and arrives whole rather
        // than cut at 39. The key is what asks for the rest.
        let whole = "Use TCP/IP device (error if multiple TCP/IP devices are available)";
        assert_eq!(footer_for(whole), "Use TCP/IP device");
        assert_eq!(footer_open(whole), whole);
    }

    #[test]
    fn a_second_sentence_goes_the_same_way_as_a_parenthetical() {
        let whole = "Add file contents to the index. Paths may repeat";
        assert_eq!(footer_in(whole, 7), "Add file contents to the index");
    }

    #[test]
    fn a_full_stop_that_closes_no_sentence_is_not_a_cut() {
        // `e.g.` and an initial each carry one. A sentence ends where two
        // letters or digits run into the stop and neither of these does.
        assert_eq!(
            clause("Set a flag, e.g. -v, on the run"),
            "Set a flag, e.g. -v, on the run"
        );
        assert_eq!(
            clause("Read A. B. Author's own file"),
            "Read A. B. Author's own file"
        );
    }

    #[test]
    fn a_description_that_is_all_parenthetical_keeps_itself() {
        // Nothing sits in front of the bracket and an empty footer says
        // less than a cut one.
        let whole = "(the rest of this is longer than the panel is wide by far)";
        assert_eq!(clause(whole), whole);
        assert!(footer_in(whole, 7).contains('…'));
    }

    #[test]
    fn a_first_clause_the_rows_still_cannot_hold_is_cut() {
        let foot = footer_in(
            "Report the state of every single one of the working tree's own files (verbosely)",
            7,
        );
        assert!(foot.contains('…'), "{foot:?}");
        assert!(!foot.contains("verbosely"), "{foot:?}");
    }

    #[test]
    fn a_second_row_is_the_key_s_to_give_and_not_the_terminal_s() {
        // 53 cells against the panel's 40. A terminal with 17 rows to
        // spare still shows one. The rows under the list are the list's
        // until the key asks for them.
        let whole = "Attach local standard input, output and error streams";
        assert_eq!(footer_rows_for(whole, 24, false).len(), 1);
        assert!(footer_for(whole).contains('…'));
        // `wrap` would break `output` over the two rows and this breaks
        // at the space in front of it. That is what lets the two join
        // back into the sentence.
        assert_eq!(footer_rows_for(whole, 24, true).len(), 2);
        assert_eq!(footer_open(whole), whole);
    }

    #[test]
    fn the_key_opens_the_word_to_every_row_the_terminal_spared() {
        // 102 cells against the 40 one row holds. The terminal has the
        // rows and the key is what spends them.
        let whole = LONG_DESCRIPTION;
        assert_eq!(footer_rows_for(whole, 24, false).len(), 1);
        assert!(footer_for(whole).contains('…'));
        assert_eq!(footer_rows_for(whole, 24, true).len(), 3);
        assert_eq!(footer_open(whole), whole);
    }

    #[test]
    fn the_key_takes_no_row_a_short_terminal_never_had() {
        // The rows it opens are the ones the list and a wrapped name
        // left. A terminal with none leaves the word where it was, down
        // to the clause the one row shows. The second description below
        // has a clause and a whole that are different text. A key that
        // only looked like it had done nothing would show there.
        for whole in [
            LONG_DESCRIPTION,
            "Use TCP/IP device (error if multiple TCP/IP devices are available)",
        ] {
            let mut item = dir("work");
            item.label = Cow::Owned(whole.to_string());
            let items = [item];
            let mut m = menu_in(&items, 0, 7, "", 0).expect("a menu");
            let shut = menu_rows(&m, NARROW, 1, 0);
            m.whole_word = true;
            assert_eq!(menu_rows(&m, NARROW, 1, 0), shut, "{whole:?}");
        }
    }

    #[test]
    fn a_clause_the_key_opened_is_not_trimmed_a_second_time() {
        // Shut the sentence does not fit its one row and the clause it
        // falls back to does not either. The row is cut. Opened this
        // terminal holds the whole of it. The key never answers with a
        // clause: trimming one is what it was pressed to undo.
        let whole = "Attach local standard input, output and error streams to a running container (and detach again)";
        assert_eq!(
            footer_for(whole),
            "Attach local standard input, output and…"
        );
        assert_eq!(footer_open(whole), whole);
    }

    /// The rows the word beside the list took, drawn as they were drawn,
    /// with the key pressed and a list of `names` names on a terminal
    /// `height` rows tall. A row the word does not reach gives nothing back.
    fn beside_rows_for(label: &str, names: usize, height: usize) -> Vec<String> {
        let mut items = dirs(names);
        items[0].display = "work".into();
        items[0].label = Cow::Owned(label.to_string());
        let mut m = menu_in(&items, 0, height, "", 0).expect("a menu");
        m.whole_word = true;
        let rows = menu_rows(&m, WIDE, 1, 0);
        (1..rows.len())
            .map(|r| right_of(&rows, r))
            .filter(|word| !word.is_empty())
            .collect()
    }

    /// The word beside the list, joined back into the sentence it is.
    fn beside(label: &str, names: usize, height: usize) -> String {
        joined(&beside_rows_for(label, names, height))
    }

    #[test]
    fn the_word_beside_the_list_is_the_whole_of_it() {
        // Beside the list the word has the list's own rows and every row the
        // terminal spared under them. The longest description git's own
        // subcommands carry is 102 cells and takes four rows of 30 beside a
        // list of seven names and beside a list of one. Nothing is trimmed:
        // the key asked for the whole of it and a clause is not that.
        let whole = "Use TCP/IP device (error if multiple TCP/IP devices are available)";
        for names in [1, 7] {
            assert_eq!(beside(LONG_DESCRIPTION, names, 24), LONG_DESCRIPTION);
            assert_eq!(beside_rows_for(LONG_DESCRIPTION, names, 24).len(), 4);
            assert_eq!(beside(whole, names, 24), whole);
        }
    }

    #[test]
    fn a_word_with_no_row_to_spare_keeps_to_the_rows_of_the_list_beside_it() {
        // Five rows of terminal and one name. The list takes its one row and
        // leaves the word nothing under it. The word keeps that row and the
        // clause it falls back to, the way it would under the list.
        let whole = "Use TCP/IP device (error if multiple TCP/IP devices are available)";
        assert_eq!(beside_rows_for(whole, 1, 5).len(), 1);
        assert_eq!(beside(whole, 1, 5), "Use TCP/IP device");
    }

    #[test]
    fn the_word_the_key_opened_outruns_the_list_beside_it() {
        // Four rows of sentence against the one row a single match leaves.
        // Each panel is as tall as what it holds. The list ends on its own
        // last name and the rows below it are the word's alone.
        let mut item = dir("work");
        item.label = Cow::Owned(LONG_DESCRIPTION.to_string());
        let items = [item];
        let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        m.whole_word = true;
        let rows = menu_rows(&m, WIDE, 1, 0);
        // The top edge and the four rows the sentence took.
        assert_eq!(rows.len(), 5);
        assert!(left_of(&rows, 1).contains("work"), "{rows:?}");
        for r in 2..rows.len() {
            assert_eq!(left_of(&rows, r), "", "{rows:?}");
            let word = cells_of_row(&right_of(&rows, r));
            assert_eq!(word, DETAIL_INNER + 2, "{rows:?}");
        }
    }

    #[test]
    fn the_key_opens_the_word_beside_the_list_as_far_as_under_it() {
        // Seven rows is the budget and one name leaves none spare under the
        // list, where the key then has nothing to give. Beside the list the
        // line under it and the word's own row are never drawn. Those two are
        // what the key opens the word into there. The frame ends as tall as
        // the one under the list.
        let mut item = dir("work");
        item.label = Cow::Owned(LONG_DESCRIPTION.to_string());
        let items = [item];
        let mut m = menu_in(&items, 0, 7, "", 0).expect("a menu");
        m.whole_word = true;
        let beside = menu_rows(&m, WIDE, 1, 0);
        assert_eq!(beside.len(), menu_rows(&m, NARROW, 1, 0).len());
        // The word's own edge and three rows of the sentence.
        assert_eq!(beside.len(), 4, "{beside:?}");
        let said = (1..beside.len()).filter(|&r| !right_of(&beside, r).is_empty());
        assert_eq!(said.count(), 3, "{beside:?}");
    }

    #[test]
    fn a_name_too_long_for_its_row_keeps_the_lists_own_column() {
        // The row it came from is up there and the cut it took is that
        // column's doing. The wrapped name therefore stays under the list
        // rather than crossing into the word's own panel. A rule separates
        // the two and carries nothing: the key that opens the word belongs on
        // the word's own edge now.
        let name = "n".repeat(PANEL_INNER * 2);
        let items = vec![dir(&name)];
        let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        m.whole_word = true;
        let rows = menu_rows(&m, WIDE, 1, 0);
        let rule = visible(&left_of(&rows, 2));
        assert!(rule.trim_start().starts_with(RULE), "{rows:?}");
        assert!(!rows[2].contains(WHOLE_WORD_BADGE), "{rows:?}");
        let wrapped: String = (3..rows.len())
            .map(|r| visible(&left_of(&rows, r)).trim().to_string())
            .collect();
        assert_eq!(wrapped, name);
        assert!(right_of(&rows, 0).contains(WHOLE_WORD_BADGE), "{rows:?}");
        assert!(right_of(&rows, 1).contains("folder"), "{rows:?}");
    }

    #[test]
    fn an_owned_label_draws_the_same_footer_as_a_static_one() {
        // The next candidate source reads its label from a spec at run time
        // rather than from a constant. Nothing about the row it draws may
        // depend on which one it was.
        let mut owned = dir("work");
        owned.label = Cow::Owned(owned.label.to_string());
        let render =
            |item: Candidate| menu_rows(&menu_in(&[item], 0, 24, "", 0).unwrap(), 80, 1, 0);
        assert_eq!(render(dir("work")), render(owned));
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
        assert_eq!(chosen_rows(&menu_rows(&m, 80, 1, 0)), vec![2]);
    }

    #[test]
    fn the_characters_what_was_typed_reached_are_marked() {
        let items = vec![dir("work/")];
        let m = menu_in(&items, 0, 24, "wk", 0).expect("a menu");
        assert_eq!(marks(&menu_rows(&m, 80, 1, 0)[1], MARK_CHOSEN), "wk");
    }

    #[test]
    fn a_row_off_the_highlight_wears_the_other_mark() {
        // The mark has to read against the ground under it and the highlighted
        // row has a ground of its own. Neither name leads with what was typed
        // in the second row's case.
        let items = vec![dir("alpha/"), dir("beta/")];
        let m = menu_in(&items, 0, 24, "a", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(marks(&rows[1], MARK_CHOSEN), "a");
        assert_eq!(marks(&rows[2], MARK), "a");
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
            rows[1].contains(&format!(
                "{NAME_CHOSEN}{MARK_CHOSEN}a{PANEL_CHOSEN}{NAME_CHOSEN}"
            )),
            "{rows:?}"
        );
        assert!(
            rows[2].contains(&format!("{NAME_MARKED}{MARK}a{PANEL}{NAME}")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_mark_on_the_home_row_is_the_one_name_colour() {
        // The home shortcut reaches a directory like any other row and its
        // name and its marks therefore read alike. Its glyph is what says
        // which directory it is. A name that changed colour there would say
        // the same thing a second time and in a second place.
        let items = vec![dir("~work/"), home()];
        let m = menu_in(&items, 0, 24, "~", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert!(
            rows[2].contains(&format!("{NAME_MARKED}{MARK}~{PANEL}{NAME}")),
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
        let row = &menu_rows(&m, 80, 1, 0)[1];
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
            rows[1].contains('\u{2026}'),
            "the name was not cut: {rows:?}"
        );
        assert_eq!(underlined(&rows[1]), "");
    }

    #[test]
    fn an_underline_stops_at_the_cut() {
        // Tab would reach past what the row shows. The run it underlines ends
        // where the name the row holds does.
        let items = vec![dir(&format!("{}/", "a".repeat(60)))];
        let m = menu_in(&items, 0, 24, "a", 60).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
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
        assert!(!rows[1].contains(MARK), "{rows:?}");
        assert_eq!(marks(&rows[2], MARK_CHOSEN), "wo");
    }

    #[test]
    fn a_mark_past_the_cut_is_not_drawn() {
        // `fit` cut this name down. A mark on a character the row no longer
        // holds would land on whatever took its place.
        let items = vec![dir(&format!("{}z/", "a".repeat(60)))];
        let m = menu_in(&items, 0, 24, "z", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
        assert!(!row.contains(MARK_CHOSEN), "{row:?}");
    }

    #[test]
    fn the_run_tab_would_add_is_underlined() {
        // Two characters are typed and the three the rows share reach one
        // past them.
        let items = vec![dir("work/"), dir("worse/")];
        let m = menu_in(&items, 0, 24, "wo", 3).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[1]), "r");
        assert_eq!(underlined(&rows[2]), "r");
    }

    #[test]
    fn the_run_tab_would_add_is_underlined_on_a_subcommand_row() {
        // Tab reads a subcommand row the way it reads a directory. The key
        // and the underline answer together, and a menu that underlined
        // nothing here would promise less than the key does.
        let items = vec![command("switch"), command("swap")];
        let m = menu_in(&items, 0, 24, "s", 2).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[1]), "w");
        assert_eq!(underlined(&rows[2]), "w");
    }

    #[test]
    fn a_hint_is_drawn_dim_after_the_name() {
        let items = vec![Candidate {
            hint: vec!["hint".to_string()],
            ..command("switch")
        }];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
        assert!(row.contains(&format!("switch {DIM}hint")), "{row:?}");
    }

    #[test]
    fn a_hint_carries_no_mark_or_underline_of_its_own() {
        // Marking and underlining answer for what a row shows of its own
        // name. The hint follows the name rather than joining it and stays
        // out of both.
        let items = vec![Candidate {
            hint: vec!["hint".to_string()],
            ..command("switch")
        }];
        let m = menu_in(&items, 0, 24, "s", 2).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
        assert_eq!(marks(row, MARK_CHOSEN), "s");
        assert_eq!(underlined(row), "w");
    }

    #[test]
    fn a_hint_with_no_room_left_by_the_name_is_dropped_rather_than_cut() {
        // A name this long fills the whole row on its own, whatever the
        // icon column takes from `PANEL_INNER`, and leaves the hint no
        // cells to sit in.
        let items = vec![Candidate {
            hint: vec!["hint".to_string()],
            ..command(&"x".repeat(PANEL_INNER))
        }];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
        assert!(!row.contains("hint"), "{row:?}");
    }

    #[test]
    fn a_hint_shows_only_the_arguments_that_still_fit() {
        // `git checkout`'s own two arguments, against the real `checkout`
        // name and the panel's own budget. `[branch, file, tag or commit]`
        // is 29 cells beside an 8-cell name and one space, which is exactly
        // the panel's 38-cell text column. `[pathspec...]` has no room left
        // after that and does not show.
        let items = vec![Candidate {
            hint: vec![
                "[branch, file, tag or commit]".to_string(),
                "[pathspec...]".to_string(),
            ],
            ..command("checkout")
        }];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
        assert!(row.contains("[branch, file, tag or commit]"), "{row:?}");
        assert!(!row.contains("[pathspec...]"), "{row:?}");
    }

    #[test]
    fn a_hint_whose_first_argument_has_no_room_shows_none_of_them() {
        // The first argument alone already outgrows what `checkout` leaves.
        // The second is never even weighed against what is left: an
        // argument list shows from the front or shows nothing.
        let items = vec![Candidate {
            hint: vec!["x".repeat(PANEL_INNER), "y".to_string()],
            ..command("checkout")
        }];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = &menu_rows(&m, 80, 1, 0)[1];
        assert!(!row.contains('x') && !row.contains('y'), "{row:?}");
    }

    #[test]
    fn a_branch_underline_counts_the_whole_name() {
        let mut items = vec![command("sample/topic"), command("sample/topaz")];
        for item in &mut items {
            item.kind = Kind::Branch;
            item.label = "branch".into();
        }
        let m = menu_in(&items, 0, 24, "sample/t", "sample/top".len()).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[1]), "op");
        assert_eq!(underlined(&rows[2]), "op");
    }

    #[test]
    fn a_row_tab_passes_over_carries_no_underline() {
        // `beta/` holds an `a` and is in the menu for it. It does not lead
        // with what was typed and Tab reads the rows that do.
        let items = vec![dir("alpha/"), dir("beta/")];
        let m = menu_in(&items, 0, 24, "a", 2).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[1]), "l");
        assert_eq!(underlined(&rows[2]), "");
    }

    #[test]
    fn a_row_that_is_not_a_name_carries_no_underline() {
        // Nothing is typed and every name leads with that. The row that runs
        // the line and the home shortcut are still not names Tab reads.
        let items = vec![run_row("x".into()), dir("work/"), home()];
        let m = menu_in(&items, 1, 24, "", 1).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        assert_eq!(underlined(&rows[1]), "");
        assert_eq!(underlined(&rows[2]), "w");
        assert_eq!(underlined(&rows[3]), "");
    }

    #[test]
    fn the_highlight_decides_whether_tab_reads_the_parent_or_the_children() {
        // Tab takes a highlighted parent row whole and reads the child
        // directories under every other highlight. The underline reads that
        // same answer, so one of the two carries it and never both.
        let items = vec![dir("work/"), parent("../")];
        let under = |selected| {
            let m = menu_in(&items, selected, 24, "", 3).expect("a menu");
            let rows = menu_rows(&m, 80, 1, 0);
            [underlined(&rows[1]), underlined(&rows[2])]
        };
        assert_eq!(under(0), ["wor", ""]);
        assert_eq!(under(1), ["", "../"]);
    }

    #[test]
    fn a_reach_no_further_than_what_was_typed_underlines_nothing() {
        // Tab has nothing to add here. The run would start past its own end
        // and an empty one is what that has to draw.
        let items = vec![dir("work/")];
        let m = menu_in(&items, 0, 24, "work/", 5).expect("a menu");
        assert_eq!(underlined(&menu_rows(&m, 80, 1, 0)[1]), "");
    }

    #[test]
    fn a_mark_and_an_underline_leave_the_width_alone() {
        let items = vec![dir("work/")];
        let plain = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let picked = menu_in(&items, 0, 24, "wo", 5).expect("a menu");
        assert_eq!(
            cells_of_row(&menu_rows(&plain, 80, 1, 0)[1]),
            cells_of_row(&menu_rows(&picked, 80, 1, 0)[1])
        );
    }

    #[test]
    fn the_panel_is_the_same_width_whatever_it_holds() {
        // A short name does not shrink the panel and a long one does not grow
        // it. The test below is where the terminal takes the width back.
        for items in [vec![dir("a")], vec![dir(&"n".repeat(60))]] {
            let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
            let row = &menu_rows(&m, 80, 5, 0)[1];
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
        let row = &menu_rows(&m, w, 1, 0)[1];
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
    fn an_escape_in_an_argument_name_is_dropped_from_the_panel() {
        // A hint is a specification's own text and `spec_dirs` lets a person
        // point that at a file of their own. It reaches a terminal in raw
        // mode, so it goes through the same guard the name does.
        let mut row = command("switch");
        row.hint = vec!["<\x1b[31mbranch>".to_string()];
        let items = vec![row];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let row = &rows[1];
        assert!(row.contains("[31mbranch"), "{row:?}");
        let cmd_fg = icons::of(m.icons, &items[0], true).1;
        let ours: usize = [PANEL_CHOSEN, NAME_CHOSEN, cmd_fg, DIM, RESET]
            .iter()
            .map(|s| s.matches('\x1b').count())
            .sum();
        assert_eq!(row.matches('\x1b').count(), ours, "{row:?}");
    }

    #[test]
    fn an_escape_in_a_directory_name_is_dropped_from_the_panel() {
        let items = vec![dir("\x1b[31mred")];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let row = &rows[1];
        assert!(row.contains("[31mred"), "{row:?}");
        // The only escapes left are the ones this module wrote itself: the
        // ground, the glyph's colour, the name's and the reset.
        let dir_fg = icons::of(m.icons, &items[0], true).1;
        let ours: usize = [PANEL_CHOSEN, NAME_CHOSEN, dir_fg, RESET]
            .iter()
            .map(|s| s.matches('\x1b').count())
            .sum();
        assert_eq!(row.matches('\x1b').count(), ours, "{row:?}");
    }

    #[test]
    fn the_glyph_on_the_highlighted_row_is_not_the_name_colour() {
        let items = vec![dir("alpha/")];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let row = menu_rows(&m, 80, 1, 0)[1].clone();
        // The glyph carries its own colour and the name names the ground's
        // bright one again behind it.
        let (icon, icon_fg) = icons::of(m.icons, &items[0], true);
        assert_ne!(icon_fg, NAME_CHOSEN);
        let want = format!("{icon_fg}{icon} {NAME_CHOSEN}");
        assert!(row.contains(&want), "{row:?}");
    }

    #[test]
    fn the_row_that_runs_the_line_holds_no_name() {
        let items = vec![run_row("work/".to_string()), dir("alpha/")];
        let m = menu_in(&items, 0, 24, "", 0).expect("a menu");
        let rows = menu_rows(&m, 80, 1, 0);
        let row = &rows[1];
        assert!(
            row.contains(icons::of(m.icons, &items[0], true).0),
            "{row:?}"
        );
        // `insert` is the only text this row could have drawn.
        assert!(!row.contains("work"), "{row:?}");
    }

    #[test]
    fn mixed_glyph_widths_keep_the_panel_aligned() {
        // Both sets, because `icon_w` is measured from the set the menu
        // carries. A set whose glyphs are wider than the other's would move
        // the column the names start in, and the text set alone would not
        // say so.
        let kinds = [
            Kind::Run,
            Kind::Dir,
            Kind::Parent,
            Kind::Special,
            Kind::Command,
            Kind::Branch,
            Kind::File,
            Kind::Option,
            Kind::Path,
        ];
        for set in [Set::Text, Set::Nerd] {
            for width in [24, 80] {
                for kind in kinds {
                    let mut item = dir("alpha/");
                    item.kind = kind;
                    let items = vec![item];
                    let mut m = menu_in(&items, 0, 24, "", 0).expect("a menu");
                    m.icons = set;
                    let rows = menu_rows(&m, width, 1, 0);
                    let row = &rows[1];
                    let name_at = row.find("alpha/").expect("the name");
                    // The literal rather than `widest` again: the column is
                    // what `widest` decides and a test that asked it would
                    // agree with a wrong answer. Three is the panel's own
                    // space, one cell of glyph and one of padding.
                    assert_eq!(cells_of_row(&row[..name_at]), 3);
                    assert_eq!(cells_of_row(&rows[1]), cells_of_row(&rows[2]));
                }
            }
        }
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
        // One input row, the line above the entries, two entries, the line
        // under them and the footer.
        assert!(out.contains("\x1b[5A"), "{out:?}");
        assert!(out.contains("d0") && out.contains("d1"), "{out:?}");
    }

    #[test]
    fn a_wrapped_line_takes_its_rows_out_of_the_detail() {
        // Ten rows, one long name and a panel that spends what is left on
        // the detail. The frame is the same height whether the line wraps or
        // not. The rows a second input row costs come off the detail.
        let items = vec![dir(&"x".repeat(120))];
        let walk_up = |text: &str| {
            let mut buf: Vec<u8> = Vec::new();
            let mut line = Line::new();
            line.insert(text);
            Ui::new(&mut buf, 0)
                .render_at(&[], &line, "", menu_in(&items, 0, 10, "", 0), 24)
                .expect("a Vec always takes a write");
            String::from_utf8(buf).expect("the frame is text")
        };
        // One input row, the line above the entry, one entry, the line under
        // it, three detail rows and the footer.
        let out = walk_up("cd abc");
        assert!(out.contains("\x1b[7A"), "{out:?}");
        // This line wraps onto a second row and the cursor sits on it. One
        // detail row pays for that row and the walk back up is shorter by
        // the one the cursor gained.
        let out = walk_up("cd abcdefghijklmnopqrstuvwxy");
        assert!(out.contains("\x1b[6A"), "{out:?}");
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
