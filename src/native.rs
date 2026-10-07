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
//! argument whose generator carries no command line, for one whose line
//! cannot say what to read and for one whose line must not run. `npm run`'s
//! argument keeps the line that walks up to a `package.json`. `pnpm remove`
//! and `pnpm run` keep that same line and read opposite halves of the file.
//! `brew install` keeps two lines that print past the cap a line's output
//! carries. Its reader reads both lists from Homebrew's own files at once and
//! leaves out a name already typed. An entry keyed by one line sees neither
//! the other list nor the words in front of the one being typed. `brew
//! upgrade` and `brew services` keep lines that must not run. `brew outdated`
//! can reach a network and `brew services list` takes about half a second.

use crate::candidates::{Candidate, DEFAULT_PRIORITY, Kind, SCAN_LIMIT};
use crate::fuzzy;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

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
    /// The argument's first name, as the specification spells it. Empty for
    /// an argument the specification leaves unnamed.
    arg: &'static str,
    /// What a row says it is, where a specification's own row would carry a
    /// description and the reader found nothing better to say.
    label: &'static str,
    read: fn(&Sources) -> Vec<Found>,
}

/// One name a reader found. `label` is what the row says of itself where
/// the reader's own label would say less. A script's body is the case.
/// `insert` is what goes on the line where the name is not. A Claude
/// session shows its title and goes in by its id.
#[derive(Clone)]
struct Found {
    name: String,
    label: Option<Cow<'static, str>>,
    insert: Option<String>,
}

/// Names that say nothing of themselves beyond the reader's own label.
fn unlabelled(names: Vec<String>) -> Vec<Found> {
    names
        .into_iter()
        .map(|name| Found {
            name,
            label: None,
            insert: None,
        })
        .collect()
}

static READERS: &[Reader] = &[
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
    // A bare `bun`, `nr` and `rushx` run a script the way `npm run` does.
    Reader {
        command: "bun",
        owners: &["bun"],
        arg: "file",
        label: "script",
        read: npm_scripts,
    },
    Reader {
        command: "nr",
        owners: &["nr"],
        arg: "script",
        label: "script",
        read: npm_scripts,
    },
    Reader {
        command: "rushx",
        owners: &["rushx"],
        arg: "Scripts",
        label: "script",
        read: npm_scripts,
    },
    // `yarn`'s own two arguments carry no name.
    Reader {
        command: "yarn",
        owners: &["yarn"],
        arg: "",
        label: "script",
        read: npm_scripts,
    },
    Reader {
        command: "yarn",
        owners: &["remove"],
        arg: "",
        label: "dependency",
        read: npm_dependencies,
    },
    Reader {
        command: "rush",
        owners: &["install", "build", "rebuild"],
        arg: "PROJECT",
        label: "project",
        read: rush_projects,
    },
    Reader {
        command: "brew",
        owners: &["install", "abv", "edit", "home"],
        arg: "formula",
        label: "formula",
        read: brew_names,
    },
    Reader {
        command: "brew",
        owners: &["upgrade"],
        arg: "outdated_formula|outdated_cask",
        label: "installed",
        read: brew_installed,
    },
    // `brew services`'s own subcommands. Their argument carries no name.
    Reader {
        command: "brew",
        owners: &["start", "stop", "restart", "run"],
        arg: "",
        label: "service",
        read: brew_services,
    },
    Reader {
        command: "claude",
        owners: &["claude"],
        arg: "session",
        label: "session",
        read: claude_sessions,
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
/// dependencies and has none either. A line the corpus reads two ways has
/// its entries in [`BY_ARGUMENT`] instead, one for each way.
static SCRIPTS: &[Script] = &[
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
    Script {
        line: &["podman", "ps", "--format", "{{ json . }}"],
        label: "container",
        parse: containers,
    },
    Script {
        line: &["podman", "ps", "-a", "--format", "{{ json . }}"],
        label: "container",
        parse: containers,
    },
    Script {
        line: &[
            "podman",
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
        line: &["podman", "volume", "list", "--format", "{{ json . }}"],
        label: "volume",
        parse: |out| json_lines(out, "Name", "Driver"),
    },
    // Podman 4 writes the network's fields in lower case.
    Script {
        line: &["podman", "network", "list", "--format", "{{ json . }}"],
        label: "network",
        parse: |out| json_lines(out, "name", "driver"),
    },
    Script {
        line: &["k3d", "cluster", "list", "--no-headers"],
        label: "cluster",
        parse: |out| {
            out.lines()
                .filter_map(|line| {
                    let mut words = line.split_whitespace();
                    let (name, servers, agents) = (words.next()?, words.next()?, words.next()?);
                    Some(labelled(
                        name,
                        format!("{servers} servers, {agents} agents"),
                    ))
                })
                .collect()
        },
    },
    Script {
        line: &["k3d", "node", "list", "--no-headers"],
        label: "node",
        parse: |out| {
            out.lines()
                .filter_map(|line| {
                    let mut words = line.split_whitespace();
                    let (name, role, cluster) = (words.next()?, words.next()?, words.next()?);
                    Some(labelled(name, format!("{role} of {cluster}")))
                })
                .collect()
        },
    },
    Script {
        line: &["k3d", "registry", "list", "--no-headers"],
        label: "registry",
        parse: |out| {
            out.lines()
                .filter_map(|line| line.split_whitespace().next())
                .map(name_only)
                .collect()
        },
    },
    Script {
        line: &["kind", "get", "clusters"],
        label: "cluster",
        parse: plain_lines,
    },
    Script {
        line: &["kind", "get", "nodes", "-A"],
        label: "node",
        parse: plain_lines,
    },
    Script {
        line: &["limactl", "list", "--quiet"],
        label: "instance",
        parse: plain_lines,
    },
    Script {
        line: &["multipass", "list", "--format=json"],
        label: "instance",
        parse: multipass_instances,
    },
    Script {
        line: &["rclone", "listremotes"],
        label: "remote",
        parse: plain_lines,
    },
    Script {
        line: &["asdf", "plugin-list"],
        label: "plugin",
        parse: plain_lines,
    },
    // `cat` is a program of its own and no shell stands between it and the
    // file.
    Script {
        line: &["cat", "copilot/.workspace"],
        label: "application",
        parse: copilot_application,
    },
    Script {
        line: &["networksetup", "-listallnetworkservices"],
        label: "network service",
        parse: network_services,
    },
    Script {
        line: &["networksetup", "-listpppoeservices"],
        label: "PPPoE service",
        parse: plain_lines,
    },
    Script {
        line: &["networksetup", "-listlocations"],
        label: "network location",
        parse: plain_lines,
    },
    Script {
        line: &["networksetup", "-listBonds"],
        label: "bond",
        parse: |out| {
            out.lines()
                .filter_map(|line| line.trim().strip_prefix("user-defined-name: "))
                .map(name_only)
                .collect()
        },
    },
    Script {
        line: &["defaults", "domains"],
        label: "domain",
        parse: |out| {
            out.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(name_only)
                .collect()
        },
    },
    // A header and then a pid, a status and a label to a line.
    Script {
        line: &["launchctl", "list"],
        label: "service",
        parse: |out| {
            out.lines()
                .skip(1)
                .filter_map(|line| line.split('\t').nth(2))
                .filter(|name| !name.is_empty())
                .map(name_only)
                .collect()
        },
    },
];

/// What each command line answered and each reader found in one menu. Both
/// maps key on an entry's address. A `const` table can give one entry two
/// addresses and the tables are therefore `static`.
///
/// Under `lines` a line runs once however many keys follow. Git's own
/// readers and the directory scan do the same. Two entries that read one
/// line two ways each run it. A line that failed stays an empty answer.
///
/// Under `reads` a reader keeps one answer. Some readers leave out a name
/// the line already holds. A reader therefore reads its files again once
/// the words in front of the one being typed change. An empty answer is not
/// kept and the next key reads again.
#[derive(Default)]
pub(crate) struct Runs {
    lines: HashMap<*const Script, Vec<Found>>,
    reads: HashMap<(*const Reader, Vec<String>), Vec<Found>>,
}

/// The lines the corpus reads two ways, each way under the names of the
/// arguments that read it that way. [`script_rows`] asks this before
/// [`SCRIPTS`].
///
/// `networksetup -listallhardwareports` names a port such as `Wi-Fi` for
/// one argument and the device under it such as `en0` for another. An
/// argument that takes only the wireless one is answered with every device,
/// because its name does not say which it is.
static BY_ARGUMENT: &[(&[&str], Script)] = &[
    (
        &["FQBN"],
        Script {
            line: &["arduino-cli", "board", "list", "--format", "json"],
            label: "board",
            parse: |out| boards(out, true),
        },
    ),
    (
        &["port"],
        Script {
            line: &["arduino-cli", "board", "list", "--format", "json"],
            label: "port",
            parse: |out| boards(out, false),
        },
    ),
    (
        &["hardwareport", "hardwarePort"],
        Script {
            line: &["networksetup", "-listallhardwareports"],
            label: "hardware port",
            parse: |out| hardware_ports(out, true),
        },
    ),
    (
        &["device", "parentdevice", "interface"],
        Script {
            line: &["networksetup", "-listallhardwareports"],
            label: "device",
            parse: |out| hardware_ports(out, false),
        },
    ),
];

/// The rows the command line `script` offers, where [`SCRIPTS`] has a reader
/// for it. `script` is a generator's own field. Only the list form names
/// its words without a shell between them and only that form is matched.
/// `arg` is the name of the argument the generator sits on.
pub(crate) fn script_rows(
    script: &Value,
    arg: &str,
    term: &str,
    cwd: &Path,
    runs: &mut Runs,
) -> Vec<Candidate> {
    let Some(words) = script.as_array() else {
        return Vec::new();
    };
    let entry = BY_ARGUMENT
        .iter()
        .find(|(args, entry)| args.contains(&arg) && same_line(words, entry.line))
        .map(|(_, entry)| entry)
        .or_else(|| SCRIPTS.iter().find(|entry| same_line(words, entry.line)));
    let Some(entry) = entry else {
        return Vec::new();
    };
    let found = runs
        .lines
        .entry(std::ptr::from_ref(entry))
        .or_insert_with(|| run(entry, cwd));
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
                insert: None,
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
                insert: None,
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
                insert: None,
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
                insert: None,
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
            // Docker joins a container's names with commas and Podman keeps
            // them in a list.
            let name = match object.get("Names")? {
                Value::Array(names) => names.first()?.as_str()?,
                names => names.as_str()?.split(',').next()?,
            };
            if name.is_empty() {
                return None;
            }
            let image = text_field(&object, "Image").unwrap_or("");
            let status = text_field(&object, "Status").unwrap_or("");
            Some(Found {
                name: name.to_string(),
                label: Some(Cow::Owned(format!("{image} {status}").trim().to_string())),
                insert: None,
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
        insert: None,
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
                insert: None,
            })
        })
        .collect()
}

/// Every instance `multipass` knows and how each one stands. The corpus
/// filters by state for some arguments. One line gives one answer here and
/// the state in the row says the rest.
fn multipass_instances(out: &str) -> Vec<Found> {
    let Ok(listed) = serde_json::from_str::<Value>(out) else {
        return Vec::new();
    };
    listed["list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|instance| {
            let name = instance["name"].as_str().filter(|n| !n.is_empty())?;
            let about = [&instance["state"], &instance["release"]]
                .into_iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            Some(Found {
                name: name.to_string(),
                label: (!about.is_empty()).then_some(Cow::Owned(about)),
                insert: None,
            })
        })
        .collect()
}

