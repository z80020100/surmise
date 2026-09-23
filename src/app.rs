//! The editing state.
//!
//! The line, the candidate list and the highlight live here rather than in the
//! picker. Nothing here touches the terminal and the ranking and the
//! acceptance rules are therefore testable on their own.

use crate::candidates::{self, Candidate, Kind, Scan};
use crate::fuzzy::{shared_bytes, starts_with_folded};
use crate::histfile;
use crate::history::History;
use crate::line::Line;
use crate::shellword;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct App {
    pub line: Line,
    pub items: Vec<Candidate>,
    pub selected: usize,
    pub dismissed: bool,
    /// Whether the last acceptance left behind the menu it found.
    ///
    /// A row that takes nothing out of the list is a row the next press takes
    /// again. The menu would draw the same names behind it for as long as the
    /// key was held. `pick` hands the line back to the shell instead and the
    /// press after that runs it.
    ///
    /// Most acceptances change the list and the menu therefore stays open on
    /// what they changed it to. A subcommand moves the walk to another node.
    /// A folder moves the scan to another directory. Git's own file query
    /// drops the file it just took and its option list drops the option. A
    /// file a specification's own `filepaths` argument offers is the one
    /// that changes nothing: the argument takes as many names as it is
    /// given, the directory has not moved, and the same names come back.
    pub menu_repeats: bool,
    /// The directory the candidates are drawn from.
    pub cwd: PathBuf,
    /// What sat to the right of the cursor when the widget opened, from the
    /// v2 stdin record. `pick::run` already answers `PASS` before an `App`
    /// exists when this starts mid-word; nothing here reads it yet.
    pub rbuffer: String,
    /// The shell's alias table, name to value, from the same record.
    /// `spec_menu` is what resolves it, on a command name ahead of a spec
    /// lookup.
    pub aliases: HashMap<String, String>,
    /// Whether the whole of the word is open rather than its one row under
    /// the list. It opens beside the list where the terminal has room for
    /// both and under it where it does not. Ctrl-O turns it on and off.
    /// `crate::state` is what carries the answer past the menu it was pressed
    /// in and `pick` is what reads it back.
    pub whole_word: bool,
    history: History,
    scan: Scan,
    git: crate::git::Completions,
    git_start: Option<usize>,
    spec_menu: crate::spec_menu::Completions,
    spec_start: Option<usize>,
}

/// Which provider answers one line, and the word it read there.
///
/// The three are asked in this order and the first to claim the line is the
/// one that answers it. `cd`'s own reader and Git's own each match a literal
/// first word, so neither can ever take the other's line and the order
/// between those two settles nothing. The order between Git's and the spec
/// menu's is the whole of the rule. Both read a `git` line now. Git's own
/// parser is the narrower of the two and what it declines — `git blame `,
/// `git stash `, `git commit -`, every finished subcommand that is not
/// `add`, `switch` or `checkout` — is what falls through to a walk of
/// `specs/git.json`. What it claims it keeps: the subcommand word, a branch
/// after `switch` or `checkout`, and an `add` path or option all stay with
/// the readers that ask the installed Git itself.
///
/// One line therefore still has exactly one provider, and that is what
/// `git_start` and `spec_start` record. `relist` sets one of the two and
/// clears the other, and [`App::highlighted`] reads them back to tell a row
/// of one provider's from a row of the other's — a question `Kind::is_git`
/// cannot answer, because both providers name their flat rows with the same
/// kinds.
enum Reader {
    Cd(candidates::Query),
    Git(crate::git::Target),
    Spec(crate::spec_menu::Target),
}

