//! The `cd` completion engine.
//!
//! Two modes. Both stay inside the directory the argument names. A token that
//! looks like a path is completed against that path. A bare word is matched
//! against the children of the current directory. An argument that already
//! names a directory then gets one row in front of whichever mode ran.

use crate::fuzzy;
use crate::histfile;
use crate::history::History;
use crate::path::expand;
use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How many directories one keystroke is allowed to come back with. A
/// directory holding more than this shows what came first rather than stalling
/// the prompt.
///
/// The count is of the directories the menu can use rather than of everything
/// the walk steps over. A limit on the entries would be spent on names no `cd`
/// can take and a directory full of files would then open an empty menu. The
/// walk itself is therefore unbounded. What that costs is one pass over a
/// directory holding hundreds of thousands of files and what it buys is a
/// menu that is not silently empty. `Scan` is what keeps that pass to one.
pub(crate) const SCAN_LIMIT: usize = 400;
/// How many rows the menu will ever be asked to hold.
pub const MAX_RESULTS: usize = 60;
/// What a row that adds a folder says it is. `ui` reads it to give such a row
/// the folder glyph where the kind alone says only that Git named the row.
pub(crate) const FOLDER: &str = "folder";
/// What a branch row that names the branch the repository is on says it is.
/// `ui` reads it for the glyph, because the kind alone says only that the row
/// is a branch.
pub(crate) const CURRENT_BRANCH: &str = "Current branch";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Command,
    Branch,
    File,
    Option,
    Path,
    Dir,
    Parent,
    Special,
    /// The argument as it stands. This row grows nothing and runs the line
    /// instead.
    Run,
}

impl Kind {
    pub fn is_git(self) -> bool {
        matches!(
            self,
            Kind::Command | Kind::Branch | Kind::File | Kind::Option | Kind::Path
        )
    }
}

#[derive(Clone)]
pub struct Candidate {
    pub display: String,
    pub insert: String,
    /// What the row is or what an option does, shown under the list.
    ///
    /// A `Cow` rather than `&'static str`, because surmise's own rows carry a
    /// constant and a spec's row will carry a description read at run time.
    pub label: Cow<'static, str>,
    /// The row's own arguments, each already formatted the way `ui` draws
    /// it dim after the name: one entry per argument, in order, empty for a
    /// row whose command takes none. `ui` shows as many whole entries as
    /// still fit and stops at the first that does not.
    pub hint: Vec<String>,
    pub kind: Kind,
    pub score: i32,
    /// What the row's own specification says it is worth, 0 through 100.
    /// [`DEFAULT_PRIORITY`] where the specification says nothing and on
    /// every row surmise names itself.
    ///
    /// `svn commit ` is what it is for. The corpus puts `-m` at 100 and
    /// alphabetical order buries the one flag that command cannot be run
    /// without under six that configure the connection.
    pub priority: u8,
}

pub struct Query {
    /// Byte offset in the line where the replaced token starts.
    pub start: usize,
    pub arg: String,
}

/// Recognise a `cd` invocation left of the cursor. Anything else predicts
/// nothing. That is the honest answer for this build.
pub fn parse(left: &str) -> Option<Query> {
    let trimmed = left.trim_start();
    let lead = left.len() - trimmed.len();
    let rest = trimmed.strip_prefix("cd")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let arg = rest.trim_start();
    // A quoted argument may hold spaces. An unquoted one may not. A second
    // argument is out of scope for this build.
    match arg.chars().next() {
        Some(q @ ('\'' | '"')) => {
            let body = &arg[q.len_utf8()..];
            if let Some(end) = body.find(q)
                && !body[end + q.len_utf8()..].trim().is_empty()
            {
                return None;
            }
        }
        _ if arg.split_whitespace().count() > 1 => return None,
        _ => {}
    }
    Some(Query {
        start: lead + 2 + (rest.len() - arg.len()),
        arg: arg.to_string(),
    })
}

/// The path `arg` reaches from `cwd`. A relative argument hangs off the
/// directory the caller named rather than off the process's own. Whether
/// anything is there is the caller's question.
///
/// `pub(crate)` rather than private: `spec_menu`'s own path templates resolve
/// an argument's own prefix against `cwd` the same way and reuse this rather
/// than carry a second copy of it.
pub(crate) fn resolved_in(arg: &str, cwd: &Path) -> PathBuf {
    let p = expand(arg);
    if p.is_absolute() { p } else { cwd.join(p) }
}