/// The boards `arduino-cli` sees on its ports, by the board's FQBN where
/// `fqbn` asks for one and by the port's address otherwise. A port no board
/// matched names nothing. The CLI writes the list on its own or under
/// `detected_ports`, by its version.
fn boards(out: &str, fqbn: bool) -> Vec<Found> {
    let Ok(listed) = serde_json::from_str::<Value>(out) else {
        return Vec::new();
    };
    let ports = listed.get("detected_ports").unwrap_or(&listed);
    ports
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|port| {
            let board = port["matching_boards"].get(0)?;
            let name = board["name"].as_str().unwrap_or("");
            let address = port["port"]["address"].as_str()?;
            Some(if fqbn {
                labelled(board["fqbn"].as_str()?, format!("{name} on {address}"))
            } else {
                labelled(address, format!("{name} port connection"))
            })
        })
        .collect()
}

/// `networksetup -listallhardwareports`: a `Hardware Port:` line and the
/// `Device:` line under it for each port. `ports` names the port and says
/// its device. Otherwise the device is the name and the port says what it
/// is.
fn hardware_ports(out: &str, ports: bool) -> Vec<Found> {
    let mut found = Vec::new();
    let mut port = None;
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("Hardware Port: ") {
            port = Some(name.trim());
        } else if let (Some(device), Some(name)) = (line.strip_prefix("Device: "), port.take()) {
            let device = device.trim();
            found.push(if ports {
                labelled(name, format!("device {device}"))
            } else {
                labelled(device, name.to_string())
            });
        }
    }
    found
}

/// The application a Copilot workspace file names. The file is YAML and
/// the one key read here sits at its top level.
fn copilot_application(out: &str) -> Vec<Found> {
    out.lines()
        .filter_map(|line| line.strip_prefix("application:"))
        .map(|value| value.trim().trim_matches(['"', '\'']))
        .filter(|name| !name.is_empty())
        .map(name_only)
        .take(1)
        .collect()
}

/// `networksetup`'s services under a line that explains the mark in front
/// of a disabled one.
fn network_services(out: &str) -> Vec<Found> {
    out.lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| match line.strip_prefix("* ") {
            Some(name) => Found {
                name: name.to_string(),
                label: Some(Cow::Borrowed("disabled network service")),
                insert: None,
            },
            None => name_only(line),
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
        insert: None,
    }
}

fn labelled(name: &str, label: String) -> Found {
    Found {
        name: name.to_string(),
        label: Some(Cow::Owned(label)),
        insert: None,
    }
}

/// Where a reader is allowed to look. The environment is read at one edge
/// and every reader takes its paths from here, which is what lets a test
/// hand one a temporary home rather than the person's own. [`crate::path`]
/// keeps `$HOME` behind a twin for that same reason.
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
    /// Where Homebrew keeps its cache. `None` means its name lists are not to
    /// be read and [`homebrew_cache`] says when.
    homebrew_cache: Option<&'a Path>,
    /// `$PATH`. It is empty when the environment says nothing and
    /// [`find_homebrew`] then finds no Homebrew.
    path: &'a OsStr,
    /// Where an Intel Mac keeps Homebrew. `brew` looks at the `brew` there.
    usr_local: &'a Path,
    /// Where Claude Code keeps its sessions. `None` means there is nowhere
    /// to look and [`claude_config`] says when.
    claude_config: Option<&'a Path>,
}

const SYSTEM_SSH_CONFIG: &str = "/etc/ssh/ssh_config";
const USR_LOCAL: &str = "/usr/local";

/// The rows a native reader offers for one `dyn` argument, or none where the
/// table has no reader for it. `command` is the first name of the
/// specification the walk ended in, `owner` is the name of the node it
/// ended on and `arg` is the argument's own first name. `line` is every word
/// in front of the one being typed. `runs` keeps a non-empty answer until
/// `line` changes.
pub(crate) fn rows(
    command: &str,
    owner: &str,
    arg: &str,
    term: &str,
    cwd: &Path,
    line: &[&str],
    runs: &mut Runs,
) -> Vec<Candidate> {
    let Some(reader) = reader(command, owner, arg) else {
        return Vec::new();
    };
    let key = (
        std::ptr::from_ref(reader),
        line.iter().map(|word| word.to_string()).collect::<Vec<_>>(),
    );
    runs.reads.retain(|kept, _| kept.0 != key.0 || *kept == key);
    let found = match runs.reads.entry(key) {
        Entry::Occupied(kept) => kept.into_mut(),
        // An empty answer is often a file caught while it is rewritten.
        // Keeping it would keep it for the whole word.
        Entry::Vacant(slot) => {
            let found = read(reader, cwd, line);
            if found.is_empty() {
                return Vec::new();
            }
            slot.insert(found)
        }
    };
    to_rows(found.iter(), reader.label, term)
}

/// What `reader` finds for `line` in `cwd`. The environment is read here and
/// nowhere below it.
fn read(reader: &Reader, cwd: &Path, line: &[&str]) -> Vec<Found> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let home = var("HOME").unwrap_or_default();
    let homebrew_cache = homebrew_cache(
        std::env::var_os("HOMEBREW_NO_INSTALL_FROM_API").is_some_and(|value| !value.is_empty()),
        var("HOMEBREW_CACHE"),
        var("XDG_CACHE_HOME"),
        &home,
    );
    let path = std::env::var_os("PATH").unwrap_or_default();
    let claude_config = claude_config(var("CLAUDE_CONFIG_DIR"), &home);
    let sources = Sources {
        cwd,
        line,
        home: &home,
        ssh_config: Path::new(SYSTEM_SSH_CONFIG),
        homebrew_cache: homebrew_cache.as_deref(),
        path: &path,
        usr_local: Path::new(USR_LOCAL),
        claude_config: claude_config.as_deref(),
    };
    (reader.read)(&sources)
}

fn reader(command: &str, owner: &str, arg: &str) -> Option<&'static Reader> {
    READERS.iter().find(|r| {
        r.command == command && r.arg == arg && (r.owners.is_empty() || r.owners.contains(&owner))
    })
}

/// One row per name `term` reaches, under `label` where the name brought
/// none of its own.
fn to_rows<'a>(
    found: impl Iterator<Item = &'a Found>,
    label: &'static str,
    term: &str,
) -> Vec<Candidate> {
    found
        .filter_map(|found| {
            let name = &found.name;
            // The row would show such a name with the character stripped and
            // insert it whole. An empty one inserts nothing. Git's own readers
            // leave both out too.
            if name.is_empty() || name.chars().any(char::is_control) {
                return None;
            }
            let score = fuzzy::score(term, name).max(
                found
                    .insert
                    .as_deref()
                    .filter(|insert| fuzzy::starts_with_folded(insert, term))
                    .and_then(|insert| fuzzy::score(term, insert)),
            )?;
            Some(Candidate {
                display: name.clone(),
                insert: found.insert.clone().unwrap_or_else(|| name.clone()),
                // A target and a host are both flat values rather than paths
                // on disk, which is what `spec_menu` already gives a
                // specification's own fixed suggestions: no folder glyph, no
                // trailing slash and plain shell quoting.
                kind: Kind::Path,
                label: found.label.clone().unwrap_or(Cow::Borrowed(label)),
                hint: Vec::new(),
                score,
                priority: DEFAULT_PRIORITY,
                cursor: None,
                verbatim: false,
            })
        })
        .collect()
}

/// How much of one file a reader reads. The 64 KiB `git.rs` already caps
/// a Git query's output at, for the same reason: a prompt is waiting on
/// this, and a makefile or an SSH configuration past that size is generated
/// rather than written. What sits past the limit is dropped and the file is
/// still read up to it.
const READ_LIMIT: u64 = 64 * 1024;

/// The first `limit` bytes of `path` and whether the file holds more than
/// was read. A file still being written can hold more as well. `None` for a
/// file that is absent, that will not read or that is not a regular one.
/// [`crate::path::regular`] says why.
fn read_capped(path: &Path, limit: u64) -> Option<(Vec<u8>, bool)> {
    let file = open_regular(path)?;
    let mut bytes = Vec::new();
    let mut capped = file.take(limit);
    capped.read_to_end(&mut bytes).ok()?;
    let more = capped.into_inner().metadata().ok()?.len() > bytes.len() as u64;
    Some((bytes, more))
}

