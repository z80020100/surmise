//! `surmise doctor`: what stands between a person and a menu that opens.
//!
//! It runs as a child of the shell that typed it and a child cannot read
//! that shell's own state. The widget exports `SURMISE_WIDGET` when it loads
//! for that reason. It names the record the widget writes, the zsh it runs
//! under and that shell's own process. Everything else is the environment
//! and the files a picker would read itself.
//!
//! Each check is one line. `ok` is a check that passed, `warn` one that
//! leaves the menu working with something missing and `fail` one that keeps
//! it from opening at all. Any `fail` makes the exit status 1.

use crate::{config, history, pick, spec_store};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

/// One line of the report.
#[derive(Debug)]
pub struct Check {
    pub level: Level,
    pub name: &'static str,
    pub says: String,
}

/// What the checks read, gathered at one edge so a test hands in its own.
pub struct Facts {
    /// This binary, resolved through every link.
    pub exe: Option<PathBuf>,
    /// What the widget runs, `$SURMISE_BIN` or `surmise`, as given.
    pub bin: String,
    /// That binary found on `PATH` and resolved the same way.
    pub bin_found: Option<PathBuf>,
    /// `$SURMISE_WIDGET`.
    pub widget: Option<String>,
    /// The process that started this one.
    pub parent: u32,
    /// What reading the config said went wrong, and where the file is.
    pub config_warning: Option<String>,
    pub config_path: Option<PathBuf>,
    /// Where the directory history goes.
    pub database: Option<PathBuf>,
    /// Whether the directory that file goes in can be written.
    pub database_writable: bool,
    /// Whether a `git` runs.
    pub git: bool,
}

impl Facts {
    /// The facts of this process.
    pub fn here() -> Facts {
        let bin = std::env::var("SURMISE_BIN")
            .ok()
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "surmise".to_string());
        let database = history::database();
        let config = config::Config::load();
        Facts {
            exe: std::env::current_exe()
                .ok()
                .and_then(|e| e.canonicalize().ok()),
            bin_found: find(&bin),
            bin,
            widget: std::env::var("SURMISE_WIDGET").ok(),
            parent: std::os::unix::process::parent_id(),
            config_warning: config.warning,
            config_path: config::path(),
            database_writable: database.as_deref().is_some_and(writable),
            database,
            git: Command::new("git")
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success()),
        }
    }
}