fn subdirs(dir: &Path, want_hidden: bool, git: bool) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // The limit sits behind the filter rather than in front of it. That is
    // what makes it a count of directories. The name is borrowed until a row
    // is going to hold it, because a directory full of files would otherwise
    // allocate one string for every name the walk throws away.
    rd.flatten()
        .filter_map(|entry| {
            let raw = entry.file_name();
            if git && raw.to_str().is_none() {
                return None;
            }
            let name = raw.to_string_lossy();
            if git && (name == ".git" || name.chars().any(char::is_control)) {
                return None;
            }
            if name.starts_with('.') && !want_hidden {
                return None;
            }
            let kind = entry.file_type().ok()?;
            // A symlink to a directory is still a directory to `cd`. Only a
            // symlink needs the second look. That look is a syscall of its own.
            (kind.is_dir() || (!git && kind.is_symlink() && entry.path().is_dir()))
                .then(|| name.into_owned())
        })
        .take(SCAN_LIMIT)
        .collect()
}

/// The listings one menu has already read.
///
/// `refresh` runs on every key and the names in a directory do not change
/// while you type into them. Walking it again for each key asks the kernel a
/// question this already holds the answer to. The history snapshot is read
/// once when the menu opens for that same reason and this is the trade for
/// the walk.
///
/// What it costs is a directory made while the menu is open. That name arrives
/// with the next menu rather than with the next key.
#[derive(Default)]
pub(crate) struct Scan {
    /// The hidden names are a listing of their own rather than a filter over
    /// one. A key that left the flag out would answer a `cd .` from a walk
    /// that never looked for them.
    walked: HashMap<(PathBuf, bool, bool), Vec<String>>,
    /// A second cache rather than a wider value in `walked`: `names` and
    /// `git_names` answer to callers that only ever want directories and
    /// changing what they hand back would reach into `cd` and `git.rs` for
    /// no gain here.
    listed: HashMap<(PathBuf, bool), Vec<(String, bool)>>,
}

impl Scan {
    fn names(&mut self, dir: &Path, want_hidden: bool) -> &[String] {
        self.names_for(dir, want_hidden, false)
    }

    pub(crate) fn git_names(&mut self, dir: &Path, want_hidden: bool) -> &[String] {
        self.names_for(dir, want_hidden, true)
    }

    fn names_for(&mut self, dir: &Path, want_hidden: bool, git: bool) -> &[String] {
        self.walked
            .entry((dir.to_path_buf(), want_hidden, git))
            .or_insert_with(|| subdirs(dir, want_hidden, git))
    }

    /// Every name in `dir`, files included, paired with whether each one is
    /// itself a directory. `spec_menu`'s `filepaths` and `folders` templates
    /// read this; `cd` has no use for a file and never calls it.
    pub(crate) fn entries(&mut self, dir: &Path, want_hidden: bool) -> &[(String, bool)] {
        self.listed
            .entry((dir.to_path_buf(), want_hidden))
            .or_insert_with(|| list_entries(dir, want_hidden))
    }
}

/// `subdirs`, without the filter that keeps a file from ever being one of
/// its results. The scan limit still applies, now to every entry rather
/// than to the directories alone: a file is a candidate here in a way it
/// never was for `cd`, so it is one more thing the limit is spent on.
fn list_entries(dir: &Path, want_hidden: bool) -> Vec<(String, bool)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|entry| {
            let raw = entry.file_name();
            let name = raw.to_string_lossy();
            if name.starts_with('.') && !want_hidden {
                return None;
            }
            let kind = entry.file_type().ok()?;
            let is_dir = kind.is_dir() || (kind.is_symlink() && entry.path().is_dir());
            Some((name.into_owned(), is_dir))
        })
        .take(SCAN_LIMIT)
        .collect()
}

/// What a row is worth where nothing says otherwise. The middle of the
/// range, which is what lets a specification push a row either way from
/// it, and the same default Q reads.
pub(crate) const DEFAULT_PRIORITY: u8 = 50;

/// A specification's own `priority` as [`rank`] reads it. The corpus is
/// JSON and carries whatever number was written into it. The range is
/// closed here rather than trusted. Q closes it the same way and exempts
/// only its auto-execute rows. No specification carries that kind of row
/// and this menu does not make one.
pub(crate) fn priority_of(raw: Option<i64>) -> u8 {
    raw.map_or(DEFAULT_PRIORITY, |p| p.clamp(0, 100) as u8)
}

pub(crate) fn folder(display: String, insert: String, score: i32) -> Candidate {
    Candidate {
        display,
        insert,
        label: Cow::Borrowed(FOLDER),
        hint: Vec::new(),
        kind: Kind::Dir,
        score,
        priority: DEFAULT_PRIORITY,
    }
}

