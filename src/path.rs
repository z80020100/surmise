//! Paths and the `~` a person types instead of one.
//!
//! Every function that needs `$HOME` has a private twin that takes it as an
//! argument. The environment is therefore read at one edge and the rest is
//! pure enough to test directly.

use std::path::{Path, PathBuf};

/// Drop a trailing separator from `$HOME`. A value made only of separators
/// keeps them, because `$HOME` set to `/` is still a home rather than an
/// absent one.
fn trim_home(raw: &str) -> &str {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() { raw } else { trimmed }
}

/// The home directory, or an empty path when `$HOME` says nothing.
fn home() -> PathBuf {
    PathBuf::from(trim_home(&std::env::var("HOME").unwrap_or_default()))
}

fn shorten_with(p: &str, home: &str) -> String {
    if home.is_empty() {
        return p.to_string();
    }
    // Component-wise stripping rather than a string prefix. It cannot mistake
    // a sibling for a child and it does not care about a trailing separator.
    let Ok(rest) = Path::new(p).strip_prefix(home) else {
        return p.to_string();
    };
    if rest.as_os_str().is_empty() {
        return "~".to_string();
    }
    format!("~/{}", rest.to_string_lossy())
}

/// Write a path the short way a person reads it, with `~` for the home
/// directory.
pub fn shorten(p: &str) -> String {
    shorten_with(p, &home().to_string_lossy())
}

fn expand_with(p: &str, home: &Path) -> PathBuf {
    // With no home to expand into, `~` is a literal name rather than a
    // shortcut. Expanding it to a bare relative path would silently point at
    // something in the current directory.
    if home.as_os_str().is_empty() {
        return PathBuf::from(p);
    }
    if p == "~" {
        return home.to_path_buf();
    }
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(p),
    }
}

/// Turn what a person typed into a path, expanding a leading `~`.
pub fn expand(p: &str) -> PathBuf {
    expand_with(p, &home())
}

fn resolve_with(arg: &str, home: &Path) -> Option<PathBuf> {
    // A bare `cd` goes home. That is the shell's own rule.
    let target = if arg.is_empty() {
        home.to_path_buf()
    } else {
        expand_with(arg, home)
    };
    target.is_dir().then_some(target)
}

/// Resolve what `cd <arg>` would move to, or None when it is not a directory.
pub fn resolve(arg: &str) -> Option<PathBuf> {
    resolve_with(arg, &home())
}

/// Keep a prompt from eating the line. Only the tail of a deep path tells a
/// person where they are. The head goes and `…/` takes its place.
pub fn compact(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let mut tail = String::new();
    for part in path.split('/').rev() {
        let next = if tail.is_empty() {
            part.to_string()
        } else {
            format!("{part}/{tail}")
        };
        // Two more cells go to the `…/` that replaces what is dropped.
        if next.chars().count() + 2 > max {
            break;
        }
        tail = next;
    }
    format!("…/{tail}")
}

#[cfg(test)]
mod tests {
    use super::{compact, expand_with, resolve_with, shorten_with, trim_home};
    use std::path::{Path, PathBuf};

    const HOME: &str = "/home/someone";

    #[test]
    fn shorten_writes_the_home_directory_as_a_tilde() {
        assert_eq!(shorten_with(HOME, HOME), "~");
        assert_eq!(shorten_with("/home/someone/work", HOME), "~/work");
    }

    #[test]
    fn shorten_leaves_a_path_outside_home_alone() {
        assert_eq!(shorten_with("/usr/bin", HOME), "/usr/bin");
        assert_eq!(shorten_with("work/alpha", HOME), "work/alpha");
    }

    #[test]
    fn shorten_does_not_match_a_sibling_by_prefix() {
        assert_eq!(shorten_with("/home/someoneelse", HOME), "/home/someoneelse");
    }

    #[test]
    fn shorten_leaves_everything_alone_without_a_home() {
        assert_eq!(shorten_with("/usr/bin", ""), "/usr/bin");
        assert_eq!(shorten_with("/", ""), "/");
    }

    #[test]
    fn a_home_of_only_separators_is_still_a_home() {
        assert_eq!(trim_home("/home/someone/"), "/home/someone");
        assert_eq!(trim_home("/home/someone"), "/home/someone");
        assert_eq!(trim_home("/"), "/");
        assert_eq!(trim_home(""), "");
    }

    #[test]
    fn shorten_works_when_home_is_the_root() {
        assert_eq!(shorten_with("/", "/"), "~");
        assert_eq!(shorten_with("/usr/bin", "/"), "~/usr/bin");
    }

    #[test]
    fn expand_turns_a_tilde_into_the_home_directory() {
        let home = Path::new(HOME);
        assert_eq!(expand_with("~", home), PathBuf::from(HOME));
        assert_eq!(
            expand_with("~/work", home),
            PathBuf::from("/home/someone/work")
        );
    }

    #[test]
    fn expand_leaves_a_tilde_that_is_part_of_a_name() {
        let home = Path::new(HOME);
        assert_eq!(expand_with("~work", home), PathBuf::from("~work"));
        assert_eq!(expand_with("a/~/b", home), PathBuf::from("a/~/b"));
    }

    #[test]
    fn expand_keeps_a_tilde_literal_without_a_home() {
        let none = Path::new("");
        assert_eq!(expand_with("~", none), PathBuf::from("~"));
        assert_eq!(expand_with("~/work", none), PathBuf::from("~/work"));
    }

    #[test]
    fn resolve_finds_a_directory_that_is_there() {
        let root = Path::new("/");
        assert_eq!(resolve_with("/", root), Some(PathBuf::from("/")));
        assert_eq!(resolve_with("~", root), Some(PathBuf::from("/")));
    }

    #[test]
    fn a_bare_argument_resolves_to_the_home_directory() {
        let root = Path::new("/");
        assert_eq!(resolve_with("", root), Some(PathBuf::from("/")));
    }

    #[test]
    fn resolve_refuses_what_is_not_a_directory() {
        let root = Path::new("/");
        assert_eq!(resolve_with("/no-such-directory-here", root), None);
        // A home that does not exist cannot be moved to either.
        assert_eq!(resolve_with("", Path::new("/no-such-home")), None);
    }

    #[test]
    fn compact_leaves_a_path_that_already_fits() {
        assert_eq!(compact("~/work", 44), "~/work");
        assert_eq!(compact("~/work", 6), "~/work");
    }

    #[test]
    fn compact_keeps_the_tail_and_marks_the_cut() {
        assert_eq!(compact("~/one/two/three", 12), "…/two/three");
    }

    #[test]
    fn compact_never_doubles_the_separator_of_an_absolute_path() {
        let out = compact("/one/two/three/four", 12);
        assert!(!out.contains("//"), "{out}");
        assert_eq!(out, "…/three/four");
    }

    #[test]
    fn compact_gives_up_rather_than_overflow_a_tiny_budget() {
        assert_eq!(compact("~/one/two/three", 3), "…/");
    }

    #[test]
    fn compact_gives_up_on_a_long_name_with_no_separator() {
        assert_eq!(compact(&"a".repeat(40), 10), "…/");
    }

    #[test]
    fn compact_counts_characters_rather_than_bytes() {
        // Eight characters and well over eight bytes.
        assert_eq!(compact("好好好好好好好好", 8), "好好好好好好好好");
    }
}