/// `path` open for reading, or `None` where [`read_capped`] would give none.
fn open_regular(path: &Path) -> Option<File> {
    crate::path::regular(path).ok()?;
    // A FIFO that took the file's place since the check above would hold a
    // plain open until somebody wrote to it. `O_NONBLOCK` returns at once and
    // the handle's own `stat` refuses it.
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

/// The first `limit` bytes of `path`, as whole lines. A read that stops
/// short of the file's end stops in the middle of a line as often as not.
/// That last partial line is dropped. Half a name is worse than a missing
/// one. An absent or unreadable file gives nothing and never an error. This
/// runs at a prompt and an error has nowhere to go.
fn read_lines(path: &Path, limit: u64) -> Vec<String> {
    let Some((bytes, more)) = read_capped(path, limit) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if more && !text.ends_with('\n') {
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
    for line in read_lines(&path, READ_LIMIT) {
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
    // A relative home would be read against the line's directory.
    let user = sources
        .home
        .is_absolute()
        .then(|| sources.home.join(".ssh"));
    let user_config = user.as_ref().map(|dir| dir.join("config"));
    for path in user_config
        .as_deref()
        .into_iter()
        .chain([sources.ssh_config])
    {
        for line in read_lines(path, READ_LIMIT) {
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
    let known_hosts = user.map(|dir| dir.join("known_hosts"));
    for line in known_hosts
        .iter()
        .flat_map(|path| read_lines(path, READ_LIMIT))
    {
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
    let (dir, bytes) = nearest(cwd, "package.json")?;
    match serde_json::from_slice(&bytes).ok()? {
        Value::Object(map) => Some((dir, map)),
        _ => None,
    }
}

/// The nearest file called `name` at or above `cwd` and the directory that
/// holds it. A tool that reads its own file from the project root finds it
/// by walking up the same way.
fn nearest(cwd: &Path, name: &str) -> Option<(PathBuf, Vec<u8>)> {
    let (dir, path) = cwd
        .ancestors()
        .map(|dir| (dir, dir.join(name)))
        .find(|(_, path)| path.is_file())?;
    Some((dir.to_path_buf(), read_capped(&path, READ_LIMIT)?.0))
}

/// The projects the nearest `rush.json` names, each by its package name and
/// with the folder it lives in.
fn rush_projects(sources: &Sources) -> Vec<Found> {
    let Some((_, bytes)) = nearest(sources.cwd, "rush.json") else {
        return Vec::new();
    };
    let Ok(rush) =
        serde_json::from_str::<Value>(&without_comments(&String::from_utf8_lossy(&bytes)))
    else {
        return Vec::new();
    };
    rush["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|project| {
            Some(Found {
                name: project["packageName"].as_str()?.to_string(),
                label: project["projectFolder"]
                    .as_str()
                    .map(|folder| Cow::Owned(folder.to_string())),
                insert: None,
            })
        })
        .collect()
}

/// `text` without its `//` and `/* */` comments, the way Rush reads its own
/// configuration. A comment marker inside a string is part of the string.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '/' && chars.peek() == Some(&'/') {
            while chars.next_if(|&n| n != '\n').is_some() {}
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut last = ' ';
            for n in chars.by_ref() {
                if last == '*' && n == '/' {
                    break;
                }
                last = n;
            }
        } else {
            in_string = c == '"';
            out.push(c);
        }
    }
    out
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
                insert: None,
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
                    insert: None,
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

/// Where Homebrew keeps its cache. `brew` itself resolves it this way and
/// takes each path as it stands. `cache` is `HOMEBREW_CACHE` and names it
/// outright. Without one it sits under `home` on macOS and under `xdg_cache`
/// or `home`'s `.cache` everywhere else. An empty `home` gives none whatever
/// `cache` says. `brew` refuses to run without a `HOME`. `no_api` is
/// `HOMEBREW_NO_INSTALL_FROM_API` and gives none even where `cache` names
/// one. Homebrew then reads its taps and leaves the name lists to go stale.
/// A `brew.env` file can set any `HOMEBREW_` variable and is not read.
fn homebrew_cache(
    no_api: bool,
    cache: Option<PathBuf>,
    xdg_cache: Option<PathBuf>,
    home: &Path,
) -> Option<PathBuf> {
    if no_api || home.as_os_str().is_empty() {
        return None;
    }
    let given = |dir: Option<PathBuf>| dir.filter(|dir| !dir.as_os_str().is_empty());
    if let Some(cache) = given(cache) {
        return Some(cache);
    }
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Caches/Homebrew"));
    }
    Some(
        given(xdg_cache)
            .unwrap_or_else(|| home.join(".cache"))
            .join("Homebrew"),
    )
}

/// Where Homebrew is installed and where it keeps its formulae.
struct Homebrew {
    prefix: PathBuf,
    cellar: PathBuf,
}

/// Homebrew as the first `brew` on `$PATH` that this user may run finds itself.
/// `brew` pays no heed to `HOMEBREW_PREFIX`. A relative entry on `$PATH` is
/// read from the line's directory and zsh reads it the same way. The prefix is
/// the directory above the physical path of the entry that holds the found
/// `brew`. A link at the `brew` file itself is not followed for the prefix. A
/// `brew` link that sits in `/usr/local/bin` therefore gives `/usr/local`.
/// `brew` drops a `..` in the entry before it resolves a link and this resolves
/// the link first. [`brew_repository`] says where the found `brew` runs from. A
/// `/usr/local/bin/brew` that links into the same repository moves the prefix
/// to `/usr/local` unless the prefix's own `Cellar` is a link. After that
/// `brew` chooses a `Cellar`. The one in the repository comes first and the one
/// under the prefix is the fallback.
///
/// The found `brew` gives nothing where its entry is exactly `.` or empty. zsh
/// runs it by its bare name and it cannot find itself. An empty `HOME` gives
/// nothing and neither does a repository without `Library/Homebrew/brew.sh`.
/// Homebrew's own `brew` cannot start without either. A script that runs
/// another `brew` has no such file in its repository and gives nothing as well.
/// `brew` refuses to start in some other states too, such as when it cannot
/// read the line's directory. This looks for none of them.
fn find_homebrew(sources: &Sources) -> Option<Homebrew> {
    if sources.home.as_os_str().is_empty() {
        return None;
    }
    let entry = std::env::split_paths(sources.path)
        .find(|entry| crate::path::runnable(&sources.cwd.join(entry).join("brew")))?;
    if matches!(entry.as_os_str().as_encoded_bytes(), b"" | b".") {
        return None;
    }
    let dir = sources.cwd.join(entry);
    let mut prefix = dir.canonicalize().ok()?.parent()?.to_path_buf();
    let repository = brew_repository(&dir.join("brew"))?;
    if !repository.join("Library/Homebrew/brew.sh").is_file() {
        return None;
    }
    let usr_local_brew = sources.usr_local.join("bin/brew");
    if usr_local_brew.is_symlink()
        && !prefix.join("Cellar").is_symlink()
        && brew_repository(&usr_local_brew).as_ref() == Some(&repository)
    {
        prefix = sources.usr_local.to_path_buf();
    }
    let cellar = Some(repository.join("Cellar"))
        .filter(|cellar| cellar.is_dir())
        .unwrap_or_else(|| prefix.join("Cellar"));
    Some(Homebrew { prefix, cellar })
}

/// The repository a `brew` file runs from. That is the directory above the
/// file's own for a file that is no link. For a link it is the directory
/// above the one that holds what the link names. `brew` reads one link and
/// no more and so does this.
fn brew_repository(brew: &Path) -> Option<PathBuf> {
    let dir = brew.parent()?;
    let bin = match std::fs::read_link(brew) {
        Ok(target) => dir.join(target.parent()?),
        Err(_) => dir.to_path_buf(),
    };
    Some(bin.canonicalize().ok()?.parent()?.to_path_buf())
}

/// How much of one Homebrew API list a reader reads in place of
/// [`READ_LIMIT`]. The formula and cask lists name every formula in
/// `homebrew/core` or every cask in `homebrew/cask` and each is already past
/// [`READ_LIMIT`]. Homebrew writes them rather than a person.
const BREW_NAMES_LIMIT: u64 = 1024 * 1024;

/// The formulae, their aliases and the casks a `brew install` can name, less
/// the ones already on the line.
///
/// The specification keeps `brew formulae` and `brew casks` and each prints
/// past the cap a line's output carries. Homebrew keeps those names one to a
/// line in `api/formula_names.txt` and `api/cask_names.txt` under its cache
/// and rewrites them when it refreshes its API data. Reading them runs
/// nothing. Another tap's formulae and casks are in neither file.
/// `api/formula_aliases.txt` beside them holds `alias|formula` lines. An
/// alias row sorts among the formulae and says the formula it stands for.
/// Without one a person who typed `python` whole would get the nearest
/// formula in its place. A name two lists hold is the formula's. The formula
/// is what `brew install` takes. `--cask` keeps the casks alone and
/// `--formula` the formulae and their aliases. Homebrew reads a name in any
/// case and an alias on the line puts the formula it stands for there too.
/// Homebrew unlinks a list before it writes it again and can leave one absent
/// for a while. The others still give their rows and an answer missing one
/// is kept for the rest of the word.
fn brew_names(sources: &Sources) -> Vec<Found> {
    let Some(cache) = sources.homebrew_cache else {
        return Vec::new();
    };
    let (no_formulae, no_casks) = kinds_ruled_out(sources.line);
    let api = cache.join("api");
    let list = |file: &str, ruled_out: bool| {
        if ruled_out {
            Vec::new()
        } else {
            read_lines(&api.join(file), BREW_NAMES_LIMIT)
        }
    };
    let aliases: Vec<(String, String)> = list("formula_aliases.txt", no_formulae)
        .into_iter()
        .filter_map(|line| {
            let (alias, formula) = line.split_once('|')?;
            Some((alias.to_string(), formula.to_string()))
        })
        .collect();
    let mut taken = lowercased(sources.line);
    let stood_for: Vec<String> = aliases
        .iter()
        .filter(|(alias, _)| taken.contains(alias))
        .map(|(_, formula)| formula.clone())
        .collect();
    taken.extend(stood_for);
    let mut formulae: Vec<(String, Cow<'static, str>)> = list("formula_names.txt", no_formulae)
        .into_iter()
        .filter(|name| !taken.contains(name))
        .map(|name| (name, Cow::Borrowed("formula")))
        .chain(
            aliases
                .into_iter()
                .filter(|(alias, formula)| !taken.contains(alias) && !taken.contains(formula))
                .map(|(alias, formula)| (alias, Cow::Owned(format!("alias of {formula}")))),
        )
        .collect();
    formulae.sort_by(|a, b| a.0.cmp(&b.0));
    let casks = list("cask_names.txt", no_casks)
        .into_iter()
        .filter(|name| !taken.contains(name))
        .map(|name| (name, Cow::Borrowed("cask")));
    let mut seen = HashSet::new();
    formulae
        .into_iter()
        .chain(casks)
        .filter(|(name, _)| seen.insert(name.clone()))
        .map(|(name, label)| Found {
            name,
            label: Some(label),
            insert: None,
        })
        .collect()
}

/// Whether a `brew` line rules out the formulae and whether it rules out the
/// casks. `--cask` keeps the casks alone and `--formula` the formulae.
fn kinds_ruled_out(line: &[&str]) -> (bool, bool) {
    let on_line = |flags: [&str; 2]| line.iter().any(|word| flags.contains(word));
    (
        on_line(["--cask", "--casks"]),
        on_line(["--formula", "--formulae"]),
    )
}

/// The words of `line` in lower case. Homebrew reads a name in any case.
fn lowercased(line: &[&str]) -> HashSet<String> {
    line.iter().map(|word| word.to_lowercase()).collect()
}

/// The installed formulae and casks a `brew upgrade` can name, less the ones
/// already on the line.
///
/// The specification keeps `brew outdated -q` for `brew upgrade`.
/// `brew outdated` can update Homebrew before it answers and that reaches a
/// network. Homebrew tells an out-of-date formula or cask by its version and
/// keeps the latest versions in a file it does not document. The ones already
/// up to date are therefore rows as well. `brew upgrade` given a name that is
/// up to date says so and leaves that formula or cask as it is. A formula is a
/// directory in the `Cellar` that holds a version of it and a cask is a
/// directory in the `Caskroom`. A cask named for an installed formula gets no
/// row and the formula keeps its own. A cask named for a formula in
/// `homebrew/core`, for one of its aliases or for the old name of a renamed one
/// gets no row either. `brew upgrade` reads such a name as the formula whether
/// or not the formula is installed. Those names come from the lists
/// [`brew_names`] reads. Without them or under `HOMEBREW_NO_INSTALL_FROM_API`
/// only an installed formula rules a cask out. `--cask` and `--formula` work as
/// they do for [`brew_names`] and `--cask` therefore gives each of those casks
/// a row.
fn brew_installed(sources: &Sources) -> Vec<Found> {
    let Some(homebrew) = find_homebrew(sources) else {
        return Vec::new();
    };
    let (no_formulae, no_casks) = kinds_ruled_out(sources.line);
    let formulae = if no_formulae {
        Vec::new()
    } else {
        subdirs(&homebrew.cellar)
    };
    let formulae = formulae
        .into_iter()
        .filter(|rack| !subdirs(&homebrew.cellar.join(rack)).is_empty())
        .map(|name| (name, "installed"));
    let casks = if no_casks {
        Vec::new()
    } else {
        let core = core_formula_names(sources, no_formulae);
        subdirs(&homebrew.prefix.join("Caskroom"))
            .into_iter()
            .filter(|name| !core.contains(name))
            .collect()
    };
    let casks = casks.into_iter().map(|name| (name, "installed cask"));
    // A name on the line is taken already and so is a formula's name by the
    // time a cask comes to it.
    let mut taken = lowercased(sources.line);
    formulae
        .chain(casks)
        .filter(|(name, _)| taken.insert(name.clone()))
        .map(|(name, label)| Found {
            name,
            label: Some(Cow::Borrowed(label)),
            insert: None,
        })
        .collect()
}

/// Every formula name in `homebrew/core`, every alias and every old name of a
/// renamed formula, or none where `--cask` is on the line or Homebrew's name
/// lists are not to be read.
fn core_formula_names(sources: &Sources, no_formulae: bool) -> HashSet<String> {
    let Some(cache) = sources.homebrew_cache.filter(|_| !no_formulae) else {
        return HashSet::new();
    };
    ["formula_names.txt", "formula_aliases.txt"]
        .iter()
        .flat_map(|file| read_lines(&cache.join("api").join(file), BREW_NAMES_LIMIT))
        .map(|line| line.split('|').next().unwrap_or_default().to_string())
        .collect()
}

/// The installed formulae whose keg holds a service file, less the ones
/// already on the line.
///
/// The specification pipes `brew services list` through a shell and that
/// takes about half a second. Homebrew writes a launchd file for a formula's
/// service into its keg as it installs it. It does so on Linux as well. A
/// newer keg names it `sh.brew.<name>.plist` and an older one
/// `homebrew.mxcl.<name>.plist`. The formulae come from the `Cellar` and the
/// file is looked for in the keg `opt` links to. A service that names itself
/// some other way is not found. No row says whether a service is running.
/// Only `launchctl` or `systemctl` can say that. `--all` on the line names
/// every service already and the line then gets no rows.
fn brew_services(sources: &Sources) -> Vec<Found> {
    if sources.line.contains(&"--all") {
        return Vec::new();
    }
    let Some(homebrew) = find_homebrew(sources) else {
        return Vec::new();
    };
    let taken = lowercased(sources.line);
    let opt = homebrew.prefix.join("opt");
    let names = subdirs(&homebrew.cellar)
        .into_iter()
        .filter(|name| {
            !taken.contains(name)
                && ["sh.brew", "homebrew.mxcl"].iter().any(|stem| {
                    opt.join(name)
                        .join(format!("{stem}.{name}.plist"))
                        .is_file()
                })
        })
        .collect();
    unlabelled(names)
}

/// The directories in `dir` in the order of their names. A link and a name
/// that starts with a dot are left out. Homebrew counts neither as a formula
/// and no link as a cask.
fn subdirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// Where Claude Code keeps its state. `$CLAUDE_CONFIG_DIR` moves it and
/// `~/.claude` is where it is otherwise. A relative home names nowhere.
fn claude_config(dir: Option<PathBuf>, home: &Path) -> Option<PathBuf> {
    dir.filter(|dir| !dir.as_os_str().is_empty())
        .or_else(|| home.is_absolute().then(|| home.join(".claude")))
}

/// The sessions Claude Code keeps for the line's directory, the newest
/// first. Each shows the title Claude's own picker shows and goes in by its
/// id. `--resume` also takes a title. Several sessions can share one and
/// Claude then opens its picker rather than resume any of them.
///
/// The picker reads the first and the last 64 KiB of each file and so does
/// this. A title the person gave with `/rename` leads. The one Claude wrote
/// itself follows and the first prompt is what is left. A session `claude
/// -p` or the SDK started is not in the picker and is not here either.
fn claude_sessions(sources: &Sources) -> Vec<Found> {
    let (Some(config), Some(cwd)) = (sources.claude_config, sources.cwd.to_str()) else {
        return Vec::new();
    };
    let dir = config.join("projects").join(claude_project(cwd));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<(SystemTime, Found)> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name();
            let id = file.to_str()?.strip_suffix(".jsonl")?;
            if !is_uuid(id) {
                return None;
            }
            let (head, tail, modified) = read_ends(&entry.path(), READ_LIMIT)?;
            let found = Found {
                name: claude_title(&head, &tail)?,
                label: Some(Cow::Owned(id[..8].to_string())),
                insert: Some(id.to_string()),
            };
            Some((modified, found))
        })
        .collect();
    sessions.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    sessions.into_iter().map(|(_, found)| found).collect()
}

/// How long a directory name Claude Code gives a project before it cuts it.
const CLAUDE_PROJECT_LIMIT: usize = 200;

/// The directory Claude Code keeps `cwd`'s sessions in. Every UTF-16 unit
/// but an ASCII letter or digit becomes `-`. A longer name is cut and ends
/// in Claude's own hash of the whole path. That hash is JavaScript's
/// `(h << 5) - h + unit` in 32 bits and then base 36.
fn claude_project(cwd: &str) -> String {
    let name: String = cwd
        .encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        })
        .collect();
    if name.len() <= CLAUDE_PROJECT_LIMIT {
        return name;
    }
    let hash = cwd.encode_utf16().fold(0i32, |h, unit| {
        h.wrapping_shl(5)
            .wrapping_sub(h)
            .wrapping_add(i32::from(unit))
    });
    let digits: Vec<char> =
        std::iter::successors(Some(hash.unsigned_abs()), |n| (*n >= 36).then_some(n / 36))
            .filter_map(|n| char::from_digit(n % 36, 36))
            .collect();
    let digits: String = digits.iter().rev().collect();
    format!("{}-{digits}", &name[..CLAUDE_PROJECT_LIMIT])
}