/// The row that runs the line.
///
/// The glyph is the whole row. The line this row would run is on the screen
/// already, one row above the menu. A name here would be that same text a
/// second time and `label` is what says what the row does instead. `insert`
/// is the argument as it stands and nothing therefore offers to add to it.
pub(crate) fn run_row(insert: String) -> Candidate {
    Candidate {
        display: String::new(),
        insert,
        label: Cow::Borrowed("run"),
        hint: Vec::new(),
        kind: Kind::Run,
        score: 0,
        priority: DEFAULT_PRIORITY,
    }
}

/// The directory an argument names and what was typed into it. Everything up
/// to the last `/` is the first and the rest is the second. The menu marks
/// the characters the second reached and the two modes below both match on
/// it.
pub fn split(arg: &str) -> (&str, &str) {
    match arg.rfind('/') {
        Some(i) => (&arg[..=i], &arg[i + 1..]),
        // A bare `~` is a whole directory rather than the start of a name in
        // this one. Nothing in the current directory is what it means.
        None if arg == "~" => ("~/", ""),
        None => ("", arg),
    }
}

fn path_mode(arg: &str, cwd: &Path, scan: &mut Scan) -> Vec<Candidate> {
    let (prefix, base) = split(arg);
    // A relative prefix hangs off the directory the caller named. Resolving it
    // against the process directory instead would answer for the wrong place.
    let dir = if prefix.is_empty() {
        cwd.to_path_buf()
    } else {
        resolved_in(prefix, cwd)
    };
    let mut out = Vec::new();
    for name in scan.names(&dir, base.starts_with('.')) {
        let Some(score) = fuzzy::score(base, name) else {
            continue;
        };
        out.push(folder(
            format!("{name}/"),
            format!("{prefix}{name}/"),
            score,
        ));
    }
    out
}

/// How well a name answers what was typed. A name that is exactly what was
/// typed outranks one that merely starts with it and both outrank one that
/// holds those characters scattered through it. A name leading with what was
/// typed is almost always the one meant and the sort therefore reads this
/// before it reads the score. A bonus added to the score could be outweighed
/// by a longer name scoring well elsewhere and a rank cannot.
///
/// Both checks fold the case the same way the score itself is measured. Each
/// being a prefix of the other is what says the two are the same name.
pub(crate) fn match_rank(base: &str, display: &str) -> u8 {
    let name = display.strip_suffix('/').unwrap_or(display);
    // Nothing typed reached nothing. Every name would otherwise hold the
    // empty string at its front and rank alike. The sort reads the same
    // either way and what this guard keeps right is the answer rather than
    // the order.
    if base.is_empty() || !fuzzy::starts_with_folded(name, base) {
        return 0;
    }
    if fuzzy::starts_with_folded(base, name) {
        2
    } else {
        1
    }
}

/// The text a row would have been typed as, which is what the reading of
/// `$HISTFILE` counted. A row's own `insert` rather than its `display`:
/// the two are the same word for a subcommand and an option, and a path
/// row shows a leaf where it inserts the whole of what was typed, so
/// `cat sample/notes.txt` in a history reaches the row `notes.txt` under
/// `sample/`. The trailing slash a folder row wears comes off, because it
/// is the menu saying the row is a folder rather than anything a person
/// typed. `match_rank` takes it off for the same reason.
fn typed_as(c: &Candidate) -> &str {
    c.insert.strip_suffix('/').unwrap_or(&c.insert)
}

/// What a row's own name is looked up under: the command the row would
/// become the second word of, and the reading of `$HISTFILE` that says how
/// often it already has been. One value rather than two parameters, because
/// neither half means anything on its own and [`rank`] already takes a
/// `&str` of its own for what was typed.
#[derive(Clone, Copy)]
pub(crate) struct UsedAfter<'a> {
    pub(crate) command: &'a str,
    pub(crate) counts: &'a histfile::Counts,
}

/// Which of a specification's three groups a row came from. A subcommand
/// leads, a value follows and an option comes last. A subcommand is the
/// next word the command is made of and a value is the word its argument
/// wants. An option is neither.
///
/// Nothing else in the sort says it. An empty term scores every row the
/// same and the name alone then decided. `-` sorts under every letter and
/// a bare `cargo ` therefore led with its 12 options, leaving all 38
/// subcommands under the fold. 323 of the 715 specifications at the top of
/// `specs/` carry both a subcommand and an option at their root.
///
/// The sort reads this after the score rather than before. How well what
/// was typed reaches a name says more than the group that name came from.
/// Typing a `-` is how a person asks for the options.
fn group_rank(kind: Kind) -> u8 {
    match kind {
        Kind::Command => 2,
        Kind::Option => 0,
        // A branch, a file, a folder and a value a spec lists are one
        // group: what the argument in hand takes. `cd`'s own menu ranks
        // its rows elsewhere and the three kinds only it makes never
        // reach this sort.
        Kind::Branch
        | Kind::File
        | Kind::Path
        | Kind::Dir
        | Kind::Parent
        | Kind::Special
        | Kind::Run => 1,
    }
}

