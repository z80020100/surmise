//! Readers for arguments the data could not carry.
//!
//! An argument marked `dyn` in a specification needed code the conversion
//! could not keep, and `specs/dynamic.txt` lists all 4854 of them. This
//! module answers some of them. Nothing here asks a server on its own
//! account. A `docker` line asks the daemon docker's own context names and
//! that is a socket on this machine unless a person pointed it elsewhere.
//!
//! Two tables do it. [`SCRIPTS`] is keyed by the command line a generator
//! runs. 3282 arguments keep that line in the data and lost only the code
//! that read its output. One generator is referenced from many arguments of
//! the same specification. One reader there therefore fills every argument
//! that shares the line. The line is the key and never the program: surmise runs
//! its own copy of a line it has a reader for and nothing a specification
//! carries runs at all. [`READERS`] is keyed by the argument's own identity
//! and reads the files the lost code would have read. It is the door for an
//! argument whose generator carries no command line, and for one whose line
//! cannot say what to read. `npm run`'s argument keeps the line that walks
//! up to a `package.json`. `pnpm remove` and `pnpm run` keep that same line
//! and read opposite halves of the file.

use crate::candidates::{Candidate, DEFAULT_PRIORITY, Kind, SCAN_LIMIT};
use crate::fuzzy;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One reader and the argument it answers for.
///
/// `specs/dynamic.txt` keys an argument by its command, the subcommands and
/// options that reach the argument's owner, and the argument's index.
/// [`crate::argwalk::Walk`] carries neither the owning option nor the index,
/// so the key here is what it does carry: the specification the walk ended
/// in, the name of the node it ended on and the argument's own name.
/// Nothing is lost for any reader below. `make`'s `target` is one argument
/// name in four places in `make.json` — the bare one and the ones behind
/// `-j`, `-B` and `-e` — and all four want the same rows, which is why the
/// four `dynamic.txt` keys collapse to one entry here. `npm`'s `workspace`
/// is the argument of `-w` under 25 subcommands and one entry with no
/// owner answers all of them.
///
/// Another reader is one more entry in [`READERS`] and one more function.
/// A command that gives one argument name two different meanings under
/// one owner is the case this key cannot tell apart. Answering it means
/// teaching `argwalk` to carry the path and the index rather than changing
/// anything here.
struct Reader {
    /// The command whose specification holds the argument.
    command: &'static str,
    /// The subcommands the argument hangs off, by their own first names.
    /// Empty where every node of the specification means the same thing by
    /// that argument's name.
    owners: &'static [&'static str],
    /// The argument's first name, as the specification spells it.
    arg: &'static str,
    /// What a row says it is, where a specification's own row would carry a
    /// description and the reader found nothing better to say.
    label: &'static str,
    read: fn(&Sources) -> Vec<Found>,
}

/// One name a reader found. `label` is what the row says of itself where
/// the reader's own label would say less. A script's body is the case.
#[derive(Clone)]
struct Found {
    name: String,
    label: Option<Cow<'static, str>>,
}

/// Names that say nothing of themselves beyond the reader's own label.
fn unlabelled(names: Vec<String>) -> Vec<Found> {
    names
        .into_iter()
        .map(|name| Found { name, label: None })
        .collect()
}

const READERS: &[Reader] = &[
    Reader {
        command: "make",
        owners: &["make"],
        arg: "target",
        label: "target",
        read: |sources| unlabelled(make_targets(sources)),
    },
    Reader {
        command: "ssh",
        owners: &["ssh"],
        arg: "user@hostname",
        label: "host",
        read: |sources| unlabelled(ssh_hosts(sources)),
    },
    Reader {
        command: "npm",
        owners: &["run"],
        arg: "script",
        label: "script",
        read: npm_scripts,
    },
    // Every other `package` in `npm.json` names something in the registry.
    // `install`, `bugs`, `docs` and `repo` search it and nothing here
    // reaches a network.
    Reader {
        command: "npm",
        owners: &["uninstall", "r", "un", "remove", "unlink", "explore"],
        arg: "package",
        label: "dependency",
        read: npm_dependencies,
    },
    Reader {
        command: "npm",
        owners: &[],
        arg: "workspace",
        label: "workspace",
        read: npm_workspaces,
    },
    // `pnpm`, `yarn` and `bun` read the same file npm does. Each one spells
    // the argument its own way.
    Reader {
        command: "pnpm",
        owners: &["pnpm", "run"],
        arg: "Scripts",
        label: "script",
        read: npm_scripts,
    },
    Reader {
        command: "pnpm",
        owners: &["update", "remove", "link", "unlink", "rebuild"],
        arg: "Package",
        label: "dependency",
        read: npm_dependencies,
    },
    Reader {
        command: "yarn",
        owners: &["run"],
        arg: "script",
        label: "script",
        read: npm_scripts,
    },
    Reader {
        command: "yarn",
        owners: &["upgrade"],
        arg: "package",
        label: "dependency",
        read: npm_dependencies,
    },
    Reader {
        command: "bun",
        owners: &["run"],
        arg: "script",
        label: "script",
        read: npm_scripts,
    },
];

/// A reader for every argument whose generator runs one command line.
struct Script {
    /// The command line, word for word as the generators in `specs/` write
    /// it. It is also what runs.
    line: &'static [&'static str],
    /// What a row says it is where the output says nothing more.
    label: &'static str,
    /// The names the output holds.
    parse: fn(&str) -> Vec<Found>,
}

