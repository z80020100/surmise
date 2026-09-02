//! The editing state.
//!
//! The line, the candidate list and the highlight live here rather than in the
//! picker. Nothing here touches the terminal and the ranking and the
//! acceptance rules are therefore testable on their own.

use crate::candidates::{self, Candidate, Kind};
use crate::fuzzy::{shared_bytes, starts_with_folded};
use crate::line::Line;
use crate::shellword;
use crate::ui;
use std::path::{Path, PathBuf};

pub struct App {
    pub line: Line,
    pub items: Vec<Candidate>,
    pub selected: usize,
    pub dismissed: bool,
    /// The directory the candidates are drawn from.
    pub cwd: PathBuf,
}

/// Quote a candidate's insertion for the shell. A leading `~` is a deliberate
/// expansion and stays outside the quotes. Everything after it is still a
/// literal name that may need them.
fn quote_insert(s: &str) -> String {
    if s == "~" {
        return s.to_string();
    }
    match s.strip_prefix("~/") {
        // Nothing behind the tilde means the name itself is `~`. Handing the
        // shell an expansion with an empty word after it would go home.
        Some("") => shellword::quote(s),
        Some(rest) => format!("~/{}", shellword::quote(rest)),
        None => shellword::quote(s),
    }
}

impl App {
    pub fn new(cwd: PathBuf) -> App {
        App {
            line: Line::new(),
            items: Vec::new(),
            selected: 0,
            dismissed: false,
            cwd,
        }
    }

    /// An App over `cwd` for a line already typed. The picker wants one and so
    /// does every test that starts from a line rather than from a list.
    pub fn over(cwd: &Path, line: &str) -> App {
        let mut a = App::new(cwd.to_path_buf());
        a.line.insert(line);
        a.refresh();
        a
    }

    pub fn refresh(&mut self) {
        self.selected = 0;
        // `arg` rather than the parse. A word nothing here may grow is one to
        // offer no menu for. The key then falls through to the shell's own
        // completion instead of opening rows nothing can take.
        self.items = match self.arg() {
            Some(q) => candidates::generate_in(&shellword::unquote(&q.arg), &self.cwd),
            None => Vec::new(),
        };
    }

    pub fn edited(&mut self) {
        self.dismissed = false;
        self.refresh();
    }

    pub fn menu_open(&self) -> bool {
        !self.dismissed && !self.items.is_empty()
    }

    fn highlighted(&self) -> Option<&Candidate> {
        if !self.menu_open() {
            return None;
        }
        self.items.get(self.selected)
    }

    /// The `cd` argument the menu answers for and where it starts. Every key
    /// that would grow the argument asks this first.
    ///
    /// `None` when the line is not a `cd`. `None` as well in the two places
    /// where the word on the line is not the word the menu read. The first is
    /// an argument ending in a space. That space is the person saying the word
    /// is finished. The second is a word that carries on to the right of the
    /// cursor: the rows come from `left_of_cursor` and so does the range a
    /// replacement covers. Writing one in there would leave the tail of the
    /// old word stranded behind it.
    fn arg(&self) -> Option<candidates::Query> {
        let q = candidates::parse(self.line.left_of_cursor())?;
        // A space inside a quote is a character of the name. Outside one it is
        // the person saying the word is finished.
        let quoted = q.arg.starts_with(['\'', '"']);
        if !quoted && q.arg.ends_with(char::is_whitespace) {
            return None;
        }
        let tail = self.line.right_of_cursor();
        (tail.is_empty() || tail.starts_with(char::is_whitespace)).then_some(q)
    }

    /// What the prediction would add to the line, drawn dim after the cursor.
    ///
    /// The match is case-exact here where everything else folds. A dim tail
    /// cannot show that the name corrects the case of what was typed and a row
    /// that only matches folded therefore shows no tail. `adds_to_the_line`
    /// is what says whether the row has something to give.
    pub fn ghost(&self) -> String {
        if !self.line.at_end() {
            return String::new();
        }
        let Some(pick) = self.highlighted() else {
            return String::new();
        };
        let Some(q) = self.arg() else {
            return String::new();
        };
        // A dim tail after a quoted argument would land past the quote that
        // closes it. `adds_to_the_line` is the question the right arrow asks
        // and it has no such trouble: `accept` puts a whole name inside the
        // quotes.
        if q.arg.starts_with(['\'', '"']) {
            return String::new();
        }
        let arg = shellword::unquote(&q.arg);
        pick.insert.strip_prefix(&arg).unwrap_or("").to_string()
    }

