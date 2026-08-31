//! A throwaway directory tree for the tests.
//!
//! `tempfile` does this too. A short guard here keeps the dependency list to
//! what the program itself needs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct Fixture(PathBuf);

impl Fixture {
    /// Make a directory holding `entries`. An entry that ends in `*` is made
    /// as a file rather than a directory.
    pub(crate) fn new(entries: &[&str]) -> Fixture {
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

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