/// Every line here only reads what the tool already has on this machine.
/// A line that asks a server, such as `gh pr list` or `kubectl get`, has no
/// entry. Neither has one whose output no reader can use as it stands.
/// `git diff --cached --name-only` names its paths from the top of the
/// repository and a line typed in a subdirectory would get the wrong file.
/// `cargo metadata` without `--no-deps` can fetch an index to resolve the
/// dependencies and has none either.
const SCRIPTS: &[Script] = &[
    Script {
        line: &["git", "--no-optional-locks", "log", "--oneline"],
        label: "commit",
        parse: oneline,
    },
    Script {
        line: &["git", "rev-list", "--all", "--oneline"],
        label: "commit",
        parse: oneline,
    },
    Script {
        line: &[
            "git",
            "--no-optional-locks",
            "branch",
            "--no-color",
            "--sort=-committerdate",
        ],
        label: "branch",
        parse: branches,
    },
    Script {
        line: &[
            "git",
            "--no-optional-locks",
            "branch",
            "-a",
            "--no-color",
            "--sort=-committerdate",
        ],
        label: "branch",
        parse: branches,
    },
    Script {
        line: &[
            "git",
            "--no-optional-locks",
            "branch",
            "-r",
            "--no-color",
            "--sort=-committerdate",
        ],
        label: "branch",
        parse: branches,
    },
    Script {
        line: &["git", "branch", "--no-color"],
        label: "branch",
        parse: branches,
    },
    Script {
        line: &["git", "--no-optional-locks", "remote", "-v"],
        label: "remote",
        parse: remotes,
    },
    Script {
        line: &["git", "--no-optional-locks", "status", "--short"],
        label: "file",
        parse: changed_paths,
    },
    Script {
        line: &["git", "--no-optional-locks", "stash", "list"],
        label: "stash",
        parse: named_lines,
    },
    Script {
        line: &[
            "git",
            "--no-optional-locks",
            "tag",
            "--list",
            "--sort=-committerdate",
        ],
        label: "tag",
        parse: |out| {
            out.lines()
                .filter(|l| !l.is_empty())
                .map(name_only)
                .collect()
        },
    },
    Script {
        line: &[
            "git",
            "--no-optional-locks",
            "config",
            "--get-regexp",
            "^alias.",
        ],
        label: "alias",
        parse: aliases,
    },
    Script {
        line: &["git", "config", "--get-regexp", ".*"],
        label: "config",
        parse: config_keys,
    },
    Script {
        line: &["docker", "ps", "--format", "{{ json . }}"],
        label: "container",
        parse: containers,
    },
    Script {
        line: &["docker", "ps", "-a", "--format", "{{ json . }}"],
        label: "container",
        parse: containers,
    },
    Script {
        line: &[
            "docker",
            "ps",
            "--filter",
            "status=paused",
            "--format",
            "{{ json . }}",
        ],
        label: "container",
        parse: containers,
    },
    Script {
        line: &["docker", "image", "ls", "--format", "{{ json . }}"],
        label: "image",
        parse: images,
    },
    Script {
        line: &["docker", "images", "-a", "--format", "{{ json . }}"],
        label: "image",
        parse: images,
    },
    Script {
        line: &[
            "docker",
            "images",
            "--format",
            "{{.Repository}} {{.Size}} {{.Tag}} {{.ID}}",
        ],
        label: "image",
        parse: |out| {
            out.lines()
                .filter_map(|line| {
                    let mut words = line.split(' ');
                    let (repository, size, tag) = (words.next()?, words.next()?, words.next()?);
                    image(repository, tag, Some(size))
                })
                .collect()
        },
    },
    Script {
        line: &["docker", "service", "list", "--format", "{{ json . }}"],
        label: "service",
        parse: |out| json_lines(out, "Name", "Image"),
    },
    Script {
        line: &["docker", "node", "list", "--format", "{{ json . }}"],
        label: "node",
        parse: |out| json_lines(out, "Hostname", "Status"),
    },
    Script {
        line: &["docker", "plugin", "list", "--format", "{{ json . }}"],
        label: "plugin",
        parse: |out| json_lines(out, "Name", "Description"),
    },
    Script {
        line: &["docker", "context", "list", "--format", "{{ json . }}"],
        label: "context",
        parse: |out| json_lines(out, "Name", "Description"),
    },
    Script {
        line: &["docker", "network", "list", "--format", "{{ json . }}"],
        label: "network",
        parse: |out| json_lines(out, "Name", "Driver"),
    },
    Script {
        line: &["docker", "stack", "list", "--format", "{{ json . }}"],
        label: "stack",
        parse: |out| json_lines(out, "Name", "Services"),
    },
    Script {
        line: &["docker", "secret", "list", "--format", "{{ json . }}"],
        label: "secret",
        parse: |out| json_lines(out, "Name", ""),
    },
    Script {
        line: &["docker", "volume", "list", "--format", "{{ json . }}"],
        label: "volume",
        parse: |out| json_lines(out, "Name", "Driver"),
    },
    Script {
        line: &["docker", "volume", "ls", "--format", "{{ json . }}"],
        label: "volume",
        parse: |out| json_lines(out, "Name", "Driver"),
    },
    Script {
        line: &["tmux", "ls"],
        label: "session",
        parse: named_lines,
    },
    Script {
        line: &["tmux", "lsw"],
        label: "window",
        parse: named_lines,
    },
    Script {
        line: &["tmux", "lsp"],
        label: "pane",
        parse: named_lines,
    },
    Script {
        line: &["tmux", "lsc"],
        label: "client",
        parse: named_lines,
    },
    Script {
        line: &["tmux", "lsb"],
        label: "buffer",
        parse: named_lines,
    },
    Script {
        line: &["brew", "list", "-1"],
        label: "installed",
        parse: plain_lines,
    },
    Script {
        line: &["brew", "list", "-1", "--cask"],
        label: "installed cask",
        parse: plain_lines,
    },
    Script {
        line: &["brew", "tap"],
        label: "tap",
        parse: plain_lines,
    },
    Script {
        line: &["rustc", "--print", "target-list"],
        label: "target",
        parse: plain_lines,
    },
    Script {
        line: &["cargo", "metadata", "--format-version", "1", "--no-deps"],
        label: "package",
        parse: cargo_packages,
    },
    Script {
        line: &["cargo", "read-manifest"],
        label: "feature",
        parse: cargo_features,
    },
];

/// What each command line answered, for the life of one menu. A line runs
/// once however many keys follow, the way Git's own readers and the
/// directory scan already do. A line that failed stays an empty answer.
#[derive(Default)]
pub(crate) struct Runs(HashMap<&'static [&'static str], Vec<Found>>);