/// The ranking `crate::git`'s own subcommand, branch and file rows share
/// with `crate::spec_menu`'s: a name that folds to exactly what was typed
/// leads, a name that merely starts with it follows, a tie inside either
/// goes to the row the shell history favours, a further tie keeps the fuzzy
/// score's own order, the specification's own `priority` breaks that, the
/// group the row came from breaks what is still level and a name breaks
/// whatever is left. One `rank` rather than one copy each is what keeps
/// the two menus from drifting apart.
///
/// `priority` sits above the group and under the score for the reason Q
/// puts it there. A number a specification wrote down says more than the
/// group a row happens to belong to and less than how well what was typed
/// reaches its name.
///
/// `used_after` is the history term, and `None` is what turns it off.
/// [`crate::histfile::Counts`] holds how often a command's *second* word
/// followed its first and nothing deeper, so only a row that would become
/// that second word may be ranked by it: a Git subcommand, or the one word
/// a spec menu offers straight after the command name. A branch name, a
/// file name, an option's value and every row further along a spec's own
/// walk sit at the third word or later, where the counts describe nothing.
/// Reading them there would not merely waste a lookup. A branch named for a
/// subcommand somebody types often would climb a list it has no business
/// leading. The caller passes the command name to look the row up under,
/// which is `"git"` for Git's own menu and the line's own first word for
/// the other.
///
/// `sort_by_cached_key` calls its key function once per row rather than
/// once per comparison a plain `sort_by` would, so the history is read once
/// for each row instead of on every one of a sort's `O(n log n)`
/// comparisons; `specs/aws/ec2.json`'s 448 subcommands are this corpus's
/// worst case for it.
pub(crate) fn rank(rows: &mut Vec<Candidate>, term: &str, used_after: Option<UsedAfter>) {
    let tier = |name: &str| {
        if !term.is_empty()
            && fuzzy::starts_with_folded(name, term)
            && fuzzy::starts_with_folded(term, name)
        {
            2
        } else {
            match_rank(term, name)
        }
    };
    rows.sort_by_cached_key(|c| {
        (
            Reverse(tier(&c.display)),
            Reverse(used_after.map_or(0, |h| h.counts.count(h.command, typed_as(c)))),
            Reverse(c.score),
            Reverse(c.priority),
            Reverse(group_rank(c.kind)),
            c.display.clone(),
        )
    });
    rows.truncate(MAX_RESULTS);
}

fn predict(arg: &str, cwd: &Path, scan: &mut Scan) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    for name in scan.names(cwd, arg.starts_with('.')) {
        let Some(score) = fuzzy::score(arg, name) else {
            continue;
        };
        out.push(folder(format!("{name}/"), format!("{name}/"), score + 40));
    }

    // Home comes last on an empty argument and not at all once something
    // is typed.
    if arg.is_empty() {
        out.push(Candidate {
            display: "~".into(),
            insert: "~".into(),
            label: Cow::Borrowed("home"),
            hint: Vec::new(),
            kind: Kind::Special,
            score: 15,
            priority: DEFAULT_PRIORITY,
        });
    }
    out
}