/// `bin` the way the shell would run it: a path as written, or the first
/// file on `PATH` for a bare name that can be run. The shell passes over one
/// that cannot.
fn find(bin: &str) -> Option<PathBuf> {
    if bin.contains('/') {
        return Path::new(bin).canonicalize().ok();
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(bin))
        .find(|path| {
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .and_then(|path| path.canonicalize().ok())
}

/// Whether the directory `file` would go in takes a write, or the nearest
/// directory above it that exists does where it is still to be made.
fn writable(file: &Path) -> bool {
    let Some(dir) = file.ancestors().skip(1).find(|dir| dir.is_dir()) else {
        return false;
    };
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: the path is a terminated string that lives for the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

fn check(level: Level, name: &'static str, says: impl Into<String>) -> Check {
    Check {
        level,
        name,
        says: says.into(),
    }
}

/// Every check, in the order the report prints them.
pub fn checks(facts: &Facts) -> Vec<Check> {
    vec![
        binary(facts),
        widget(facts),
        check(
            Level::Ok,
            "specs",
            format!(
                "{} commands from corpus {}",
                spec_store::commands(&[]).len(),
                spec_store::corpus_version()
            ),
        ),
        settings(facts),
        history_dir(facts),
        if facts.git {
            check(Level::Ok, "git", "on PATH")
        } else {
            check(
                Level::Warn,
                "git",
                "not on PATH. Git's own menu and its readers answer nothing",
            )
        },
    ]
}

fn binary(facts: &Facts) -> Check {
    match (&facts.bin_found, &facts.exe) {
        (None, _) => check(
            Level::Fail,
            "binary",
            format!(
                "{} is not on PATH. The widget runs it on every key",
                facts.bin
            ),
        ),
        (Some(found), Some(exe)) if found != exe => check(
            Level::Warn,
            "binary",
            format!(
                "the widget runs {} and this is {}. Set SURMISE_BIN or reorder PATH",
                found.display(),
                exe.display()
            ),
        ),
        (Some(found), _) => check(Level::Ok, "binary", found.display().to_string()),
    }
}

fn widget(facts: &Facts) -> Check {
    let Some(widget) = &facts.widget else {
        return check(
            Level::Fail,
            "widget",
            "this shell has not loaded it. Add eval \"$(surmise init zsh)\" to ~/.zshrc or the bash line to ~/.bashrc",
        );
    };
    let mut fields = widget.split(' ');
    let (tag, under, shell) = (fields.next(), fields.next(), fields.next());
    if tag != Some(pick::RECORD_TAG) {
        return check(
            Level::Warn,
            "widget",
            "this shell holds an older one. Run the eval again or start a new shell",
        );
    }
    if shell.and_then(|pid| pid.parse::<u32>().ok()) != Some(facts.parent) {
        return check(
            Level::Warn,
            "widget",
            "loaded by another shell. Run this from the prompt it should answer",
        );
    }
    check(
        Level::Ok,
        "widget",
        // The zsh widget writes its version alone and every other one leads
        // with the shell's name.
        match under.unwrap_or("").split_once('/') {
            Some((name, version)) => format!("loaded in {name} {version}"),
            None => format!("loaded in zsh {}", under.unwrap_or("")),
        },
    )
}

fn settings(facts: &Facts) -> Check {
    let path = facts.config_path.as_ref().map_or_else(
        || "no home to place one under".to_string(),
        |p| p.display().to_string(),
    );
    match &facts.config_warning {
        Some(warning) => check(Level::Warn, "config", format!("{path}: {warning}")),
        None => check(Level::Ok, "config", path),
    }
}

fn history_dir(facts: &Facts) -> Check {
    match &facts.database {
        None => check(
            Level::Warn,
            "history",
            "no absolute XDG_DATA_HOME or HOME. Directory changes are not recorded",
        ),
        Some(db) if !facts.database_writable => check(
            Level::Warn,
            "history",
            format!(
                "{} cannot be written. Directory changes are not recorded",
                db.display()
            ),
        ),
        Some(db) => check(Level::Ok, "history", db.display().to_string()),
    }
}

/// The report as it prints, and whether anything in it failed.
pub fn report(checks: &[Check]) -> (String, bool) {
    let mut out = String::new();
    for c in checks {
        let level = match c.level {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Fail => "fail",
        };
        out.push_str(&format!("{level:<4}  {:<7}  {}\n", c.name, c.says));
    }
    (out, checks.iter().any(|c| c.level == Level::Fail))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shell that has everything it needs. Each test takes one thing away.
    fn healthy() -> Facts {
        let exe = PathBuf::from("/sample/bin/surmise");
        Facts {
            exe: Some(exe.clone()),
            bin: "surmise".to_string(),
            bin_found: Some(exe),
            widget: Some(format!("{} 5.9 4242", pick::RECORD_TAG)),
            parent: 4242,
            config_warning: None,
            config_path: Some(PathBuf::from("/sample/config.toml")),
            database: Some(PathBuf::from("/sample/data/surmise/history.sqlite3")),
            database_writable: true,
            git: true,
        }
    }

    fn level(facts: &Facts, name: &str) -> Level {
        checks(facts)
            .into_iter()
            .find(|c| c.name == name)
            .expect("every check reports")
            .level
    }

    #[test]
    fn a_healthy_shell_passes_every_check() {
        let (out, failed) = report(&checks(&healthy()));
        assert!(!failed, "{out}");
        assert!(out.lines().all(|line| line.starts_with("ok")), "{out}");
        assert!(out.contains("loaded in zsh 5.9"), "{out}");
        // The bash widget names its shell before the version.
        let mut f = healthy();
        f.widget = Some(format!("{} bash/5.2.37 4242", pick::RECORD_TAG));
        let (out, failed) = report(&checks(&f));
        assert!(!failed, "{out}");
        assert!(out.contains("loaded in bash 5.2.37"), "{out}");
    }

    #[test]
    fn each_check_says_what_is_missing() {
        let mut f = healthy();
        f.bin_found = None;
        assert_eq!(level(&f, "binary"), Level::Fail);
        let mut f = healthy();
        f.bin_found = Some(PathBuf::from("/sample/other/surmise"));
        assert_eq!(level(&f, "binary"), Level::Warn);
        let mut f = healthy();
        f.widget = None;
        assert_eq!(level(&f, "widget"), Level::Fail);
        assert!(report(&checks(&f)).1);
        let mut f = healthy();
        f.widget = Some("surmise-record-3 5.9 4242".to_string());
        assert_eq!(level(&f, "widget"), Level::Warn);
        let mut f = healthy();
        f.parent = 1;
        assert_eq!(level(&f, "widget"), Level::Warn);
        let mut f = healthy();
        f.config_warning = Some("line 1: expected a value".to_string());
        assert_eq!(level(&f, "config"), Level::Warn);
        let mut f = healthy();
        f.database_writable = false;
        assert_eq!(level(&f, "history"), Level::Warn);
        let mut f = healthy();
        f.database = None;
        assert_eq!(level(&f, "history"), Level::Warn);
        let mut f = healthy();
        f.git = false;
        assert_eq!(level(&f, "git"), Level::Warn);
    }

    #[test]
    fn a_path_resolves_whole_and_a_name_through_path() {
        let f = crate::fixture::Fixture::new(&["sample*"]);
        let sample = f.path().join("sample");
        assert_eq!(find(sample.to_str().unwrap()), sample.canonicalize().ok());
        assert_eq!(find("/no-such-directory-here/surmise"), None);
    }

    #[test]
    fn a_directory_still_to_be_made_answers_for_the_nearest_one_above() {
        let f = crate::fixture::Fixture::new(&[]);
        assert!(writable(&f.path().join("not/yet/made/history.sqlite3")));
    }
}