/// The rows the command line `script` offers, where [`SCRIPTS`] has a reader
/// for it. `script` is a generator's own field. Only the list form names
/// its words without a shell between them and only that form is matched.
pub(crate) fn script_rows(
    script: &Value,
    term: &str,
    cwd: &Path,
    runs: &mut Runs,
) -> Vec<Candidate> {
    let Some(words) = script.as_array() else {
        return Vec::new();
    };
    let Some(entry) = SCRIPTS.iter().find(|entry| same_line(words, entry.line)) else {
        return Vec::new();
    };
    let found = runs.0.entry(entry.line).or_insert_with(|| run(entry, cwd));
    to_rows(found.iter(), entry.label, term)
}

/// Whether a generator's `words` are `line` word for word.
fn same_line(words: &[Value], line: &[&str]) -> bool {
    words.len() == line.len()
        && words
            .iter()
            .zip(line)
            .all(|(word, want)| word.as_str() == Some(*want))
}

/// What `entry`'s line prints in `cwd` within the budget Git's own queries
/// keep. A line that fails, runs past that or prints past its cap answers
/// nothing and prints nothing at the prompt.
fn run(entry: &Script, cwd: &Path) -> Vec<Found> {
    let [program, args @ ..] = entry.line else {
        return Vec::new();
    };
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    match crate::git::read_output(&mut command, crate::git::timeout()) {
        Some((status, out)) if status.success() => (entry.parse)(&out),
        _ => Vec::new(),
    }
}

/// `abc1234 Subject line`: the hash and what the commit says.
fn oneline(out: &str) -> Vec<Found> {
    out.lines()
        .filter_map(|line| {
            let (hash, subject) = line.split_once(' ').unwrap_or((line, ""));
            (!hash.is_empty()).then(|| Found {
                name: hash.to_string(),
                label: (!subject.is_empty()).then(|| Cow::Owned(subject.to_string())),
            })
        })
        .collect()
}

/// `git branch`'s own list. The mark in front of the current branch and of
/// one checked out in another worktree comes off. A detached `HEAD` names
/// no branch and a symbolic remote `HEAD` points at a branch the list
/// already holds. A remote branch loses its `remotes/` and keeps its
/// remote's name.
fn branches(out: &str) -> Vec<Found> {
    let names = out
        .lines()
        .map(|line| line.get(2..).unwrap_or("").trim())
        .filter(|name| !name.is_empty() && !name.starts_with('(') && !name.contains(" -> "))
        .map(|name| name.strip_prefix("remotes/").unwrap_or(name).to_string())
        .collect();
    unlabelled(names)
}

/// `origin\turl (fetch)` and the same remote again for `(push)`. One row
/// per remote, with the address it fetches from.
fn remotes(out: &str) -> Vec<Found> {
    let mut seen = HashSet::new();
    out.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            let url = rest.split(' ').next().unwrap_or("");
            seen.insert(name.to_string()).then(|| Found {
                name: name.to_string(),
                label: (!url.is_empty()).then(|| Cow::Owned(url.to_string())),
            })
        })
        .collect()
}

/// `git status --short`: two status letters, a space and the path. A rename
/// names the path it went to. Git puts a path holding an unusual character
/// in quotes with its own escapes, and such a path is left out rather than
/// read back wrong.
fn changed_paths(out: &str) -> Vec<Found> {
    out.lines()
        .filter_map(|line| {
            let path = line.get(3..)?;
            let path = path.rsplit_once(" -> ").map_or(path, |(_, to)| to);
            (!path.is_empty() && !path.starts_with('"')).then(|| name_only(path))
        })
        .collect()
}

/// `alias.co checkout`: the alias and what it stands for.
fn aliases(out: &str) -> Vec<Found> {
    out.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            let name = key.strip_prefix("alias.")?;
            Some(Found {
                name: name.to_string(),
                label: (!value.is_empty()).then(|| Cow::Owned(value.to_string())),
            })
        })
        .collect()
}

/// `user.name Sample`: each key once. A value can hold a token or a
/// password and no row says what one is set to.
fn config_keys(out: &str) -> Vec<Found> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for line in out.lines() {
        push_unique(line.split(' ').next().unwrap_or(""), &mut seen, &mut names);
    }
    unlabelled(names)
}

/// One name per line, as `brew list -1` and `rustc --print target-list`
/// print them.
fn plain_lines(out: &str) -> Vec<Found> {
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(name_only)
        .collect()
}

/// One JSON object per line, the shape every `--format '{{ json . }}'`
/// prints. `name` is the field a row is named by and `about` the one it says
/// of itself, where it says anything.
fn json_lines(out: &str, name: &str, about: &str) -> Vec<Found> {
    objects(out)
        .filter_map(|object| {
            let found = text_field(&object, name)?;
            Some(Found {
                name: found.to_string(),
                label: text_field(&object, about).map(|t| Cow::Owned(t.to_string())),
            })
        })
        .collect()
}

fn objects(out: &str) -> impl Iterator<Item = Map<String, Value>> + '_ {
    out.lines()
        .filter_map(|line| serde_json::from_str::<Map<String, Value>>(line).ok())
}

/// A field that holds text, or `None` for one that is absent, empty or not
/// text at all.
fn text_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// A container under the first of its names and with what it runs and how
/// it stands.
fn containers(out: &str) -> Vec<Found> {
    objects(out)
        .filter_map(|object| {
            let name = text_field(&object, "Names")?.split(',').next()?;
            let image = text_field(&object, "Image").unwrap_or("");
            let status = text_field(&object, "Status").unwrap_or("");
            Some(Found {
                name: name.to_string(),
                label: Some(Cow::Owned(format!("{image} {status}").trim().to_string())),
            })
        })
        .collect()
}

fn images(out: &str) -> Vec<Found> {
    objects(out)
        .filter_map(|object| {
            image(
                text_field(&object, "Repository")?,
                text_field(&object, "Tag")?,
                text_field(&object, "Size"),
            )
        })
        .collect()
}

