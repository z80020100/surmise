//! An interactive zsh in a home this command makes and takes away again.
//!
//! `surmise demo` is how a person tries the menu before editing a `.zshrc`
//! of their own, and how a reviewer reads a branch back at a real terminal.
//! It points `$HOME`, `$ZDOTDIR`, the two XDG directories and `$HISTFILE` at
//! a throwaway directory and starts zsh on it. The directory goes when that
//! shell exits.
//!
//! **The working directory is the one the command was run in.** A menu here
//! therefore completes on the person's own files, their own branches and the
//! makefile beside them rather than on a repository this module made up. A
//! line run in that shell runs where they started it.
//!
//! The throwaway home is what a demo cannot borrow from them. The directory
//! history, the state file and the config go into it rather than into the
//! ones they keep. The SSH configuration and the history file are fixtures
//! because `$HOME` and `$HISTFILE` both move and the readers that answer
//! from those two would have nothing to show otherwise.
//!
//! It is not a sandbox and nothing here claims to be one. The filesystem
//! around that home is the real one. What moves is where surmise, zsh and
//! Git look for the files a person keeps.

use std::fs::DirBuilder;
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The startup file the demo shell reads, compiled in for the reason the
/// widget is: `cargo install` places a binary and nothing beside it.
///
/// It carries no path of its own. `$SURMISE_BIN` is the one thing it reads
/// out of the environment and [`shell`] is what puts it there, so `make
/// shell` checks the bytes that run rather than a template of them.
const RC: &str = include_str!("../shell/demo.zsh");

/// The hosts the demo SSH configuration carries. `$HOME` moves, so the
/// person's own configuration cannot be read from the demo and this is what
/// `ssh ` answers from instead. Every name resolves nowhere. `.invalid` is
/// the domain reserved for exactly that and a demo therefore cannot name a
/// machine somebody owns.
const SSH_HOSTS: [&str; 4] = ["build-north", "build-south", "cache-relay", "docs-mirror"];

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

/// Where each part of a demo home went.
struct Layout {
    home: PathBuf,
    zdotdir: PathBuf,
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
    shell(&layout, &binary)
}

/// Write the three files the demo home is.
fn build(root: &Path) -> Result<Layout, String> {
    let zdotdir = root.join("zdotdir");
    let layout = Layout {
        home: root.join("home"),
        histfile: zdotdir.join(".histfile"),
        zdotdir,
    };
    write(&layout.zdotdir.join(".zshrc"), RC)?;
    write(&layout.histfile, HISTORY)?;
    write(&layout.home.join(".ssh").join("config"), &ssh_config())?;
    // Nothing makes the XDG directories here. `history.rs` and `state.rs`
    // each make their own with a mode of their own when they first write,
    // and a directory made here would be the one they found instead.
    Ok(layout)
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
    // The working directory is inherited rather than set. It is the whole of
    // what a demo shows a person about their own files.
    cmd.env("HOME", &layout.home);
    cmd.env("ZDOTDIR", &layout.zdotdir);
    cmd.env("XDG_DATA_HOME", layout.home.join(".local").join("share"));
    cmd.env("XDG_CONFIG_HOME", layout.home.join(".config"));
    cmd.env("HISTFILE", &layout.histfile);
    cmd.env("SURMISE_BIN", binary);
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

    /// A demo home laid out under a directory of the test's own.
    fn laid_out() -> (Fixture, Layout) {
        let f = Fixture::new(&[]);
        let layout = build(f.path()).expect("a demo home");
        (f, layout)
    }

    /// Every file under `dir`, at any depth.
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).expect("a directory").flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out.sort();
        out
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

    /// The home is three files. A fourth would be the demo deciding
    /// something for a person rather than lending them a shell.
    #[test]
    fn the_home_holds_three_files_and_nothing_else() {
        let (f, layout) = laid_out();
        let mut want = vec![
            layout.home.join(".ssh").join("config"),
            layout.histfile.clone(),
            layout.zdotdir.join(".zshrc"),
        ];
        want.sort();
        assert_eq!(walk(f.path()), want);
    }
}
