//! Putting a file where another one already is, whole or not at all.
//!
//! Two files here are written this way and both have a reader that may run at
//! the same moment: another shell opening a menu reads `state.toml` while this
//! shell is leaving one, and `config.toml` is read by every picker a person
//! runs. A write in place leaves an empty file behind for as long as it takes
//! to fill, and a reader landing there reads nothing rather than waiting.

use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Put `text` at `path` with permissions `mode`, making the directory above
/// it if it is not there yet.
///
/// Written beside the file and renamed over it rather than written in place.
/// A rename is one step to a reader: it sees the whole of this text or the
/// whole of what was there before and never a half-filled file. A rename also
/// replaces a symbolic link planted where the file goes rather than writing
/// through it. The process id keeps two shells off each other's working file
/// and the one left by a process that died mid-write is this one's to clear.
///
/// The caller names `mode` rather than this deciding one. `state.toml` is
/// surmise's own and is always its own to read. `config.toml` was a person's
/// before surmise ever wrote to it and keeps whatever they gave it.
pub fn replace(path: &Path, text: &str, mode: u32) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the file has no parent directory"))?;
    // Owner-only, the same as the directory the history database is made in.
    // It is usually that directory.
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(".{}", std::process::id()));
    let temp = PathBuf::from(temp);
    let _ = std::fs::remove_file(&temp);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)?
        .write_all(text.as_bytes())?;
    // A rename that will not land leaves the file it wrote behind. Nothing
    // else would ever clear it: the name carries this process's own id and
    // the next write is another process.
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_write_makes_the_directory_above_it() {
        let f = Fixture::new(&[]);
        let path = f.path().join("one/two/file.toml");
        replace(&path, "a = 1\n", 0o600).expect("a write");
        assert_eq!(std::fs::read_to_string(&path).expect("it back"), "a = 1\n");
    }

    #[test]
    fn a_write_replaces_what_was_there_and_leaves_no_working_file() {
        let f = Fixture::new(&[]);
        let path = f.path().join("file.toml");
        replace(&path, "first\n", 0o600).expect("a write");
        replace(&path, "second\n", 0o600).expect("a write");
        assert_eq!(std::fs::read_to_string(&path).expect("it back"), "second\n");
        let left: Vec<String> = std::fs::read_dir(f.path())
            .expect("the directory")
            .map(|e| e.expect("an entry").file_name().to_string_lossy().into())
            .collect();
        assert_eq!(left, ["file.toml"]);
    }

    #[test]
    fn the_caller_names_the_permissions() {
        let f = Fixture::new(&[]);
        for mode in [0o600, 0o644] {
            let path = f.path().join(format!("{mode:o}.toml"));
            replace(&path, "a = 1\n", mode).expect("a write");
            let got = std::fs::metadata(&path).expect("it back").permissions();
            assert_eq!(got.mode() & 0o777, mode);
        }
    }

    #[test]
    fn a_symbolic_link_where_the_file_goes_is_replaced_rather_than_written_through() {
        // A link planted here would otherwise let a write reach whatever it
        // points at, with the permissions of that file rather than this one.
        let f = Fixture::new(&[]);
        let elsewhere = f.path().join("elsewhere.toml");
        std::fs::write(&elsewhere, "not this\n").expect("a file");
        let path = f.path().join("file.toml");
        std::os::unix::fs::symlink(&elsewhere, &path).expect("a link");
        replace(&path, "this\n", 0o600).expect("a write");
        assert_eq!(std::fs::read_to_string(&path).expect("it back"), "this\n");
        assert_eq!(
            std::fs::read_to_string(&elsewhere).expect("it back"),
            "not this\n"
        );
        assert!(
            !std::fs::symlink_metadata(&path)
                .expect("it back")
                .is_symlink()
        );
    }
}