/// An image under `repository:tag`, the name docker itself takes, and its
/// size. One that lost its repository or its tag has no such name and is
/// left out.
fn image(repository: &str, tag: &str, size: Option<&str>) -> Option<Found> {
    if repository == "<none>" || tag == "<none>" {
        return None;
    }
    Some(Found {
        name: format!("{repository}:{tag}"),
        label: size.map(|size| Cow::Owned(size.to_string())),
    })
}

/// `name: what the tool says of it`, the shape `git stash list` and every
/// `tmux` list print. `stash@{0}: WIP on main` is the stash and what it
/// holds. A window leads with its index rather than its name. tmux writes
/// its flags straight after the name and a name can end in one of them.
/// Every argument the window list fills but `break-pane -n` is a target and
/// an index names one.
fn named_lines(out: &str) -> Vec<Found> {
    out.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(": ")?;
            Some(Found {
                name: name.to_string(),
                label: Some(Cow::Owned(rest.to_string())),
            })
        })
        .collect()
}

/// The packages `cargo metadata` describes, each under its own name.
fn cargo_packages(out: &str) -> Vec<Found> {
    let Ok(metadata) = serde_json::from_str::<Value>(out) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        push_unique(
            package["name"].as_str().unwrap_or(""),
            &mut seen,
            &mut names,
        );
    }
    unlabelled(names)
}

/// The features the manifest in front of the line declares.
fn cargo_features(out: &str) -> Vec<Found> {
    let Ok(manifest) = serde_json::from_str::<Value>(out) else {
        return Vec::new();
    };
    let names = manifest["features"]
        .as_object()
        .map(|features| features.keys().cloned().collect())
        .unwrap_or_default();
    unlabelled(names)
}

fn name_only(name: &str) -> Found {
    Found {
        name: name.to_string(),
        label: None,
    }
}

/// Where a reader is allowed to look. The environment is read at one edge
/// and every reader takes its paths from here, which is what lets a test
/// hand one a temporary home rather than the person's own. `path` keeps
/// `$HOME` behind a twin for that same reason.
struct Sources<'a> {
    /// The directory the line is being typed in.
    cwd: &'a Path,
    /// Every word on the line in front of the one being typed. A reader for
    /// an argument that takes several words leaves out a name already there.
    line: &'a [&'a str],
    /// `$HOME`, or an empty path when the environment says nothing.
    home: &'a Path,
    /// The system-wide SSH configuration.
    ssh_config: &'a Path,
}

const SYSTEM_SSH_CONFIG: &str = "/etc/ssh/ssh_config";

/// The rows a native reader offers for one `dyn` argument, or none where the
/// table has no reader for it. `command` is the first name of the
/// specification the walk ended in, `owner` is the name of the node it
/// ended on and `arg` is the argument's own first name. `line` is every word
/// in front of the one being typed.
pub(crate) fn rows(
    command: &str,
    owner: &str,
    arg: &str,
    term: &str,
    cwd: &Path,
    line: &[&str],
) -> Vec<Candidate> {
    let Some(reader) = reader(command, owner, arg) else {
        return Vec::new();
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let sources = Sources {
        cwd,
        line,
        home: Path::new(&home),
        ssh_config: Path::new(SYSTEM_SSH_CONFIG),
    };
    candidates(reader, &sources, term)
}

fn reader(command: &str, owner: &str, arg: &str) -> Option<&'static Reader> {
    READERS.iter().find(|r| {
        r.command == command && r.arg == arg && (r.owners.is_empty() || r.owners.contains(&owner))
    })
}

fn candidates(reader: &Reader, sources: &Sources, term: &str) -> Vec<Candidate> {
    to_rows((reader.read)(sources).iter(), reader.label, term)
}

