//! Git subcommand, branch and file completion from the installed Git.

use crate::candidates::{
    Candidate, FOLDER, Kind, MAX_RESULTS, Query, SCAN_LIMIT, Scan, match_rank,
};
use crate::fuzzy;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// What the prompt allows the query. `read_commands` takes its patience as an
/// argument rather than reading this, because a test measured against the
/// prompt's own budget fails on a loaded machine rather than on the code. The
/// file and branch readers are reached through `App` rather than called
/// directly, so a test moves this rather than passing an argument.
static TIMEOUT_MS: AtomicU64 = AtomicU64::new(250);

fn timeout() -> Duration {
    Duration::from_millis(TIMEOUT_MS.load(Ordering::Relaxed))
}

/// Give the readers longer than a prompt would. `Fixture::new` is the one
/// caller and the installed binary keeps the prompt's own budget.
pub(crate) fn widen_timeout() {
    TIMEOUT_MS.store(10_000, Ordering::Relaxed);
}
const OUTPUT_LIMIT: usize = 64 * 1024;
/// The groups the menu offers. `nohelpers` drops the names that exist for
/// Git's own scripts. `alias` adds the ones this machine configured.
const LIST_CMDS: &str = "--list-cmds=list-mainporcelain,list-ancillarymanipulators,list-ancillaryinterrogators,nohelpers,alias";

