//! Readers for arguments the data could not carry.
//!
//! An argument marked `dyn` in a specification needed code the conversion
//! could not keep, and `specs/dynamic.txt` lists all 4854 of them. This
//! module answers two by reading the files that code would have read.
//! Nothing here spawns a process and nothing here touches a network.
//!
//! `plans/phase-2-spec-runtime.md` §5.1 describes two tables and this is the
//! second of them: readers keyed by the argument's own identity, which is
//! the door for an argument whose generator carries no command line at all.
//! The first table is missing. It would be keyed by a generator's own
//! `script`, and it is the larger half by a wide margin: 3282 arguments keep
//! a command line in the data and lost only the code that read its output,
//! and because one generator is referenced from many arguments of the same
//! specification, a single reader there fills every argument sharing that
//! line. Neither argument below kept a command line, so that table would
//! have no work to do for them and is not built here.

use crate::candidates::{Candidate, DEFAULT_PRIORITY, Kind};
use crate::fuzzy;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// One reader and the argument it answers for.
///
/// `specs/dynamic.txt` keys an argument by its command, the subcommands and
/// options that reach the argument's owner, and the argument's index.
/// [`crate::argwalk::Walk`] carries neither the owning option nor the index,
/// so the key here is the pair it does carry: the name of the node the walk
/// ended on and the argument's own name. Nothing is lost for either reader
/// below. `make`'s `target` is one argument name in four places in
/// `make.json` — the bare one and the ones behind `-j`, `-B` and `-e` — and
/// all four want the same rows, which is why the four `dynamic.txt` keys
/// collapse to one entry here.
///
/// A third reader is one more entry in [`READERS`] and one more function.
/// A command that gives one argument name two different meanings is the
/// case this key cannot tell apart, and answering it means teaching
/// `argwalk` to carry the path and the index rather than changing anything
/// here.
struct Reader {
    /// The subcommand the argument hangs off, by its own first name. For
    /// both readers below that is the command itself.
    owner: &'static str,
    /// The argument's first name, as the specification spells it.
    arg: &'static str,
    /// What a row says it is, where a specification's own row would carry a
    /// description.
    label: &'static str,
    read: fn(&Sources) -> Vec<String>,
}

const READERS: &[Reader] = &[
    Reader {
        owner: "make",
        arg: "target",
        label: "target",
        read: make_targets,
    },
    Reader {
        owner: "ssh",
        arg: "user@hostname",
        label: "host",
        read: ssh_hosts,
    },
];

/// Where a reader is allowed to look. The environment is read at one edge
/// and every reader takes its paths from here, which is what lets a test
/// hand one a temporary home rather than the person's own. `path` keeps
/// `$HOME` behind a twin for that same reason.
struct Sources<'a> {
    /// The directory the line is being typed in.
    cwd: &'a Path,
    /// `$HOME`, or an empty path when the environment says nothing.
    home: &'a Path,
    /// The system-wide SSH configuration.
    ssh_config: &'a Path,
}

const SYSTEM_SSH_CONFIG: &str = "/etc/ssh/ssh_config";

/// The rows a native reader offers for one `dyn` argument, or none where the
/// table has no reader for it. `owner` is the name of the node the walk
/// ended on and `arg` is the argument's own first name.
pub(crate) fn rows(owner: &str, arg: &str, term: &str, cwd: &Path) -> Vec<Candidate> {
    let Some(reader) = reader(owner, arg) else {
        return Vec::new();
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let sources = Sources {
        cwd,
        home: Path::new(&home),
        ssh_config: Path::new(SYSTEM_SSH_CONFIG),
    };
    candidates(reader, &sources, term)
}

fn reader(owner: &str, arg: &str) -> Option<&'static Reader> {
    READERS.iter().find(|r| r.owner == owner && r.arg == arg)
}

fn candidates(reader: &Reader, sources: &Sources, term: &str) -> Vec<Candidate> {
    (reader.read)(sources)
        .into_iter()
        .filter_map(|name| {
            let score = fuzzy::score(term, &name)?;
            Some(Candidate {
                display: name.clone(),
                insert: name,
                // A target and a host are both flat values rather than paths
                // on disk, which is what `spec_menu` already gives a
                // specification's own fixed suggestions: no folder glyph, no
                // trailing slash and plain shell quoting.
                kind: Kind::Path,
                label: Cow::Borrowed(reader.label),
                hint: Vec::new(),
                score,
                priority: DEFAULT_PRIORITY,
            })
        })
        .collect()
}

/// How much of any one file a reader reads. The 64 KiB `git.rs` already caps
/// a Git query's output at, for the same reason: a prompt is waiting on
/// this, and a makefile or an SSH configuration past that size is generated
/// rather than written. What sits past the limit is dropped and the file is
/// still read up to it.
const READ_LIMIT: u64 = 64 * 1024;