/// One row per name `term` reaches, under `label` where the name brought
/// none of its own.
fn to_rows<'a>(
    found: impl Iterator<Item = &'a Found>,
    label: &'static str,
    term: &str,
) -> Vec<Candidate> {
    found
        .filter_map(|Found { name, label: own }| {
            // The row would show such a name with the character stripped and
            // insert it whole. Git's own readers leave one out too.
            if name.chars().any(char::is_control) {
                return None;
            }
            let score = fuzzy::score(term, name)?;
            Some(Candidate {
                display: name.clone(),
                insert: name.clone(),
                // A target and a host are both flat values rather than paths
                // on disk, which is what `spec_menu` already gives a
                // specification's own fixed suggestions: no folder glyph, no
                // trailing slash and plain shell quoting.
                kind: Kind::Path,
                label: own.clone().unwrap_or(Cow::Borrowed(label)),
                hint: Vec::new(),
                score,
                priority: DEFAULT_PRIORITY,
                cursor: None,
                verbatim: false,
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

/// The first [`READ_LIMIT`] bytes of `path`. `None` for a file that is
/// absent, that will not read or that is not a regular one.
/// [`crate::path::regular`] says why.
fn read_capped(path: &Path) -> Option<Vec<u8>> {
    crate::path::regular(path).ok()?;
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The first [`READ_LIMIT`] bytes of `path`, as whole lines. A read that
/// stops at the limit stops in the middle of a line as often as not, and
/// that last partial line is dropped, because half a name is worse than a
/// missing one. An absent or unreadable file gives nothing and never an
/// error. This runs at a prompt and an error has nowhere to go.
fn read_lines(path: &Path) -> Vec<String> {
    let Some(bytes) = read_capped(path) else {
        return Vec::new();
    };
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
    // A configuration lists its hosts in the order a person wrote them.
    // `known_hosts` lists them in the order ssh first met them and that says
    // nothing.
    let configured = out.len();
    let known_hosts = sources.home.join(".ssh").join("known_hosts");
    for line in read_lines(&known_hosts) {
        for name in known_hosts_names(&line) {
            if !is_pattern(name) {
                push_unique(name, &mut seen, &mut out);
            }
        }
    }
    out[configured..].sort();
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

/// The `package.json` nearest `cwd` and the directory holding it. The walk
/// goes up to the root, the way npm finds the project a line in a
/// subdirectory belongs to. A file past [`READ_LIMIT`] is cut short and no
/// longer parses. It gives nothing rather than half its names.
fn package_json(cwd: &Path) -> Option<(PathBuf, Map<String, Value>)> {
    let (dir, path) = cwd
        .ancestors()
        .map(|dir| (dir, dir.join("package.json")))
        .find(|(_, path)| path.is_file())?;
    match serde_json::from_slice(&read_capped(&path)?).ok()? {
        Value::Object(map) => Some((dir.to_path_buf(), map)),
        _ => None,
    }
}

/// The scripts `npm run` can run, each described by what it runs.
fn npm_scripts(sources: &Sources) -> Vec<Found> {
    let Some((_, package)) = package_json(sources.cwd) else {
        return Vec::new();
    };
    let Some(Value::Object(scripts)) = package.get("scripts") else {
        return Vec::new();
    };
    scripts
        .iter()
        .filter_map(|(name, body)| {
            Some(Found {
                name: name.clone(),
                label: Some(Cow::Owned(body.as_str()?.to_string())),
            })
        })
        .collect()
}

/// The three lists of `package.json` a package can be uninstalled from, and
/// what a row from each says it is.
const DEPENDENCIES: [(&str, &str); 3] = [
    ("dependencies", "dependency"),
    ("devDependencies", "dev dependency"),
    ("optionalDependencies", "optional dependency"),
];

/// The packages this project depends on, less the ones already on the line.
///
/// `-g` asks about the packages installed for the whole machine instead.
/// Only running npm can say where those live. That line therefore gets
/// nothing rather than the project's own list.
fn npm_dependencies(sources: &Sources) -> Vec<Found> {
    if sources.line.iter().any(|w| matches!(*w, "-g" | "--global")) {
        return Vec::new();
    }
    let Some((_, package)) = package_json(sources.cwd) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (list, label) in DEPENDENCIES {
        let Some(Value::Object(names)) = package.get(list) else {
            continue;
        };
        for name in names.keys() {
            if !sources.line.contains(&name.as_str()) && seen.insert(name) {
                out.push(Found {
                    name: name.clone(),
                    label: Some(Cow::Borrowed(label)),
                });
            }
        }
    }
    // The three lists make one menu and their order says nothing. Each
    // `serde_json` map is already in the order of its keys.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Whether a `workspaces` entry is a pattern rather than a path.
fn is_glob(entry: &str) -> bool {
    entry.contains(['*', '?', '[', '{', '!'])
}

/// A workspace path as two spellings of it compare. A leading `./` names
/// the same directory as no prefix at all.
fn plain_path(path: &str) -> &str {
    path.trim_start_matches("./")
}

/// The workspaces `-w` can name, less the ones already on the line.
///
/// An entry is a path or a pattern. `-w` takes a path and a pattern
/// matches none. `packages/*` therefore becomes every directory under
/// `packages` that holds a `package.json` of its own. That is by far the
/// commonest shape. An entry with a leading `!` takes a path back out. Any
/// other pattern is left out rather than guessed at.
fn npm_workspaces(sources: &Sources) -> Vec<Found> {
    let Some((root, package)) = package_json(sources.cwd) else {
        return Vec::new();
    };
    let Some(Value::Array(entries)) = package.get("workspaces") else {
        return Vec::new();
    };
    // Every spelling is compared by the directory it reaches. `packages/*`
    // and `./packages/sample/` both reach `packages/sample`. One such
    // directory is one row under the first spelling the file gives it.
    let reach = |path: &str| {
        let dir = root.join(plain_path(path).trim_end_matches('/'));
        dir.canonicalize().unwrap_or(dir)
    };
    let removed: Vec<PathBuf> = entries
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|entry| entry.strip_prefix('!'))
        .filter(|path| !is_glob(path))
        .map(reach)
        .collect();
    let on_line: Vec<PathBuf> = sources.line.iter().map(|word| reach(word)).collect();
    let left_out = |path: &str| {
        let key = reach(path);
        removed.contains(&key) || on_line.contains(&key)
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut offer = |path: &str| {
        if !path.is_empty() && seen.insert(reach(path)) {
            out.push(path.to_string());
        }
    };
    for entry in entries.iter().filter_map(Value::as_str) {
        match entry.strip_suffix("/*") {
            Some(parent) if !is_glob(parent) => {
                for child in workspace_dirs(&root.join(parent)) {
                    let path = format!("{parent}/{child}");
                    if !left_out(&path) {
                        offer(&path);
                    }
                }
            }
            _ if !is_glob(entry) && !left_out(entry) => offer(entry),
            _ => {}
        }
    }
    unlabelled(out)
}

/// The directories under `dir` that hold a `package.json`, by name. A name
/// with a leading dot is one `*` does not match.
fn workspace_dirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            (!name.starts_with('.') && entry.path().join("package.json").is_file()).then_some(name)
        })
        .take(SCAN_LIMIT)
        .collect();
    // A directory listing has no order of its own and the menu keeps the
    // order a reader gives it.
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, soon};
    use crate::spec::Subcommand;

    /// Every host name below is invented. A test that read the real
    /// `~/.ssh` would put a person's own hosts into the suite's output, so
    /// `home` is always a fixture and never `$HOME`.
    fn sources<'a>(cwd: &'a Path, home: &'a Path, ssh_config: &'a Path) -> Sources<'a> {
        Sources {
            cwd,
            line: &[],
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
    fn an_ssh_file_that_is_a_fifo_is_never_opened() {
        let f = ssh_home(None, None);
        f.fifo(".ssh/config");
        let home = f.path().to_path_buf();
        let hosts = soon(move || ssh_hosts(&sources(nowhere(), &home, nowhere())));
        assert_eq!(hosts, Some(Vec::new()));
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
        // `known_hosts` says nothing by its order and the names come sorted.
        assert_eq!(ssh_hosts(&s), ["10.0.0.1", "build-box", "sample-host"]);
    }

    #[test]
    fn no_ssh_files_give_no_hosts() {
        let f = Fixture::new(&[]);
        let s = sources(nowhere(), f.path(), nowhere());
        assert!(ssh_hosts(&s).is_empty());
    }

    #[test]
    fn the_table_answers_for_its_own_arguments_and_for_nothing_else() {
        assert!(reader("make", "make", "target").is_some());
        assert!(reader("ssh", "ssh", "user@hostname").is_some());
        assert!(reader("npm", "run", "script").is_some());
        assert!(reader("npm", "r", "package").is_some());
        // An entry with no owner answers under every node of its command.
        assert!(reader("npm", "audit", "workspace").is_some());
        assert!(reader("make", "make", "no-such-argument").is_none());
        assert!(reader("chown", "chown", "owner").is_none());
        // `install`'s own `package` searches the registry.
        assert!(reader("npm", "install", "package").is_none());
        // The same pair under another command's specification is not this.
        assert!(reader("cargo", "run", "script").is_none());
    }

    /// Every argument one entry of [`READERS`] can reach, by the name of the
    /// node it hangs off. An option's own argument hangs off the node the
    /// option sits on. `spec_menu` hands the table that same node.
    fn dynamic_owners<'a>(node: &'a Subcommand, arg: &str, out: &mut Vec<&'a str>) {
        let own = node.args.iter();
        let options = node
            .options
            .values()
            .chain(node.persistent_options.values());
        if own
            .chain(options.flat_map(|opt| opt.args.iter()))
            .any(|a| a.dynamic && a.name.first().is_some_and(|n| n == arg))
            && let Some(name) = node.name.first()
        {
            out.push(name);
        }
        for child in node.subcommands.values() {
            dynamic_owners(child, arg, out);
        }
    }

    #[test]
    fn every_reader_still_matches_an_argument_the_committed_data_marks_dyn() {
        // `make specs` rewrites `specs/` from whatever the upstream wrote. A
        // regeneration that renames an argument or drops its `dyn` would
        // turn a reader off with nothing to say so. This is what says so.
        for reader in READERS {
            let spec = crate::spec::load(reader.command, &[]).expect(reader.command);
            let mut owners = Vec::new();
            dynamic_owners(&spec, reader.arg, &mut owners);
            assert!(!owners.is_empty(), "{} {}", reader.command, reader.arg);
            for owner in reader.owners {
                assert!(
                    owners.contains(owner),
                    "{} {owner} {}",
                    reader.command,
                    reader.arg
                );
            }
        }
    }

    /// Whether `value` holds a generator whose `script` is `line` anywhere
    /// inside it.
    fn holds_script(value: &Value, line: &[&str]) -> bool {
        match value {
            Value::Object(map) => {
                map.get("script")
                    .and_then(Value::as_array)
                    .is_some_and(|words| same_line(words, line))
                    || map.values().any(|v| holds_script(v, line))
            }
            Value::Array(items) => items.iter().any(|v| holds_script(v, line)),
            _ => false,
        }
    }

    #[test]
    fn every_script_reader_still_matches_a_line_the_committed_data_runs() {
        // The same guard as the one above for the other table. A line the
        // upstream rewrites would leave its reader matching nothing.
        let mut files = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("specs")];
        let mut specs = Vec::new();
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if path.extension().is_some_and(|e| e == "json") {
                specs
                    .push(serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap());
            }
        }
        for entry in SCRIPTS {
            assert!(
                specs.iter().any(|spec| holds_script(spec, entry.line)),
                "{:?}",
                entry.line
            );
        }
    }

    #[test]
    fn a_line_with_no_reader_never_runs() {
        let mut runs = Runs::default();
        let line = serde_json::json!(["sh", "-c", "touch sample"]);
        let f = Fixture::new(&[]);
        assert!(script_rows(&line, "", f.path(), &mut runs).is_empty());
        assert!(!f.path().join("sample").exists());
        assert!(runs.0.is_empty());
        // Only the list form names its words without a shell between them.
        let text = serde_json::json!("git --no-optional-locks log --oneline");
        assert!(script_rows(&text, "", f.path(), &mut runs).is_empty());
    }

    #[test]
    fn a_line_runs_once_per_menu_and_answers_from_the_repository() {
        let f = Fixture::new(&[]);
        f.init_git(&["sample-topic"]);
        let line = serde_json::json!([
            "git",
            "--no-optional-locks",
            "branch",
            "--no-color",
            "--sort=-committerdate"
        ]);
        let mut runs = Runs::default();
        let rows = script_rows(&line, "", f.path(), &mut runs);
        let mut names: Vec<&str> = rows.iter().map(|r| r.insert.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["sample-main", "sample-topic"]);
        // A branch made now is the next menu's to find.
        f.git(&["branch", "sample-later", "sample-main"]);
        assert_eq!(script_rows(&line, "", f.path(), &mut runs).len(), 2);
        // Outside a repository Git fails and the line answers nothing.
        let outside = Fixture::new(&[]);
        assert!(script_rows(&line, "", outside.path(), &mut Runs::default()).is_empty());
    }

    #[test]
    fn each_git_line_reads_its_own_output() {
        let names = |found: Vec<Found>| found.into_iter().map(|f| f.name).collect::<Vec<_>>();
        let log = oneline("abc1234 Sample subject\ndef5678 Another\n");
        assert_eq!(log[0].name, "abc1234");
        assert_eq!(log[0].label.as_deref(), Some("Sample subject"));
        assert_eq!(
            names(branches(
                "* sample-main\n+ sample-tree\n  sample-topic\n  (HEAD detached at abc1234)\n  remotes/origin/HEAD -> origin/sample-main\n  remotes/origin/sample-topic\n"
            )),
            [
                "sample-main",
                "sample-tree",
                "sample-topic",
                "origin/sample-topic"
            ]
        );
        let remote = remotes(
            "origin\thttps://example.invalid/sample.git (fetch)\norigin\thttps://example.invalid/sample.git (push)\n",
        );
        assert_eq!(remote.len(), 1);
        assert_eq!(
            remote[0].label.as_deref(),
            Some("https://example.invalid/sample.git")
        );
        assert_eq!(
            names(changed_paths(
                " M sample.txt\nR  old.txt -> new.txt\n?? \"odd name\"\n"
            )),
            ["sample.txt", "new.txt"]
        );
        let alias = aliases("alias.co checkout\nalias.st status --short\n");
        assert_eq!(alias[1].name, "st");
        assert_eq!(alias[1].label.as_deref(), Some("status --short"));
        assert_eq!(
            names(config_keys(
                "user.name Sample\nremote.origin.fetch +a\nremote.origin.fetch +b\n"
            )),
            ["user.name", "remote.origin.fetch"]
        );
        assert!(config_keys("credential.sample secret\n")[0].label.is_none());
        let stash = named_lines("stash@{0}: WIP on sample-main: abc1234 Sample\n");
        assert_eq!(stash[0].name, "stash@{0}");
        assert_eq!(
            stash[0].label.as_deref(),
            Some("WIP on sample-main: abc1234 Sample")
        );
    }

    #[test]
    fn each_tool_line_reads_its_own_output() {
        let names = |found: Vec<Found>| found.into_iter().map(|f| f.name).collect::<Vec<_>>();
        let ps = containers(
            "{\"Names\":\"sample-web,sample-alias\",\"Image\":\"sample:1\",\"Status\":\"Up 2 hours\"}\nnot json\n",
        );
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "sample-web");
        assert_eq!(ps[0].label.as_deref(), Some("sample:1 Up 2 hours"));
        assert_eq!(
            names(images(
                "{\"Repository\":\"sample\",\"Tag\":\"1\",\"Size\":\"5MB\"}\n{\"Repository\":\"<none>\",\"Tag\":\"<none>\"}\n"
            )),
            ["sample:1"]
        );
        let networks = json_lines(
            "{\"Name\":\"sample-net\",\"Driver\":\"bridge\"}\n",
            "Name",
            "Driver",
        );
        assert_eq!(networks[0].label.as_deref(), Some("bridge"));
        assert_eq!(
            names(named_lines(
                "sample: 2 windows (created Thu)\nother: 1 windows\n"
            )),
            ["sample", "other"]
        );
        let windows = named_lines("0: sample-VIM* (1 panes) [80x24]\n");
        assert_eq!(windows[0].name, "0");
        assert_eq!(
            windows[0].label.as_deref(),
            Some("sample-VIM* (1 panes) [80x24]")
        );
        assert_eq!(
            names(plain_lines("sample\n\n  other  \n")),
            ["sample", "other"]
        );
        assert_eq!(
            names(cargo_packages(
                "{\"packages\":[{\"name\":\"sample\"},{\"name\":\"sample\"},{\"name\":\"other\"}]}"
            )),
            ["sample", "other"]
        );
        let mut features = names(cargo_features(
            "{\"features\":{\"sample\":[],\"default\":[\"sample\"]}}",
        ));
        features.sort_unstable();
        assert_eq!(features, ["default", "sample"]);
        assert!(cargo_packages("not json").is_empty());
    }

    #[test]
    fn pnpm_yarn_and_bun_read_the_same_package_json_npm_does() {
        assert!(reader("pnpm", "run", "Scripts").is_some());
        assert!(reader("pnpm", "pnpm", "Scripts").is_some());
        assert!(reader("pnpm", "remove", "Package").is_some());
        assert!(reader("yarn", "run", "script").is_some());
        assert!(reader("yarn", "upgrade", "package").is_some());
        assert!(reader("bun", "run", "script").is_some());
        // `add` and `install` search the registry.
        assert!(reader("pnpm", "add", "package").is_none());
        assert!(reader("yarn", "add", "package").is_none());
    }

    #[test]
    fn an_argument_with_no_reader_gives_no_rows() {
        assert!(rows("chown", "chown", "owner", "", nowhere(), &[]).is_empty());
    }

    /// A project directory holding `package` as its `package.json`. Every
    /// name in one is invented.
    fn project(package: &str) -> Fixture {
        let f = Fixture::new(&[]);
        std::fs::write(f.path().join("package.json"), package).unwrap();
        f
    }

    fn found_names(found: &[Found]) -> Vec<&str> {
        found.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn the_scripts_come_with_what_they_run() {
        let f = project(
            r#"{"scripts": {"sample-build": "sample-tool build", "sample-test": "sample-tool test"}}"#,
        );
        let found = npm_scripts(&sources(f.path(), nowhere(), nowhere()));
        assert_eq!(found_names(&found), ["sample-build", "sample-test"]);
        assert_eq!(found[0].label.as_deref(), Some("sample-tool build"));
    }

    #[test]
    fn a_line_in_a_subdirectory_reads_the_project_above_it() {
        let f = project(r#"{"scripts": {"sample-build": "sample-tool build"}}"#);
        std::fs::create_dir_all(f.path().join("src/inner")).unwrap();
        let cwd = f.path().join("src/inner");
        let found = npm_scripts(&sources(&cwd, nowhere(), nowhere()));
        assert_eq!(found_names(&found), ["sample-build"]);
    }

    #[test]
    fn no_package_json_and_a_broken_one_give_no_scripts() {
        // A fixture sits under the temporary directory and the walk would
        // read any `package.json` above it. The root holds none.
        assert!(npm_scripts(&sources(nowhere(), nowhere(), nowhere())).is_empty());
        let f = project(r#"{"scripts": {"sample-build": "#);
        assert!(npm_scripts(&sources(f.path(), nowhere(), nowhere())).is_empty());
    }

    const DEPENDING: &str = r#"{
        "dependencies": {"sample-lib": "^1.0.0"},
        "devDependencies": {"sample-dev": "^2.0.0", "sample-lib": "^1.0.0"},
        "optionalDependencies": {"sample-extra": "^3.0.0"}
    }"#;

    #[test]
    fn every_list_a_package_can_be_uninstalled_from_is_read_once() {
        let f = project(DEPENDING);
        let found = npm_dependencies(&sources(f.path(), nowhere(), nowhere()));
        assert_eq!(
            found_names(&found),
            ["sample-dev", "sample-extra", "sample-lib"]
        );
        assert_eq!(found[0].label.as_deref(), Some("dev dependency"));
        // The first list that names a package is the one its row says.
        assert_eq!(found[2].label.as_deref(), Some("dependency"));
    }

    #[test]
    fn a_dependency_already_on_the_line_is_left_out() {
        let f = project(DEPENDING);
        let line = ["npm", "uninstall", "sample-lib"];
        let s = Sources {
            line: &line,
            ..sources(f.path(), nowhere(), nowhere())
        };
        assert_eq!(
            found_names(&npm_dependencies(&s)),
            ["sample-dev", "sample-extra"]
        );
    }

    #[test]
    fn a_global_uninstall_gets_no_project_dependency() {
        let f = project(DEPENDING);
        for flag in ["-g", "--global"] {
            let line = ["npm", "uninstall", flag];
            let s = Sources {
                line: &line,
                ..sources(f.path(), nowhere(), nowhere())
            };
            assert!(npm_dependencies(&s).is_empty(), "{flag}");
        }
    }

    #[test]
    fn a_workspace_pattern_becomes_the_packages_under_it() {
        let f = project(r#"{"workspaces": ["packages/*", "tools/sample-cli", "apps/**"]}"#);
        for dir in [
            "packages/sample-one",
            "packages/sample-two",
            "packages/not-a-package",
            "packages/.hidden",
        ] {
            std::fs::create_dir_all(f.path().join(dir)).unwrap();
        }
        for dir in [
            "packages/sample-one",
            "packages/sample-two",
            "packages/.hidden",
        ] {
            std::fs::write(f.path().join(dir).join("package.json"), "{}").unwrap();
        }
        let mut found: Vec<String> = npm_workspaces(&sources(f.path(), nowhere(), nowhere()))
            .into_iter()
            .map(|f| f.name)
            .collect();
        found.sort();
        // A literal entry is offered as written whether or not it is there.
        // A pattern this cannot expand is left out.
        assert_eq!(
            found,
            [
                "packages/sample-one",
                "packages/sample-two",
                "tools/sample-cli"
            ]
        );
    }

    #[test]
    fn one_workspace_spelt_two_ways_is_one_row() {
        let f = project(
            r#"{"workspaces": ["./packages/sample-one/", "packages/*", "packages/sample-one"]}"#,
        );
        std::fs::create_dir_all(f.path().join("packages/sample-one")).unwrap();
        std::fs::write(f.path().join("packages/sample-one/package.json"), "{}").unwrap();
        let found: Vec<String> = npm_workspaces(&sources(f.path(), nowhere(), nowhere()))
            .into_iter()
            .map(|f| f.name)
            .collect();
        // The first spelling the file gives it.
        assert_eq!(found, ["./packages/sample-one/"]);
        // Any spelling of it on the line already takes it out.
        let line = ["-w", "packages/sample-one/"];
        let s = Sources {
            line: &line,
            ..sources(f.path(), nowhere(), nowhere())
        };
        assert!(npm_workspaces(&s).is_empty());
    }

    #[test]
    fn a_negated_workspace_entry_takes_its_path_back_out() {
        let f = project(r#"{"workspaces": ["packages/*", "!packages/sample-old"]}"#);
        for dir in ["packages/sample-new", "packages/sample-old"] {
            std::fs::create_dir_all(f.path().join(dir)).unwrap();
            std::fs::write(f.path().join(dir).join("package.json"), "{}").unwrap();
        }
        let found = npm_workspaces(&sources(f.path(), nowhere(), nowhere()));
        assert_eq!(found_names(&found), ["packages/sample-new"]);
    }

    #[test]
    fn a_leading_dot_slash_does_not_keep_two_spellings_of_one_path_apart() {
        let f = project(r#"{"workspaces": ["./packages/*", "!packages/sample-old"]}"#);
        for dir in [
            "packages/sample-new",
            "packages/sample-old",
            "packages/sample-used",
        ] {
            std::fs::create_dir_all(f.path().join(dir)).unwrap();
            std::fs::write(f.path().join(dir).join("package.json"), "{}").unwrap();
        }
        let line = ["npm", "run", "-w", "packages/sample-used"];
        let s = Sources {
            line: &line,
            ..sources(f.path(), nowhere(), nowhere())
        };
        assert_eq!(found_names(&npm_workspaces(&s)), ["./packages/sample-new"]);
    }

    #[test]
    fn a_name_holding_a_control_character_is_left_out() {
        let f = project(
            r#"{"scripts": {"sample-build": "sample-tool build", "sample\u001b-bad": "sample-tool"}}"#,
        );
        let rows = rows("npm", "run", "script", "", f.path(), &[]);
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert_eq!(names, ["sample-build"]);
    }

    #[test]
    fn a_workspace_already_on_the_line_is_left_out() {
        let f = project(r#"{"workspaces": ["tools/sample-cli", "tools/sample-web"]}"#);
        let line = ["npm", "run", "-w", "tools/sample-cli"];
        let s = Sources {
            line: &line,
            ..sources(f.path(), nowhere(), nowhere())
        };
        assert_eq!(found_names(&npm_workspaces(&s)), ["tools/sample-web"]);
    }

    #[test]
    fn a_script_row_is_described_by_its_body_and_a_workspace_by_the_reader() {
        let f = project(
            r#"{"scripts": {"sample-build": "sample-tool build"}, "workspaces": ["tools/sample-cli"]}"#,
        );
        let script = rows("npm", "run", "script", "", f.path(), &[]);
        assert_eq!(script[0].label, "sample-tool build");
        let workspace = rows("npm", "run", "workspace", "", f.path(), &[]);
        assert_eq!(workspace[0].label, "workspace");
    }

    #[test]
    fn a_host_row_carries_the_name_it_would_insert_and_its_label() {
        let f = ssh_home(Some("Host sample-host\n"), None);
        let s = sources(nowhere(), f.path(), nowhere());
        let reader = reader("ssh", "ssh", "user@hostname").expect("ssh has a reader");
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
        let reader = reader("ssh", "ssh", "user@hostname").expect("ssh has a reader");
        assert!(candidates(reader, &s, "zzzz").is_empty());
    }
}