impl Reader {
    /// The word this provider would replace, which is all `App::arg` wants
    /// of it.
    fn into_word(self) -> candidates::Query {
        match self {
            Reader::Cd(word) => word,
            Reader::Git(target) => target.word,
            Reader::Spec(target) => target.word,
        }
    }
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

/// Quote a row's insertion the way the row's own kind asks. A file name is a
/// Git pathspec as well as a shell word. `None` is the prefix several rows
/// share and therefore no row's whole name.
fn quote_kind(kind: Option<Kind>, s: &str) -> String {
    match kind {
        Some(Kind::File) => crate::git::quote_file(s),
        Some(Kind::Option) => s.to_string(),
        Some(Kind::Path) => shellword::quote(s),
        _ => quote_insert(s),
    }
}

fn finishes_word(kind: Kind, name: &str, arg: &str) -> bool {
    match kind {
        Kind::Option => !name.ends_with('='),
        Kind::File | Kind::Path => !name.ends_with('/') || name == arg,
        _ => kind.is_git(),
    }
}

/// What Tab would do to the line.
struct Common {
    /// Byte offset in the line where the argument starts.
    start: usize,
    /// The name the argument would spell. It is unquoted and `accept_common`
    /// is what quotes it.
    name: String,
    /// How many characters of a row's own name `name` covers. The menu
    /// underlines the run of that past what was typed.
    reach: usize,
    /// The row `name` is the whole name of. `None` when `name` is the prefix
    /// several rows share and therefore no row's whole name. `accept_common`
    /// reads this to know a whole subcommand went in.
    whole: Option<Kind>,
}

impl Common {
    /// `name` in place of the argument that starts at `start`. The reach is
    /// what is left of `name` once the directory the rows come from is taken
    /// off the front of it. Every row draws that directory's children and the
    /// name each one holds is the part the reach is measured in. Git names
    /// use zero for `dir` and measure the whole name.
    fn new(start: usize, name: String, dir: usize, whole: Option<Kind>) -> Common {
        Common {
            reach: name.chars().count().saturating_sub(dir),
            start,
            name,
            whole,
        }
    }
}

impl App {
    pub fn new(cwd: PathBuf) -> App {
        App {
            line: Line::new(),
            items: Vec::new(),
            selected: 0,
            dismissed: false,
            menu_repeats: false,
            cwd,
            rbuffer: String::new(),
            aliases: HashMap::new(),
            whole_word: false,
            history: History::default(),
            scan: Scan::default(),
            git: crate::git::Completions::default(),
            git_start: None,
            spec_menu: crate::spec_menu::Completions::default(),
            spec_start: None,
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

    /// The directory history and the command history, set together and
    /// without a refresh of their own. A caller that has both sets both and
    /// then refreshes once, rather than paying a rebuild of the whole
    /// candidate list for each one learned.
    ///
    /// `cmd_history` goes to both providers that rank by it, a clone for one
    /// of them. Never more than one answers a given line, so which of the
    /// two gets the real reading is nothing to track here, and `cd`'s own
    /// candidates read neither field at all.
    pub(crate) fn seed_history(&mut self, history: History, cmd_history: histfile::Counts) {
        self.history = history;
        self.git.cmd_history = cmd_history.clone();
        self.spec_menu.cmd_history = cmd_history;
    }

    pub fn refresh(&mut self) {
        self.items = self.relist();
        self.selected = 0;
        // A list built for a line rather than for a row taken off one. Only
        // `replace_arg` has a list to compare this against and it sets the
        // answer itself.
        self.menu_repeats = false;
    }

    /// The provider that reads the line as it stands, and what it read
    /// there. [`Reader`] is where the order between the three is written
    /// down, and this is the one place that order is applied: `arg`,
    /// `relist` and `highlighted` all ask this rather than trying the
    /// parsers themselves and each settling the tie its own way.
    fn reader(&self) -> Option<Reader> {
        let left = self.line.left_of_cursor();
        if let Some(word) = candidates::parse(left) {
            return Some(Reader::Cd(word));
        }
        if let Some(target) = crate::git::parse(left) {
            return Some(Reader::Git(target));
        }
        crate::spec_menu::parse(left, self.line.right_of_cursor(), &self.aliases).map(Reader::Spec)
    }

    /// Whether `word` is one the menu may still grow. It is not in two
    /// places, where the word on the line is no longer the word the menu
    /// read. The first is an argument ending in a space. That space is the
    /// person saying the word is finished. The second is a word that carries
    /// on to the right of the cursor: the rows come from `left_of_cursor` and
    /// so does the range a replacement covers. Writing one in there would
    /// leave the tail of the old word stranded behind it.
    fn growable(&self, word: &candidates::Query) -> bool {
        // A space inside a quote is a character of the name. Outside one it is
        // the person saying the word is finished.
        let quoted = word.arg.starts_with(['\'', '"']);
        if !quoted && word.arg.ends_with(char::is_whitespace) {
            return false;
        }
        let tail = self.line.right_of_cursor();
        tail.is_empty() || tail.starts_with(char::is_whitespace)
    }

    /// The rows the line as it stands asks for. It also records which
    /// provider read the line.
    ///
    /// `refresh` is what stores them. [`App::replace_arg`] asks for them
    /// separately. It has the rows the menu already held to measure them
    /// against and a list stored over the top of those would leave it nothing
    /// to measure.
    fn relist(&mut self) -> Vec<Candidate> {
        let reader = self.reader();
        // Where the word the rows answer for begins, under the provider that
        // read it. Never both at once: one line has one provider and
        // `highlighted` reads these back to tell whose row it is holding.
        (self.git_start, self.spec_start) = match &reader {
            Some(Reader::Git(target)) => (Some(target.word.start), None),
            Some(Reader::Spec(target)) => (None, Some(target.word.start)),
            _ => (None, None),
        };
        // `growable` rather than the parse alone. A word nothing here may
        // grow is one to offer no menu for. The key then falls through to the
        // shell's own completion instead of opening rows nothing can take.
        match reader {
            Some(Reader::Git(mut target)) if self.growable(&target.word) => {
                target.exclude_tail(self.line.right_of_cursor());
                self.git.complete(&target, &self.cwd)
            }
            Some(Reader::Spec(target)) if self.growable(&target.word) => {
                self.spec_menu
                    .complete(&target, &self.cwd, &self.history, &mut self.scan)
            }
            Some(Reader::Cd(word)) if self.growable(&word) => candidates::generate_in(
                &shellword::unquote(&word.arg),
                &self.cwd,
                &self.history,
                &mut self.scan,
            ),
            _ => Vec::new(),
        }
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
        let pick = self.items.get(self.selected)?;
        // A row from the flat, non-directory providers goes stale once the
        // cursor no longer sits on the word it would replace. `Kind::is_git`
        // is what puts a row in that group and no more than that: Git's own
        // menu and the spec menu both draw their rows with those kinds, so
        // which provider a row came from is `git_start` and `spec_start`'s
        // answer rather than the kind's. `relist` sets one of the two and
        // clears the other, and the line has to still read as that same
        // provider's, on the same word, for the row to be worth anything.
        //
        // A list a caller staged by hand rather than through `relist` has
        // neither recorded, and a fresh read of the line is the whole of the
        // answer there.
        if pick.kind.is_git() {
            let stale = match self.reader() {
                Some(Reader::Git(target)) => {
                    self.spec_start.is_some()
                        || !target.accepts(pick.kind)
                        || self
                            .git_start
                            .is_some_and(|start| start != target.word.start)
                }
                Some(Reader::Spec(target)) => {
                    self.git_start.is_some()
                        || self
                            .spec_start
                            .is_some_and(|start| start != target.word.start)
                }
                // Nothing reads the line any more, or `cd`'s own reader does
                // and it names none of these rows.
                _ => true,
            };
            if stale {
                return None;
            }
        }
        Some(pick)
    }

    /// The word the menu completes and its byte offset. `None` when no
    /// provider reads the line, and `None` for a word [`App::growable`]
    /// says the menu may no longer grow.
    fn arg(&self) -> Option<candidates::Query> {
        let word = self.reader()?.into_word();
        self.growable(&word).then_some(word)
    }

    /// Whether Git's own menu is what this run is on. `pick` asks before it
    /// loads the directory history and again after each key. Git's own rows
    /// have nothing to take from that history and the second question is
    /// whether the run is over.
    ///
    /// Git's own parser rather than [`App::reader`]: a `git` line the spec
    /// menu answers is one Git's own menu declined, and it wants the
    /// directory history and the ending of every other spec menu. `git blame `
    /// weighs a folder row the way `ls ` does.
    ///
    /// The whole line rather than the half in front of the cursor. The
    /// question is what the run is for rather than what the next key would
    /// grow. A cursor moved off the argument does not change that.
    pub fn completes_git(&self) -> bool {
        crate::git::parse(self.line.text()).is_some()
    }

    /// What was typed into the directory the rows come from. The menu marks
    /// the characters this reached in each name.
    ///
    /// Empty once the cursor has moved off the argument. The rows are still
    /// the ones that word reached and the menu no longer answers for it.
    pub fn typed(&self) -> String {
        let Some(q) = self.arg() else {
            return String::new();
        };
        let arg = shellword::unquote(&q.arg);
        // Splitting into a directory prefix and a name typed into it is a
        // `cd` idea. Git and the spec provider both hand back a flat name and
        // want the whole argument read back rather than split at a `/` that
        // may not even be there.
        if matches!(self.reader(), Some(Reader::Cd(_))) {
            candidates::split(&arg).1.to_string()
        } else {
            arg
        }
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
        // A name the shell would not take as it stands goes on the line inside
        // quotes or behind a pathspec prefix. A dim tail can draw neither and
        // the row therefore shows none. `accept` still takes the whole name.
        if quote_kind(Some(pick.kind), &pick.insert) != pick.insert {
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
            && (pick.kind.is_git() || shared_bytes(&pick.insert, &arg) < pick.insert.len())
    }

    /// Whether Enter is a request to run the line rather than to grow it.
    ///
    /// The row that runs the line says so itself. So does an empty menu: there
    /// is nothing left to take and the line as it stands is the whole answer.
    pub fn runs_the_line(&self) -> bool {
        self.highlighted().is_none_or(|pick| pick.kind == Kind::Run)
    }

    /// Put `s` on the line in place of the argument and draw the menu again
    /// for what the line now says. `whole` is whether `s` spells a row's own
    /// name rather than the prefix several of them share.
    ///
    /// The names the menu held are what the new list is measured against.
    /// [`App::menu_repeats`] is that measurement and the picker is what reads
    /// it. A prefix is measured against nothing. The rows it came from are
    /// the rows that still match it. The list it leaves behind is the same
    /// list and the menu is still the answer to the word being typed.
    fn replace_arg(&mut self, start: usize, s: &str, whole: bool) {
        self.line.replace_back_to(start, s);
        self.dismissed = false;
        let after = self.relist();
        self.menu_repeats = whole
            && after
                .iter()
                .map(|c| &c.insert)
                .eq(self.items.iter().map(|c| &c.insert));
        self.items = after;
        self.selected = 0;
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
        let mut insert = quote_kind(Some(pick.kind), &pick.insert);
        if finishes_word(pick.kind, &pick.insert, &shellword::unquote(&q.arg))
            && self.line.right_of_cursor().is_empty()
        {
            insert.push(' ');
        }
        self.replace_arg(q.start, &insert, true);
        true
    }

    /// How far into a row's own name Tab would reach. 0 when Tab would leave
    /// the line alone.
    ///
    /// The menu underlines the run past what was typed. The key and the
    /// underline read the same answer and the line therefore cannot promise
    /// what the key will not do.
    pub fn reach(&self) -> usize {
        self.common().map_or(0, |c| c.reach)
    }

    /// The name Tab would leave the argument spelling. `None` when Tab would
    /// leave the line alone.
    ///
    /// Tab takes a highlighted parent row whole. Otherwise it offers what the
    /// child directories agree on. One row that leads with what was typed goes
    /// in whole. A prefix that adds nothing to the line leaves it alone.
    fn common(&self) -> Option<Common> {
        let selected = self.highlighted()?;
        let q = self.arg()?;
        let quoted = q.arg.starts_with(['\'', '"']);
        let arg = shellword::unquote(&q.arg);
        let dir = if selected.kind.is_git() {
            0
        } else {
            candidates::split(&arg).0.chars().count()
        };
        if selected.kind == Kind::Parent {
            return Some(Common::new(
                q.start,
                selected.insert.clone(),
                dir,
                Some(selected.kind),
            ));
        }
        if selected.kind == Kind::Run && candidates::split(&arg).1 == ".." {
            return None;
        }
        // A match is a subsequence and need not lead with what was typed. Only
        // the rows that do can agree on something to add to it. The row that
        // runs the line offers the argument back unchanged. Navigation rows
        // do not take part in the prefix the children share.
        let agreeing: Vec<&Candidate> = self
            .items
            .iter()
            .filter(|c| {
                (c.kind == Kind::Dir || c.kind.is_git())
                    && (!selected.kind.is_git() || c.kind == selected.kind)
                    && starts_with_folded(&c.insert, &arg)
            })
            .collect();
        // The count is measured in `head` throughout. What was typed can be a
        // different length from the name it reaches.
        let (head, keep) = match agreeing.as_slice() {
            [] => return None,
            // One row has nothing to disagree with. `quote_insert` closes a
            // quote behind a whole name and this row therefore goes in inside
            // one too.
            [one] => {
                return Some(Common::new(
                    q.start,
                    one.insert.clone(),
                    dir,
                    Some(one.kind),
                ));
            }
            [head, rest @ ..] => {
                let head = head.insert.as_str();
                (
                    head,
                    rest.iter().fold(head.len(), |keep, other| {
                        keep.min(shared_bytes(head, &other.insert))
                    }),
                )
            }
        };
        // Half a name cannot carry the quote that closes it.
        if quoted {
            return None;
        }
        // Two names that fold together share the whole of one of them. A
        // prefix that is a complete name is not a prefix and the menu has not
        // said which of the two it means.
        if keep == head.len() {
            return None;
        }
        let prefix = &head[..keep];
        // The count was measured with the case set aside and the rows can
        // spell that span differently. A spelling only one of them carries is
        // not what they agreed on.
        if agreeing
            .iter()
            .any(|other| !other.insert.starts_with(prefix))
        {
            return None;
        }
        // Nothing left to add. The comparison is on the bytes rather than on
        // the count, because a prefix that only corrects the case of what was
        // typed is still something to add.
        if prefix == arg {
            return None;
        }
        // A word the shell would not read as a single literal. `quote_kind`
        // is the quoting `accept` would apply and a prefix it would quote is
        // half a name inside a quote it cannot close. A bare `~` is the one
        // string it leaves alone for the home row's sake rather than for half
        // a name. Half a name is what this is.
        if prefix == "~" || quote_kind(Some(selected.kind), prefix) != prefix {
            return None;
        }
        Some(Common::new(q.start, prefix.to_string(), dir, None))
    }

    /// Take the prefix the directory rows in the menu share. `common` is what
    /// decides and this is what puts the answer on the line.
    ///
    /// `false` when there was nothing to take. Nothing on the screen would
    /// say so on its own: the line is the same line and the menu is the same
    /// menu. The caller rings the bell instead.
    pub fn accept_common(&mut self) -> bool {
        let Some(c) = self.common() else {
            return false;
        };
        let kind = c.whole.or_else(|| {
            self.highlighted()
                .filter(|pick| pick.kind == Kind::Option)
                .map(|pick| pick.kind)
        });
        let mut insert = quote_kind(kind, &c.name);
        if c.whole
            .is_some_and(|kind| finishes_word(kind, &c.name, &self.typed()))
            && self.line.right_of_cursor().is_empty()
        {
            insert.push(' ');
        }
        self.replace_arg(c.start, &insert, c.whole.is_some());
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
    fn a_complete_git_subcommand_still_offers_its_space() {
        let mut a = staged("git sample", &["sample"]);
        a.items[0].kind = candidates::Kind::Command;
        assert!(a.adds_to_the_line());
        assert!(!a.runs_the_line());
        assert!(a.accept());
        assert_eq!(a.line.text(), "git sample ");
        assert!(!a.menu_open());
    }

    #[test]
    fn a_git_subcommand_keeps_the_arguments_after_the_cursor() {
        let mut a = staged("git samp --example", &["sample"]);
        a.items[0].kind = candidates::Kind::Command;
        cursor_after(&mut a, 8);
        assert!(a.accept());
        assert_eq!(a.line.text(), "git sample --example");
    }

    fn branches(line: &str, names: &[&str]) -> App {
        let mut a = staged(line, names);
        for item in &mut a.items {
            item.kind = candidates::Kind::Branch;
            item.label = "branch".into();
        }
        a
    }

    fn files(line: &str, names: &[&str]) -> App {
        let mut a = staged(line, names);
        for item in &mut a.items {
            item.kind = candidates::Kind::File;
            item.label = "file".into();
        }
        a
    }

    #[test]
    fn add_keeps_remaining_files_available_and_reopens_after_a_quoted_word() {
        let f = Fixture::new(&["sample one*", "sample two*"]);
        f.init_git(&[]);
        let mut a = App::over(f.path(), "git add sample o");
        // The unquoted space starts another argument.
        assert_eq!(a.typed(), "o");
        a = App::over(f.path(), "git add 'sample o");
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add 'sample one' ");
        assert!(a.menu_open());
        // The query drops the file it just took. The menu that comes back is
        // therefore a menu of its own rather than the one that was already
        // there.
        assert!(!a.menu_repeats);
        assert_eq!(
            a.items
                .iter()
                .filter(|c| c.kind == candidates::Kind::File)
                .count(),
            1
        );
        assert_eq!(a.items[0].insert, "sample two");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git add 'sample one' 'sample two' ");
        assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Option));
        let reopened = App::over(f.path(), "git add 'sample one' ");
        assert_eq!(
            reopened
                .items
                .iter()
                .filter(|c| c.kind == candidates::Kind::File)
                .count(),
            1
        );
        assert_eq!(reopened.items[0].insert, "sample two");
        assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "");
    }

    #[test]
    fn moving_to_an_earlier_file_does_not_take_a_stale_row() {
        let f = Fixture::new(&["sample-one*", "sample-two*"]);
        f.init_git(&[]);
        let mut a = App::over(f.path(), "git add sample-one sample-t");
        cursor_after(&mut a, "git add sample-one".len());
        assert!(!a.accept());
        assert_eq!(a.line.text(), "git add sample-one sample-t");
    }

    #[test]
    fn add_excludes_files_on_both_sides_of_the_cursor() {
        let f = Fixture::new(&["sample-one*", "sample-two*", "sample-three*"]);
        f.init_git(&[]);
        let mut a = App::over(f.path(), "git add sample-one samp sample-two");
        cursor_after(&mut a, "git add sample-one samp".len());
        a.refresh();
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].insert, "sample-three");
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add sample-one sample-three sample-two");
    }

