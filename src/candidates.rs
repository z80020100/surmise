//! The `cd` completion engine.
//!
//! Two modes. Both stay inside the directory the argument names. A token that
//! looks like a path is completed against that path. A bare word is matched
//! against the children of the current directory. An argument that already
//! names a directory then gets one row in front of whichever mode ran.

use crate::fuzzy;
use crate::history::History;
use crate::path::expand;
use std::path::{Path, PathBuf};

/// How many directory entries one keystroke is allowed to look at. A directory
/// with more than this shows what came first rather than stalling the prompt.
const SCAN_LIMIT: usize = 400;
/// How many rows the menu will ever be asked to hold.
pub const MAX_RESULTS: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Dir,
    Special,
    /// The argument as it stands. This row grows nothing and runs the line
    /// instead.
    Run,
}

#[derive(Clone)]
pub struct Candidate {
    pub display: String,
    pub insert: String,
    /// What the row is, shown under the list. One word.
    pub label: &'static str,
    pub kind: Kind,
    pub score: i32,
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
fn resolved_in(arg: &str, cwd: &Path) -> PathBuf {
    let p = expand(arg);
    if p.is_absolute() { p } else { cwd.join(p) }
}

fn subdirs(dir: &Path, want_hidden: bool) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten().take(SCAN_LIMIT) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && !want_hidden {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // A symlink to a directory is still a directory to `cd`. Only a
        // symlink needs the second look. That look is a syscall of its own.
        if kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()) {
            out.push(name);
        }
    }
    out
}

pub(crate) fn folder(display: String, insert: String, score: i32) -> Candidate {
    Candidate {
        display,
        insert,
        label: "folder",
        kind: Kind::Dir,
        score,
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
        label: "run",
        kind: Kind::Run,
        score: 0,
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

fn path_mode(arg: &str, cwd: &Path) -> Vec<Candidate> {
    let (prefix, base) = split(arg);
    // A relative prefix hangs off the directory the caller named. Resolving it
    // against the process directory instead would answer for the wrong place.
    let dir = if prefix.is_empty() {
        cwd.to_path_buf()
    } else {
        resolved_in(prefix, cwd)
    };
    let mut out = Vec::new();
    for name in subdirs(&dir, base.starts_with('.')) {
        let Some(score) = fuzzy::score(base, &name) else {
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
fn match_rank(base: &str, display: &str) -> u8 {
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

fn predict(arg: &str, cwd: &Path) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    for name in subdirs(cwd, arg.starts_with('.')) {
        let Some(score) = fuzzy::score(arg, &name) else {
            continue;
        };
        out.push(folder(format!("{name}/"), format!("{name}/"), score + 40));
    }

    // The two places every shell can go from anywhere. They come last on an
    // empty argument and not at all once something is typed.
    if arg.is_empty() {
        out.push(Candidate {
            display: "..".into(),
            insert: "../".into(),
            label: "parent",
            kind: Kind::Special,
            score: 20,
        });
        out.push(Candidate {
            display: "~".into(),
            insert: "~".into(),
            label: "home",
            kind: Kind::Special,
            score: 15,
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

pub(crate) fn generate_in(arg: &str, cwd: &Path, history: &History) -> Vec<Candidate> {
    let looks_like_path = arg.contains('/') || arg.starts_with('~') || arg.starts_with('.');

    let out = if looks_like_path {
        path_mode(arg, cwd)
    } else {
        predict(arg, cwd)
    };

    // History orders names inside their match rank. Resolve each path once
    // rather than on each comparison. The snapshot stays fixed between keys.
    let mut weighted: Vec<_> = out
        .into_iter()
        .map(|c| {
            let weight = if c.kind == Kind::Dir {
                history.weight(&resolved_in(&c.insert, cwd))
            } else {
                0.0
            };
            (c, weight)
        })
        .collect();
    let (_, base) = split(arg);
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
    out.truncate(MAX_RESULTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    fn generate_in(arg: &str, cwd: &Path) -> Vec<Candidate> {
        super::generate_in(arg, cwd, &History::default())
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
        let mut got = subdirs(f.path(), false);
        got.sort();
        assert_eq!(got, ["alpha", "beta"]);
    }

    #[test]
    fn subdirs_hides_a_dot_directory_until_it_is_asked_for() {
        let f = Fixture::new(&["alpha", ".hidden"]);
        assert_eq!(subdirs(f.path(), false), ["alpha"]);
        let mut all = subdirs(f.path(), true);
        all.sort();
        assert_eq!(all, [".hidden", "alpha"]);
    }

    #[test]
    fn subdirs_says_nothing_about_a_directory_it_cannot_read() {
        assert!(subdirs(Path::new("/no-such-directory-here"), false).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_directory_counts_as_one() {
        let f = Fixture::new(&["alpha", "readme*"]);
        std::os::unix::fs::symlink(f.path().join("alpha"), f.path().join("link")).unwrap();
        std::os::unix::fs::symlink(f.path().join("readme"), f.path().join("dead")).unwrap();
        let mut got = subdirs(f.path(), false);
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
        assert_eq!(displays(&got), ["work/", "..", "~"]);
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
        assert_eq!(displays(&got), ["", "alpha/", "beta/"]);
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
        assert_eq!(displays(&got), [""]);
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
        assert_eq!(displays(&got), ["~/", "..", "~"]);
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
}
