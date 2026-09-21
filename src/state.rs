//! What one menu leaves for the next.
//!
//! `$XDG_DATA_HOME/surmise/state.toml`, beside the directory history and
//! resolved the same way `history.rs` resolves that. This is not
//! `config.toml`: a file a person writes is not one surmise may rewrite.
//! What is here is surmise's own note to itself rather than a setting
//! anybody asked for. `config.rs` keeps a resolver of this shape too and
//! says the same thing about agreeing with the other one.
//!
//! Nothing here reaches the prompt. A file that will not read leaves the
//! defaults standing and a write that will not land is one menu's answer
//! lost. The menu draws either way.

use serde::{Deserialize, Serialize};
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// What the last menu was left set to and the whole of what the file
/// holds. `#[serde(default)]` is what keeps a file naming none of these
/// keys from being an error. A key only a later surmise writes is no error
/// either.
#[derive(Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct State {
    /// Whether the word under the list is open to every row the terminal
    /// spared. `false` is the one row it takes unasked.
    pub whole_word: bool,
}

impl State {
    /// What the file at [`path`] says, and the defaults where there is no
    /// file, no directory to keep one in, or nothing this build can parse.
    pub fn load() -> State {
        path().map_or_else(State::default, |p| Self::load_from(&p))
    }

    fn load_from(path: &Path) -> State {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&text).unwrap_or_default()
    }

    /// Leave this for the next menu. A failure is silent: the picker owes
    /// the prompt nothing and a key that did what the screen shows has
    /// already done the half of its job the person can see.
    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = self.save_to(&p);
        }
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("state has no parent directory"))?;
        // The same mode and the same owner-only file the history database's
        // own directory is made with. It is usually that directory.
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        // Written beside the file and renamed over it rather than written in
        // place. A rename is one step to a reader: another shell opening a
        // menu sees the whole of this answer or the whole of the last one
        // and never the empty file a write in place leaves behind for as
        // long as it takes to fill. A rename also replaces a symbolic link
        // planted where the file goes rather than writing through it. The
        // process id keeps two shells off each other's working file and the
        // one left by a process that died mid-save is this one's to clear.
        // `toml` writes the whole struct rather than a `format!` per field.
        // A field added to `State` is then written by the same derive that
        // reads it back and cannot be left out of one of the two.
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        let mut temp = path.as_os_str().to_os_string();
        temp.push(format!(".{}", std::process::id()));
        let temp = PathBuf::from(temp);
        let _ = std::fs::remove_file(&temp);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?
            .write_all(text.as_bytes())?;
        // A rename that will not land leaves the file it wrote behind.
        // Nothing else would ever clear it: the name carries this
        // process's own id and the next save is another process.
        std::fs::rename(&temp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
    }
}

/// Where the state file would be, whether or not it exists. `None` means
/// there is no absolute directory to keep one in and the defaults stand.
pub fn path() -> Option<PathBuf> {
    location(
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn location(data: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    data.filter(|p| p.is_absolute())
        .or_else(|| {
            home.filter(|p| p.is_absolute())
                .map(|p| p.join(".local/share"))
        })
        .map(|p| p.join("surmise/state.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    #[test]
    fn a_missing_file_gives_defaults() {
        let f = Fixture::new(&[]);
        let state = State::load_from(&f.path().join("missing/state.toml"));
        assert_eq!(state, State::default());
    }

    #[test]
    fn what_one_menu_saved_is_what_the_next_one_reads() {
        let f = Fixture::new(&[]);
        let dir = f.path().join("data/surmise");
        let path = dir.join("state.toml");
        let state = State { whole_word: true };
        state.save_to(&path).expect("a saved state");
        assert_eq!(State::load_from(&path), state);
        State::default().save_to(&path).expect("a saved state");
        assert_eq!(State::load_from(&path), State::default());
        // The file each save writes first and renames over this one. A save
        // that left it behind would leave it in the person's data directory
        // for good.
        let left: Vec<_> = std::fs::read_dir(&dir)
            .expect("the directory")
            .map(|e| e.expect("an entry").file_name())
            .collect();
        assert_eq!(left, ["state.toml"]);
    }

    #[test]
    fn a_save_that_cannot_land_leaves_nothing_behind() {
        let f = Fixture::new(&[]);
        let dir = f.path().join("data/surmise");
        let path = dir.join("state.toml");
        // Something else left a directory where the file goes. A rename
        // will not replace one and the file the save wrote first is the
        // save's own to clear.
        std::fs::create_dir_all(&path).expect("a directory in the way");
        assert!(State { whole_word: true }.save_to(&path).is_err());
        let left: Vec<_> = std::fs::read_dir(&dir)
            .expect("the directory")
            .map(|e| e.expect("an entry").file_name())
            .collect();
        assert_eq!(left, ["state.toml"]);
    }

    #[test]
    fn a_save_replaces_a_link_planted_where_the_file_goes() {
        let f = Fixture::new(&[]);
        let dir = f.path().join("data/surmise");
        std::fs::create_dir_all(&dir).expect("the directory");
        let other = f.path().join("elsewhere");
        std::fs::write(&other, "keep me").expect("a file");
        let path = dir.join("state.toml");
        std::os::unix::fs::symlink(&other, &path).expect("a link");
        let state = State { whole_word: true };
        state.save_to(&path).expect("a saved state");
        assert_eq!(
            std::fs::read_to_string(&other).expect("the file"),
            "keep me"
        );
        assert_eq!(State::load_from(&path), state);
    }

    #[test]
    fn a_file_that_does_not_parse_gives_defaults() {
        let f = Fixture::new(&[]);
        let path = f.path().join("state.toml");
        std::fs::write(&path, "whole_word = [this is not toml").expect("a file");
        assert_eq!(State::load_from(&path), State::default());
    }

    #[test]
    fn the_path_falls_back_from_xdg_data_home_to_the_home_share() {
        let path = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            location(path("/data"), path("/home/demo")),
            path("/data/surmise/state.toml")
        );
        for data in [None, path(""), path("relative")] {
            assert_eq!(
                location(data, path("/home/demo")),
                path("/home/demo/.local/share/surmise/state.toml")
            );
        }
        assert!(location(None, None).is_none());
        assert!(location(path("relative"), path("relative")).is_none());
    }
}