/// The first [`READ_LIMIT`] bytes of `path`, as whole lines. A read that
/// stops at the limit stops in the middle of a line as often as not, and
/// that last partial line is dropped, because half a name is worse than a
/// missing one. An absent or unreadable file gives nothing and never an
/// error. This runs at a prompt and an error has nowhere to go.
fn read_lines(path: &Path) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file.take(READ_LIMIT).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let truncated = bytes.len() as u64 == READ_LIMIT;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if truncated && !text.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn push_unique(name: &str, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    if name.is_empty() {
        return;
    }
    if seen.insert(name.to_string()) {
        out.push(name.to_string());
    }
}

/// The names make itself tries, in make's own order.
const MAKEFILE_NAMES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];

/// The targets make reserves for itself. Each one is a directive rather than
/// something to build, so none of them is a row. Any other name that starts
/// with a dot is an ordinary file a makefile can genuinely have a rule for
/// and stays.
const SPECIAL_TARGETS: &[&str] = &[
    ".DEFAULT",
    ".DELETE_ON_ERROR",
    ".EXPORT_ALL_VARIABLES",
    ".IGNORE",
    ".INTERMEDIATE",
    ".LOW_RESOLUTION_TIME",
    ".NOTINTERMEDIATE",
    ".NOTPARALLEL",
    ".ONESHELL",
    ".PHONY",
    ".POSIX",
    ".PRECIOUS",
    ".SECONDARY",
    ".SECONDEXPANSION",
    ".SILENT",
    ".SUFFIXES",
    ".WAIT",
];

const PHONY: &str = ".PHONY";

/// The targets of the makefile in `sources.cwd`, in the order the file gives
/// them.
///
/// This reads the text rather than asking make for its own answer. `make -qp`
/// is the thorough way and it is the wrong way here, because it expands the
/// makefile, and every `$(shell ...)` in it with it: completing a line would
/// then run whatever the makefile runs. It is also slow on a large tree.
/// Reading the text costs nothing and can surprise nobody.
fn make_targets(sources: &Sources) -> Vec<String> {
    let Some(path) = MAKEFILE_NAMES
        .iter()
        .map(|name| sources.cwd.join(name))
        .find(|path| path.is_file())
    else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in read_lines(&path) {
        let Some((head, rest)) = rule_halves(&line) else {
            continue;
        };
        for name in head.split_whitespace() {
            // `.PHONY: build test` builds nothing of its own. The names it
            // lists are the ones an author most wants completed.
            if name == PHONY {
                for phony in rest.split_whitespace() {
                    if is_target_name(phony) {
                        push_unique(phony, &mut seen, &mut out);
                    }
                }
            } else if !SPECIAL_TARGETS.contains(&name) && is_target_name(name) {
                push_unique(name, &mut seen, &mut out);
            }
        }
    }
    out
}

/// A name worth offering. A `$` makes it a variable reference rather than a
/// name, and a `%` makes the rule a pattern that stands for a shape rather
/// than for a target anyone types.
fn is_target_name(name: &str) -> bool {
    !name.contains('$') && !name.contains('%')
}

/// The two halves of a rule line: what sits before its first `:` and what
/// sits after. `None` for a line that names no target — one that does not
/// start in column one, one that is a comment, and one whose colon belongs
/// to a `:=` or `::=` assignment. A `::` that is not an assignment is a
/// double-colon rule and names targets like any other line.
fn rule_halves(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(char::is_whitespace) || line.starts_with('#') {
        return None;
    }
    let colon = line.find(':')?;
    let (head, after) = line.split_at(colon);
    let rest = match after[1..].strip_prefix(':') {
        Some(double) => double,
        None => &after[1..],
    };
    if rest.starts_with('=') {
        return None;
    }
    // `FOO ?= a:b` and its kin hide the colon inside the value. A rule never
    // carries an `=` in front of its own colon.
    if head.contains('=') {
        return None;
    }
    Some((head, rest))
}

/// What a hashed `known_hosts` entry starts with. There is no name inside
/// one to read.
const HASHED: &str = "|1|";

/// Host names from configuration alone. Never from the network and never by
/// running `ssh`.
///
/// An `Include` directive is not followed. Following one means resolving a
/// glob against `~/.ssh` or against `/etc/ssh` depending on which file holds
/// it, and guarding a chain of includes that is free to reach itself. That
/// is a reader of its own rather than a line of this one, so a host that
/// only an included file names does not appear. This sentence is what a
/// person looking for such a host should land on.
fn ssh_hosts(sources: &Sources) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let user_config = sources.home.join(".ssh").join("config");
    for path in [user_config.as_path(), sources.ssh_config] {
        for line in read_lines(path) {
            for name in host_line_names(&line) {
                if !is_pattern(name) {
                    push_unique(name, &mut seen, &mut out);
                }
            }
        }
    }
    let known_hosts = sources.home.join(".ssh").join("known_hosts");
    for line in read_lines(&known_hosts) {
        for name in known_hosts_names(&line) {
            if !is_pattern(name) {
                push_unique(name, &mut seen, &mut out);
            }
        }
    }
    out
}