/// Whether `id` is a UUID in the shape Claude Code names its sessions.
fn is_uuid(id: &str) -> bool {
    id.split('-').map(str::len).eq([8, 4, 4, 4, 12])
        && id.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit())
}

/// The first and the last `limit` bytes of `path` and when it last changed.
/// A file no longer than `limit` is both. Either end can cut a line short
/// and no reader below takes a line that does not parse.
fn read_ends(path: &Path, limit: u64) -> Option<(String, String, SystemTime)> {
    let mut file = open_regular(path)?;
    let meta = file.metadata().ok()?;
    let mut head = Vec::new();
    file.by_ref().take(limit).read_to_end(&mut head).ok()?;
    let tail = if meta.len() > limit {
        file.seek(SeekFrom::Start(meta.len() - limit)).ok()?;
        let mut tail = Vec::new();
        file.take(limit).read_to_end(&mut tail).ok()?;
        tail
    } else {
        head.clone()
    };
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    Some((text(&head), text(&tail), meta.modified().ok()?))
}

/// The title Claude Code's own picker shows for one session, or `None` for a
/// session the picker leaves out. A line break or any other control
/// character in it shows as a space.
fn claude_title(head: &str, tail: &str) -> Option<String> {
    let entrypoint = record_field(head.lines(), "entrypoint")
        .or_else(|| record_field(tail.lines().rev(), "entrypoint"));
    if matches!(entrypoint.as_deref(), Some("sdk-cli" | "sdk-ts" | "sdk-py")) {
        return None;
    }
    let first = head.lines().filter(|line| line.contains("\"parentUuid\":"));
    if matches!(
        record_field(first.take(1), "sessionKind").as_deref(),
        Some("daemon" | "daemon-worker")
    ) {
        return None;
    }
    let given =
        |key| record_field(tail.lines().rev(), key).filter(|title| !title.trim().is_empty());
    let title = given("customTitle")
        .or_else(|| given("aiTitle"))
        .or_else(|| first_prompt(head))?;
    let title = title.replace(char::is_control, " ");
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// The string `key` holds in the first of `lines` that is a JSON object
/// holding one.
fn record_field<'a>(mut lines: impl Iterator<Item = &'a str>, key: &str) -> Option<String> {
    let probe = format!("\"{key}\":");
    lines.find_map(|line| {
        if !line.contains(&probe) {
            return None;
        }
        let record: Value = serde_json::from_str(line).ok()?;
        record[key].as_str().map(str::to_string)
    })
}

/// How many UTF-16 units of a first prompt Claude Code's picker keeps.
const PROMPT_LIMIT: usize = 200;

/// The first thing a person typed into a session, found the way Claude
/// Code's picker finds it. A tool's answer, a note Claude adds itself and
/// text that opens with a tag are passed over. A slash command stands in
/// where nothing else was typed and a `!` command shows behind a `!`.
fn first_prompt(head: &str) -> Option<String> {
    let mut command = None;
    for line in head.lines() {
        if !line.contains("\"type\":\"user\"") || line.contains("\"tool_result\"") {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record["type"] != "user"
            || record["isMeta"] == true
            || record["isCompactSummary"] == true
        {
            continue;
        }
        let texts: Vec<&str> = match &record["message"]["content"] {
            Value::String(text) => vec![text],
            Value::Array(blocks) => blocks
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect(),
            _ => continue,
        };
        for text in texts {
            let text = text.replace('\n', " ");
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            if let Some(name) = between(text, "<command-name>", "</command-name>") {
                command.get_or_insert_with(|| name.to_string());
                continue;
            }
            if let Some(input) = between(text, "<bash-input>", "</bash-input>") {
                return Some(format!("! {}", input.trim()));
            }
            if opens_with_tag(text) || text.starts_with("[Request interrupted by user") {
                continue;
            }
            // Claude cuts at UTF-16 units and drops a character the limit splits.
            let mut units = 0;
            let cut = text.char_indices().find(|(_, c)| {
                units += c.len_utf16();
                units > PROMPT_LIMIT
            });
            return Some(match cut {
                Some((at, _)) => format!("{}…", text[..at].trim_end()),
                None => text.to_string(),
            });
        }
    }
    command
}

/// The text between the first `open` in `text` and the `close` after it.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let len = text[start..].find(close)?;
    Some(&text[start..start + len])
}

