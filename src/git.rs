//! Git subcommand and branch completion from the installed Git.

use crate::candidates::{Candidate, Kind, MAX_RESULTS, Query, match_rank};
use crate::fuzzy;
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// What the prompt allows the query. `read_commands` takes its patience as an
/// argument rather than reading this, because a test measured against the
/// prompt's own budget fails on a loaded machine rather than on the code.
const TIMEOUT: Duration = Duration::from_millis(250);
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
}

/// Read the subcommand or the first branch argument. Options and shell syntax
/// stay with the shell's completion.
pub(crate) fn parse(left: &str) -> Option<Target> {
    let trimmed = left.trim_start_matches([' ', '\t']);
    let rest = trimmed.strip_prefix("git")?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let word = rest.trim_start_matches([' ', '\t']);
    let (arg, kind) = match word.split_once([' ', '\t']) {
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

fn read_output(command: &mut Command, patience: Duration) -> Option<(ExitStatus, String)> {
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
) -> Option<(ExitStatus, String)> {
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
    Some((status, String::from_utf8(bytes).ok()?))
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
            TIMEOUT.checked_sub(start.elapsed())?,
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

/// Cache command names and branch names separately for one menu.
/// An empty result stays empty until the next menu.
#[derive(Default)]
pub(crate) struct Completions {
    names: Option<Vec<String>>,
    branches: Option<Vec<String>>,
}

impl Completions {
    pub(crate) fn candidates(&mut self, arg: &str, cwd: &Path, kind: Kind) -> Vec<Candidate> {
        let names = if kind == Kind::Branch {
            self.branches
                .get_or_insert_with(|| read_branches(cwd).unwrap_or_default())
        } else {
            self.names.get_or_insert_with(|| {
                read_commands(Command::new("git").current_dir(cwd).arg(LIST_CMDS), TIMEOUT)
                    .unwrap_or_default()
            })
        };
        let label = if kind == Kind::Branch {
            "branch"
        } else {
            "command"
        };
        let mut out: Vec<_> = names
            .iter()
            .filter_map(|name| {
                // The score decides before the name is cloned. Most names
                // reach nothing on a word with anything typed into it.
                let score = fuzzy::score(arg, name)?;
                Some(Candidate {
                    display: name.clone(),
                    insert: name.clone(),
                    label,
                    kind,
                    score,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            match_rank(arg, &b.display)
                .cmp(&match_rank(arg, &a.display))
                .then(b.score.cmp(&a.score))
                .then(a.display.cmp(&b.display))
        });
        out.truncate(MAX_RESULTS);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let rows = commands.candidates("sw", Path::new("/no-such-directory-here"), Kind::Command);
        assert_eq!(
            rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            ["sw", "switch", "show"]
        );
        assert!(
            commands
                .candidates("zzz", Path::new("."), Kind::Command)
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
