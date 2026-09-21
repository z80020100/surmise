//! An interactive zsh in a home this command makes and takes away again.
//!
//! `surmise demo` is how a person tries the menu before editing a `.zshrc`
//! of their own, and how a reviewer reads a branch back at a real terminal.
//! It points `$HOME`, `$ZDOTDIR`, the two XDG directories and `$HISTFILE` at
//! a throwaway directory, writes a small repository under it and starts zsh
//! there. The directory goes when that shell exits.
//!
//! It is not a sandbox and nothing here claims to be one. The filesystem
//! around that home is the real one and a command typed in the demo shell
//! runs the way it always would. What moves is where surmise, zsh and Git
//! look for the files a person keeps.
//!
//! [`crate::fixture::Fixture`] makes a throwaway tree as well and this does
//! not use it. That one panics where a test wants a panic, widens the Git
//! query budget because no prompt is waiting on a test, and commits an empty
//! tree. A demo wants none of the three.

use std::fs::DirBuilder;
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The startup file the demo shell reads, compiled in for the reason the
/// widget is: `cargo install` places a binary and nothing beside it.
///
/// It carries no path of its own. `$SURMISE_BIN` is the one thing it reads
/// out of the environment and [`shell`] is what puts it there, so `make
/// shell` checks the bytes that run rather than a template of them.
const RC: &str = include_str!("../shell/demo.zsh");

/// The targets the demo makefile carries. `make ` in the demo shell is the
/// native reader answering out of this list.
const MAKE_TARGETS: [&str; 5] = ["build", "test", "lint", "package", "clean"];

/// The hosts the demo SSH configuration carries. Every one of them resolves
/// nowhere. `.invalid` is the domain reserved for exactly that and a demo
/// therefore cannot name a machine somebody owns.
const SSH_HOSTS: [&str; 4] = ["build-north", "build-south", "cache-relay", "docs-mirror"];

/// The branches beside the one the commit lands on.
const BRANCHES: [&str; 4] = [
    "feature/blue-label",
    "feature/green-label",
    "fix/tab-order",
    "release/2.0",
];

/// What the demo repository commits. `git add ` offers what [`dirty`] puts
/// on top of these afterwards.
const COMMITTED: [(&str, &str); 5] = [
    (
        "README.md",
        "# sample\n\nA repository `surmise demo` made. Nothing in it is real.\n",
    ),
    ("docs/guide.md", "# Guide\n\nOne page.\n"),
    ("src/main.rs", "fn main() {\n    println!(\"sample\");\n}\n"),
    ("src/helper.rs", "pub fn helper() -> u8 {\n    1\n}\n"),
    ("assets/icons/marker.svg", "<svg></svg>\n"),
];

/// What the working tree carries on top of that commit. `git add ` has a
/// modification and two untracked files to offer because of it. The first
/// path is one [`COMMITTED`] already wrote.
const DIRT: [(&str, &str); 3] = [
    (
        "src/main.rs",
        "fn main() {\n    println!(\"sample\");\n    println!(\"edited\");\n}\n",
    ),
    ("docs/notes.md", "Untracked.\n"),
    ("assets/icons/arrow.svg", "<svg></svg>\n"),
];

/// The history the `$HISTFILE` tie-break reads. A demo opens on the order a
/// person's own typing would have earned rather than on the alphabet. Both
/// shapes zsh writes are in it: `EXTENDED_HISTORY` puts a timestamp in front
/// of the line and a plain history file is the line alone.
const HISTORY: &str = ": 1758000001:0;git status\n\
                       : 1758000002:0;git status\n\
                       : 1758000003:0;cargo test\n\
                       : 1758000004:0;git commit -m sample\n\
                       : 1758000005:0;docker ps\n\
                       : 1758000006:0;git status\n\
                       : 1758000007:0;docker ps\n\
                       git status\n\
                       git switch feature/blue-label\n\
                       cargo test\n\
                       git status\n\
                       docker compose up\n\
                       git status\n";

/// The directory the demo lives in for as long as its shell does.
struct Root(PathBuf);

impl Root {
    /// A directory of this process's own under the temporary one.
    ///
    /// The counter answers a name already taken. That is a second demo in
    /// one process, which nothing does today, or a directory a killed run
    /// left behind under a process id the system has since handed out
    /// again. Neither is this run's to delete and the next name is free.
    fn new() -> Result<Root, String> {
        let base = std::env::temp_dir();
        let pid = std::process::id();
        for n in 0..100 {
            let path = base.join(format!("surmise-demo-{pid}-{n}"));
            // The mode `history.rs` makes its own directory with. The
            // temporary directory is shared and what a person types in the
            // demo reaches the database under this one.
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Root(path)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("cannot make {}: {e}", path.display())),
            }
        }
        Err(format!(
            "no free name for a demo home under {}",
            base.display()
        ))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where each part of a demo went.
