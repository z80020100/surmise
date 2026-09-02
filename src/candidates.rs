//! The `cd` completion engine.
//!
//! Two modes. Both stay inside the directory the argument names. A token
//! that looks like a path is completed against that path. A bare word is
//! matched against the children of the current directory.

use crate::fuzzy;
use crate::path::expand;
use std::path::Path;

/// How many directory entries one keystroke is allowed to look at. A directory
/// with more than this shows what came first rather than stalling the prompt.
const SCAN_LIMIT: usize = 400;
/// How many rows the menu will ever be asked to hold.
pub const MAX_RESULTS: usize = 60;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Dir,
    Special,
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

fn path_mode(arg: &str, cwd: &Path) -> Vec<Candidate> {
    let (prefix, base) = match arg.rfind('/') {
        Some(i) => (&arg[..=i], &arg[i + 1..]),
        // A bare `~` is a whole directory rather than the start of a name in
        // this one. Nothing in the current directory is what it means.
        None if arg == "~" => ("~/", ""),
        None => ("", arg),
    };
    // A relative prefix hangs off the directory the caller named. Resolving it
    // against the process directory instead would answer for the wrong place.
    let dir = if prefix.is_empty() {
        cwd.to_path_buf()
    } else {
        let p = expand(prefix);
        if p.is_absolute() { p } else { cwd.join(p) }
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

/// A name that starts with what was typed is almost always the one meant. It
/// therefore outranks a match that merely holds those characters in order.
fn prefix_bonus(arg: &str, name: &str) -> i32 {
    if arg.is_empty() {
        return 0;
    }
    // The same per-character folding the score itself is measured with.
    if fuzzy::starts_with_folded(name, arg) {
        60
    } else {
        0
    }
}

fn predict(arg: &str, cwd: &Path) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    for name in subdirs(cwd, arg.starts_with('.')) {
        let Some(base) = fuzzy::score(arg, &name) else {
            continue;
        };
        let score = base + 40 + prefix_bonus(arg, &name);
        out.push(folder(format!("{name}/"), format!("{name}/"), score));
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

pub(crate) fn generate_in(arg: &str, cwd: &Path) -> Vec<Candidate> {
    let looks_like_path = arg.contains('/') || arg.starts_with('~') || arg.starts_with('.');

    let mut out = if looks_like_path {
        path_mode(arg, cwd)
    } else {
        predict(arg, cwd)
    };

    // Highest score first, then alphabetical so equal scores hold still
    // between keystrokes.
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.display.cmp(&b.display))
    });
    out.truncate(MAX_RESULTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

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
    fn a_prefix_outranks_a_scattered_match() {
        assert_eq!(prefix_bonus("wo", "work"), 60);
        assert_eq!(prefix_bonus("WO", "work"), 60);
        assert_eq!(prefix_bonus("wo", "awork"), 0);
        assert_eq!(prefix_bonus("", "work"), 0);
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
        assert_eq!(displays(&got), ["alpha/", "beta/"]);
        assert_eq!(got[0].insert, "work/alpha/");
    }

    #[test]
    fn a_path_argument_takes_no_specials() {
        let f = Fixture::new(&["work/alpha"]);
        let got = generate_in("work/", f.path());
        assert!(got.iter().all(|c| c.kind == Kind::Dir));
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