/// Whether `text` opens with a tag such as `<local-command-stdout>`.
fn opens_with_tag(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('<') else {
        return false;
    };
    let len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    rest.starts_with(|c: char| c.is_ascii_lowercase())
        && rest[len..].starts_with(|c: char| c.is_whitespace() || c == '>')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, soon};
    use crate::spec::Subcommand;
    use std::os::unix::fs::PermissionsExt;

    /// Every host name below is invented. A test that read the real
    /// `~/.ssh` would put a person's own hosts into the suite's output, so
    /// `home` is always a fixture and never `$HOME`.
    fn sources<'a>(cwd: &'a Path, home: &'a Path, ssh_config: &'a Path) -> Sources<'a> {
        Sources {
            cwd,
            line: &[],
            home,
            ssh_config,
            homebrew_cache: None,
            path: OsStr::new(""),
            usr_local: nowhere(),
            claude_config: None,
        }
    }

    fn candidates(reader: &Reader, sources: &Sources, term: &str) -> Vec<Candidate> {
        to_rows((reader.read)(sources).iter(), reader.label, term)
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
            .any(|a| a.dynamic && a.name.first().map_or("", String::as_str) == arg)
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
    fn a_line_read_two_ways_reads_the_way_its_argument_asks() {
        let ports = "Hardware Port: Wi-Fi\nDevice: en0\nEthernet Address: 00:00:00:00:00:00\n\nHardware Port: Sample Bridge\nDevice: bridge0\n";
        let names = |found: Vec<Found>| found.into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(
            names(hardware_ports(ports, true)),
            ["Wi-Fi", "Sample Bridge"]
        );
        let devices = hardware_ports(ports, false);
        assert_eq!(devices[0].name, "en0");
        assert_eq!(devices[0].label.as_deref(), Some("Wi-Fi"));
        let listed = r#"{"detected_ports": [
            {"port": {"address": "/dev/sample0"}, "matching_boards": [{"name": "Sample Board", "fqbn": "sample:avr:board"}]},
            {"port": {"address": "/dev/sample1"}}]}"#;
        assert_eq!(names(boards(listed, true)), ["sample:avr:board"]);
        assert_eq!(names(boards(listed, false)), ["/dev/sample0"]);
        // An older CLI writes the list on its own.
        let older = r#"[{"port": {"address": "/dev/sample0"}, "matching_boards": [{"name": "Sample Board", "fqbn": "sample:avr:board"}]}]"#;
        assert_eq!(names(boards(older, true)), ["sample:avr:board"]);
    }

    #[test]
    fn every_argument_a_two_way_line_names_still_holds_that_line() {
        // `make specs` could rename an argument and leave the entry reaching
        // nothing. Each name has to sit on an argument that runs the line.
        fn holds(value: &Value, name: &str, line: &[&str]) -> bool {
            match value {
                Value::Object(map) => {
                    let named = map.get("name").is_some_and(|n| {
                        n.as_str() == Some(name)
                            || n.as_array().is_some_and(|all| {
                                all.first().and_then(Value::as_str) == Some(name)
                            })
                    });
                    (named && holds_script(value, line))
                        || map.values().any(|v| holds(v, name, line))
                }
                Value::Array(items) => items.iter().any(|v| holds(v, name, line)),
                _ => false,
            }
        }
        let specs: Vec<Value> = ["arduino-cli", "networksetup", "networkQuality"]
            .iter()
            .map(|name| {
                let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("specs")
                    .join(format!("{name}.json"));
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
            })
            .collect();
        for (args, entry) in BY_ARGUMENT {
            for arg in *args {
                assert!(
                    specs.iter().any(|spec| holds(spec, arg, entry.line)),
                    "{arg} {:?}",
                    entry.line
                );
            }
        }
    }

    #[test]
    fn a_line_with_no_reader_never_runs() {
        let mut runs = Runs::default();
        let line = serde_json::json!(["sh", "-c", "touch sample"]);
        let f = Fixture::new(&[]);
        assert!(script_rows(&line, "", "", f.path(), &mut runs).is_empty());
        assert!(!f.path().join("sample").exists());
        assert!(runs.lines.is_empty());
        // Only the list form names its words without a shell between them.
        let text = serde_json::json!("git --no-optional-locks log --oneline");
        assert!(script_rows(&text, "", "", f.path(), &mut runs).is_empty());
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
        let rows = script_rows(&line, "", "", f.path(), &mut runs);
        let mut names: Vec<&str> = rows.iter().map(|r| r.insert.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["sample-main", "sample-topic"]);
        // A branch made now is the next menu's to find.
        f.git(&["branch", "sample-later", "sample-main"]);
        assert_eq!(script_rows(&line, "", "", f.path(), &mut runs).len(), 2);
        // Outside a repository Git fails and the line answers nothing.
        let outside = Fixture::new(&[]);
        assert!(script_rows(&line, "", "", outside.path(), &mut Runs::default()).is_empty());
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

    /// What the entry for `line` makes of `out`.
    fn parsed(line: &[&str], out: &str) -> Vec<(String, Option<String>)> {
        let entry = SCRIPTS.iter().find(|e| e.line == line).expect("an entry");
        (entry.parse)(out)
            .into_iter()
            .map(|f| (f.name, f.label.map(String::from)))
            .collect()
    }

    #[test]
    fn each_local_tool_line_reads_its_own_output() {
        let named = |name: &str| (name.to_string(), None);
        let said = |name: &str, label: &str| (name.to_string(), Some(label.to_string()));
        assert_eq!(
            parsed(
                &["podman", "ps", "--format", "{{ json . }}"],
                "{\"Names\":[\"sample-pod\"],\"Image\":\"sample:1\",\"Status\":\"Up\"}\n{\"Names\":[]}\n"
            ),
            [said("sample-pod", "sample:1 Up")]
        );
        assert_eq!(
            parsed(
                &["podman", "network", "list", "--format", "{{ json . }}"],
                "{\"name\":\"sample-net\",\"driver\":\"bridge\"}\n"
            ),
            [said("sample-net", "bridge")]
        );
        assert_eq!(
            parsed(
                &["k3d", "cluster", "list", "--no-headers"],
                "sample   1/1   0/0   true\n\n"
            ),
            [said("sample", "1/1 servers, 0/0 agents")]
        );
        assert_eq!(
            parsed(
                &["k3d", "node", "list", "--no-headers"],
                "k3d-sample-server-0   server   sample   running\n"
            ),
            [said("k3d-sample-server-0", "server of sample")]
        );
        assert_eq!(
            parsed(
                &["k3d", "registry", "list", "--no-headers"],
                "k3d-sample.localhost   registry   sample   running\n"
            ),
            [named("k3d-sample.localhost")]
        );
        assert_eq!(
            parsed(
                &["multipass", "list", "--format=json"],
                "{\"list\":[{\"name\":\"sample\",\"state\":\"Running\",\"release\":\"24.04 LTS\"},{\"name\":\"\"}]}"
            ),
            [said("sample", "Running 24.04 LTS")]
        );
        assert!(parsed(&["multipass", "list", "--format=json"], "not json").is_empty());
        assert_eq!(
            parsed(
                &["cat", "copilot/.workspace"],
                "# sample\napplication: 'sample-app'\n"
            ),
            [named("sample-app")]
        );
        assert_eq!(
            parsed(
                &["networksetup", "-listallnetworkservices"],
                "An asterisk (*) denotes that a network service is disabled.\nSample LAN\n* Sample VPN\n"
            ),
            [
                named("Sample LAN"),
                said("Sample VPN", "disabled network service")
            ]
        );
        assert_eq!(
            parsed(
                &["networksetup", "-listBonds"],
                "interface name: bond0\nuser-defined-name: Sample Bond\n"
            ),
            [named("Sample Bond")]
        );
        assert_eq!(
            parsed(&["defaults", "domains"], "sample.one, sample.two\n"),
            [named("sample.one"), named("sample.two")]
        );
        assert_eq!(
            parsed(
                &["launchctl", "list"],
                "PID\tStatus\tLabel\n-\t0\tsample.agent\n123\t0\tother.agent\n"
            ),
            [named("sample.agent"), named("other.agent")]
        );
    }

    #[test]
    fn rush_reads_its_projects_from_the_nearest_rush_json() {
        let f = Fixture::new(&["apps/sample"]);
        std::fs::write(
            f.path().join("rush.json"),
            "// Rush writes comments into its own file.\n{\n  /* the projects */\n  \"projects\": [\n    {\"packageName\": \"sample-app\", \"projectFolder\": \"apps/sample\"},\n    {\"packageName\": \"//not-a-comment\"}\n  ]\n}\n",
        )
        .unwrap();
        let rows = rows(
            "rush",
            "build",
            "PROJECT",
            "",
            &f.path().join("apps/sample"),
            &[],
            &mut Runs::default(),
        );
        let names: Vec<&str> = rows.iter().map(|r| r.insert.as_str()).collect();
        assert_eq!(names, ["sample-app", "//not-a-comment"]);
        assert_eq!(rows[0].label, "apps/sample");
    }

    #[test]
    fn pnpm_yarn_and_bun_read_the_same_package_json_npm_does() {
        assert!(reader("pnpm", "run", "Scripts").is_some());
        assert!(reader("pnpm", "pnpm", "Scripts").is_some());
        assert!(reader("pnpm", "remove", "Package").is_some());
        assert!(reader("yarn", "run", "script").is_some());
        assert!(reader("yarn", "upgrade", "package").is_some());
        assert!(reader("bun", "run", "script").is_some());
        // A bare `bun`, `nr`, `rushx` and `yarn` run a script too.
        assert!(reader("bun", "bun", "file").is_some());
        assert!(reader("nr", "nr", "script").is_some());
        assert!(reader("rushx", "rushx", "Scripts").is_some());
        assert!(reader("yarn", "yarn", "").is_some());
        assert!(reader("yarn", "remove", "").is_some());
        // `add` and `install` search the registry.
        assert!(reader("pnpm", "add", "package").is_none());
        assert!(reader("yarn", "add", "package").is_none());
    }

    #[test]
    fn an_argument_with_no_reader_gives_no_rows() {
        assert!(
            rows(
                "chown",
                "chown",
                "owner",
                "",
                nowhere(),
                &[],
                &mut Runs::default()
            )
            .is_empty()
        );
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
        let rows = rows(
            "npm",
            "run",
            "script",
            "",
            f.path(),
            &[],
            &mut Runs::default(),
        );
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
        let script = rows(
            "npm",
            "run",
            "script",
            "",
            f.path(),
            &[],
            &mut Runs::default(),
        );
        assert_eq!(script[0].label, "sample-tool build");
        let workspace = rows(
            "npm",
            "run",
            "workspace",
            "",
            f.path(),
            &[],
            &mut Runs::default(),
        );
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

    /// A Homebrew cache holding the two name lists a test writes into it.
    /// Homebrew writes each without a newline after its last name. Every name
    /// in one is invented.
    fn homebrew(formulae: &str, casks: &str) -> Fixture {
        let f = Fixture::new(&["api"]);
        std::fs::write(f.path().join("api/formula_names.txt"), formulae).unwrap();
        std::fs::write(f.path().join("api/cask_names.txt"), casks).unwrap();
        f
    }

    fn brew_sources<'a>(cache: &'a Path, line: &'a [&'a str]) -> Sources<'a> {
        Sources {
            line,
            homebrew_cache: Some(cache),
            ..sources(nowhere(), nowhere(), nowhere())
        }
    }

    #[test]
    fn brew_names_its_formulae_then_its_casks_and_a_shared_name_once() {
        let f = homebrew("sample-shared\nsample-tool", "sample-app\nsample-shared");
        let found = brew_names(&brew_sources(f.path(), &[]));
        assert_eq!(
            found_names(&found),
            ["sample-shared", "sample-tool", "sample-app"]
        );
        assert_eq!(found[0].label.as_deref(), Some("formula"));
        assert_eq!(found[2].label.as_deref(), Some("cask"));
    }

    #[test]
    fn brew_leaves_out_a_name_on_the_line_and_the_list_a_flag_rules_out() {
        let f = homebrew("sample-other\nsample-tool", "sample-app");
        let line = ["brew", "install", "sample-tool"];
        let found = brew_names(&brew_sources(f.path(), &line));
        assert_eq!(found_names(&found), ["sample-other", "sample-app"]);
        for flag in ["--cask", "--casks"] {
            let line = ["brew", "install", flag];
            let found = brew_names(&brew_sources(f.path(), &line));
            assert_eq!(found_names(&found), ["sample-app"], "{flag}");
        }
        for flag in ["--formula", "--formulae"] {
            let line = ["brew", "install", flag];
            let found = brew_names(&brew_sources(f.path(), &line));
            assert_eq!(
                found_names(&found),
                ["sample-other", "sample-tool"],
                "{flag}"
            );
        }
    }

    #[test]
    fn a_brew_list_past_the_read_limit_is_read_whole() {
        let (mut names, mut size) = (Vec::new(), 0);
        while size < READ_LIMIT as usize + 4096 {
            let name = format!("cask-{}", names.len());
            size += name.len() + 1;
            names.push(name);
        }
        let f = homebrew("sample-tool", &names.join("\n"));
        let found = brew_names(&brew_sources(f.path(), &[]));
        assert_eq!(found.len(), names.len() + 1);
    }

    #[test]
    fn no_homebrew_cache_gives_no_names() {
        assert!(brew_names(&brew_sources(nowhere(), &[])).is_empty());
        let s = sources(nowhere(), nowhere(), nowhere());
        assert!(brew_names(&s).is_empty());
    }

    /// A cache whose formula `sample-tool@2` has the aliases `sample-tool`
    /// and `sample-tool2`. Homebrew sorts the alias file by whole lines.
    fn aliased() -> Fixture {
        let f = homebrew("sample-tool@2\nsample-top", "sample-app");
        std::fs::write(
            f.path().join("api/formula_aliases.txt"),
            "sample-tool2|sample-tool@2\nsample-tool|sample-tool@2\nno-separator",
        )
        .unwrap();
        f
    }

    #[test]
    fn an_alias_sorts_among_the_formulae_and_says_what_it_stands_for() {
        let f = aliased();
        let found = brew_names(&brew_sources(f.path(), &[]));
        assert_eq!(
            found_names(&found),
            [
                "sample-tool",
                "sample-tool2",
                "sample-tool@2",
                "sample-top",
                "sample-app"
            ]
        );
        assert_eq!(found[0].label.as_deref(), Some("alias of sample-tool@2"));
        let line = ["brew", "install", "--cask"];
        let found = brew_names(&brew_sources(f.path(), &line));
        assert_eq!(found_names(&found), ["sample-app"]);
    }

    #[test]
    fn a_formula_on_the_line_under_any_name_or_case_is_left_out() {
        let f = aliased();
        for typed in ["sample-tool@2", "Sample-Tool", "sample-tool2"] {
            let line = ["brew", "install", typed];
            let found = brew_names(&brew_sources(f.path(), &line));
            assert_eq!(found_names(&found), ["sample-top", "sample-app"], "{typed}");
        }
    }

    #[test]
    fn the_homebrew_cache_is_where_brew_itself_looks() {
        let home = Path::new("/home/sample-user");
        let path = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            homebrew_cache(false, path("/sample/cache"), path("/sample/xdg"), home),
            path("/sample/cache")
        );
        let (default, xdg) = if cfg!(target_os = "macos") {
            let default = "/home/sample-user/Library/Caches/Homebrew";
            (default, default)
        } else {
            ("/home/sample-user/.cache/Homebrew", "/sample/xdg/Homebrew")
        };
        assert_eq!(homebrew_cache(false, path(""), None, home), path(default));
        assert_eq!(
            homebrew_cache(false, None, path("/sample/xdg"), home),
            path(xdg)
        );
    }

    #[test]
    fn no_api_data_and_no_home_give_no_homebrew_cache() {
        let home = Path::new("/home/sample-user");
        let cache = Some(PathBuf::from("/sample/cache"));
        assert_eq!(homebrew_cache(true, cache, None, home), None);
        let cache = Some(PathBuf::from("/sample/cache"));
        assert_eq!(homebrew_cache(false, cache, None, Path::new("")), None);
    }

    #[test]
    fn a_missing_list_leaves_the_others_their_rows() {
        let f = Fixture::new(&["api"]);
        std::fs::write(f.path().join("api/formula_names.txt"), "sample-tool").unwrap();
        let found = brew_names(&brew_sources(f.path(), &[]));
        assert_eq!(found_names(&found), ["sample-tool"]);
    }

    #[test]
    fn brew_answers_the_subcommands_that_name_any_formula_or_cask() {
        for owner in ["install", "abv", "edit", "home"] {
            assert!(reader("brew", owner, "formula").is_some(), "{owner}");
        }
        // `brew uninstall` names what is installed and a line answers it.
        assert!(reader("brew", "uninstall", "formula").is_none());
    }

    /// A Homebrew prefix holding `entries` and a `bin/brew` that runs.
    fn homebrew_prefix_fixture(entries: &[&str]) -> Fixture {
        let f = Fixture::new(&[&["bin", "bin/brew*"], entries].concat());
        make_runnable(&f.path().join("bin/brew"));
        give_brew_sh(f.path());
        f
    }

    /// Puts in `repository` the file a `brew` there runs.
    fn give_brew_sh(repository: &Path) {
        let library = repository.join("Library/Homebrew");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("brew.sh"), "").unwrap();
    }

    fn prefix_sources<'a>(path: &'a OsStr, line: &'a [&'a str]) -> Sources<'a> {
        Sources {
            line,
            path,
            ..sources(nowhere(), nowhere(), nowhere())
        }
    }

    #[test]
    fn brew_upgrade_offers_the_installed_formulae_then_the_casks() {
        let f = homebrew_prefix_fixture(&[
            "Cellar/sample-tool/1.0",
            "Cellar/sample-other/2.0",
            "Cellar/.sample-hidden/1.0",
            "Cellar/sample-empty",
            "Caskroom/sample-app/3.0",
            "Caskroom/sample-tool/1.0",
        ]);
        std::os::unix::fs::symlink("sample-tool", f.path().join("Cellar/sample-link")).unwrap();
        let path = f.path().join("bin").into_os_string();
        let found = brew_installed(&prefix_sources(&path, &[]));
        assert_eq!(
            found_names(&found),
            ["sample-other", "sample-tool", "sample-app"]
        );
        assert_eq!(found[0].label.as_deref(), Some("installed"));
        assert_eq!(found[2].label.as_deref(), Some("installed cask"));
        let line = ["brew", "upgrade", "Sample-Other"];
        let found = brew_installed(&prefix_sources(&path, &line));
        assert_eq!(found_names(&found), ["sample-tool", "sample-app"]);
        let line = ["brew", "upgrade", "--cask"];
        let found = brew_installed(&prefix_sources(&path, &line));
        assert_eq!(found_names(&found), ["sample-app", "sample-tool"]);
        let line = ["brew", "upgrade", "--formula"];
        let found = brew_installed(&prefix_sources(&path, &line));
        assert_eq!(found_names(&found), ["sample-other", "sample-tool"]);
    }

    #[test]
    fn brew_upgrade_leaves_a_cask_named_for_a_core_formula_or_alias_out() {
        let f = homebrew_prefix_fixture(&[
            "Caskroom/sample-alias/1.0",
            "Caskroom/sample-app/1.0",
            "Caskroom/sample-core/1.0",
        ]);
        let cache = homebrew("sample-core\nsample-formula", "");
        let aliases = cache.path().join("api/formula_aliases.txt");
        std::fs::write(aliases, "sample-alias|sample-formula").unwrap();
        let path = f.path().join("bin").into_os_string();
        let found = |line: &'static [&'static str]| {
            let s = Sources {
                homebrew_cache: Some(cache.path()),
                ..prefix_sources(&path, line)
            };
            brew_installed(&s)
        };
        assert_eq!(found_names(&found(&[])), ["sample-app"]);
        // `--cask` reaches the cask and so does a line read without the lists.
        let all = ["sample-alias", "sample-app", "sample-core"];
        assert_eq!(found_names(&found(&["brew", "upgrade", "--cask"])), all);
        assert_eq!(
            found_names(&brew_installed(&prefix_sources(&path, &[]))),
            all
        );
    }

    #[test]
    fn brew_services_offers_the_kegs_that_carry_a_service_file() {
        let f = homebrew_prefix_fixture(&[
            "Cellar/sample-db@2",
            "Cellar/sample-new",
            "Cellar/sample-older",
            "Cellar/sample-cli",
            "Cellar/sample-named",
            "opt/sample-db@2",
            "opt/sample-new",
            "opt/sample-older",
            "opt/sample-cli/bin",
            "opt/sample-named",
            "opt/sample-db@2/sh.brew.sample-db@2.plist*",
            "opt/sample-new/sh.brew.sample-new.plist*",
            "opt/sample-older/homebrew.mxcl.sample-older.plist*",
            "opt/sample-named/sh.brew.sample-other.plist*",
        ]);
        let path = f.path().join("bin").into_os_string();
        let found = brew_services(&prefix_sources(&path, &[]));
        assert_eq!(
            found_names(&found),
            ["sample-db@2", "sample-new", "sample-older"]
        );
        let line = ["brew", "services", "start", "sample-new"];
        let found = brew_services(&prefix_sources(&path, &line));
        assert_eq!(found_names(&found), ["sample-db@2", "sample-older"]);
        let line = ["brew", "services", "stop", "--all"];
        assert!(brew_services(&prefix_sources(&path, &line)).is_empty());
    }

    #[test]
    fn no_homebrew_prefix_gives_no_installed_names_or_services() {
        let s = sources(nowhere(), nowhere(), nowhere());
        assert!(brew_installed(&s).is_empty());
        assert!(brew_services(&s).is_empty());
    }

    fn make_runnable(path: &Path) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The prefix and the `Cellar` that `path` reaches from `cwd`. `usr_local`
    /// stands in for `/usr/local`.
    fn located(path: &OsStr, cwd: &Path, usr_local: &Path) -> Option<(PathBuf, PathBuf)> {
        let s = Sources {
            path,
            cwd,
            usr_local,
            ..sources(nowhere(), nowhere(), nowhere())
        };
        find_homebrew(&s).map(|h| (h.prefix, h.cellar))
    }

    #[test]
    fn homebrew_is_where_the_first_brew_on_path_that_runs_finds_itself() {
        let f = homebrew_prefix_fixture(&[
            "idle",
            "idle/brew*",
            "local/bin",
            "deep",
            "later/bin",
            "later/bin/brew*",
        ]);
        make_runnable(&f.path().join("later/bin/brew"));
        let brew = f.path().join("bin/brew");
        std::os::unix::fs::symlink(&brew, f.path().join("local/bin/brew")).unwrap();
        std::os::unix::fs::symlink("../bin", f.path().join("deep/linked")).unwrap();
        let root = f.path().canonicalize().unwrap();
        let on_path = |dirs: &[&str]| {
            let dirs = dirs.iter().map(|dir| f.path().join(dir));
            located(&std::env::join_paths(dirs).unwrap(), nowhere(), nowhere())
        };
        let local = (root.join("local"), root.join("local/Cellar"));
        // A `brew` that does not run is passed over and the first that does
        // wins.
        assert_eq!(
            on_path(&["idle", "bin", "later/bin"]),
            Some((root.clone(), root.join("Cellar")))
        );
        assert_eq!(on_path(&["idle"]), None);
        // A linked directory resolves and a linked `brew` does not.
        assert_eq!(on_path(&["deep/linked"]).map(|h| h.0), Some(root.clone()));
        assert_eq!(on_path(&["local/bin"]), Some(local.clone()));
        // A `Cellar` in the repository a linked `brew` runs from comes first.
        std::fs::create_dir(f.path().join("Cellar")).unwrap();
        std::fs::create_dir(f.path().join("local/Cellar")).unwrap();
        assert_eq!(
            on_path(&["local/bin"]),
            Some((local.0, root.join("Cellar")))
        );
        // A relative entry is read from the line's directory. A `brew` in an
        // entry that is exactly `.` or empty gives nothing.
        let bin = f.path().join("bin");
        let prefix =
            |path: &str, cwd: &Path| located(OsStr::new(path), cwd, nowhere()).map(|h| h.0);
        assert_eq!(prefix("bin", f.path()), Some(root.clone()));
        assert_eq!(prefix("./", &bin), Some(root.clone()));
        for path in ["", ":", ".", ".:"] {
            assert_eq!(prefix(path, &bin), None, "{path:?}");
        }
        // The shell goes no further than that `brew`. Where none runs there
        // the search goes on.
        let later = format!(":{}", bin.display());
        assert_eq!(prefix(&later, &bin), None);
        assert_eq!(prefix(&later, &f.path().join("idle")), Some(root.clone()));
    }

    #[test]
    fn a_brew_that_cannot_start_finds_no_homebrew() {
        let f = homebrew_prefix_fixture(&["later/bin", "later/bin/brew*"]);
        make_runnable(&f.path().join("later/bin/brew"));
        let bin = f.path().join("bin").into_os_string();
        let found = |home: &Path| {
            let s = Sources {
                path: &bin,
                ..sources(nowhere(), home, nowhere())
            };
            find_homebrew(&s).is_some()
        };
        assert!(found(nowhere()));
        // `brew` refuses to run without a `HOME`.
        assert!(!found(Path::new("")));
        // Nor does it run without a `brew.sh` in its repository. The shell
        // starts the first `brew` it finds and goes no further.
        let dirs = [f.path().join("later/bin"), f.path().join("bin")];
        let path = std::env::join_paths(dirs).unwrap();
        assert_eq!(located(&path, nowhere(), nowhere()), None);
    }

    #[test]
    fn a_usr_local_brew_into_the_same_repository_moves_the_prefix() {
        // The fixture's own `bin/brew` is the repository's.
        let f = homebrew_prefix_fixture(&[
            "usr/local/bin",
            "home/bin",
            "other/bin",
            "other/bin/brew*",
            "away/bin",
        ]);
        let brew = f.path().join("bin/brew");
        let usr_local = f.path().join("usr/local");
        let usr_local_brew = usr_local.join("bin/brew");
        std::os::unix::fs::symlink(&brew, f.path().join("home/bin/brew")).unwrap();
        let home = f.path().join("home/bin").into_os_string();
        let home_prefix = f.path().join("home").canonicalize().unwrap();
        let prefix = || located(&home, nowhere(), &usr_local).map(|h| h.0);
        // A file that is no link moves nothing. That holds for a `brew` that
        // links to it as well. A link into another repository moves nothing
        // either.
        std::fs::write(&usr_local_brew, "").unwrap();
        make_runnable(&usr_local_brew);
        give_brew_sh(&usr_local);
        std::os::unix::fs::symlink(&usr_local_brew, f.path().join("away/bin/brew")).unwrap();
        let away = f.path().join("away/bin").into_os_string();
        let away_prefix = f.path().join("away").canonicalize().unwrap();
        assert_eq!(
            located(&away, nowhere(), &usr_local).map(|h| h.0),
            Some(away_prefix)
        );
        assert_eq!(prefix(), Some(home_prefix.clone()));
        std::fs::remove_file(&usr_local_brew).unwrap();
        std::os::unix::fs::symlink(f.path().join("other/bin/brew"), &usr_local_brew).unwrap();
        assert_eq!(prefix(), Some(home_prefix.clone()));
        std::fs::remove_file(&usr_local_brew).unwrap();
        std::os::unix::fs::symlink(&brew, &usr_local_brew).unwrap();
        assert_eq!(
            located(&home, nowhere(), &usr_local),
            Some((usr_local.clone(), usr_local.join("Cellar")))
        );
        // Not where the prefix's own `Cellar` is a link.
        std::os::unix::fs::symlink("elsewhere", f.path().join("home/Cellar")).unwrap();
        assert_eq!(prefix(), Some(home_prefix));
    }

    #[test]
    fn brew_reads_one_link_to_find_its_repository() {
        // `hb/bin/brew` links into `hb/Homebrew` the way Homebrew installs
        // itself. `usr/local/bin/brew` links to that link.
        let f = Fixture::new(&[
            "hb/Homebrew/bin",
            "hb/Homebrew/bin/brew*",
            "hb/bin",
            "usr/local/bin",
        ]);
        make_runnable(&f.path().join("hb/Homebrew/bin/brew"));
        give_brew_sh(&f.path().join("hb/Homebrew"));
        std::os::unix::fs::symlink("../Homebrew/bin/brew", f.path().join("hb/bin/brew")).unwrap();
        let usr_local = f.path().join("usr/local");
        std::os::unix::fs::symlink(f.path().join("hb/bin/brew"), usr_local.join("bin/brew"))
            .unwrap();
        let root = f.path().canonicalize().unwrap();
        let path = f.path().join("hb/bin").into_os_string();
        // One link from `usr/local/bin/brew` reaches `hb` and `hb/bin/brew`
        // runs from `hb/Homebrew`. The prefix therefore stays.
        assert_eq!(
            located(&path, nowhere(), &usr_local),
            Some((root.join("hb"), root.join("hb/Cellar")))
        );
    }

    #[test]
    fn the_caskroom_and_opt_stay_under_the_prefix_when_the_cellar_does_not() {
        let f = Fixture::new(&[
            "repo/bin",
            "repo/bin/brew*",
            "repo/Cellar/sample-tool/1.0",
            "repo/Caskroom/sample-decoy",
            "repo/opt/sample-tool",
            "prefix/bin",
            "prefix/Caskroom/sample-app",
            "prefix/opt/sample-tool",
            "prefix/opt/sample-tool/sh.brew.sample-tool.plist*",
        ]);
        let brew = f.path().join("repo/bin/brew");
        make_runnable(&brew);
        give_brew_sh(&f.path().join("repo"));
        std::os::unix::fs::symlink(&brew, f.path().join("prefix/bin/brew")).unwrap();
        let path = f.path().join("prefix/bin").into_os_string();
        let found = brew_installed(&prefix_sources(&path, &[]));
        assert_eq!(found_names(&found), ["sample-tool", "sample-app"]);
        let found = brew_services(&prefix_sources(&path, &[]));
        assert_eq!(found_names(&found), ["sample-tool"]);
    }

    #[test]
    fn a_limit_drops_only_the_line_it_cuts() {
        let f = Fixture::new(&[]);
        let path = f.path().join("names");
        std::fs::write(&path, "sample-a\nsample-b").unwrap();
        assert_eq!(read_lines(&path, 17), ["sample-a", "sample-b"]);
        // A cut at a newline keeps the line in front of it and a cut inside
        // a line drops that line.
        assert_eq!(read_lines(&path, 9), ["sample-a"]);
        assert_eq!(read_lines(&path, 12), ["sample-a"]);
    }

    #[test]
    fn an_empty_name_gets_no_row() {
        let found = unlabelled(vec![String::new(), "sample-a".to_string()]);
        let rows = to_rows(found.iter(), "value", "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].insert, "sample-a");
    }

    #[test]
    fn a_reader_keeps_one_answer_for_the_words_in_front_of_the_one_typed() {
        let f = Fixture::new(&[]);
        let mut runs = Runs::default();
        let mut ask = |line: &[&str]| -> Vec<String> {
            rows("make", "make", "target", "", f.path(), line, &mut runs)
                .into_iter()
                .map(|r| r.insert)
                .collect()
        };
        // An empty answer is not kept.
        assert!(ask(&["make"]).is_empty());
        let makefile = f.path().join("Makefile");
        std::fs::write(&makefile, "sample-build:\n").unwrap();
        assert_eq!(ask(&["make"]), ["sample-build"]);
        std::fs::write(&makefile, "sample-test:\n").unwrap();
        assert_eq!(ask(&["make"]), ["sample-build"]);
        assert_eq!(ask(&["make", "sample-build"]), ["sample-test"]);
        // The answer for the shorter line was let go.
        std::fs::write(&makefile, "sample-last:\n").unwrap();
        assert_eq!(ask(&["make"]), ["sample-last"]);
        assert_eq!(runs.reads.len(), 1);
    }

    #[test]
    fn a_relative_home_reads_no_user_ssh_file() {
        let home = ssh_home(
            Some("Host sample-user-host\n"),
            Some("sample-known ssh-ed25519 AAAASAMPLE\n"),
        );
        let system = Fixture::new(&[]);
        let path = system.path().join("ssh_config");
        std::fs::write(&path, "Host sample-host\n").unwrap();
        // As many `..` as the process's own directory is deep reach `/` and
        // the rest names the fixture. The relative home still reaches that
        // `.ssh` and `ssh_hosts` must not read it.
        let up = std::env::current_dir().unwrap().components().count();
        let relative = std::iter::repeat_n("..", up)
            .collect::<PathBuf>()
            .join(home.path().strip_prefix("/").unwrap());
        assert!(relative.join(".ssh/config").is_file());
        let s = sources(nowhere(), &relative, &path);
        assert_eq!(ssh_hosts(&s), ["sample-host"]);
    }

    const SESSION_ONE: &str = "00000000-0000-4000-8000-000000000001";
    const SESSION_TWO: &str = "00000000-0000-4000-8000-000000000002";
    const SESSION_THREE: &str = "00000000-0000-4000-8000-000000000003";

    /// The record Claude Code writes for a prompt a person typed. Every prompt
    /// and title below is invented.
    fn prompt(text: &str) -> Value {
        serde_json::json!({
            "type": "user",
            "parentUuid": null,
            "entrypoint": "cli",
            "message": {"role": "user", "content": text},
        })
    }

    /// A Claude Code configuration directory and the directory in it that
    /// holds `cwd`'s sessions.
    fn claude_home(cwd: &Path) -> (Fixture, PathBuf) {
        let f = Fixture::new(&[]);
        let dir = f
            .path()
            .join("projects")
            .join(claude_project(cwd.to_str().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        (f, dir)
    }

    /// A session holding `records`, last changed `age` seconds ago.
    fn claude_session(dir: &Path, id: &str, records: &[Value], age: u64) {
        let path = dir.join(format!("{id}.jsonl"));
        let body: String = records.iter().map(|r| format!("{r}\n")).collect();
        std::fs::write(&path, body).unwrap();
        let when = SystemTime::now() - std::time::Duration::from_secs(age);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn claude_sources<'a>(cwd: &'a Path, config: &'a Path) -> Sources<'a> {
        Sources {
            claude_config: Some(config),
            ..sources(cwd, nowhere(), nowhere())
        }
    }

    #[test]
    fn claude_names_a_directory_the_way_claude_does() {
        assert_eq!(
            claude_project("/Users/sample/my.project_one"),
            "-Users-sample-my-project-one"
        );
        // Node gives `wacs1q` for this path's hash.
        let long = format!("/sample-{}/日本", "a".repeat(210));
        assert_eq!(
            claude_project(&long),
            format!("-sample-{}-wacs1q", "a".repeat(192))
        );
    }

    #[test]
    fn claude_sessions_show_their_titles_newest_first_and_go_in_by_id() {
        let cwd = Path::new("/sample/work");
        let (f, dir) = claude_home(cwd);
        claude_session(
            &dir,
            SESSION_ONE,
            &[
                prompt("sample first prompt"),
                serde_json::json!({"type": "ai-title", "aiTitle": "Sample older title"}),
                serde_json::json!({"type": "ai-title", "aiTitle": "Sample ai title"}),
                serde_json::json!({"type": "custom-title", "customTitle": "sample\nrenamed"}),
            ],
            30,
        );
        claude_session(
            &dir,
            SESSION_TWO,
            &[
                prompt("sample prompt"),
                serde_json::json!({"type": "ai-title", "aiTitle": "Sample ai title"}),
            ],
            10,
        );
        claude_session(&dir, SESSION_THREE, &[prompt("sample  only\tprompt")], 20);
        let s = claude_sources(cwd, f.path());
        let reader = reader("claude", "claude", "session").unwrap();
        let rows = candidates(reader, &s, "");
        let shown: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.display.as_str(), r.insert.as_str(), r.label.as_ref()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Sample ai title", SESSION_TWO, "00000000"),
                ("sample only prompt", SESSION_THREE, "00000000"),
                ("sample renamed", SESSION_ONE, "00000000"),
            ]
        );
        // A word of the title finds the row and the id is what goes in.
        let rows = candidates(reader, &s, "renamed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].insert, SESSION_ONE);
    }

    #[test]
    fn claude_sessions_match_id_prefixes_and_the_full_id() {
        let cwd = Path::new("/sample/work");
        let (f, dir) = claude_home(cwd);
        let id = "abf00000-0000-4000-8000-000000000001";
        claude_session(&dir, id, &[prompt("Sample session")], 0);
        claude_session(&dir, SESSION_TWO, &[prompt("Other session")], 0);
        let s = claude_sources(cwd, f.path());
        let reader = reader("claude", "claude", "session").unwrap();
        for end in [1, 2, 3, 8, 9, 13, id.len()] {
            let term = &id[..end];
            let rows = candidates(reader, &s, term);
            assert_eq!(rows.len(), 1, "{term}");
            assert_eq!(rows[0].display, "Sample session");
            assert_eq!(rows[0].insert, id);
            assert_eq!(rows[0].label, "abf00000");
        }
        let rows = candidates(reader, &s, "ABF");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].insert, id);
        assert!(candidates(reader, &s, "fff").is_empty());
    }

    #[test]
    fn claude_session_ids_match_only_prefixes_while_titles_match_fuzzily() {
        let cwd = Path::new("/sample/work");
        let (f, dir) = claude_home(cwd);
        let ids = [
            "abf00000-0000-4000-8000-000000000001",
            "0abf0000-0000-4000-8000-000000000002",
            "a0b0f000-0000-4000-8000-000000000003",
        ];
        for id in ids {
            claude_session(&dir, id, &[prompt("Sample session")], 0);
        }
        let s = claude_sources(cwd, f.path());
        let reader = reader("claude", "claude", "session").unwrap();
        let rows = candidates(reader, &s, "abf");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].insert, ids[0]);
        assert!(candidates(reader, &s, "000000000001").is_empty());
        assert_eq!(candidates(reader, &s, "smss").len(), ids.len());
    }

    #[test]
    fn claude_sessions_leave_out_what_its_own_picker_leaves_out() {
        let cwd = Path::new("/sample/work");
        let (f, dir) = claude_home(cwd);
        let print = serde_json::json!({
            "type": "user",
            "parentUuid": null,
            "entrypoint": "sdk-cli",
            "message": {"content": "sample print prompt"},
        });
        claude_session(&dir, SESSION_ONE, &[print], 0);
        let daemon = serde_json::json!({
            "type": "user",
            "parentUuid": null,
            "sessionKind": "daemon",
            "message": {"content": "sample daemon prompt"},
        });
        claude_session(&dir, SESSION_TWO, &[daemon], 0);
        // Nothing was typed into this one.
        claude_session(
            &dir,
            SESSION_THREE,
            &[serde_json::json!({"type": "mode", "mode": "normal"})],
            0,
        );
        claude_session(&dir, "sample-notes", &[prompt("sample notes")], 0);
        std::fs::create_dir(dir.join("00000000-0000-4000-8000-000000000004.jsonl")).unwrap();
        let s = claude_sources(cwd, f.path());
        assert!(claude_sessions(&s).is_empty());
        // Another directory's sessions are not this one's.
        let s = claude_sources(Path::new("/sample/other"), f.path());
        assert!(claude_sessions(&s).is_empty());
    }

    #[test]
    fn a_first_prompt_is_what_a_person_typed() {
        let lines =
            |records: &[Value]| -> String { records.iter().map(|r| format!("{r}\n")).collect() };
        let meta = serde_json::json!({
            "type": "user",
            "isMeta": true,
            "message": {"content": "sample skill body"},
        });
        let result = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "tool_result", "content": "sample"}]},
        });
        let command = prompt("<command-name>/clear</command-name>");
        let stdout = prompt("<local-command-stdout></local-command-stdout>");
        let typed = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "text", "text": "sample\ntyped"}]},
        });
        let head = lines(&[meta.clone(), command.clone(), stdout.clone(), result, typed]);
        assert_eq!(first_prompt(&head).as_deref(), Some("sample typed"));
        // A slash command stands in where nothing else was typed.
        let head = lines(&[meta, command, stdout]);
        assert_eq!(first_prompt(&head).as_deref(), Some("/clear"));
        let head = lines(&[prompt("<bash-input> ls sample </bash-input>")]);
        assert_eq!(first_prompt(&head).as_deref(), Some("! ls sample"));
        let head = lines(&[prompt(&"a".repeat(300))]);
        assert_eq!(first_prompt(&head), Some(format!("{}…", "a".repeat(200))));
        // An emoji is two UTF-16 units and the limit falls between them.
        let head = lines(&[prompt(&format!("{}😀b", "a".repeat(199)))]);
        assert_eq!(first_prompt(&head), Some(format!("{}…", "a".repeat(199))));
        assert_eq!(first_prompt(""), None);
    }

    #[test]
    fn claude_reads_the_two_ends_of_a_long_session() {
        let f = Fixture::new(&[]);
        let path = f.path().join("sample.jsonl");
        std::fs::write(&path, "head\nmiddle\ntail\n").unwrap();
        let (head, tail, _) = read_ends(&path, 6).unwrap();
        assert_eq!((head.as_str(), tail.as_str()), ("head\nm", "\ntail\n"));
        let (head, tail, _) = read_ends(&path, 64).unwrap();
        assert_eq!(head, tail);
    }

    #[test]
    fn claude_keeps_its_state_where_its_own_variable_says() {
        let home = Path::new("/sample-home");
        assert_eq!(
            claude_config(Some(PathBuf::from("/sample-config")), home),
            Some(PathBuf::from("/sample-config"))
        );
        assert_eq!(
            claude_config(Some(PathBuf::new()), home),
            Some(PathBuf::from("/sample-home/.claude"))
        );
        assert_eq!(claude_config(None, Path::new("sample-home")), None);
    }
}