/// Does the shell read `arg` as a place in its directory stack rather than as
/// a path? Quoting the word does not change that, because `cd` reads its own
/// argument after the shell has taken the quotes off. `cd '+2'` is therefore
/// still the second entry whatever `./+2` holds.
fn dir_stack_spec(arg: &str) -> bool {
    match arg.strip_prefix(['+', '-']) {
        Some(rest) => rest.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

pub(crate) fn generate_in(
    arg: &str,
    cwd: &Path,
    history: &History,
    scan: &mut Scan,
) -> Vec<Candidate> {
    let looks_like_path = arg.contains('/') || arg.starts_with('~') || arg.starts_with('.');

    let mut out = if looks_like_path {
        path_mode(arg, cwd, scan)
    } else {
        predict(arg, cwd, scan)
    };

    // History orders names inside their match rank. Resolve each path once
    // rather than on each comparison. The snapshot stays fixed between keys.
    // Only the directory prefix expands. A child named `~` stays literal.
    let (prefix, base) = split(arg);
    let dir = resolved_in(prefix, cwd);
    if let Some(score) = fuzzy::score(base, "..")
        && dir.is_dir()
    {
        out.push(Candidate {
            display: "../".into(),
            insert: format!("{prefix}../"),
            label: Cow::Borrowed("parent"),
            hint: Vec::new(),
            kind: Kind::Parent,
            // Nothing typed puts this row behind the children and ahead of
            // home. `predict` scores a child 40 and home 15. `path_mode`
            // scores a child 0 and offers no home row to sit above.
            score: if base.is_empty() {
                if looks_like_path { -1 } else { 20 }
            } else {
                score
            },
            priority: DEFAULT_PRIORITY,
        });
    }
    let mut weighted: Vec<_> = out
        .into_iter()
        .map(|c| {
            let weight = if c.kind == Kind::Dir {
                history.weight(&dir.join(&c.display))
            } else {
                0.0
            };
            (c, weight)
        })
        .collect();
    // Exact names precede prefixes. Prefixes precede other matches. History
    // ranks the names inside each of those. Equal weights use the score and
    // then the name. That last key is what holds the menu still between
    // keystrokes. `read_dir` answers in no order of its own.
    weighted.sort_by(|(a, aw), (b, bw)| {
        match_rank(base, &b.display)
            .cmp(&match_rank(base, &a.display))
            .then_with(|| bw.total_cmp(aw))
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.display.cmp(&b.display))
    });
    let mut out: Vec<_> = weighted.into_iter().map(|(c, _)| c).collect();

    // The row that runs the line goes in front of that order rather than into
    // it. A line that already names a directory is the one most often meant
    // and a score would leave that to chance. A bare `cd` goes home and the
    // shell needs no row to say so.
    if !arg.is_empty() && !dir_stack_spec(arg) && resolved_in(arg, cwd).is_dir() {
        out.insert(0, run_row(arg.to_string()));
    }
    // Keep a way up even when the children fill the menu.
    if let Some(parent) = out.iter().position(|c| c.kind == Kind::Parent)
        && parent >= MAX_RESULTS
    {
        out.swap(MAX_RESULTS - 1, parent);
    }
    out.truncate(MAX_RESULTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    fn generate_in(arg: &str, cwd: &Path) -> Vec<Candidate> {
        super::generate_in(arg, cwd, &History::default(), &mut Scan::default())
    }

    #[test]
    fn an_argument_splits_at_the_last_slash() {
        assert_eq!(split("wo"), ("", "wo"));
        assert_eq!(split("work/al"), ("work/", "al"));
        assert_eq!(split("work/"), ("work/", ""));
        // A bare tilde is a whole directory rather than a name in this one.
        // Nothing was typed into it.
        assert_eq!(split("~"), ("~/", ""));
        assert_eq!(split("~/pro"), ("~/", "pro"));
    }

    fn displays(items: &[Candidate]) -> Vec<&str> {
        items.iter().map(|c| c.display.as_str()).collect()
    }

    #[test]
    fn parse_takes_a_cd_invocation() {
        let q = parse("cd wor").unwrap();
        assert_eq!(q.arg, "wor");
        assert_eq!(q.start, 3);
    }

    #[test]
    fn parse_points_start_at_the_argument() {
        for left in ["cd wor", "   cd wor", "cd    wor"] {
            let q = parse(left).unwrap();
            assert_eq!(&left[q.start..], "wor", "{left:?}");
        }
    }

    #[test]
    fn parse_takes_an_empty_argument() {
        let q = parse("cd ").unwrap();
        assert_eq!(q.arg, "");
        assert_eq!(q.start, 3);
    }

    #[test]
    fn parse_wants_the_whole_word_cd() {
        assert!(parse("cdx wor").is_none());
        assert!(parse("cd").is_none());
        assert!(parse("").is_none());
        assert!(parse("ls wor").is_none());
    }

    #[test]
    fn parse_ignores_a_cd_that_is_not_first() {
        assert!(parse("echo x; cd wor").is_none());
        assert!(parse("ls && cd wor").is_none());
    }

    #[test]
    fn parse_takes_a_tab_as_the_separator() {
        let q = parse("cd\twor").unwrap();
        assert_eq!(q.arg, "wor");
        assert_eq!(q.start, 3);
    }

    #[test]
    fn parse_skips_every_space_before_the_argument() {
        let q = parse("cd   ").unwrap();
        assert_eq!(q.arg, "");
        assert_eq!(q.start, 5);
    }

    #[test]
    fn parse_refuses_a_second_argument() {
        assert!(parse("cd a b").is_none());
        assert!(parse("cd a  b").is_none());
    }

    #[test]
    fn parse_keeps_a_quoted_argument_with_a_space() {
        assert_eq!(parse("cd 'my docs").unwrap().arg, "'my docs");
        assert_eq!(parse("cd 'my docs'").unwrap().arg, "'my docs'");
        assert_eq!(parse("cd \"my docs\"").unwrap().arg, "\"my docs\"");
    }

    #[test]
    fn parse_refuses_a_word_after_a_closed_quote() {
        assert!(parse("cd 'my docs' b").is_none());
    }

    #[test]
    fn subdirs_lists_directories_and_nothing_else() {
        let f = Fixture::new(&["alpha", "beta", "readme*"]);
        let mut got = subdirs(f.path(), false, false);
        got.sort();
        assert_eq!(got, ["alpha", "beta"]);
    }

    #[test]
    fn subdirs_hides_a_dot_directory_until_it_is_asked_for() {
        let f = Fixture::new(&["alpha", ".hidden"]);
        assert_eq!(subdirs(f.path(), false, false), ["alpha"]);
        let mut all = subdirs(f.path(), true, false);
        all.sort();
        assert_eq!(all, [".hidden", "alpha"]);
    }

    #[test]
    fn subdirs_counts_directories_rather_than_entries() {
        // The limit is what one keystroke comes back with rather than what it
        // steps over. Spending it on files is what left a directory holding
        // thousands of them with no menu at all.
        let mut entries: Vec<String> = (0..SCAN_LIMIT).map(|i| format!("file-{i}*")).collect();
        entries.extend((0..SCAN_LIMIT).map(|i| format!("dir-{i}")));
        let names: Vec<&str> = entries.iter().map(String::as_str).collect();
        let f = Fixture::new(&names);
        assert_eq!(subdirs(f.path(), false, false).len(), SCAN_LIMIT);
    }

    #[test]
    fn list_entries_names_a_file_as_one_and_a_directory_as_the_other() {
        let f = Fixture::new(&["alpha", "beta*"]);
        let mut got = list_entries(f.path(), false);
        got.sort();
        assert_eq!(
            got,
            [("alpha".to_string(), true), ("beta".to_string(), false)]
        );
    }

    #[test]
    fn list_entries_counts_every_entry_towards_the_limit() {
        // Unlike `subdirs`, a file is a candidate here rather than something
        // the walk throws away, so the limit is spent on the whole listing.
        let mut entries: Vec<String> = (0..SCAN_LIMIT / 2).map(|i| format!("file-{i}*")).collect();
        entries.extend((0..SCAN_LIMIT / 2).map(|i| format!("dir-{i}")));
        entries.push("one-more*".to_string());
        let names: Vec<&str> = entries.iter().map(String::as_str).collect();
        let f = Fixture::new(&names);
        assert_eq!(list_entries(f.path(), false).len(), SCAN_LIMIT);
    }

    #[test]
    fn a_scan_walks_a_directory_once_and_keeps_what_it_found() {
        let f = Fixture::new(&["alpha"]);
        let mut scan = Scan::default();
        assert_eq!(scan.names(f.path(), false), ["alpha"]);
        std::fs::create_dir(f.path().join("beta")).unwrap();
        assert_eq!(scan.names(f.path(), false), ["alpha"]);
    }

    #[test]
    fn a_scan_holds_the_hidden_names_apart_from_the_plain_ones() {
        let f = Fixture::new(&["alpha", ".hidden"]);
        let mut scan = Scan::default();
        assert_eq!(scan.names(f.path(), false), ["alpha"]);
        let mut all = scan.names(f.path(), true).to_vec();
        all.sort();
        assert_eq!(all, [".hidden", "alpha"]);
    }

    #[test]
    fn a_scans_entries_are_also_walked_once_and_kept() {
        let f = Fixture::new(&["alpha", "beta*"]);
        let mut scan = Scan::default();
        let mut first = scan.entries(f.path(), false).to_vec();
        first.sort();
        assert_eq!(
            first,
            [("alpha".to_string(), true), ("beta".to_string(), false)]
        );
        std::fs::write(f.path().join("gamma"), b"").unwrap();
        let mut second = scan.entries(f.path(), false).to_vec();
        second.sort();
        assert_eq!(second, first);
    }

    #[test]
    fn subdirs_says_nothing_about_a_directory_it_cannot_read() {
        assert!(subdirs(Path::new("/no-such-directory-here"), false, false).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_directory_counts_as_one() {
        let f = Fixture::new(&["alpha", "readme*"]);
        std::os::unix::fs::symlink(f.path().join("alpha"), f.path().join("link")).unwrap();
        std::os::unix::fs::symlink(f.path().join("readme"), f.path().join("dead")).unwrap();
        let mut got = subdirs(f.path(), false, false);
        got.sort();
        assert_eq!(got, ["alpha", "link"]);
    }

    #[test]
    fn predict_offers_the_children_of_the_current_directory() {
        let f = Fixture::new(&["work", "worse", "other"]);
        let got = generate_in("wor", f.path());
        assert_eq!(displays(&got), ["work/", "worse/"]);
    }

    #[test]
    fn predict_puts_a_prefix_match_first() {
        let f = Fixture::new(&["awork", "work"]);
        let got = generate_in("wor", f.path());
        assert_eq!(displays(&got)[0], "work/");
    }

    #[test]
    fn bare_and_path_arguments_rank_exact_then_prefix_then_fuzzy_matches() {
        // The padding drives the long name's score under `a_wo`'s and rank is
        // then the only thing holding it above the scattered matches. The two
        // path prefixes are what prove that: the score bonus this replaced
        // never ran in path mode and 255 is too short a name for the bare
        // argument to tell the two apart.
        let long = format!("wo{}", "x".repeat(240));
        let f = Fixture::new(&["wo", &long, "a_wo", "z_w_x_o"]);
        let absolute = format!("{}/", f.path().display());
        let long_display = format!("{long}/");
        for prefix in ["", "./", absolute.as_str()] {
            for base in ["wo", "WO"] {
                let got = generate_in(&format!("{prefix}{base}"), f.path());
                let dirs: Vec<_> = got.iter().filter(|c| c.kind == Kind::Dir).collect();
                let names: Vec<_> = dirs.iter().map(|c| c.display.as_str()).collect();
                assert_eq!(names, ["wo/", &long_display, "a_wo/", "z_w_x_o/"]);
            }
        }
    }

    #[test]
    fn a_priority_outside_the_range_is_closed_to_it() {
        // Every one of the 4504 values the corpus carries already sits
        // between 0 and 100. A `spec_dirs` file is hand written and a
        // regenerated corpus is whatever the upstream wrote. The range is
        // closed here rather than trusted.
        assert_eq!(priority_of(None), DEFAULT_PRIORITY);
        assert_eq!(priority_of(Some(0)), 0);
        assert_eq!(priority_of(Some(100)), 100);
        assert_eq!(priority_of(Some(-1)), 0);
        assert_eq!(priority_of(Some(1_000)), 100);
    }

    #[test]
    fn an_exact_name_outranks_a_longer_one_that_ties_on_score() {
        // `work` and `Worka` both score 112 against `Work` and the name key
        // would then put `Worka/` first. `W` sorts below `w`. The exact rank
        // is the only thing holding the name that was typed above the one
        // that merely starts with it. A case-insensitive filesystem also
        // gives `Work` a row that runs the line and the filter is what leaves
        // that row out.
        let f = Fixture::new(&["work", "Worka"]);
        let got = generate_in("Work", f.path());
        let dirs: Vec<&str> = got
            .iter()
            .filter(|c| c.kind == Kind::Dir)
            .map(|c| c.display.as_str())
            .collect();
        assert_eq!(dirs, ["work/", "Worka/"]);
    }

    #[test]
    fn an_empty_argument_offers_the_parent_and_home_last() {
        let f = Fixture::new(&["work"]);
        let got = generate_in("", f.path());
        assert_eq!(displays(&got), ["work/", "../", "~"]);
    }

    #[test]
    fn the_parent_and_home_go_away_once_something_is_typed() {
        let f = Fixture::new(&["work"]);
        let got = generate_in("w", f.path());
        assert_eq!(displays(&got), ["work/"]);
    }

    #[test]
    fn a_path_argument_looks_inside_that_path() {
        let f = Fixture::new(&["work/alpha", "work/beta", "other/gamma"]);
        let got = generate_in("work/", f.path());
        // The run row comes first and carries no name of its own.
        assert_eq!(displays(&got), ["", "alpha/", "beta/", "../"]);
        assert_eq!(got[1].insert, "work/alpha/");
    }

    #[test]
    fn a_path_argument_takes_no_specials() {
        let f = Fixture::new(&["work/alpha"]);
        assert!(
            generate_in("work/", f.path())
                .iter()
                .all(|c| c.kind != Kind::Special)
        );
    }

    #[test]
    fn an_argument_that_already_names_a_directory_gets_a_row_to_run_it() {
        let f = Fixture::new(&["work", "workshop"]);
        let got = generate_in("work", f.path());
        assert_eq!(displays(&got), ["", "work/", "workshop/"]);
        assert_eq!(got[0].kind, Kind::Run);
        assert_eq!(got[0].label, "run");
        assert_eq!(got[0].insert, "work");
    }

    #[test]
    fn a_directory_holding_nothing_still_offers_the_row_that_runs() {
        let f = Fixture::new(&["work"]);
        let got = generate_in("work/", f.path());
        assert_eq!(displays(&got), ["", "../"]);
        assert_eq!(got[0].kind, Kind::Run);
    }

    #[test]
    fn a_directory_stack_entry_offers_no_row_to_run() {
        // `cd +2` is the second entry of the stack whatever `./+2` holds and
        // quoting the word does not change that. The directory row below is
        // the way in, because its trailing slash is not a stack entry.
        let f = Fixture::new(&["+2"]);
        let got = generate_in("+2", f.path());
        assert!(got.iter().all(|c| c.kind != Kind::Run));
        assert_eq!(displays(&got), ["+2/"]);
        assert!(dir_stack_spec("+2"));
        assert!(dir_stack_spec("-12"));
        assert!(dir_stack_spec("-"));
        assert!(!dir_stack_spec("-x"));
        assert!(!dir_stack_spec("+2a"));
        assert!(!dir_stack_spec("work"));
    }

    #[test]
    fn a_bare_tilde_lists_what_a_tilde_slash_lists() {
        // `path_mode` splits on the last slash and a bare `~` leaves none. The
        // current directory used to answer for it. Scoring that directory's
        // children against a literal `~` answered for the wrong place. `~/`
        // was always right and the two now agree.
        let f = Fixture::new(&["tilde-marker"]);
        let dirs = |items: &[Candidate]| -> Vec<String> {
            items
                .iter()
                .filter(|c| c.kind == Kind::Dir)
                .map(|c| c.insert.clone())
                .collect()
        };
        let bare = generate_in("~", f.path());
        assert_eq!(dirs(&bare), dirs(&generate_in("~/", f.path())));
        assert!(
            !displays(&bare).contains(&"tilde-marker/"),
            "the current directory got in: {:?}",
            displays(&bare)
        );
    }

    #[test]
    fn an_empty_argument_offers_no_row_to_run() {
        // A bare `cd` goes home and the shell needs no help to say so.
        let f = Fixture::new(&["work"]);
        let got = generate_in("", f.path());
        assert!(got.iter().all(|c| c.kind != Kind::Run));
    }

    #[test]
    fn a_file_of_that_name_offers_no_row_to_run() {
        let f = Fixture::new(&["alpha", "al*"]);
        assert_eq!(displays(&generate_in("al", f.path())), ["alpha/"]);
    }

    #[test]
    fn a_dot_argument_reaches_the_hidden_directories() {
        let f = Fixture::new(&[".config", ".cache", "work"]);
        let items = generate_in(".c", f.path());
        let mut got = displays(&items);
        got.sort();
        assert_eq!(got, [".cache/", ".config/"]);
    }

    #[test]
    fn a_child_named_like_the_home_shortcut_still_shows() {
        // Deduplicating by resolved path used to hide this one behind the `~`
        // shortcut. Both resolve to the same place. A real child also outranks
        // a shortcut and therefore comes first.
        let f = Fixture::new(&["~"]);
        let got = generate_in("", f.path());
        assert_eq!(displays(&got), ["~/", "../", "~"]);
    }

    #[test]
    fn nothing_matching_gives_nothing() {
        let f = Fixture::new(&["work"]);
        assert!(generate_in("zzz", f.path()).is_empty());
    }

    #[test]
    fn the_list_never_outgrows_the_menu() {
        let names: Vec<String> = (0..MAX_RESULTS + 5).map(|i| format!("dir{i:03}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let f = Fixture::new(&refs);
        assert_eq!(generate_in("dir", f.path()).len(), MAX_RESULTS);
    }

    #[test]
    fn a_full_menu_keeps_its_parent_row() {
        let names: Vec<String> = (0..MAX_RESULTS + 5).map(|i| format!("dir{i:03}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let f = Fixture::new(&refs);
        for arg in ["", "./"] {
            let got = generate_in(arg, f.path());
            assert_eq!(got.len(), MAX_RESULTS);
            assert_eq!(got.last().unwrap().kind, Kind::Parent);
            assert_eq!(got.last().unwrap().insert, format!("{arg}../"));
        }
    }

    #[test]
    fn a_missing_directory_offers_no_parent_row() {
        let f = Fixture::new(&[]);
        assert!(generate_in("missing/", f.path()).is_empty());
        assert!(generate_in("missing/..", f.path()).is_empty());
    }
}