/// A word the shell reads as one plain literal rather than as syntax. The
/// subcommand goes on the line as it stands and a name carrying a quote or a
/// semicolon would arrive there as something to run. A leading `-` is an
/// option rather than a name. An empty word is one nobody has typed yet and
/// each caller answers that for itself.
fn plain_name(word: &str) -> bool {
    !word.starts_with('-')
        && word
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

pub(crate) struct Target {
    pub word: Query,
    pub kind: Kind,
    pub taken: Vec<String>,
    add: Option<Add>,
}

impl Target {
    pub fn accepts(&self, kind: Kind) -> bool {
        self.kind == kind
            || (self.kind == Kind::File
                && kind == Kind::Option
                && self.word.arg.is_empty()
                && self.add.as_ref().is_some_and(|a| !a.paths_only))
    }
    pub fn exclude_tail(&mut self, tail: &str) {
        if self.kind == Kind::File
            && let Some(words) = words(tail)
        {
            let mut context = Add {
                paths_only: self.add.as_ref().is_some_and(|add| add.paths_only),
                ..Add::default()
            };
            for word in words.into_iter().filter(|w| !w.value.is_empty()) {
                if context.consume(&word.value, &mut self.taken).is_none() {
                    break;
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Value {
    Chmod,
    File,
}

struct AddOption {
    names: &'static [&'static str],
    description: &'static str,
    value: Option<Value>,
}

// The Git add options in the Q completion spec. Descriptions fit the existing
// footer rather than adding another panel to the menu.
const ADD_OPTIONS: &[AddOption] = &[
    AddOption {
        names: &["-n", "--dry-run"],
        description: "Preview changes",
        value: None,
    },
    AddOption {
        names: &["-v", "--verbose"],
        description: "Show added files",
        value: None,
    },
    AddOption {
        names: &["-f", "--force"],
        description: "Include ignored files",
        value: None,
    },
    AddOption {
        names: &["-i", "--interactive"],
        description: "Select changes interactively",
        value: None,
    },
    AddOption {
        names: &["-p", "--patch"],
        description: "Select changes by hunk",
        value: None,
    },
    AddOption {
        names: &["-e", "--edit"],
        description: "Edit changes before staging",
        value: None,
    },
    AddOption {
        names: &["-u", "--update"],
        description: "Update tracked files",
        value: None,
    },
    AddOption {
        names: &["-A", "--all", "--no-ignore-removal"],
        description: "Include additions and deletions",
        value: None,
    },
    AddOption {
        names: &["--no-all", "--ignore-removal"],
        description: "Exclude deletions",
        value: None,
    },
    AddOption {
        names: &["-N", "--intent-to-add"],
        description: "Record paths without content",
        value: None,
    },
    AddOption {
        names: &["--refresh"],
        description: "Refresh file information",
        value: None,
    },
    AddOption {
        names: &["--ignore-errors"],
        description: "Continue after indexing errors",
        value: None,
    },
    AddOption {
        names: &["--ignore-missing"],
        description: "Check missing files in preview",
        value: None,
    },
    AddOption {
        names: &["--no-warn-embedded-repo"],
        description: "Suppress nested repo warnings",
        value: None,
    },
    AddOption {
        names: &["--renormalize"],
        description: "Normalize tracked content again",
        value: None,
    },
    AddOption {
        names: &["--chmod"],
        description: "Set the staged executable bit",
        value: Some(Value::Chmod),
    },
    AddOption {
        names: &["--pathspec-from-file"],
        description: "Read paths from a file",
        value: Some(Value::File),
    },
    AddOption {
        names: &["--pathspec-file-nul"],
        description: "Read NUL-separated paths",
        value: None,
    },
    AddOption {
        names: &["--"],
        description: "End options and enter paths",
        value: None,
    },
];

#[derive(Clone, Copy, Default, Hash, Eq, PartialEq)]
struct FileMode {
    force: bool,
    tracked: bool,
    cached: bool,
    no_removal: bool,
}

#[derive(Default)]
struct Add {
    used: Vec<usize>,
    files: FileMode,
    paths_only: bool,
    from_file: bool,
    value: Option<Value>,
    attached: bool,
}

impl Add {
    fn option(&mut self, name: &str) -> Option<Option<Value>> {
        let (id, option) = ADD_OPTIONS
            .iter()
            .enumerate()
            .find(|(_, o)| o.names.contains(&name))?;
        self.used.push(id);
        match name {
            "--" => self.paths_only = true,
            "-f" | "--force" => self.files.force = true,
            "-u" | "--update" | "-p" | "--patch" | "-e" | "--edit" => self.files.tracked = true,
            "-A" | "--all" | "--no-ignore-removal" => {
                self.files.tracked = false;
                self.files.no_removal = false;
            }
            "--no-all" | "--ignore-removal" => self.files.no_removal = true,
            "--renormalize" | "--refresh" => {
                self.files.tracked = true;
                self.files.cached = true;
            }
            _ => {}
        }
        Some(option.value)
    }

    fn consume(&mut self, word: &str, taken: &mut Vec<String>) -> Option<()> {
        if let Some(value) = self.value.take() {
            self.consume_value(value, word)?;
        } else if !self.paths_only && word.starts_with('-') && word != "-" {
            if let Some((name, value)) = word.split_once('=') {
                let expected = self.option(name)??;
                self.consume_value(expected, value)?;
            } else if word.starts_with("--") || word.len() == 2 {
                self.value = self.option(word)?;
            } else {
                for flag in word.strip_prefix('-')?.chars() {
                    if self.option(&format!("-{flag}"))?.is_some() {
                        return None;
                    }
                }
            }
        } else {
            if !relative_path(word) || self.from_file {
                return None;
            }
            taken.push(file_path(word).to_string());
        }
        Some(())
    }

    fn consume_value(&mut self, value: Value, word: &str) -> Option<()> {
        match value {
            Value::Chmod if !["+x", "-x"].contains(&word) => return None,
            Value::File => self.from_file = true,
            _ => {}
        }
        Some(())
    }
}

struct Word {
    start: usize,
    value: String,
}

// Read literal shell words without asking a shell to expand them. The last
// word can have an open quote because it is still being typed.
fn words(input: &str) -> Option<Vec<Word>> {
    let mut out = Vec::new();
    let mut chars = input.char_indices().peekable();
    loop {
        while chars.peek().is_some_and(|(_, c)| matches!(c, ' ' | '\t')) {
            chars.next();
        }
        let start = chars.peek().map_or(input.len(), |(i, _)| *i);
        let mut value = String::new();
        let mut quote = None;
        let mut separated = false;
        while let Some((_, c)) = chars.next() {
            match (quote, c) {
                (None, ' ' | '\t') => {
                    separated = true;
                    break;
                }
                (None, '=') if value.is_empty() => return None,
                (None, '\'' | '"') => quote = Some(c),
                (Some(q), c) if c == q => quote = None,
                (None | Some('"'), '\\') => {
                    let (_, next) = chars.next()?;
                    if next.is_control() {
                        return None;
                    }
                    if quote == Some('"') && !"$`\"\\".contains(next) {
                        value.push('\\');
                    }
                    value.push(next);
                }
                (None, c) if !(c.is_alphanumeric() || "/._-+@,=:".contains(c)) => {
                    return None;
                }
                (Some('"'), '$' | '`') => return None,
                (_, c) if c.is_control() => return None,
                (_, c) => value.push(c),
            }
        }
        out.push(Word { start, value });
        if !separated {
            break;
        }
    }
    Some(out)
}

fn file_path(word: &str) -> &str {
    let mut path = word.strip_prefix(":(literal)").unwrap_or(word);
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    if path.is_empty() && !word.is_empty() {
        "."
    } else {
        path
    }
}

fn parse_add(left: &str, rest: &str) -> Option<Target> {
    let mut words = words(rest)?;
    let current = words.pop()?;
    let mut add = Add::default();
    let mut taken = Vec::new();
    for word in words {
        add.consume(&word.value, &mut taken)?;
    }
    if !add.paths_only
        && add.value.is_none()
        && let Some((name, _)) = current.value.split_once('=')
        && name.starts_with('-')
    {
        add.value = Some(add.option(name)??);
        add.attached = true;
    }
    let kind = match add.value {
        Some(Value::Chmod) => Kind::Option,
        Some(Value::File) => Kind::Path,
        None if !add.paths_only && current.value.starts_with('-') => Kind::Option,
        None => {
            if !relative_path(&current.value) || add.from_file && !current.value.is_empty() {
                return None;
            }
            Kind::File
        }
    };
    Some(Target {
        word: Query {
            start: left.len() - rest.len() + current.start,
            arg: if kind == Kind::Option {
                current.value.clone()
            } else if current.value.is_empty() {
                String::new()
            } else {
                crate::shellword::quote(
                    current
                        .value
                        .strip_prefix(":(literal)")
                        .unwrap_or(&current.value),
                )
            },
        },
        kind,
        taken,
        add: Some(add),
    })
}

fn relative_path(word: &str) -> bool {
    let path = file_path(word);
    !path.starts_with('/') && !path.split('/').any(|part| part == "..")
}

/// Read a subcommand, the first branch argument or an add argument.
/// Global options and shell expansions stay with the shell's completion.
pub(crate) fn parse(left: &str) -> Option<Target> {
    let trimmed = left.trim_start_matches([' ', '\t']);
    let rest = trimmed.strip_prefix("git")?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let word = rest.trim_start_matches([' ', '\t']);
    let (arg, kind) = match word.split_once([' ', '\t']) {
        Some(("add", rest)) => return parse_add(left, rest),
        Some(("switch" | "checkout", rest)) => {
            let arg = rest.trim_start_matches([' ', '\t']);
            if arg.starts_with('-')
                || !arg
                    .chars()
                    .all(|c| c.is_alphanumeric() || "/._-+@,".contains(c))
            {
                return None;
            }
            (arg, Kind::Branch)
        }
        None if plain_name(word) => (word, Kind::Command),
        _ => return None,
    };
    Some(Target {
        word: Query {
            start: left.len() - arg.len(),
            arg: arg.to_string(),
        },
        kind,
        taken: Vec::new(),
        add: None,
    })
}

/// Every name `command` printed, sorted, unique and free of anything a shell
/// would read as syntax. `None` when the command failed, when it outlasted
/// `patience` or when it wrote more than `OUTPUT_LIMIT`.
///
/// The child writes into one end of a socket pair rather than into a pipe.
/// `std::process` has no deadline of its own and a socket is what carries one.
fn read_commands(command: &mut Command, patience: Duration) -> Option<Vec<String>> {
    let (status, output) = read_output(command, patience)?;
    if !status.success() {
        return None;
    }
    let mut names: Vec<String> = output
        .lines()
        .filter(|name| !name.is_empty() && plain_name(name))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Some(names)
}

/// The child's output as text. `None` when a byte of it is not UTF-8, because
/// a caller here reads the whole answer as one string and has no name of its
/// own to drop.
fn read_output(command: &mut Command, patience: Duration) -> Option<(ExitStatus, String)> {
    let (status, bytes) = read_output_bytes(command, patience)?;
    Some((status, String::from_utf8(bytes).ok()?))
}

/// The child's output as it stands. A caller that can drop one name rather
/// than the whole answer reads this instead.
fn read_output_bytes(command: &mut Command, patience: Duration) -> Option<(ExitStatus, Vec<u8>)> {
    let (mut reader, writer) = UnixStream::pair().ok()?;
    // The wait goes on before the child can close its end. macOS refuses this
    // option on a pair whose peer has gone and a command that finished ahead
    // of the first read would otherwise leave this thread with no wait at all.
    reader.set_read_timeout(Some(patience)).ok()?;
    let start = Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // The command builder must release its copy before the reader can see EOF.
    command.stdout(Stdio::null());
    let result = collect_output(&mut child, &mut reader, patience, start);
    if result.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Read the child's output and exit status within `patience` measured from `start`.
/// The caller kills the child when this function returns `None`.
fn collect_output(
    child: &mut Child,
    reader: &mut UnixStream,
    patience: Duration,
    start: Instant,
) -> Option<(ExitStatus, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let remaining = patience.checked_sub(start.elapsed())?;
        // The budget is spent. This is what ends the loop rather than the
        // wait below, which cannot be trusted to be set at all.
        if remaining.is_zero() {
            return None;
        }
        // Shorten the wait as the budget goes. macOS refuses this option once
        // the child has closed its end of the pair, and the read after such a
        // refusal answers from what is already there rather than waiting. The
        // wait in force is never longer than the one `read_commands` set, so a
        // refusal here is not a reason to give up.
        let _ = reader.set_read_timeout(Some(remaining));
        let count = reader.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > OUTPUT_LIMIT {
            return None;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if start.elapsed() >= patience {
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    Some((status, bytes))
}

/// Map a fetched ref to the branch name Git can infer from it.
fn fetched_branch(spec: &str, reference: &str) -> Option<String> {
    let (source, destination) = spec.trim_start_matches('+').split_once(':')?;
    let source = source.strip_prefix("refs/heads/")?;
    match (source.split_once('*'), destination.split_once('*')) {
        (Some((before, after)), Some((prefix, suffix))) => {
            let middle = reference.strip_prefix(prefix)?.strip_suffix(suffix)?;
            Some(format!("{before}{middle}{after}"))
        }
        (None, None) if reference == destination => Some(source.to_string()),
        _ => None,
    }
}

fn excluded_branch(spec: &str, branch: &str) -> bool {
    let Some(pattern) = spec.strip_prefix("^refs/heads/") else {
        return false;
    };
    match pattern.split_once('*') {
        Some((prefix, suffix)) => branch
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.ends_with(suffix)),
        None => pattern == branch,
    }
}

fn branch_names(refs: &str, config: &str, guess: bool) -> Vec<String> {
    let entries: Vec<_> = config
        .split('\0')
        .filter_map(|s| s.split_once('\n'))
        .collect();
    let default_remote = entries
        .iter()
        .rev()
        .find_map(|(key, value)| (*key == "checkout.defaultremote").then_some(*value));
    let fetches: Vec<_> = entries
        .iter()
        .filter_map(|(key, value)| {
            Some((key.strip_prefix("remote.")?.strip_suffix(".fetch")?, *value))
        })
        .collect();
    let mut local = BTreeSet::new();
    let mut remote: HashMap<String, BTreeSet<&str>> = HashMap::new();
    for line in refs.lines() {
        let Some((reference, "")) = line.split_once('\t') else {
            continue;
        };
        if let Some(name) = reference.strip_prefix("refs/heads/") {
            local.insert(name.to_string());
        } else if guess {
            for (name, spec) in &fetches {
                if let Some(branch) = fetched_branch(spec, reference)
                    && !fetches.iter().any(|(other, excluded)| {
                        name == other && excluded_branch(excluded, &branch)
                    })
                {
                    remote.entry(branch).or_default().insert(name);
                }
            }
        }
    }
    for (branch, remotes) in remote {
        if remotes.len() == 1 || default_remote.is_some_and(|name| remotes.contains(name)) {
            local.insert(branch);
        }
    }
    local
        .into_iter()
        .filter(|name| {
            !name.is_empty() && !name.starts_with('-') && !name.chars().any(char::is_control)
        })
        .collect()
}

/// All reads share the menu's time and output budgets. No query contacts a remote.
fn read_branches(cwd: &Path) -> Option<Vec<String>> {
    let start = Instant::now();
    let mut remaining_bytes = OUTPUT_LIMIT;
    let mut query = |args: &[&str], absent_ok: bool| {
        let (status, output) = read_output(
            Command::new("git").current_dir(cwd).args(args),
            timeout().checked_sub(start.elapsed())?,
        )?;
        remaining_bytes = remaining_bytes.checked_sub(output.len())?;
        (status.success() || (absent_ok && status.code() == Some(1))).then_some(output)
    };
    let refs = query(
        &[
            "for-each-ref",
            "--format=%(refname)%09%(symref)",
            "refs/heads",
            "refs/remotes",
        ],
        false,
    )?;
    let guess = query(
        &[
            "config",
            "--type=bool",
            "--default=true",
            "--get",
            "checkout.guess",
        ],
        false,
    )?;
    let guess = guess.trim() == "true";
    let config = if guess {
        query(
            &[
                "config",
                "--null",
                "--get-regexp",
                r"^(checkout\.defaultremote|remote\..*\.fetch)$",
            ],
            true,
        )?
    } else {
        String::new()
    };
    Some(branch_names(&refs, &config, guess))
}

/// The names in one `ls-files -z` answer, sorted and unique. A name the query
/// did not spell in UTF-8 costs that name rather than the whole answer, and so
/// does a name the terminal would read as something else.
fn file_names(output: &[u8]) -> Vec<String> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|name| std::str::from_utf8(name).ok())
        .filter(|name| {
            !name.is_empty() && !name.ends_with('/') && !name.chars().any(char::is_control)
        })
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The paths `git add` would take, relative to `cwd`. The unstaged changes and
/// the untracked files below it, with Git's own ignore rules in force. The
/// query has the menu's own time and output budgets.
fn read_files(cwd: &Path) -> Option<Vec<String>> {
    read_files_with(cwd, FileMode::default())
}

fn read_files_with(cwd: &Path, mode: FileMode) -> Option<Vec<String>> {
    let mut args = vec![
        "ls-files",
        if mode.cached {
            "--cached"
        } else {
            "--modified"
        },
    ];
    if !mode.tracked {
        args.push("--others");
    }
    if !mode.force {
        args.push("--exclude-standard");
    }
    args.extend(["-z", "--"]);
    let (status, output) =
        read_output_bytes(Command::new("git").current_dir(cwd).args(args), timeout())?;
    status.success().then(|| {
        file_names(&output)
            .into_iter()
            .filter(|name| !mode.no_removal || cwd.join(name).symlink_metadata().is_ok())
            .collect()
    })
}

/// Quote a whole file name for both Git's pathspec rules and the shell.
pub(crate) fn quote_file(name: &str) -> String {
    let pathspec = if name.starts_with(':') || name.contains(['*', '?', '[', '\\']) {
        format!(":(literal){name}")
    } else if name.starts_with('-') {
        format!("./{name}")
    } else {
        name.to_string()
    };
    crate::shellword::quote(&pathspec)
}

/// Cache command names, branch names and file names separately for one menu.
/// An empty result stays empty until the next menu.
#[derive(Default)]
pub(crate) struct Completions {
    names: Option<Vec<String>>,
    branches: Option<Vec<String>>,
    files: Option<Vec<String>>,
    file_modes: HashMap<FileMode, Vec<String>>,
    dirs: Scan,
    path_files: HashMap<PathBuf, Vec<String>>,
}

/// Whether the line already names `name` or a folder holding it. An empty
/// word is nobody's path and `git add ''` is where one comes from. It has to
/// go before the comparison, because `Path::new("")` is the front of every
/// path and every row would otherwise be one the line already had.
fn covered(name: &str, taken: &[String]) -> bool {
    let name = Path::new(file_path(name));
    taken.iter().filter(|s| !s.is_empty()).any(|s| {
        let path = Path::new(file_path(s));
        path == Path::new(".") || name.starts_with(path)
    })
}

/// The folder an argument names and what was typed into it.
/// `candidates::split` answers that for `cd` and reads a bare `~` as a folder
/// of its own. Git takes the character literally and has no such case.
fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').map_or(("", path), |(_, base)| {
        (&path[..path.len() - base.len()], base)
    })
}

/// Whether Git would read a pathspec under `prefix`. It refuses one that goes
/// beyond a symbolic link and `subdirs` therefore offers no such folder. A
/// prefix somebody typed reaches one all the same. Nothing under `.git` is
/// anybody's pathspec either.
fn reachable(cwd: &Path, prefix: &str) -> bool {
    let mut path = cwd.to_path_buf();
    for part in Path::new(prefix).components() {
        if part.as_os_str() == ".git" {
            return false;
        }
        path.push(part);
        if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return false;
        }
    }
    true
}

fn row(arg: &str, name: &str, label: &'static str, kind: Kind) -> Option<Candidate> {
    Some(Candidate {
        display: name.to_string(),
        insert: name.to_string(),
        label,
        kind,
        score: fuzzy::score(arg, name)?,
    })
}

fn rank(rows: &mut Vec<Candidate>, arg: &str) {
    let tier = |name: &str| {
        if !arg.is_empty()
            && fuzzy::starts_with_folded(name, arg)
            && fuzzy::starts_with_folded(arg, name)
        {
            2
        } else {
            match_rank(arg, name)
        }
    };
    rows.sort_by(|a, b| {
        tier(&b.display)
            .cmp(&tier(&a.display))
            .then(b.score.cmp(&a.score))
            .then(a.display.cmp(&b.display))
    });
    rows.truncate(MAX_RESULTS);
}

impl Completions {
    pub(crate) fn complete(&mut self, target: &Target, cwd: &Path) -> Vec<Candidate> {
        let arg = crate::shellword::unquote(&target.word.arg);
        let Some(add) = &target.add else {
            return self.candidates(&arg, cwd, target.kind, &target.taken);
        };
        if let Some(value) = add.value {
            let lead = if add.attached {
                match value {
                    Value::Chmod => "--chmod=",
                    Value::File => "--pathspec-from-file=",
                }
            } else {
                ""
            };
            let mut out = match value {
                Value::Chmod => ["+x", "-x"]
                    .into_iter()
                    .filter_map(|v| {
                        row(
                            &arg,
                            &format!("{lead}{v}"),
                            "Set the staged executable bit",
                            Kind::Option,
                        )
                    })
                    .collect(),
                Value::File => self.path_arguments(&arg, lead, cwd),
            };
            rank(&mut out, &arg);
            return out;
        }
        let mut out = Vec::new();
        if target.kind == Kind::File && !add.from_file {
            if add.files == FileMode::default() {
                out = self.candidates(&arg, cwd, Kind::File, &target.taken);
            } else {
                let files = self
                    .file_modes
                    .entry(add.files)
                    .or_insert_with(|| read_files_with(cwd, add.files).unwrap_or_default());
                out.extend(
                    files
                        .iter()
                        .filter(|name| !covered(name, &target.taken))
                        .filter_map(|name| {
                            let name = if arg.starts_with("./") {
                                format!("./{name}")
                            } else {
                                name.clone()
                            };
                            row(&arg, &name, "file", Kind::File)
                        }),
                );
            }
            let (prefix, base) = split_path(&arg);
            let dir = cwd.join(prefix);
            if arg == "." && !covered(".", &target.taken) {
                out.extend(row(&arg, ".", FOLDER, Kind::File));
            }
            if reachable(cwd, prefix) {
                if !prefix.is_empty()
                    && base.is_empty()
                    && dir.is_dir()
                    && !covered(prefix, &target.taken)
                {
                    out.extend(row(&arg, prefix, FOLDER, Kind::File));
                }
                for name in self.dirs.git_names(&dir, base.starts_with('.')) {
                    let path = format!("{prefix}{name}/");
                    if !covered(&path, &target.taken) {
                        out.extend(row(&arg, &path, FOLDER, Kind::File));
                    }
                }
            }
        }
        if !add.paths_only && (arg.is_empty() || target.kind == Kind::Option) {
            for (_, option) in ADD_OPTIONS
                .iter()
                .enumerate()
                .filter(|(id, _)| !add.used.contains(id))
            {
                for name in option.names {
                    let insert = if option.value == Some(Value::Chmod) {
                        format!("{name}=")
                    } else {
                        name.to_string()
                    };
                    if let Some(mut candidate) =
                        row(&arg, &insert, option.description, Kind::Option)
                    {
                        if arg.is_empty() {
                            candidate.score -= 1000;
                        }
                        out.push(candidate);
                    }
                }
            }
        }
        rank(&mut out, &arg);
        out
    }

    fn path_arguments(&mut self, arg: &str, lead: &str, cwd: &Path) -> Vec<Candidate> {
        let path = arg.strip_prefix(lead).unwrap_or(arg);
        let (prefix, base) = split_path(path);
        let dir = cwd.join(prefix);
        let names = self.path_files.entry(dir.clone()).or_insert_with(|| {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                return Vec::new();
            };
            entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    if name == ".git" || name.chars().any(char::is_control) {
                        return None;
                    }
                    let kind = entry.file_type().ok()?;
                    Some(if kind.is_dir() {
                        format!("{name}/")
                    } else {
                        name
                    })
                })
                .take(SCAN_LIMIT)
                .collect()
        });
        names
            .iter()
            .filter(|name| base.starts_with('.') || !name.starts_with('.'))
            .filter_map(|name| {
                row(
                    arg,
                    &format!("{lead}{prefix}{name}"),
                    if name.ends_with('/') {
                        FOLDER
                    } else {
                        "Read paths from this file"
                    },
                    Kind::Path,
                )
            })
            .chain(
                (path == "-" || path.is_empty())
                    .then(|| {
                        row(
                            arg,
                            &format!("{lead}-"),
                            "Read paths from standard input",
                            Kind::Path,
                        )
                    })
                    .flatten(),
            )
            .collect()
    }

    pub(crate) fn candidates(
        &mut self,
        arg: &str,
        cwd: &Path,
        kind: Kind,
        taken: &[String],
    ) -> Vec<Candidate> {
        let (names, label) = match kind {
            Kind::File => (
                self.files
                    .get_or_insert_with(|| read_files(cwd).unwrap_or_default()),
                "file",
            ),
            Kind::Branch => (
                self.branches
                    .get_or_insert_with(|| read_branches(cwd).unwrap_or_default()),
                "branch",
            ),
            _ => (
                self.names.get_or_insert_with(|| {
                    read_commands(
                        Command::new("git").current_dir(cwd).arg(LIST_CMDS),
                        timeout(),
                    )
                    .unwrap_or_default()
                }),
                "command",
            ),
        };
        // A `./` the person typed is theirs to keep. Git prints none and the
        // word the line already carries is the one the rows have to match.
        let dotted = kind == Kind::File && arg.starts_with("./");
        let mut out: Vec<_> = names
            .iter()
            .filter(|name| kind != Kind::File || !covered(name, taken))
            .filter_map(|name| {
                let name = if dotted {
                    Cow::Owned(format!("./{name}"))
                } else {
                    Cow::Borrowed(name.as_str())
                };
                // The score decides before the name is cloned. Most names
                // reach nothing on a word with anything typed into it.
                let score = fuzzy::score(arg, &name)?;
                Some(Candidate {
                    display: name.to_string(),
                    insert: name.into_owned(),
                    label,
                    kind,
                    score,
                })
            })
            .collect();
        rank(&mut out, arg);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Fixture::git` runs with no user configuration. The readers under test
    /// run with the machine's own, because a person's ignore rules are theirs
    /// to keep. `--exclude-standard` would otherwise read whatever
    /// `core.excludesFile` this machine names. A repository setting outranks
    /// both that and the system's.
    /// Long enough that a loaded machine cannot turn a success into a timeout.
    const PATIENT: Duration = Duration::from_secs(10);
    /// Short enough that the command under test is the slow one.
    const IMPATIENT: Duration = Duration::from_millis(50);

    #[test]
    fn only_a_plain_first_subcommand_is_completed() {
        for line in ["git ", "git sw", "  git\tsw"] {
            let target = parse(line).unwrap();
            let q = target.word;
            assert_eq!(target.kind, Kind::Command);
            assert_eq!(&line[q.start..], q.arg);
        }
        for line in [
            "git",
            "github ",
            "git -C ",
            "git sw ",
            "git 'sw",
            "git $(echo)",
            "git sw;",
            "echo git ",
            "git \n",
        ] {
            assert!(parse(line).is_none(), "{line:?}");
        }
    }

    #[test]
    fn command_names_are_unique_and_cannot_insert_shell_syntax() {
        let names = read_commands(
            Command::new("/bin/sh")
                .args(["-c", "printf 'sample\nsample\nother\n--bad\nx;false\n'"]),
            PATIENT,
        )
        .unwrap();
        assert_eq!(names, ["other", "sample"]);
    }

    #[test]
    fn a_failed_or_slow_command_offers_nothing() {
        assert!(
            read_commands(&mut Command::new("/no-such-surmise-test-command"), PATIENT).is_none()
        );
        assert!(
            read_commands(
                Command::new("/bin/sh").args(["-c", "printf sample; exit 1"]),
                PATIENT
            )
            .is_none()
        );
        // The first of these holds the pipe open. The second closes it and
        // then holds the process. Neither may outlast the patience it got.
        for script in ["exec sleep 2", "exec 1>&-; exec sleep 2"] {
            assert!(
                read_commands(Command::new("/bin/sh").args(["-c", script]), IMPATIENT).is_none()
            );
        }
    }

    #[test]
    fn a_command_that_closed_its_output_early_is_still_read() {
        // The child closes its end of the pair and then holds on. macOS
        // refuses a read timeout on a socket in that state and the names are
        // already on this side of it. A read that gave up there would drop
        // every name a command wrote before it finished.
        let names = read_commands(
            Command::new("/bin/sh").args(["-c", "printf 'sample\\n'; exec 1>&-; exec sleep 0.05"]),
            PATIENT,
        );
        assert_eq!(names, Some(vec!["sample".to_string()]));
    }

    #[test]
    fn excessive_output_is_rejected() {
        assert!(
            read_commands(
                Command::new("/bin/sh").args(["-c", "head -c 65537 /dev/zero"]),
                PATIENT
            )
            .is_none()
        );
    }

    #[test]
    fn a_snapshot_ranks_exact_then_prefix_then_fuzzy_names() {
        let mut commands = Completions {
            names: Some(vec![
                "sample".into(),
                "sw".into(),
                "switch".into(),
                "show".into(),
            ]),
            ..Default::default()
        };
        let rows = commands.candidates(
            "sw",
            Path::new("/no-such-directory-here"),
            Kind::Command,
            &[],
        );
        assert_eq!(
            rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            ["sw", "switch", "show"]
        );
        assert!(
            commands
                .candidates("zzz", Path::new("."), Kind::Command, &[])
                .is_empty()
        );
    }

    #[test]
    fn only_the_first_plain_switch_or_checkout_argument_is_completed() {
        for line in [
            "git switch ",
            "git checkout sample/topic",
            "  git\tswitch\t\t範例/一",
        ] {
            let target = parse(line).unwrap();
            assert_eq!(target.kind, Kind::Branch);
            assert_eq!(&line[target.word.start..], target.word.arg);
        }
        for line in [
            "git switch -",
            "git switch -c ",
            "git checkout -- ",
            "git checkout sample file",
            "git switch sample ",
            "git -C sample switch ",
            "git checkout 'sample",
            "git switch sample;",
            "git switch sample$(false)",
            "git checkout sample\n",
            "git checkout sample\\",
            "git merge sample",
        ] {
            assert!(parse(line).is_none(), "{line:?}");
        }
    }

    #[test]
    fn add_completes_literal_relative_file_arguments() {
        for line in ["git add ", "git add sample/file", " git\tadd\t./範例"] {
            let target = parse(line).unwrap();
            assert_eq!(target.kind, Kind::File);
            assert_eq!(&line[target.word.start..], target.word.arg);
        }
        for arg in [
            "*.rs",
            "/sample",
            "../sample",
            "sample/../other",
            "sample;false",
        ] {
            assert!(parse(&format!("git add {arg}")).is_none(), "{arg:?}");
        }
    }

    #[test]
    fn add_reads_completed_quoted_words_and_the_current_word() {
        let line = "git add './sample file' ':(literal)sample*' sample\\ two 'third fi";
        let target = parse(line).unwrap();
        assert_eq!(target.taken, ["sample file", "sample*", "sample two"]);
        assert_eq!(crate::shellword::unquote(&target.word.arg), "third fi");
        assert_eq!(&line[target.word.start..], "'third fi");
        let target = parse("git add sample\t ").unwrap();
        assert_eq!(target.taken, ["sample"]);
        assert_eq!(target.word.arg, "");
        let target = parse("git add sample\\ ").unwrap();
        assert!(target.taken.is_empty());
        assert_eq!(crate::shellword::unquote(&target.word.arg), "sample ");
        for line in [
            "git add =git ",
            "git add sample $(false)",
            "git add sample; false ",
            "git add \"$HOME\" ",
            "git add sample | cat",
            "git add sample\nfalse",
        ] {
            assert!(parse(line).is_none(), "{line:?}");
        }
    }

    #[test]
    fn selected_files_are_removed_before_the_result_limit() {
        let f = crate::fixture::Fixture::new(&[]);
        f.init_git(&[]);
        let names: Vec<_> = (0..80).map(|n| format!("sample-{n:02}")).collect();
        for name in &names {
            std::fs::write(f.path().join(name), "sample").unwrap();
        }
        let rows = Completions::default().candidates("", f.path(), Kind::File, &names[..70]);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].insert, "sample-70");
    }

    #[test]
    fn add_options_keep_their_values_separate_from_paths() {
        for line in [
            "git add -nvf sample ",
            "git add --chmod=+x sample ",
            "git add --chmod -x sample ",
            "git add sample --verbose ",
        ] {
            let target = parse(line).unwrap();
            assert_eq!(target.taken, ["sample"]);
            assert_eq!(target.kind, Kind::File);
        }
        for line in ["git add --chmod=", "git add --chmod "] {
            let target = parse(line).unwrap();
            let rows =
                Completions::default().complete(&target, Path::new("/no-such-directory-here"));
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().any(|c| c.insert.ends_with("+x")));
            assert!(rows.iter().any(|c| c.insert.ends_with("-x")));
        }
        let target = parse("git add -- -sample ").unwrap();
        assert_eq!(target.taken, ["-sample"]);
        assert!(target.add.unwrap().paths_only);
        assert_eq!(parse("git add - sample").unwrap().taken, ["-"]);
        for line in [
            "git add --unknown sample ",
            "git add --chmod=bad ",
            "git add --force=bad ",
            "git add --pathspec-from-file sample extra",
        ] {
            assert!(parse(line).is_none(), "{line}");
        }
    }

    #[test]
    fn add_offers_option_descriptions_and_omits_used_aliases() {
        let f = crate::fixture::Fixture::new(&["sample*"]);
        f.init_git(&[]);
        let mut c = Completions::default();
        let rows = c.complete(&parse("git add ").unwrap(), f.path());
        assert_eq!(rows[0].insert, "sample");
        assert!(
            rows.iter()
                .any(|r| r.insert == "--force" && r.label == "Include ignored files")
        );
        let rows = c.complete(&parse("git add -n --").unwrap(), f.path());
        assert!(!rows.iter().any(|r| r.insert == "--dry-run"));
        assert!(rows.iter().any(|r| r.insert == "--chmod="));
        let rows = c.complete(&parse("git add -- ").unwrap(), f.path());
        assert!(rows.iter().all(|r| r.kind == Kind::File));
    }

    #[test]
    fn add_option_modes_query_the_files_the_option_can_use() {
        let f = crate::fixture::Fixture::new(&["changed*", "deleted*", "clean*"]);
        f.init_git(&[]);
        f.git(&["add", "."]);
        std::fs::write(f.path().join("changed"), "sample").unwrap();
        std::fs::remove_file(f.path().join("deleted")).unwrap();
        std::fs::write(f.path().join("new"), "sample").unwrap();
        std::fs::write(f.path().join("ignored"), "sample").unwrap();
        std::fs::create_dir_all(f.path().join(".git/info")).unwrap();
        std::fs::write(f.path().join(".git/info/exclude"), "ignored\n").unwrap();
        let index = std::fs::read(f.path().join(".git/index")).unwrap();
        let mut c = Completions::default();
        for (line, expected) in [
            ("git add -- ", vec!["changed", "deleted", "new"]),
            (
                "git add -f -- ",
                vec!["changed", "deleted", "ignored", "new"],
            ),
            ("git add -u -- ", vec!["changed", "deleted"]),
            (
                "git add --renormalize -- ",
                vec!["changed", "clean", "deleted"],
            ),
            ("git add --no-all -- ", vec!["changed", "new"]),
        ] {
            let rows = c.complete(&parse(line).unwrap(), f.path());
            assert_eq!(
                rows.iter().map(|r| r.insert.as_str()).collect::<Vec<_>>(),
                expected,
                "{line}"
            );
        }
        assert_eq!(std::fs::read(f.path().join(".git/index")).unwrap(), index);
    }

    #[test]
    fn add_directories_include_empty_folders_and_exclude_selected_subtrees() {
        let f = crate::fixture::Fixture::new(&[
            "sample/nested",
            "sample/file*",
            "sample-other*",
            "empty",
            ".hidden",
        ]);
        f.init_git(&[]);
        let mut c = Completions::default();
        let rows = c.complete(&parse("git add -- ").unwrap(), f.path());
        assert!(rows.iter().any(|r| r.insert == "empty/"));
        assert!(rows.iter().any(|r| r.insert == "sample/"));
        assert!(rows.iter().all(|r| !r.insert.starts_with('.')));
        let rows = c.complete(&parse("git add sample/").unwrap(), f.path());
        assert_eq!(rows[0].insert, "sample/");
        assert!(rows.iter().any(|r| r.insert == "sample/nested/"));
        let rows = c.complete(&parse("git add sample/ -- ").unwrap(), f.path());
        assert!(rows.iter().all(|r| !r.insert.starts_with("sample/")));
        assert!(rows.iter().any(|r| r.insert == "sample-other"));
        let rows = c.complete(&parse("git add sample//./file -- ").unwrap(), f.path());
        assert!(rows.iter().all(|r| r.insert != "sample/file"));
        let rows = c.complete(&parse("git add -- ./ ").unwrap(), f.path());
        assert!(rows.is_empty());
        let rows = c.complete(&parse("git add -- .").unwrap(), f.path());
        assert_eq!(rows[0].insert, ".");
        assert!(rows.iter().any(|r| r.insert == ".hidden/"));
        assert!(rows.iter().all(|r| r.insert != ".git/"));
    }

    #[test]
    fn git_folder_snapshots_exclude_links_and_control_characters() {
        let f = crate::fixture::Fixture::new(&["sample", "sample\u{fffd}", "sample\nline"]);
        f.init_git(&[]);
        std::os::unix::fs::symlink(f.path().join("sample"), f.path().join("sample-link")).unwrap();
        let mut c = Completions::default();
        let target = parse("git add -- samp").unwrap();
        let rows = c.complete(&target, f.path());
        let folders: Vec<_> = rows
            .iter()
            .filter(|r| r.label == "folder")
            .map(|r| r.insert.as_str())
            .collect();
        assert_eq!(folders, ["sample/", "sample\u{fffd}/"]);
        std::fs::create_dir(f.path().join("sample-later")).unwrap();
        assert!(
            c.complete(&target, f.path())
                .iter()
                .all(|r| r.insert != "sample-later/")
        );
        assert!(
            Completions::default()
                .complete(&target, f.path())
                .iter()
                .any(|r| r.insert == "sample-later/")
        );
        // Git refuses a pathspec beyond a link and reads nothing under `.git`.
        // The real folder beside them is what says the rows are missing for
        // that reason rather than for want of a menu.
        for prefix in ["sample-link/", ".git/"] {
            assert!(
                c.complete(&parse(&format!("git add -- {prefix}")).unwrap(), f.path())
                    .is_empty(),
                "{prefix}"
            );
        }
        assert!(
            !c.complete(&parse("git add -- sample/").unwrap(), f.path())
                .is_empty()
        );
    }

    #[test]
    fn add_offers_options_and_folders_without_a_repository() {
        let f = crate::fixture::Fixture::new(&["sample", "sample-file*"]);
        let mut c = Completions::default();
        let rows = c.complete(&parse("git add samp").unwrap(), f.path());
        assert!(
            rows.iter()
                .any(|r| r.insert == "sample/" && r.label == FOLDER)
        );
        let rows = c.complete(&parse("git add --f").unwrap(), f.path());
        assert!(rows.iter().any(|r| r.insert == "--force"));
    }

    #[test]
    fn pathspec_file_arguments_use_filesystem_names_including_clean_files() {
        let f = crate::fixture::Fixture::new(&["sample dir", "sample dir/list*", "sample list*"]);
        f.init_git(&[]);
        f.git(&["add", "."]);
        let mut c = Completions::default();
        for (line, expected) in [
            ("git add --pathspec-from-file sample", "sample list"),
            (
                "git add --pathspec-from-file=sample",
                "--pathspec-from-file=sample list",
            ),
            (
                "git add --pathspec-from-file 'sample dir/",
                "sample dir/list",
            ),
        ] {
            let rows = c.complete(&parse(line).unwrap(), f.path());
            assert!(
                rows.iter()
                    .any(|r| r.insert == expected && r.kind == Kind::Path),
                "{line}"
            );
        }
        let rows = c.complete(
            &parse("git add --pathspec-from-file 'sample list' ").unwrap(),
            f.path(),
        );
        assert!(rows.iter().all(|r| r.kind == Kind::Option));
    }

    #[test]
    fn a_name_that_is_not_utf8_costs_that_name_rather_than_the_answer() {
        assert_eq!(file_names(b"keep\0caf\xe9\0also\0"), ["also", "keep"]);
        assert!(file_names(b"").is_empty());
    }

    #[test]
    fn files_are_unstaged_changes_and_untracked_paths_with_standard_exclusions() {
        use crate::fixture::Fixture;
        use std::fs;
        let f = Fixture::new(&[
            "sample", "fresh", "clean*", "changed*", "deleted*", "staged*",
        ]);
        f.init_git(&[]);
        fs::write(f.path().join(".gitignore"), "ignored\n").unwrap();
        fs::write(f.path().join("sample/.gitignore"), "hidden\n").unwrap();
        f.git(&["add", "."]);
        fs::write(f.path().join("changed"), "sample").unwrap();
        fs::remove_file(f.path().join("deleted")).unwrap();
        for name in [
            "fresh/file",
            "sample/new",
            "ignored",
            "sample/hidden",
            "info-ignored",
        ] {
            fs::write(f.path().join(name), "sample").unwrap();
        }
        fs::create_dir_all(f.path().join(".git/info")).unwrap();
        fs::write(f.path().join(".git/info/exclude"), "info-ignored\n").unwrap();
        let index = fs::read(f.path().join(".git/index")).unwrap();
        assert_eq!(
            read_files(f.path()).unwrap(),
            ["changed", "deleted", "fresh/file", "sample/new"]
        );
        assert_eq!(read_files(&f.path().join("sample")).unwrap(), ["new"]);
        assert_eq!(fs::read(f.path().join(".git/index")).unwrap(), index);
        assert!(!f.path().join(".git/index.lock").exists());
    }

    #[test]
    fn file_names_keep_spaces_and_shell_syntax_but_drop_terminal_controls() {
        use crate::fixture::Fixture;
        let f = Fixture::new(&[]);
        f.git(&[
            "init",
            "--quiet",
            "--template=",
            "--initial-branch=sample-main",
        ]);
        let mut names = vec![
            "sample file",
            "sample$(false)'suffix",
            "-sample",
            "sample[1]",
            "範例",
        ];
        for name in names
            .iter()
            .chain(["sample\nline", "sample\u{1b}[31m"].iter())
        {
            std::fs::write(f.path().join(name), "sample").unwrap();
        }
        names.sort();
        assert_eq!(read_files(f.path()).unwrap(), names);
        assert!(read_files(Fixture::new(&[]).path()).is_none());
    }

    #[test]
    fn file_candidates_rank_whole_paths_and_keep_one_snapshot() {
        use crate::fixture::Fixture;
        let f = Fixture::new(&["sample", "sample/top*", "sample/topic*", "sample/a_top*"]);
        f.init_git(&[]);
        let mut completions = Completions::default();
        let rows = completions.candidates("sample/top", f.path(), Kind::File, &[]);
        assert_eq!(
            rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            ["sample/top", "sample/topic", "sample/a_top"]
        );
        assert!(
            rows.iter()
                .all(|c| c.kind == Kind::File && c.label == "file")
        );
        std::fs::write(f.path().join("sample/new"), "sample").unwrap();
        assert!(
            completions
                .candidates("sample/new", f.path(), Kind::File, &[])
                .is_empty()
        );
        let rows = completions.candidates("./sample/top", f.path(), Kind::File, &[]);
        assert_eq!(rows[0].insert, "./sample/top");
        assert_eq!(
            Completions::default()
                .candidates("sample/new", f.path(), Kind::File, &[])
                .len(),
            1
        );
    }

    #[test]
    fn branches_merge_local_names_and_unambiguous_remote_names() {
        let refs = concat!(
            "refs/heads/sample-main\t\n",
            "refs/heads/sample/topic\t\n",
            "refs/remotes/sample-a/sample/topic\t\n",
            "refs/remotes/sample-a/remote/topic\t\n",
            "refs/remotes/sample-a/shared\t\n",
            "refs/remotes/sample-b/shared\t\n",
            "refs/remotes/sample-a/HEAD\trefs/remotes/sample-a/sample-main\n",
            "refs/remotes/unconfigured/ignored\t\n",
        );
        let config = concat!(
            "remote.sample-a.fetch\n+refs/heads/*:refs/remotes/sample-a/*\0",
            "remote.sample-b.fetch\n+refs/heads/*:refs/remotes/sample-b/*\0",
        );
        assert_eq!(
            branch_names(refs, config, true),
            ["remote/topic", "sample-main", "sample/topic"]
        );
        assert_eq!(
            branch_names(refs, config, false),
            ["sample-main", "sample/topic"]
        );
        let default = format!("{config}checkout.defaultremote\nsample-b\0");
        assert_eq!(
            branch_names(refs, &default, true),
            ["remote/topic", "sample-main", "sample/topic", "shared"]
        );
    }

    #[test]
    fn remote_names_follow_fetch_mappings_and_exclusions() {
        let refs = concat!(
            "refs/remotes/sample/team/topic\t\n",
            "refs/remotes/sample/team/hidden/topic\t\n",
            "refs/remotes/fixed\t\n",
        );
        let config = concat!(
            "remote.sample/team.fetch\n+refs/heads/*:refs/remotes/sample/team/*\0",
            "remote.sample/team.fetch\n^refs/heads/hidden/*\0",
            "remote.sample/team.fetch\nrefs/heads/fixed-topic:refs/remotes/fixed\0",
        );
        assert_eq!(branch_names(refs, config, true), ["fixed-topic", "topic"]);
    }

    #[test]
    fn a_name_the_shell_or_the_terminal_would_read_as_something_else_is_dropped() {
        // Git's own `check-ref-format` refuses all three of these and no
        // repository a test can build will offer one. This function reads
        // strings rather than a repository and a ref written by hand reaches
        // it either way. A leading `-` would arrive at Git as an option and an
        // escape would be the terminal's to obey rather than the menu's to
        // draw.
        let refs = concat!(
            "refs/heads/sample-main\t\n",
            "refs/heads/\t\n",
            "refs/heads/-sample\t\n",
            "refs/heads/sam\u{1b}[31mple\t\n",
        );
        assert_eq!(branch_names(refs, "", false), ["sample-main"]);
    }

    #[test]
    fn branch_ranking_uses_the_whole_name_and_a_cached_snapshot() {
        let mut completions = Completions {
            branches: Some(vec![
                "sample/topic".into(),
                "sample/top".into(),
                "sample/tip".into(),
            ]),
            ..Default::default()
        };
        let rows = completions.candidates(
            "sample/top",
            Path::new("/no-such-directory-here"),
            Kind::Branch,
            &[],
        );
        assert_eq!(
            rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            ["sample/top", "sample/topic"]
        );
        assert!(
            rows.iter()
                .all(|c| c.kind == Kind::Branch && c.label == "branch")
        );
    }
}