/// A name that describes which hosts a block applies to rather than naming
/// one to connect to. `Host *` and `Host !build-box` are both that.
fn is_pattern(name: &str) -> bool {
    name.starts_with('!') || name.contains(['*', '?'])
}

/// The names on a `Host` line. The keyword is case-insensitive and
/// `ssh_config` lets an `=` stand in for the whitespace after it.
/// `HostName` and `HostKeyAlias` begin with the same four letters and are
/// not this.
fn host_line_names(line: &str) -> Vec<&str> {
    let line = line.trim();
    let Some(end) = line.find(|c: char| c.is_whitespace() || c == '=') else {
        return Vec::new();
    };
    let (keyword, rest) = line.split_at(end);
    if !keyword.eq_ignore_ascii_case("Host") {
        return Vec::new();
    }
    rest.trim_start()
        .trim_start_matches('=')
        .split_whitespace()
        .take_while(|name| !name.starts_with('#'))
        .collect()
}

/// The host names one `known_hosts` line holds: its first field, split on
/// commas. A hashed entry is skipped, because the name in it is a digest
/// rather than text. `@cert-authority` and `@revoked` put a marker in front
/// of the names, so the field after such a marker is the one to read.
fn known_hosts_names(line: &str) -> Vec<&str> {
    let mut fields = line.split_whitespace();
    let Some(mut first) = fields.next() else {
        return Vec::new();
    };
    if first.starts_with('#') {
        return Vec::new();
    }
    if first.starts_with('@') {
        let Some(after_marker) = fields.next() else {
            return Vec::new();
        };
        first = after_marker;
    }
    if first.starts_with(HASHED) {
        return Vec::new();
    }
    first.split(',').collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    /// Every host name below is invented. A test that read the real
    /// `~/.ssh` would put a person's own hosts into the suite's output, so
    /// `home` is always a fixture and never `$HOME`.
    fn sources<'a>(cwd: &'a Path, home: &'a Path, ssh_config: &'a Path) -> Sources<'a> {
        Sources {
            cwd,
            home,
            ssh_config,
        }
    }

    /// A path that cannot exist, for a source a test does not use.
    fn nowhere() -> &'static Path {
        Path::new("/no-such-directory-here")
    }

    fn makefile(body: &str) -> Fixture {
        let f = Fixture::new(&[]);
        std::fs::write(f.path().join("Makefile"), body).unwrap();
        f
    }

    #[test]
    fn a_makefile_gives_its_targets_in_order() {
        let f = makefile(
            "\
CARGO := cargo
build:
\t$(CARGO) build
release: build
\t$(CARGO) build --release
",
        );
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), ["build", "release"]);
    }

    #[test]
    fn a_phony_line_gives_the_names_it_lists() {
        let f = makefile(".PHONY: build check\nbuild:\n\t:\n");
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), ["build", "check"]);
    }

    #[test]
    fn a_directive_a_pattern_and_an_assignment_are_not_targets() {
        let f = makefile(
            "\
.SUFFIXES:
CARGO := cargo
FLAGS ?= a:b
%.o: %.c
\t:
$(BINARY): build
\t:
build:
\t:
",
        );
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), ["build"]);
    }

    #[test]
    fn a_double_colon_rule_is_a_target_and_a_double_colon_assignment_is_not() {
        let f = makefile("CARGO ::= cargo\nbuild:: check\n\t:\n");
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), ["build"]);
    }

    #[test]
    fn an_indented_line_and_a_comment_name_nothing() {
        let f = makefile("# comment: not a rule\nbuild:\n\tinner: not a rule\n");
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), ["build"]);
    }

    #[test]
    fn a_dotted_name_make_does_not_reserve_stays() {
        let f = makefile(".env:\n\t:\n");
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), [".env"]);
    }

    #[test]
    fn the_first_of_makes_own_names_wins() {
        let f = makefile("from-makefile:\n\t:\n");
        std::fs::write(f.path().join("GNUmakefile"), "from-gnumakefile:\n\t:\n").unwrap();
        let s = sources(f.path(), nowhere(), nowhere());
        assert_eq!(make_targets(&s), ["from-gnumakefile"]);
    }

    #[test]
    fn no_makefile_gives_no_targets() {
        let f = Fixture::new(&[]);
        let s = sources(f.path(), nowhere(), nowhere());
        assert!(make_targets(&s).is_empty());
    }

    #[test]
    fn a_makefile_past_the_limit_is_read_up_to_it() {
        let mut body = String::new();
        let mut n = 0;
        while body.len() < READ_LIMIT as usize + 4096 {
            body.push_str(&format!("target-{n}:\n\t:\n"));
            n += 1;
        }
        let f = makefile(&body);
        let s = sources(f.path(), nowhere(), nowhere());
        let targets = make_targets(&s);
        assert!(targets.contains(&"target-0".to_string()));
        assert!(!targets.contains(&format!("target-{}", n - 1)));
    }

    /// A home holding an `.ssh` directory with the files a test writes into
    /// it. Every name in one is invented.
    fn ssh_home(config: Option<&str>, known_hosts: Option<&str>) -> Fixture {
        let f = Fixture::new(&[".ssh"]);
        if let Some(text) = config {
            std::fs::write(f.path().join(".ssh").join("config"), text).unwrap();
        }
        if let Some(text) = known_hosts {
            std::fs::write(f.path().join(".ssh").join("known_hosts"), text).unwrap();
        }
        f
    }

    #[test]
    fn a_config_gives_its_host_names_and_not_its_patterns() {
        let f = ssh_home(
            Some(
                "\
Host *
  ServerAliveInterval 60

Host sample-host build-box
  HostName sample-host.example.invalid
  User sample-user

Host !staging-box
  Port 2222
",
            ),
            None,
        );
        let s = sources(nowhere(), f.path(), nowhere());
        assert_eq!(ssh_hosts(&s), ["sample-host", "build-box"]);
    }

    #[test]
    fn the_host_keyword_is_case_insensitive_and_takes_an_equals() {
        let f = ssh_home(
            Some("host=sample-host\nHostName other.example.invalid\n"),
            None,
        );
        let s = sources(nowhere(), f.path(), nowhere());
        assert_eq!(ssh_hosts(&s), ["sample-host"]);
    }

    #[test]
    fn the_system_config_is_read_too_and_a_repeat_appears_once() {
        let f = ssh_home(Some("Host sample-host\n"), None);
        let system = Fixture::new(&[]);
        let path = system.path().join("ssh_config");
        std::fs::write(&path, "Host sample-host build-box\n").unwrap();
        let s = sources(nowhere(), f.path(), &path);
        assert_eq!(ssh_hosts(&s), ["sample-host", "build-box"]);
    }

    #[test]
    fn a_hashed_known_hosts_entry_is_skipped_and_a_plain_one_is_read() {
        let f = ssh_home(
            None,
            Some(
                "\
|1|c2FtcGxl|c2FtcGxl= ssh-ed25519 AAAASAMPLE
sample-host,10.0.0.1 ssh-ed25519 AAAASAMPLE
@cert-authority build-box ssh-ed25519 AAAASAMPLE
# a comment
",
            ),
        );
        let s = sources(nowhere(), f.path(), nowhere());
        assert_eq!(ssh_hosts(&s), ["sample-host", "10.0.0.1", "build-box"]);
    }

    #[test]
    fn no_ssh_files_give_no_hosts() {
        let f = Fixture::new(&[]);
        let s = sources(nowhere(), f.path(), nowhere());
        assert!(ssh_hosts(&s).is_empty());
    }

    #[test]
    fn the_table_answers_for_both_arguments_and_for_nothing_else() {
        assert!(reader("make", "target").is_some());
        assert!(reader("ssh", "user@hostname").is_some());
        assert!(reader("make", "no-such-argument").is_none());
        assert!(reader("chown", "owner").is_none());
    }

    #[test]
    fn an_argument_with_no_reader_gives_no_rows() {
        assert!(rows("chown", "owner", "", nowhere()).is_empty());
    }

    #[test]
    fn a_host_row_carries_the_name_it_would_insert_and_its_label() {
        let f = ssh_home(Some("Host sample-host\n"), None);
        let s = sources(nowhere(), f.path(), nowhere());
        let reader = reader("ssh", "user@hostname").expect("ssh has a reader");
        let rows = candidates(reader, &s, "sam");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display, "sample-host");
        assert_eq!(rows[0].insert, "sample-host");
        assert_eq!(rows[0].label, "host");
        assert_eq!(rows[0].kind, Kind::Path);
    }

    #[test]
    fn a_term_that_matches_nothing_gives_no_rows() {
        let f = ssh_home(Some("Host sample-host\n"), None);
        let s = sources(nowhere(), f.path(), nowhere());
        let reader = reader("ssh", "user@hostname").expect("ssh has a reader");
        assert!(candidates(reader, &s, "zzzz").is_empty());
    }
}
