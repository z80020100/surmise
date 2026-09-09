//! A throwaway directory tree for the tests.
//!
//! `tempfile` does this too. A short guard here keeps the dependency list to
//! what the program itself needs.
//!
//! This is public rather than `#[cfg(test)]`, because an integration test
//! links the library as an ordinary crate and a `#[cfg(test)]` module is not
//! compiled into that build.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Fixture(PathBuf);

impl Fixture {
    /// Make a directory holding `entries`. An entry that ends in `*` is made
    /// as a file rather than a directory.
    pub fn new(entries: &[&str]) -> Fixture {
        // No prompt is waiting on a test and every fixture-backed one spawns
        // Git more than once.
        crate::git::widen_timeout();
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "surmise-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        for e in entries {
            match e.strip_suffix('*') {
                Some(file) => std::fs::write(root.join(file), b"").unwrap(),
                None => std::fs::create_dir_all(root.join(e)).unwrap(),
            }
        }
        Fixture(root)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Run Git with synthetic identities and no user configuration.
    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Sample")
            .env("GIT_AUTHOR_EMAIL", "sample@example.invalid")
            .env("GIT_COMMITTER_NAME", "Sample")
            .env("GIT_COMMITTER_EMAIL", "sample@example.invalid")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_string()
    }

    /// Create local branches at one empty commit.
    pub fn init_git(&self, branches: &[&str]) {
        self.git(&[
            "init",
            "--quiet",
            "--template=",
            "--initial-branch=sample-main",
        ]);
        // The machine's own excludes file reaches this repository through any
        // Git the code under test runs for itself. The `env_clear` above speaks
        // for these calls alone and cannot speak for those. A name a developer
        // ignores at home would otherwise go missing from a menu a test reads.
        self.git(&["config", "core.excludesFile", "/dev/null"]);
        let tree = self.git(&["hash-object", "-t", "tree", "-w", "--stdin"]);
        let commit = self.git(&["commit-tree", &tree, "-m", "Sample"]);
        self.git(&["update-ref", "refs/heads/sample-main", &commit]);
        for branch in branches {
            self.git(&["update-ref", &format!("refs/heads/{branch}"), &commit]);
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
