//! Git subcommand completion from the installed Git.

use crate::candidates::{Candidate, Kind, MAX_RESULTS, Query, match_rank};
use crate::fuzzy;
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

/// Recognise the subcommand word of a `git` left of the cursor. Anything that
/// is not a plain name predicts nothing. An option, a quote and every piece of
/// shell syntax belong to the shell's own completion. So does a word with a
/// second one behind it.
pub(crate) fn parse(left: &str) -> Option<Query> {
    let trimmed = left.trim_start_matches([' ', '\t']);
    let rest = trimmed.strip_prefix("git")?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let arg = rest.trim_start_matches([' ', '\t']);
    if !plain_name(arg) {
        return None;
    }
    Some(Query {
        start: left.len() - arg.len(),
        arg: arg.to_string(),
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

/// The names this machine's Git offers. The menu asks Git once and answers
/// every later key from the same list. A query that came back with nothing
/// keeps that answer rather than asking again at the next keystroke.
#[derive(Default)]
pub(crate) struct Commands(Option<Vec<String>>);

impl Commands {
    pub(crate) fn candidates(&mut self, arg: &str, cwd: &Path) -> Vec<Candidate> {
        let names = self.0.get_or_insert_with(|| {
            read_commands(Command::new("git").current_dir(cwd).arg(LIST_CMDS), TIMEOUT)
                .unwrap_or_default()
        });
        let mut out: Vec<_> = names
            .iter()
            .filter_map(|name| {
                // The score decides before the name is cloned. Most names
                // reach nothing on a word with anything typed into it.
                let score = fuzzy::score(arg, name)?;
                Some(Candidate {
                    display: name.clone(),
                    insert: name.clone(),
                    label: "command",
                    kind: Kind::Command,
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
            let q = parse(line).unwrap();
            assert_eq!(&line[q.start..], q.arg);
        }
        for line in [
            "git",
            "github ",
            "git -C ",
            "git switch ",
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
        let mut commands = Commands(Some(vec![
            "sample".into(),
            "sw".into(),
            "switch".into(),
            "show".into(),
        ]));
        let rows = commands.candidates("sw", Path::new("/no-such-directory-here"));
        assert_eq!(
            rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            ["sw", "switch", "show"]
        );
        assert!(commands.candidates("zzz", Path::new(".")).is_empty());
    }
}