struct Layout {
    home: PathBuf,
    zdotdir: PathBuf,
    repo: PathBuf,
    histfile: PathBuf,
}

/// Build the demo home and hand the terminal to a zsh inside it. This
/// returns when that shell exits, and the home goes with the return.
pub fn run() -> Result<(), String> {
    // The demo is whatever binary was run rather than whatever the PATH
    // holds. A person trying a build out of a clone has no other one.
    let binary =
        std::env::current_exe().map_err(|e| format!("cannot find the running binary: {e}"))?;
    let root = Root::new()?;
    let layout = build(&root.0)?;
    if !init_repo(&layout.repo, &layout.home) {
        // Every menu that is not Git's own still works. Saying so beats a
        // demo that silently drops a third of what its banner offers.
        eprintln!("surmise: no Git here. The demo opens on a directory rather than a repository.");
    }
    dirty(&layout.repo)?;
    shell(&layout, &binary)
}

/// Write everything that has to be there before the commit is made.
fn build(root: &Path) -> Result<Layout, String> {
    let zdotdir = root.join("zdotdir");
    let layout = Layout {
        home: root.join("home"),
        repo: root.join("repo"),
        histfile: zdotdir.join(".histfile"),
        zdotdir,
    };
    write(&layout.zdotdir.join(".zshrc"), RC)?;
    write(&layout.histfile, HISTORY)?;
    write(&layout.home.join(".ssh").join("config"), &ssh_config())?;
    // Nothing makes the XDG directories here. `history.rs` and `state.rs`
    // each make their own with a mode of their own when they first write,
    // and a directory made here would be the one they found instead.
    write(&layout.repo.join("Makefile"), &makefile())?;
    for (name, text) in COMMITTED {
        write(&layout.repo.join(name), text)?;
    }
    Ok(layout)
}

/// Put the working tree out of step with the commit.
fn dirty(repo: &Path) -> Result<(), String> {
    for (name, text) in DIRT {
        write(&repo.join(name), text)?;
    }
    Ok(())
}

/// One recipe per target and `@echo` as the whole of it. A demo completes a
/// command line rather than runs one.
fn makefile() -> String {
    MAKE_TARGETS
        .iter()
        .map(|target| format!("{target}:\n\t@echo {target}\n\n"))
        .collect()
}

fn ssh_config() -> String {
    SSH_HOSTS
        .iter()
        .map(|host| format!("Host {host}\n  HostName {host}.example.invalid\n  User sample\n\n"))
        .collect()
}

/// Write `text` at `path`, making the directories above it first.
fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Make the repository the demo opens in. `false` where Git is missing or
/// refused a step. That is not fatal: the rest of the demo stands without a
/// repository and [`run`] says what was lost.
fn init_repo(repo: &Path, home: &Path) -> bool {
    let steps: [&[&str]; 4] = [
        &["init", "--quiet", "--template=", "--initial-branch=main"],
        // The machine's own excludes file reaches this repository through
        // the Git a menu runs for itself. A name the person ignores at home
        // would otherwise go missing from what `git add ` offers here.
        &["config", "core.excludesFile", "/dev/null"],
        &["add", "--all", "."],
        &["commit", "--quiet", "-m", "Sample"],
    ];
    steps.iter().all(|args| git(repo, home, args).is_some())
        && BRANCHES
            .iter()
            .all(|branch| git(repo, home, &["branch", branch]).is_some())
}