    /// Whether the highlighted row would add anything to the line.
    ///
    /// The right arrow asks this rather than asking `ghost`. The ghost is empty
    /// whenever the name corrects the case of what was typed and empty inside a
    /// quote as well. `accept` takes the row in both of those places. The ghost
    /// is empty on the row that runs the line too and that row really has
    /// nothing to add.
    pub fn adds_to_the_line(&self) -> bool {
        let Some(pick) = self.highlighted() else {
            return false;
        };
        let Some(q) = self.arg() else {
            return false;
        };
        let arg = shellword::unquote(&q.arg);
        starts_with_folded(&pick.insert, &arg)
            && shared_bytes(&pick.insert, &arg) < pick.insert.len()
    }

    /// Cell offset of the argument inside the line. The menu hangs under it.
    /// This counts cells rather than bytes, because the whitespace a person
    /// may put in front of `cd` can be wider than one cell.
    pub fn arg_col(&self) -> usize {
        match candidates::parse(self.line.left_of_cursor()) {
            Some(q) => ui::cells(&self.line.text()[..q.start]),
            None => 0,
        }
    }

    /// Whether Enter is a request to run the line rather than to grow it.
    ///
    /// The row that runs the line says so itself. So does an empty menu: there
    /// is nothing left to take and the line as it stands is the whole answer.
    pub fn runs_the_line(&self) -> bool {
        self.highlighted().is_none_or(|pick| pick.kind == Kind::Run)
    }

    /// Put `s` on the line in place of the argument and draw the menu again
    /// for what the line now says.
    fn replace_arg(&mut self, start: usize, s: &str) {
        self.line.replace_back_to(start, s);
        self.dismissed = false;
        self.refresh();
    }

    /// Take the highlighted row's whole name. `false` when there was nothing
    /// to take. That is what tells Enter the line is the whole answer.
    pub fn accept(&mut self) -> bool {
        let Some(pick) = self.highlighted() else {
            return false;
        };
        let Some(q) = self.arg() else {
            return false;
        };
        let insert = quote_insert(&pick.insert);
        self.replace_arg(q.start, &insert);
        true
    }

