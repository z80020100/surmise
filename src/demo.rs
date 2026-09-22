//! A clean interactive zsh on the real filesystem, with surmise the one
//! thing loaded into it.
//!
//! `surmise demo` is how a person tries the menu before editing a `.zshrc`
//! of their own, and how a reviewer reads a branch back at a real terminal.
//! Both of them are asking whether this works for them, so everything it
//! reads is theirs. `$HOME` is theirs, the working directory is the one the
//! command was run in, and the config, the directory history and the state
//! file are the ones an installed surmise would read and write. A `settings
//! set` typed here changes what they keep and a `cd` run here is a visit
//! their next menu ranks by. That is the point rather than a cost: a
//! setting that only ever reached a copy would answer nothing about the
//! setting.
//!
//! **The working directory is the one the command was run in.** A menu here
//! therefore completes on the person's own files, their own branches and the
//! makefile beside them rather than on a repository this module made up. A
//! line run in that shell runs where they started it.
//!
//! `$ZDOTDIR` is the one thing that moves. A person trying surmise before
//! they install it has no `eval "$(surmise init zsh)"` line in a `.zshrc` of
//! their own and writing one into it is the one thing a demo may not do, so
//! the demo writes a `.zshrc` of its own under a throwaway directory and
//! sends the shell at it. `-d` drops the machine's own `/etc` files beside
//! that. What is loaded in that shell is therefore surmise and nothing
//! else, which is what leaves a menu that misbehaves nowhere to hide.
//!
//! `$HISTFILE` is the one thing read and never written. `SAVEHIST=0` in that
//! `.zshrc` is what keeps the demo's own lines out of the file, because a
//! line typed in a demo is worth nothing to the ranking afterwards.
//!
//! Which file that is, is a guess. zsh exports no `HISTFILE` and the
//! `.zshrc` naming it is the one this shell does not read, so the startup
//! file falls back to `~/.zsh_history` and says so where nothing is there.
//! Reading a person's own `.zshrc` for the name would load everything else
//! in it as well, which is the one thing a clean shell is for not doing.
//!
//! It is not a sandbox and nothing here claims to be one. The filesystem is
//! the real one and so is everything surmise writes to it.

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

/// The directory the demo's own startup file lives in for as long as its
/// shell does.
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
            // temporary directory above this one is shared and a directory
            // a process makes for itself there is its own.
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Root(path)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("cannot make {}: {e}", path.display())),
            }
        }
        Err(format!(
            "no free name for a demo directory under {}",
            base.display()
        ))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write the demo's own startup file and hand the terminal to a zsh reading
/// it. This returns when that shell exits, and the file goes with the
/// return.
pub fn run() -> Result<(), String> {
    // The demo is whatever binary was run rather than whatever the PATH
    // holds. A person trying a build out of a clone has no other one.
    let binary =
        std::env::current_exe().map_err(|e| format!("cannot find the running binary: {e}"))?;
    let root = Root::new()?;
    let zdotdir = build(&root.0)?;
    shell(&zdotdir, &binary)
}

/// Write the demo's own `.zshrc` under `root` and answer the directory it
/// went in.
fn build(root: &Path) -> Result<PathBuf, String> {
    let zdotdir = root.join("zdotdir");
    write(&zdotdir.join(".zshrc"), RC)?;
    Ok(zdotdir)
}

/// What the demo shell is given beside what it inherits.
///
/// Two names and no more. The config, the directory history and the state
/// file are each resolved out of `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME` and
/// `$HOME`, so a name of the demo's own for any of those three would send
/// surmise somewhere other than where an installed one writes. `$HOME` and
/// `$HISTFILE` are inherited for that same reason and the readers that
/// answer from them — the SSH hosts, the `~` row, the `$HISTFILE`
/// tie-break — then answer out of what a person has.
fn environment(zdotdir: &Path, binary: &Path) -> [(&'static str, PathBuf); 2] {
    [
        ("ZDOTDIR", zdotdir.to_path_buf()),
        ("SURMISE_BIN", binary.to_path_buf()),
    ]
}

/// Write `text` at `path`, making the directories above it first.
///
/// The mode is the one `atomic.rs` and `history.rs` each make their own
/// directory with. The temporary directory this sits under is shared.
fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Hand the terminal to zsh and wait for it to leave.
fn shell(zdotdir: &Path, binary: &Path) -> Result<(), String> {
    let mut cmd = Command::new("zsh");
    cmd.arg("-i");
    // `-d` drops the machine's own `/etc` startup files. `$ZDOTDIR` is then
    // the whole of what this shell reads, which is what has surmise be the
    // one thing loaded in it. Debian and Ubuntu also ship an `/etc/zsh/zshrc`
    // that calls `compinit` with no flag and on a machine whose completion
    // directories are group writable that call asks the terminal a question
    // ahead of the first prompt. `tests/pty/zsh.rs` passes the same flag for
    // the same reason.
    cmd.arg("-d");
    // The working directory is inherited rather than set. It is the whole of
    // what a demo shows a person about their own files.
    for (name, value) in environment(zdotdir, binary) {
        cmd.env(name, value);
    }
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
/// parent left to take its startup file away afterwards.
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
    fn laid_out() -> (Fixture, PathBuf) {
        let f = Fixture::new(&[]);
        let zdotdir = build(f.path()).expect("a demo");
        (f, zdotdir)
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
    fn the_directory_goes_when_the_demo_does() {
        let path = Root::new().expect("a demo directory").0.clone();
        assert!(!path.exists(), "{} stayed", path.display());
    }

    /// `include_str!` cannot say what it took. This is the claim that the
    /// file the demo shell reads is the widget-installing one in `shell/`.
    #[test]
    fn the_shell_reads_the_committed_startup_file() {
        let (_f, zdotdir) = laid_out();
        let rc = std::fs::read_to_string(zdotdir.join(".zshrc")).expect("a zshrc");
        assert_eq!(rc, RC);
        assert!(RC.contains("eval \"$($SURMISE_BIN init zsh)\""));
    }

    /// The startup file is the whole of what `build` writes. Everything else
    /// a demo touches is a file the person keeps and a second file written
    /// here would be a copy of one of those, which is the answer this
    /// command stopped giving. The shell that reads the file writes a
    /// `.zcompdump` beside it and that goes with the directory.
    #[test]
    fn the_startup_file_is_the_whole_of_what_the_demo_writes() {
        let (f, zdotdir) = laid_out();
        assert_eq!(walk(f.path()), [zdotdir.join(".zshrc")]);
    }

    /// The demo names the directory its own startup file is in and nothing
    /// else. A name here for the config, the data directory or the home
    /// would be surmise reading somewhere an installed one never would.
    #[test]
    fn the_demo_names_no_directory_but_the_one_its_startup_file_is_in() {
        let (_f, zdotdir) = laid_out();
        let binary = Path::new("/usr/local/bin/surmise");
        let env = environment(&zdotdir, binary);
        assert_eq!(
            env.iter().map(|(name, _)| *name).collect::<Vec<&str>>(),
            ["ZDOTDIR", "SURMISE_BIN"]
        );
        assert_eq!(env[0].1, zdotdir);
        assert_eq!(env[1].1, binary);
    }
}