/// Run Git in `repo` with a synthetic identity and none of the machine's own
/// configuration. What it printed, or `None` where Git is missing or the
/// command failed.
///
/// The output is captured rather than inherited. This runs before the demo
/// shell draws its first prompt and a line from Git there would be the first
/// thing a person saw.
fn git(repo: &Path, home: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Sample")
        .env("GIT_AUTHOR_EMAIL", "sample@example.invalid")
        .env("GIT_COMMITTER_NAME", "Sample")
        .env("GIT_COMMITTER_EMAIL", "sample@example.invalid")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Hand the terminal to zsh and wait for it to leave.
fn shell(layout: &Layout, binary: &Path) -> Result<(), String> {
    let mut cmd = Command::new("zsh");
    cmd.arg("-i");
    // `-d` drops the machine's own `/etc` startup files. `$ZDOTDIR` is then
    // the whole of what this shell reads. Debian and Ubuntu ship an
    // `/etc/zsh/zshrc` that calls `compinit` with no flag and on a machine
    // whose completion directories are group writable that call asks the
    // terminal a question ahead of the first prompt. `tests/pty/zsh.rs`
    // passes the same flag for the same reason.
    cmd.arg("-d");
    cmd.current_dir(&layout.repo);
    cmd.env("HOME", &layout.home);
    cmd.env("ZDOTDIR", &layout.zdotdir);
    cmd.env("XDG_DATA_HOME", layout.home.join(".local").join("share"));
    cmd.env("XDG_CONFIG_HOME", layout.home.join(".config"));
    cmd.env("HISTFILE", &layout.histfile);
    cmd.env("SURMISE_BIN", binary);
    // Git reads `/etc/gitconfig` whatever `$HOME` says. One machine's
    // system-wide aliases in the demo menu would not be surmise.
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    // SAFETY: `signal` with `SIG_DFL` is async-signal-safe and touches no
    // memory of ours, which is what a closure between fork and exec is
    // allowed to do. It is here because the parent ignores those two keys
    // below and a child inherits that. Nothing typed in the demo shell
    // could be interrupted without it.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            Ok(())
        });
    }
    hold_the_keys();
    // The status belongs to the last command a person ran in the demo rather
    // than to the demo. What this command has to report is whether the shell
    // started at all.
    cmd.status().map_err(|e| format!("cannot start zsh: {e}"))?;
    Ok(())
}

/// Take Ctrl-C and Ctrl-\ away from this process for the rest of its life.
///
/// The terminal sends both to every process in the foreground group and this
/// one is in that group. Dying there would leave the demo shell up with no
/// parent left to take its home away afterwards.
fn hold_the_keys() {
    // SAFETY: `signal` with `SIG_IGN` is async-signal-safe and touches no
    // memory of ours.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    /// A demo laid out under a directory of the test's own.
    fn laid_out() -> (Fixture, Layout) {
        let f = Fixture::new(&[]);
        let layout = build(f.path()).expect("a demo home");
        (f, layout)
    }

    #[test]
    fn the_home_goes_when_the_demo_does() {
        let path = Root::new().expect("a demo home").0.clone();
        assert!(!path.exists(), "{} stayed", path.display());
    }

    /// `include_str!` cannot say what it took. This is the claim that the
    /// file the demo shell reads is the widget-installing one in `shell/`.
    #[test]
    fn the_shell_reads_the_committed_startup_file() {
        let (_f, layout) = laid_out();
        let rc = std::fs::read_to_string(layout.zdotdir.join(".zshrc")).expect("a zshrc");
        assert_eq!(rc, RC);
        assert!(RC.contains("eval \"$($SURMISE_BIN init zsh)\""));
    }

    #[test]
    fn the_makefile_is_one_the_make_reader_answers_from() {
        let (_f, layout) = laid_out();
        let rows = crate::native::rows("make", "target", "", &layout.repo);
        let names: Vec<&str> = rows.iter().map(|row| row.display.as_str()).collect();
        for target in MAKE_TARGETS {
            assert!(names.contains(&target), "{names:?}");
        }
    }

    /// The demo ships in the repository and a name in it reaches anybody who
    /// runs one. Every host here has to be a name that resolves nowhere.
    #[test]
    fn no_host_in_the_ssh_configuration_is_a_real_one() {
        let (_f, layout) = laid_out();
        let text =
            std::fs::read_to_string(layout.home.join(".ssh").join("config")).expect("a config");
        for host in SSH_HOSTS {
            assert!(text.contains(&format!("Host {host}\n")), "{text}");
        }
        for line in text.lines().filter(|l| l.contains("HostName")) {
            assert!(line.ends_with(".example.invalid"), "{line}");
        }
    }

    #[test]
    fn the_repository_arrives_with_branches_and_something_to_stage() {
        let (_f, layout) = laid_out();
        assert!(init_repo(&layout.repo, &layout.home), "no repository");
        dirty(&layout.repo).expect("a working tree");
        let read = |args: &[&str]| git(&layout.repo, &layout.home, args).expect("a Git answer");
        let status = read(&["status", "--porcelain"]);
        assert!(status.contains(" M src/main.rs"), "{status}");
        assert!(status.contains("?? docs/notes.md"), "{status}");
        assert!(status.contains("?? assets/icons/arrow.svg"), "{status}");
        let branches = read(&["branch", "--format=%(refname:short)"]);
        for branch in BRANCHES {
            assert!(branches.contains(branch), "{branches}");
        }
    }
}