    pub fn step(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::{App, candidates, quote_insert};
    use crate::fixture::Fixture;
    use std::path::PathBuf;

    /// An App with a known list and no directory behind it. The list is what
    /// is under test. A `refresh` that finds nothing therefore does no harm.
    fn staged(line: &str, inserts: &[&str]) -> App {
        let mut a = App::new(PathBuf::from("/no-such-directory-here"));
        a.line.insert(line);
        a.items = inserts
            .iter()
            .map(|s| candidates::folder((*s).to_string(), (*s).to_string(), 0))
            .collect();
        a
    }

    fn cursor_after(a: &mut App, chars_in: usize) {
        a.line.home();
        for _ in 0..chars_in {
            a.line.right();
        }
    }

    #[test]
    fn refresh_fills_the_list_from_the_named_directory() {
        let f = Fixture::new(&["work", "other"]);
        let a = App::over(f.path(), "cd wor");
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].insert, "work/");
    }

    #[test]
    fn refresh_empties_the_list_when_the_line_is_not_a_cd() {
        let f = Fixture::new(&["work"]);
        let a = App::over(f.path(), "ls wor");
        assert!(a.items.is_empty());
        assert!(!a.menu_open());
    }

    #[test]
    fn refresh_takes_the_quoting_off_the_argument() {
        let f = Fixture::new(&["my docs"]);
        let a = App::over(f.path(), "cd 'my d");
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].insert, "my docs/");
    }

    #[test]
    fn refresh_puts_the_highlight_back_on_the_first_row() {
        let f = Fixture::new(&["work", "worse"]);
        let mut a = App::over(f.path(), "cd wor");
        a.step(1);
        assert_eq!(a.selected, 1);
        a.refresh();
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn ghost_shows_what_the_highlighted_row_would_add() {
        let a = staged("cd wo", &["work/"]);
        assert_eq!(a.ghost(), "rk/");
    }

    #[test]
    fn ghost_says_nothing_when_the_cursor_is_not_at_the_end() {
        let mut a = staged("cd wo", &["work/"]);
        cursor_after(&mut a, 4);
        assert_eq!(a.ghost(), "");
    }

    #[test]
    fn ghost_says_nothing_for_a_quoted_argument() {
        assert_eq!(staged("cd 'wo", &["work/"]).ghost(), "");
        assert_eq!(staged("cd \"wo", &["work/"]).ghost(), "");
    }

    #[test]
    fn ghost_says_nothing_when_the_row_is_not_a_continuation() {
        let a = staged("cd wo", &["awork/"]);
        assert_eq!(a.ghost(), "");
    }

    #[test]
    fn ghost_says_nothing_once_the_menu_is_dismissed() {
        let mut a = staged("cd wo", &["work/"]);
        a.dismissed = true;
        assert_eq!(a.ghost(), "");
    }

    #[test]
    fn arg_col_counts_cells_rather_than_bytes() {
        // An ideographic space is three bytes wide and two cells wide.
        let a = staged("\u{3000}cd work", &["work/"]);
        assert_eq!(a.arg_col(), 5);
    }

    #[test]
    fn arg_col_is_zero_when_the_line_is_not_a_cd() {
        assert_eq!(staged("ls work", &["work/"]).arg_col(), 0);
    }

    #[test]
    fn step_wraps_at_both_ends() {
        let mut a = staged("cd ", &["a/", "b/", "c/"]);
        a.step(-1);
        assert_eq!(a.selected, 2);
        a.step(1);
        assert_eq!(a.selected, 0);
        a.step(5);
        assert_eq!(a.selected, 2);
    }

    #[test]
    fn step_on_an_empty_list_does_nothing() {
        let mut a = staged("cd ", &[]);
        a.step(1);
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn accept_replaces_the_token_and_keeps_the_tail() {
        let mut a = staged("cd wo tail", &["work/"]);
        cursor_after(&mut a, 5);
        a.accept();
        assert_eq!(a.line.text(), "cd work/ tail");
        assert_eq!(a.line.left_of_cursor(), "cd work/");
    }

    #[test]
    fn accept_quotes_a_name_that_holds_a_space() {
        let mut a = staged("cd my", &["my docs/"]);
        a.accept();
        assert_eq!(a.line.text(), "cd 'my docs/'");
    }

    #[test]
    fn accept_replaces_a_quote_the_person_had_opened() {
        let mut a = staged("cd 'my d", &["my docs/"]);
        a.accept();
        assert_eq!(a.line.text(), "cd 'my docs/'");
    }

    #[test]
    fn accept_leaves_a_leading_tilde_for_the_shell_to_expand() {
        let mut a = staged("cd ~/wo", &["~/work/"]);
        a.accept();
        assert_eq!(a.line.text(), "cd ~/work/");
    }

    #[test]
    fn accept_does_nothing_while_the_menu_is_closed() {
        let mut a = staged("cd wo", &["work/"]);
        a.dismissed = true;
        a.accept();
        assert_eq!(a.line.text(), "cd wo");
    }

    #[test]
    fn enter_runs_the_line_on_the_row_that_says_so() {
        let f = Fixture::new(&["work"]);
        assert!(App::over(f.path(), "cd work").runs_the_line());
    }

    #[test]
    fn a_directory_row_is_not_a_request_to_run_the_line() {
        let f = Fixture::new(&["work"]);
        assert!(!App::over(f.path(), "cd wor").runs_the_line());
    }

    #[test]
    fn an_empty_menu_leaves_enter_the_line_as_it_stands() {
        let f = Fixture::new(&["work"]);
        assert!(App::over(f.path(), "cd zzz").runs_the_line());
    }

    #[test]
    fn accept_says_so_when_it_has_nothing_to_take() {
        // The cursor has moved left of the argument the menu answers for and
        // the parse therefore fails. Enter reads the `false` and runs the line
        // rather than sitting there doing nothing.
        let f = Fixture::new(&["work", "worse"]);
        let mut a = App::over(f.path(), "cd wor");
        a.line.home();
        assert!(a.menu_open());
        assert!(!a.runs_the_line());
        assert!(!a.accept());
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn accept_says_so_when_it_takes_a_row() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        assert!(a.accept());
        assert_eq!(a.line.text(), "cd work/");
    }

    #[test]
    fn a_row_that_corrects_the_case_still_completes() {
        // The ghost is empty here, because a dim tail cannot show that `WO`
        // becomes `wo`. The right arrow asks `adds_to_the_line` for that
        // reason.
        let mut a = staged("cd WO", &["work/"]);
        assert_eq!(a.ghost(), "");
        assert!(a.adds_to_the_line());
        a.accept();
        assert_eq!(a.line.text(), "cd work/");
    }

    #[test]
    fn the_row_that_runs_the_line_completes_nothing() {
        let f = Fixture::new(&["work"]);
        let a = App::over(f.path(), "cd work");
        assert!(a.runs_the_line());
        assert!(!a.adds_to_the_line());
    }

    #[test]
    fn a_trailing_space_ends_the_word_and_the_menu_with_it() {
        // `cd work ` is a finished word. There is nothing here to grow and no
        // menu to draw. The key falls through to the shell's own completion
        // rather than opening rows nothing can take.
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd work ");
        assert!(a.items.is_empty());
        assert!(!a.menu_open());
        assert_eq!(a.ghost(), "");
        assert!(!a.adds_to_the_line());
        assert!(!a.accept());
        assert_eq!(a.line.text(), "cd work ");
    }

    #[test]
    fn a_space_inside_a_quote_is_a_character_of_the_name() {
        // The finished-word rule reads a space at the end of the argument. A
        // space inside a quote is not that: the name carries on and the
        // closing quote has not been typed yet.
        let f = Fixture::new(&["my docs/inner"]);
        let mut a = App::over(f.path(), "cd 'my ");
        assert_eq!(a.items.len(), 1);
        assert!(a.adds_to_the_line());
        a.accept();
        assert_eq!(a.line.text(), "cd 'my docs/'");
    }

    #[test]
    fn a_word_that_carries_on_past_the_cursor_is_left_whole() {
        // The rows answer for `wor` and the replacement would cover `wor`
        // alone. Writing `work/` in there used to strand the `k` and leave
        // `cd workk` behind. Every key that would grow the argument refuses.
        let f = Fixture::new(&["work", "workshop"]);
        let mut a = App::over(f.path(), "cd work");
        a.line.left();
        assert!(a.menu_open());
        assert_eq!(a.ghost(), "");
        assert!(!a.adds_to_the_line());
        a.accept();
        assert_eq!(a.line.text(), "cd work");
    }

    #[test]
    fn a_second_argument_is_still_a_word_to_grow() {
        // The guard reads the character right of the cursor rather than asking
        // for the end of the line. A tail behind a space is a word of its own
        // and the argument in front of it is finished.
        let mut a = staged("cd wo tail", &["work/"]);
        cursor_after(&mut a, 5);
        assert!(a.adds_to_the_line());
        assert!(a.accept());
        assert_eq!(a.line.text(), "cd work/ tail");
    }

    #[test]
    fn the_right_arrow_takes_a_row_inside_a_quote() {
        // `accept` puts a whole name inside the quotes and closes them behind
        // it. The ghost cannot draw that and the right arrow therefore asks
        // `adds_to_the_line` rather than the ghost. Tab and Enter take the
        // same row.
        let f = Fixture::new(&["my docs/inner"]);
        let mut a = App::over(f.path(), "cd 'my d");
        assert_eq!(a.ghost(), "");
        assert!(a.adds_to_the_line());
        a.accept();
        assert_eq!(a.line.text(), "cd 'my docs/'");
    }

    #[test]
    fn a_directory_named_like_the_home_shortcut_goes_in_as_a_name() {
        // `cd ~/''` is the home directory rather than the child named `~`.
        // The candidate list keeps that child and the insertion has to as
        // well.
        let f = Fixture::new(&["~"]);
        let mut a = App::over(f.path(), "cd ");
        assert_eq!(a.items[0].insert, "~/");
        a.accept();
        assert_eq!(a.line.text(), "cd '~/'");
    }

    #[test]
    fn edited_reopens_a_menu_that_was_dismissed() {
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd wor");
        a.dismissed = true;
        assert!(!a.menu_open());
        a.edited();
        assert!(a.menu_open());
    }

    #[test]
    fn an_insertion_keeps_a_tilde_outside_the_quotes() {
        assert_eq!(quote_insert("~"), "~");
        assert_eq!(quote_insert("~/work/"), "~/work/");
        assert_eq!(quote_insert("~/my docs/"), "~/'my docs/'");
        // The home row is the one bare `~`. A tilde with a slash and nothing
        // else is the name of a directory in the way.
        assert_eq!(quote_insert("~/"), "'~/'");
    }

    #[test]
    fn an_insertion_that_is_not_a_tilde_is_quoted_as_a_literal() {
        assert_eq!(quote_insert("work/"), "work/");
        assert_eq!(quote_insert("my docs/"), "'my docs/'");
        assert_eq!(quote_insert("../"), "../");
        assert_eq!(quote_insert("-x/"), "'-x/'");
        assert_eq!(quote_insert("~work/"), "'~work/'");
    }
}