    #[test]
    fn add_does_not_exclude_a_path_named_by_an_option_value_after_the_cursor() {
        let f = Fixture::new(&["sample-one*", "sample-two*"]);
        f.init_git(&[]);
        let mut a = App::over(f.path(), "git add samp --pathspec-from-file sample-one");
        cursor_after(&mut a, "git add samp".len());
        a.refresh();
        assert!(a.items.iter().any(|c| c.insert == "sample-one"));
        assert!(a.items.iter().any(|c| c.insert == "sample-two"));
    }

    #[test]
    fn add_descends_into_a_folder_and_accepts_the_whole_folder_on_a_second_enter() {
        let f = Fixture::new(&["sample dir/inner", "sample dir/file*", "other*"]);
        f.init_git(&[]);
        let mut a = App::over(f.path(), "git add 'sample d");
        a.selected = a
            .items
            .iter()
            .position(|c| c.insert == "sample dir/")
            .unwrap();
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add 'sample dir/'");
        assert_eq!(a.items[0].insert, "sample dir/");
        assert!(a.items.iter().any(|c| c.insert == "sample dir/inner/"));
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add 'sample dir/' ");
        assert!(a.items.iter().all(|c| !c.insert.starts_with("sample dir/")));
        assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "");
    }

    #[test]
    fn add_option_acceptance_preserves_separators_and_quotes_option_file_values() {
        let f = Fixture::new(&["sample list*", "sample*"]);
        f.init_git(&[]);
        let mut a = App::over(f.path(), "git add --chm");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git add --chmod=");
        assert!(a.items.iter().any(|c| c.insert == "--chmod=+x"));
        a.selected = a
            .items
            .iter()
            .position(|c| c.insert == "--chmod=+x")
            .unwrap();
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add --chmod=+x ");
        assert!(a.items.iter().any(|c| c.insert == "sample"));
        let mut a = App::over(f.path(), "git add --pathspec-from-file='sample l");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git add '--pathspec-from-file=sample list' ");
        assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Option));
        let mut a = App::over(f.path(), "git add --ignore");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git add --ignore-");
    }

    #[test]
    fn file_acceptance_quotes_whole_names_for_the_shell_and_git() {
        for (name, expected) in [
            ("sample file", "'sample file'"),
            ("sample$(false)'suffix", "'sample$(false)'\\''suffix'"),
            ("sample[1]", "':(literal)sample[1]'"),
            ("sample*", "':(literal)sample*'"),
            ("sample?", "':(literal)sample?'"),
            ("sample\\name", "':(literal)sample\\name'"),
            (":sample", "':(literal):sample'"),
            ("-sample", "./-sample"),
            ("~/sample", "'~/sample'"),
        ] {
            for tab in [false, true] {
                let mut a = files("git add ", &[name]);
                assert!(a.adds_to_the_line());
                assert!(if tab { a.accept_common() } else { a.accept() });
                assert_eq!(a.line.text(), format!("git add {expected} "));
                assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Option));
            }
        }
    }

    #[test]
    fn file_prefixes_cover_whole_paths_without_expanding_them() {
        let mut a = files("git add sample/t", &["sample/topic", "sample/topaz"]);
        assert_eq!(a.typed(), "sample/t");
        assert_eq!(a.reach(), "sample/top".len());
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git add sample/top");
        for names in [
            ["~/sample/a", "~/sample/b"],
            [":sample/a", ":sample/b"],
            ["sample[1]a", "sample[1]b"],
        ] {
            let mut a = files("git add ", &names);
            assert_eq!(a.reach(), 0);
            assert!(!a.accept_common());
            assert_eq!(a.line.text(), "git add ");
        }
    }

    #[test]
    fn file_acceptance_preserves_the_tail_and_refuses_a_stale_command_position() {
        let mut a = files("git add samp second", &["sample file"]);
        cursor_after(&mut a, "git add samp".len());
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add 'sample file' second");
        let mut a = files("git add samp", &["sample"]);
        cursor_after(&mut a, "git add".len());
        assert!(a.completes_git());
        assert!(!a.accept());
        assert!(!a.accept_common());
        assert_eq!(a.line.text(), "git add samp");
    }

    #[test]
    fn branch_prefixes_and_underlines_include_the_slashes() {
        let mut a = branches("git switch sample/t", &["sample/topic", "sample/topical"]);
        assert_eq!(a.typed(), "sample/t");
        assert_eq!(a.reach(), 0);
        a.items[1].insert = "sample/topaz".into();
        assert_eq!(a.reach(), "sample/top".len());
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git switch sample/top");
    }

    #[test]
    fn branch_acceptance_finishes_the_word_without_running() {
        for line in ["git switch sample/t", "git checkout sample/topic"] {
            let mut a = branches(line, &["sample/topic"]);
            assert!(a.adds_to_the_line());
            assert!(!a.runs_the_line());
            assert!(a.accept());
            assert!(a.line.text().ends_with("sample/topic "));
            // The branch is finished and Git's own reader has left the line
            // with it. What the line reads as now is the walk's answer for
            // the word behind the branch, which here is that subcommand's own
            // options. No second branch is offered: the two readers answer one
            // line each and this is where that shows. `pick` ends the run on
            // this line and the next Tab is what opens it.
            assert!(!a.items.is_empty());
            assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Option));
        }
        let mut a = branches("git switch sample/t", &["sample/topic"]);
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git switch sample/topic ");
    }

    #[test]
    fn branch_acceptance_quotes_shell_syntax_and_preserves_the_tail() {
        let mut a = branches("git checkout sample", &["sample$(false)'suffix"]);
        assert!(a.accept());
        assert_eq!(a.line.text(), "git checkout 'sample$(false)'\\''suffix' ");
        let mut a = branches("git checkout sample/t --", &["sample/topic"]);
        cursor_after(&mut a, "git checkout sample/t".len());
        assert!(a.accept());
        assert_eq!(a.line.text(), "git checkout sample/topic --");
    }

    #[test]
    fn a_branch_cannot_replace_the_subcommand_after_cursor_movement() {
        let mut a = branches("git switch sample/t", &["sample/topic"]);
        cursor_after(&mut a, "git switch".len());
        assert!(!a.accept());
        assert!(!a.accept_common());
        assert_eq!(a.line.text(), "git switch sample/t");
    }

    #[test]
    fn refresh_fills_the_list_from_the_named_directory() {
        let f = Fixture::new(&["work", "other"]);
        let a = App::over(f.path(), "cd wor");
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].insert, "work/");
    }

    #[test]
    fn two_dots_offer_execution_and_a_parent_to_browse() {
        let f = Fixture::new(&["level/inner", "level/..cache", "sibling"]);
        let mut a = App::over(&f.path().join("level"), "cd ..");
        assert!(a.runs_the_line());
        assert!(!a.accept_common());
        assert_eq!(a.items[1].insert, "../");
        a.step(1);
        assert!(!a.runs_the_line());
        assert_eq!(a.reach(), 3);
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "cd ../");
        assert!(a.items.iter().any(|c| c.insert == "../sibling/"));
        assert!(a.items.iter().any(|c| c.insert == "../../"));
    }

    #[test]
    fn a_parent_row_does_not_limit_the_prefix_of_child_directories() {
        let f = Fixture::new(&["level/alpha", "level/alps"]);
        let mut a = App::over(f.path(), "cd level/");
        assert!(a.items.iter().any(|c| c.insert == "level/../"));
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "cd level/alp");
    }

    #[test]
    fn a_parent_row_keeps_the_quote_around_a_path_with_spaces() {
        let f = Fixture::new(&["my docs/inner", "other"]);
        let mut a = App::over(f.path(), "cd 'my docs/..");
        a.step(1);
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "cd 'my docs/../'");
        assert!(a.items.iter().any(|c| c.insert == "my docs/../other/"));
    }

    #[test]
    fn refresh_reads_the_directory_once_and_types_against_what_it_read() {
        let f = Fixture::new(&["alpha"]);
        let mut a = App::over(f.path(), "cd a");
        assert_eq!(a.items.len(), 1);
        // A menu already open is answering from the walk it opened with. A
        // directory made after that arrives with the next menu.
        std::fs::create_dir(f.path().join("alps")).unwrap();
        a.line.insert("l");
        a.edited();
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].insert, "alpha/");
    }

    #[test]
    fn refresh_empties_the_list_when_the_line_is_not_a_cd() {
        // `ls` completes its own argument now that `spec_menu` answers a
        // `filepaths` template; `zzz` is what still matches nothing there,
        // in the fixture or among `ls`'s own options.
        let f = Fixture::new(&["work"]);
        let a = App::over(f.path(), "ls zzz");
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
    fn ghost_says_nothing_for_a_name_that_needs_quoting() {
        // `accept` puts such a name inside quotes or behind a pathspec prefix
        // and a dim tail can draw neither. The right arrow asks
        // `adds_to_the_line` and takes the row whole.
        let mut a = staged("cd my", &["my docs/"]);
        assert_eq!(a.ghost(), "");
        assert!(a.adds_to_the_line());
        assert!(a.accept());
        assert_eq!(a.line.text(), "cd 'my docs/'");
        let mut a = files("git add sam", &["sample*"]);
        assert_eq!(a.ghost(), "");
        assert!(a.adds_to_the_line());
        assert!(a.accept());
        assert_eq!(a.line.text(), "git add ':(literal)sample*' ");
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
    fn a_file_a_specification_offers_leaves_the_list_unchanged() {
        // `ls` takes as many names as it is given. The argument the second
        // name would fill is therefore the argument the first one filled.
        // Nothing came out of the list and another press would put `one` on
        // the line a second time.
        let f = Fixture::new(&["one*", "two*"]);
        let mut a = App::over(f.path(), "ls ");
        assert!(a.accept());
        assert_eq!(a.line.text(), "ls one ");
        assert!(a.menu_repeats);
    }

    #[test]
    fn a_folder_row_opens_the_directory_it_named() {
        let f = Fixture::new(&["level/inner"]);
        let mut a = App::over(f.path(), "cd lev");
        assert!(a.accept());
        assert_eq!(a.line.text(), "cd level/");
        assert!(!a.menu_repeats);
    }

    #[test]
    fn a_subcommand_row_opens_what_it_named() {
        let f = Fixture::new(&[]);
        let mut a = App::over(f.path(), "docker contai");
        assert!(a.accept());
        assert_eq!(a.line.text(), "docker container ");
        assert!(!a.menu_repeats);
    }

    #[test]
    fn a_prefix_two_rows_share_leaves_the_menu_on_them() {
        // `wor` reaches both the names it came from. The list behind it is
        // therefore the same list. The word is still being typed and the menu
        // is still the answer to it.
        let f = Fixture::new(&["work", "worse"]);
        let mut a = App::over(f.path(), "cd wo");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "cd wor");
        assert!(!a.menu_repeats);
    }

    #[test]
    fn typing_the_line_on_leaves_the_menu_answering_for_it() {
        // The measurement belongs to an acceptance. A key that edits the line
        // opens whatever menu the new line asks for.
        let f = Fixture::new(&["one*", "two*"]);
        let mut a = App::over(f.path(), "ls ");
        assert!(a.accept());
        assert!(a.menu_repeats);
        a.line.insert("t");
        a.edited();
        assert!(!a.menu_repeats);
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
    fn tab_takes_the_prefix_the_rows_share() {
        let mut a = staged("cd wo", &["work/", "worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn tab_takes_a_menu_of_one_row_whole() {
        let mut a = staged("cd wo", &["work/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd work/");
    }

    #[test]
    fn tab_takes_a_menu_of_one_row_whole_inside_a_quote() {
        // The one row goes in ahead of the rule that leaves a quoted argument
        // alone. `quote_insert` has a whole name here and closes the quote
        // behind it. There is nothing left for that rule to protect.
        let mut a = staged("cd 'my", &["my docs/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd 'my docs/'");
    }

    #[test]
    fn tab_leaves_the_line_alone_when_the_rows_share_nothing_more() {
        let mut a = staged("cd wor", &["work/", "worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn tab_says_whether_it_took_anything() {
        let mut took = staged("cd wo", &["work/", "worse/"]);
        assert!(took.accept_common());
        // The rows agree on what the line already holds.
        let mut nothing = staged("cd wor", &["work/", "worse/"]);
        assert!(!nothing.accept_common());
    }

    #[test]
    fn the_reach_is_what_tab_would_take_measured_in_a_name() {
        // The rows share `wor` and each one draws its own name. Three
        // characters of that name are what Tab would leave on the line.
        let a = staged("cd wo", &["work/", "worse/"]);
        assert_eq!(a.reach(), 3);
    }

    #[test]
    fn one_row_reaches_the_whole_of_its_name() {
        let a = staged("cd wo", &["work/"]);
        assert_eq!(a.reach(), 5);
    }

    #[test]
    fn a_tab_that_would_leave_the_line_alone_reaches_nothing() {
        let a = staged("cd wor", &["work/", "worse/"]);
        assert_eq!(a.reach(), 0);
    }

    #[test]
    fn the_reach_counts_the_name_rather_than_the_argument() {
        let f = Fixture::new(&["work/alpha"]);
        let a = App::over(f.path(), "cd work/al");
        // The one row draws `alpha/`. The `work/` in front of it on the line
        // is the directory that row came from rather than part of its name.
        assert_eq!(a.reach(), 6);
    }

    #[test]
    fn tab_takes_the_directory_case_over_the_typed_case() {
        let mut a = staged("cd WO", &["work/", "worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn tab_leaves_a_prefix_the_shell_would_take_apart() {
        let mut a = staged("cd my", &["my docs/", "my drafts/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd my");
    }

    #[test]
    fn tab_takes_a_shared_prefix_under_a_tilde() {
        let mut a = staged("cd ~/wo", &["~/work/", "~/worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd ~/wor");
    }

    #[test]
    fn tab_leaves_a_prefix_under_a_tilde_that_the_shell_would_take_apart() {
        let mut a = staged("cd ~/my", &["~/my docs/", "~/my drafts/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd ~/my");
    }

    #[test]
    fn tab_leaves_a_quoted_argument_alone() {
        let mut a = staged("cd 'wo", &["work/", "worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd 'wo");
    }

    #[test]
    fn tab_ignores_a_row_that_does_not_lead_with_what_was_typed() {
        let mut a = staged("cd wk", &["work/", "weekly/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wk");
    }

    #[test]
    fn tab_answers_for_the_menu_while_the_row_that_runs_holds_the_highlight() {
        // `refresh` puts the highlight on the row that runs the line after
        // every keystroke. Tab looks past that row to the directory rows and
        // still has the one below to offer.
        let f = Fixture::new(&["work"]);
        let mut a = App::over(f.path(), "cd work");
        assert!(a.runs_the_line());
        a.accept_common();
        assert_eq!(a.line.text(), "cd work/");
    }

    #[test]
    fn tab_offers_a_shared_prefix_from_under_the_row_that_runs() {
        let f = Fixture::new(&["work/alpha", "work/alps"]);
        let mut a = App::over(f.path(), "cd work/");
        assert!(a.runs_the_line());
        a.accept_common();
        assert_eq!(a.line.text(), "cd work/alp");
    }

    #[test]
    fn tab_takes_the_row_that_agreed_rather_than_the_highlighted_one() {
        // `willow/` matches `wo` as a subsequence and does not lead with it.
        // The one row that does is `work/` and that is the one to take.
        let mut a = staged("cd wo", &["work/", "willow/"]);
        a.selected = 1;
        a.accept_common();
        assert_eq!(a.line.text(), "cd work/");
    }

    #[test]
    fn tab_leaves_two_real_names_that_fold_together_alone() {
        // `İ` folds to `i` here and macOS leaves it alone. Both of these are
        // therefore directories of their own on the volume the tests run on. A
        // pair differing only in case cannot stand in: the filesystem would
        // keep one of the two.
        let f = Fixture::new(&["i-work", "\u{130}-work"]);
        let mut a = App::over(f.path(), "cd i");
        assert_eq!(a.items.len(), 2, "the volume kept one of the two");
        a.accept_common();
        assert_eq!(a.line.text(), "cd i");
    }

    #[test]
    fn tab_leaves_two_names_that_fold_together_alone() {
        // The two share the whole of one of them. There is no prefix to take
        // and nothing has said which of the two the person meant.
        let mut a = staged("cd ", &["Work/", "work/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd ");
    }

    #[test]
    fn tab_folds_in_every_row_past_the_second() {
        // The third row is the restrictive one. A fold that stopped at the
        // second would take `wab` and one of the three does not carry it.
        let mut a = staged("cd wa", &["wabc/", "wabd/", "waxx/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wa");
    }

    #[test]
    fn tab_leaves_a_prefix_the_shell_would_read_as_the_home_directory() {
        // `quote_insert` leaves a bare `~` alone for the home row's sake. Half
        // a name is not that row and `cd ~` goes somewhere else entirely.
        let mut a = staged("cd ", &["~alpha/", "~beta/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd ");
    }

    #[test]
    fn tab_leaves_a_prefix_the_shell_would_read_as_a_stack_entry() {
        // zsh reads `cd +2` as the second entry of the directory stack.
        let mut a = staged("cd +", &["+2alpha/", "+2beta/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd +");
    }

    #[test]
    fn tab_closes_the_quote_behind_a_whole_name_beside_a_row_that_runs() {
        // The argument already names a directory and the row that runs the
        // line is therefore in the menu as well. The one directory row under
        // it is a whole name and `accept`'s quoting closes the quote behind
        // it.
        let f = Fixture::new(&["my docs/inner"]);
        let mut a = App::over(f.path(), "cd 'my docs/'");
        a.accept_common();
        assert_eq!(a.line.text(), "cd 'my docs/inner/'");
    }

    #[test]
    fn tab_leaves_the_highlight_where_it_was_when_it_changes_nothing() {
        let mut a = staged("cd wor", &["work/", "worse/"]);
        a.step(1);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wor");
        assert_eq!(a.selected, 1);
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
        a.accept_common();
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
        a.accept_common();
        assert_eq!(a.line.text(), "cd 'my docs/'");
    }

    #[test]
    fn tab_leaves_a_prefix_only_one_row_spells_that_way() {
        // The two agree on three folded characters and disagree on how to
        // spell them. `Wor` is a spelling `worse` does not carry and on a
        // case-sensitive volume it names nothing at all.
        let mut a = staged("cd wo", &["Work/", "worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wo");
    }

    #[test]
    fn tab_corrects_the_case_when_it_adds_no_characters() {
        // The rows share three characters and three is what was typed. The
        // spelling is still theirs to give and the count is not what says
        // whether there is anything left to add.
        let mut a = staged("cd WOR", &["work/", "worse/"]);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wor");
    }

    #[test]
    fn a_word_that_carries_on_past_the_cursor_is_left_whole() {
        // The rows answer for `wor` and the replacement would cover `wor`
        // alone. Writing `work/` in there used to strand the `k` and leave
        // `cd workk` behind. Every key that would grow the argument refuses.
        let f = Fixture::new(&["work", "workshop"]);
        for take in [
            (|a: &mut App| {
                a.accept_common();
            }) as fn(&mut App),
            |a: &mut App| {
                a.accept();
            },
        ] {
            let mut a = App::over(f.path(), "cd work");
            a.line.left();
            assert!(a.menu_open());
            assert_eq!(a.ghost(), "");
            assert!(!a.adds_to_the_line());
            take(&mut a);
            assert_eq!(a.line.text(), "cd work");
        }
    }

    #[test]
    fn a_cursor_left_of_the_whole_argument_takes_nothing_either() {
        // `willow/` matches `wo` as a subsequence. With the cursor back at the
        // start of the argument every row leads with the empty string and the
        // prefix they share used to go in on top of what was already there.
        let f = Fixture::new(&["work", "willow"]);
        let mut a = App::over(f.path(), "cd wo");
        a.line.left();
        a.line.left();
        assert_eq!(a.items.len(), 2);
        a.accept_common();
        assert_eq!(a.line.text(), "cd wo");
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

    /// An App with no directory behind it. The spec provider never touches
    /// the filesystem, so the same "no such directory" stand-in `staged`
    /// uses for its own menu is enough here too.
    fn spec_over(line: &str) -> App {
        App::over(&PathBuf::from("/no-such-directory-here"), line)
    }

    #[test]
    fn a_command_with_no_spec_gives_no_rows() {
        let a = spec_over("zzz-not-a-real-command-xyz sub");
        assert!(a.items.is_empty());
    }

    #[test]
    fn every_provider_still_resolves() {
        // Git and `cd` keep answering through their own code; the spec
        // provider only ever sees a line neither of them claimed. A real
        // directory is what the first two need to answer at all.
        let f = Fixture::new(&["child/"]);
        for line in [
            "git ",
            "cd ",
            "docker ",
            "npm ",
            "cargo ",
            "docker container ",
        ] {
            assert!(
                !App::over(f.path(), line).items.is_empty(),
                "{line:?} found nothing"
            );
        }
    }

    #[test]
    fn a_spec_orders_its_rows_alphabetically_once_nothing_narrows_them() {
        let a = spec_over("docker ");
        assert_eq!(a.items.len(), 58);
        assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Command));
        assert_eq!(a.items[0].insert, "attach");
        assert_eq!(
            a.items[0].label,
            "Attach local standard input, output, and error streams to a running container,"
        );
        assert_eq!(a.items[1].insert, "build");
        assert_eq!(a.items[1].label, "Build an image from a Dockerfile");
        assert_eq!(a.items[2].insert, "builder");
        assert_eq!(a.items[2].label, "Manage builds");
    }

    #[test]
    fn a_nested_subcommand_offers_its_own_children_and_nothing_above_them() {
        let a = spec_over("docker container ");
        assert_eq!(a.items.len(), 24);
        assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Command));
        // `attach` sits at the root too; the row here is `container`'s own
        // child rather than the one the walk left behind.
        assert_eq!(a.items[0].insert, "attach");
        assert!(a.items.iter().all(|c| c.insert != "container"));
    }

    #[test]
    fn a_subcommand_with_aliases_appears_once() {
        let a = spec_over("npm ");
        let installs: Vec<_> = a
            .items
            .iter()
            .filter(|c| ["install", "i", "add"].contains(&c.insert.as_str()))
            .collect();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].insert, "install");
    }

    #[test]
    fn an_option_already_on_the_line_does_not_come_back() {
        // `--verbose` takes no argument of its own, so the walk still offers
        // every other option for the empty word behind it. `--color` would
        // not: it has one, and a mandatory pending argument is what the walk
        // now forces the next word to fill instead.
        let a = spec_over("cargo --verbose ");
        assert!(a.items.iter().all(|c| c.insert != "--verbose"));
        assert!(
            a.items
                .iter()
                .any(|c| c.kind == candidates::Kind::Option && c.insert == "-h")
        );
    }

    #[test]
    fn a_mandatory_option_argument_offers_its_own_suggestions_instead() {
        let a = spec_over("cargo --color ");
        assert!(a.items.iter().all(|c| c.kind != candidates::Kind::Option));
        let names: Vec<&str> = a.items.iter().map(|c| c.insert.as_str()).collect();
        assert_eq!(names, ["always", "auto", "never"]);
        assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Path));
    }

    #[test]
    fn tab_takes_the_shared_prefix_a_spec_menu_agrees_on() {
        // `load`, `login`, `logout` and `logs` are every docker subcommand
        // that starts with `l`, and `lo` is as far as all four agree.
        let mut a = spec_over("docker l");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "docker lo");
    }

    #[test]
    fn tab_takes_a_spec_subcommand_whole_once_it_is_the_only_match() {
        let mut a = spec_over("docker contai");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "docker container ");
    }

    #[test]
    fn enter_takes_the_highlighted_spec_row() {
        let mut a = spec_over("docker ");
        assert_eq!(a.items[0].insert, "attach");
        assert!(a.accept());
        assert_eq!(a.line.text(), "docker attach ");
    }

    #[test]
    fn a_spec_row_cannot_replace_the_word_after_cursor_movement() {
        let mut a = spec_over("docker container ls");
        cursor_after(&mut a, "docker container".len());
        assert!(!a.accept());
        assert!(!a.accept_common());
        assert_eq!(a.line.text(), "docker container ls");
    }

    #[test]
    fn a_finished_git_subcommand_the_corpus_never_had_still_offers_nothing() {
        // `git sample ` is a finished subcommand with a trailing space, which
        // `crate::git::parse` does not read as a Git line at all. The walk
        // answers it now, where nothing did before, and comes back with
        // nothing to show: `sample` names no subcommand of `git`'s own
        // specification, so it is spent on `git`'s optional `alias` argument,
        // and a node that declares subcommands offers neither them nor its
        // own options behind such an argument. A person who accepted an
        // alias, or a command the installed Git has and the corpus never did,
        // is offered neither `add` to put after it nor `--bare`, and Git
        // would refuse both.
        let a = spec_over("git sample ");
        assert!(
            a.items.is_empty(),
            "{:?}",
            a.items
                .iter()
                .map(|c| c.insert.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_git_subcommand_argument_the_specification_carries_opens_a_menu() {
        // `crate::git::parse` reads no Git line here: `blame` is a finished
        // subcommand and the word behind it is neither a branch nor an `add`
        // path. `specs/git.json` says that word is a file and the walk is
        // what answers it now.
        let f = Fixture::new(&["assets", "readme*"]);
        let a = App::over(f.path(), "git blame ");
        let names: Vec<&str> = a.items.iter().map(|c| c.insert.as_str()).collect();
        assert!(names.contains(&"readme"), "{names:?}");
    }

    #[test]
    fn a_second_git_subcommand_with_a_file_argument_opens_its_menu_too() {
        // `clean` carries `{"name": "path", "template": "filepaths"}`, which
        // is `blame`'s own shape under another name, and neither declares a
        // subcommand, so the rule that closed `git`'s root options reads
        // neither of them. The two therefore answer alike for the same
        // reason, and this is what says so rather than the shapes matching.
        let f = Fixture::new(&["assets", "readme*"]);
        let a = App::over(f.path(), "git clean ");
        let names: Vec<&str> = a.items.iter().map(|c| c.insert.as_str()).collect();
        assert!(names.contains(&"readme"), "{names:?}");
    }

    #[test]
    fn a_git_argument_that_wants_a_folder_offers_no_file() {
        // `git clone <repository> [directory]`. The second argument is a
        // `folders` template and a file is no answer to it, the same as
        // `make -C`'s own.
        let f = Fixture::new(&["assets", "readme*"]);
        let a = App::over(f.path(), "git clone sample ");
        let names: Vec<&str> = a.items.iter().map(|c| c.insert.as_str()).collect();
        assert!(names.contains(&"assets/"), "{names:?}");
        assert!(!names.contains(&"readme"), "{names:?}");
    }

    #[test]
    fn a_git_subcommands_own_options_reach_the_menu_with_their_descriptions() {
        // A dash behind a finished subcommand is a line Git's own parser
        // declines outright. The walk reads it as the start of one of
        // `commit`'s own options and the word under the list is that
        // option's own sentence.
        let a = spec_over("git commit -");
        assert!(!a.items.is_empty());
        assert!(a.items.iter().all(|c| c.kind == candidates::Kind::Option));
        assert!(a.items.iter().any(|c| c.label != "option"));
    }

    #[test]
    fn a_git_subcommand_with_children_of_its_own_offers_them() {
        let a = spec_over("git stash ");
        let names: Vec<&str> = a.items.iter().map(|c| c.insert.as_str()).collect();
        for name in ["push", "pop", "list", "drop"] {
            assert!(names.contains(&name), "{names:?}");
        }
    }

    #[test]
    fn gits_own_readers_keep_every_line_they_already_claimed() {
        // The walk picks up only what `crate::git::parse` declined. A branch
        // after `switch` or `checkout` and a path after `add` are still its
        // own, and `specs/git.json` marks both of those arguments `dyn`
        // besides, so nothing would answer them here.
        let f = Fixture::new(&["sample-file*"]);
        f.init_git(&["sample-topic"]);
        for line in ["git switch ", "git checkout "] {
            let a = App::over(f.path(), line);
            assert!(
                a.items
                    .iter()
                    .any(|c| c.kind == candidates::Kind::Branch && c.insert == "sample-topic"),
                "{line}"
            );
        }
        let added = App::over(f.path(), "git add ");
        assert!(
            added
                .items
                .iter()
                .any(|c| c.kind == candidates::Kind::File && c.insert == "sample-file")
        );
    }

    #[test]
    fn an_accepted_git_subcommand_leaves_a_line_the_walk_answers() {
        // Where the two providers meet. Git's own menu names the subcommand
        // and the walk answers the word behind it, so the line an acceptance
        // leaves has a menu of files rather than none at all. `pick` ends the
        // run on that line and the next Tab is what opens the menu.
        let f = Fixture::new(&["readme*"]);
        let mut a = App::over(f.path(), "git blam");
        assert!(a.accept_common());
        assert_eq!(a.line.text(), "git blame ");
        assert!(a.items.iter().any(|c| c.insert == "readme"));
    }

    #[test]
    fn a_spec_row_on_a_git_line_goes_stale_when_the_cursor_leaves_its_word() {
        // One line, two providers at two cursor positions: the rows answer
        // for the file word and the line under the cursor now reads as Git's
        // own subcommand word. A row of one provider's is worth nothing
        // under the other and `spec_start` is what says so.
        let f = Fixture::new(&["readme*"]);
        let mut a = App::over(f.path(), "git blame read");
        assert!(a.items.iter().any(|c| c.insert == "readme"));
        cursor_after(&mut a, "git blame".len());
        assert!(!a.accept());
        assert!(!a.accept_common());
        assert_eq!(a.line.text(), "git blame read");
    }

    #[test]
    fn a_wrapped_git_re_roots_at_gits_own_specification() {
        let a = spec_over("sudo git ");
        assert!(a.items.iter().any(|c| c.insert == "switch"));
        assert!(a.items.iter().any(|c| c.insert == "--version"));
    }

    #[test]
    fn a_re_root_can_itself_point_at_a_third_specification() {
        // `aws`'s own `account` subcommand is itself a `loadSpec` pointer to
        // `aws/account`, so this line asks for three specifications before it
        // can answer at all: `sudo`'s own, then `aws`'s, then `aws/account`'s.
        // One retry past the first re-root would not be enough to see it.
        let a = spec_over("sudo aws account ");
        assert!(
            a.items
                .iter()
                .any(|c| c.insert == "get-primary-email" && c.kind == candidates::Kind::Command)
        );
    }

    #[test]
    fn a_wrapped_subcommand_behaves_as_it_would_unwrapped() {
        // `switch`'s own argument has no static suggestion beyond `-`, the
        // quick way back to the last branch; a real branch name needs the
        // generator this menu does not run. Its options still show, the same
        // as they would from a bare `git switch `.
        let a = spec_over("sudo git switch ");
        assert!(
            a.items
                .iter()
                .any(|c| c.insert == "-" && c.kind == candidates::Kind::Path)
        );
        assert!(
            a.items
                .iter()
                .any(|c| c.insert == "--discard-changes" && c.kind == candidates::Kind::Option)
        );
        assert!(a.items.iter().all(|c| c.insert != "switch"));
    }

    #[test]
    fn a_second_keystroke_keeps_answering_from_the_spec_the_first_re_rooted_to() {
        // Nothing here proves the second keystroke skipped a disk read on
        // its own; `spec_menu`'s own tests do that directly. This is the
        // behaviour that read buys: the re-root survives a keystroke that
        // does not touch the command name at all, the same way a `cd`
        // menu keeps answering from the directory its own first read found.
        let f = Fixture::new(&["child/"]);
        let mut a = App::over(f.path(), "sudo git ");
        let before = a.items.len();
        assert!(a.items.iter().any(|c| c.insert == "switch"));
        a.line.insert("s");
        a.edited();
        assert!(a.items.len() < before);
        assert!(a.items.iter().any(|c| c.insert == "switch"));
        assert!(a.items.iter().any(|c| c.insert == "status"));
        assert!(a.items.iter().all(|c| c.insert != "add"));
    }

    #[test]
    fn a_wrapped_command_with_no_specification_degrades_to_no_rows() {
        let a = spec_over("exec zzz-not-a-real-command-xyz ");
        assert!(a.items.is_empty());
    }
}
