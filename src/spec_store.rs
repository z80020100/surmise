//! The compiled-in spec store, and the loose files `spec_dirs` names on top
//! of it. `build.rs` compresses every file under `specs/` into one blob and a
//! sorted table; this module is the byte-level lookup over both. It does not
//! parse a spec, follow a `versionedSpecPath` pointer, or model anything —
//! `plans/phase-2-spec-runtime.md` §1 is where that lands, on top of the
//! bytes this module hands back.
//!
//! `get` and `commands` take `spec_dirs` as a parameter rather than reading
//! `Config` themselves. Reading it themselves would need a `static` to hold
//! it, and a `static` set once cannot answer two tests that want different
//! directories in the same test binary — exactly the case `spec_dirs`
//! exists for. A parameter has no shared state, so nothing races. The
//! process-wide entry points below still exist, for whatever later phase
//! wires this into `App` for a real picker session, and cache what they read
//! behind a `OnceLock` because that caller is one process per menu and reads
//! the config only once regardless.

use flate2::read::DeflateDecoder;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/spec_table.rs"));

static BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/specs.blob"));

/// A spec's decompressed JSON bytes, or `None` if neither `spec_dirs` nor the
/// committed corpus answers `name`. Each directory in `spec_dirs` is tried in
/// order before the compiled-in data, so a person's own spec for a private
/// tool answers first and a stale public one can be overridden the same way.
///
/// `name` comes from a shell line. A `..` component or an absolute path is
/// refused before it reaches the filesystem, rather than trusted to stay
/// inside the directories it was given.
pub fn get(name: &str, spec_dirs: &[PathBuf]) -> Option<Vec<u8>> {
    if is_untrusted(name) {
        return get_compiled(name);
    }
    spec_dirs
        .iter()
        .find_map(|dir| read_loose(dir, name))
        .or_else(|| get_compiled(name))
}

/// The process-wide `get`, over the config this process was started with. A
/// name in `disabled_commands` answers `None` here, before `spec_dirs` or the
/// compiled-in data is even asked.
pub fn get_configured(name: &str) -> Option<Vec<u8>> {
    let config = config();
    if disabled(name, &config.disabled_commands) {
        return None;
    }
    get(name, &config.spec_dirs)
}

/// `true` when `disabled_commands` names `name`, in which case a spec lookup
/// for it never happens at all.
fn disabled(name: &str, disabled_commands: &[String]) -> bool {
    disabled_commands.iter().any(|c| c == name)
}

fn get_compiled(name: &str) -> Option<Vec<u8>> {
    let index = locate(name, "").or_else(|| locate(name, "/index"))?;
    let (_, offset, compressed_len, plain_len) = SPEC_TABLE[index];
    decompress(offset, compressed_len, plain_len)
}

/// `name` refused before it reaches a filesystem path: a `..` component
/// would read outside every directory it was given and a leading `/` would
/// name any file on the machine.
fn is_untrusted(name: &str) -> bool {
    let path = Path::new(name);
    path.is_absolute() || path.components().any(|c| c == Component::ParentDir)
}

/// `<dir>/<name>.json`, then `<dir>/<name>/index.json`, the same order `get`
/// tries the compiled-in data in. A file on disk is plain JSON, unlike the
/// compiled path, so this reads it whole rather than decompressing it. An
/// unreadable directory or an unreadable file is not an error; it is skipped
/// like any other candidate that does not answer `name`.
fn read_loose(dir: &Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(format!("{name}.json")))
        .ok()
        .or_else(|| std::fs::read(dir.join(name).join("index.json")).ok())
}

/// The compiled-in command list plus the top-level names each directory in
/// `spec_dirs` holds, sorted and de-duplicated. Without the disk names, a
/// private spec could answer Tab on its own name but could never open the
/// menu on a bare space, because that menu offers only what this function
/// lists.
pub fn commands(spec_dirs: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = COMMANDS.iter().map(|&s| s.to_string()).collect();
    for dir in spec_dirs {
        names.extend(loose_names(dir));
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// The process-wide `commands`, over the config this process was started
/// with. The union is filesystem work worth doing once, and a picker is one
/// process per menu, so a `OnceLock` here never outlives the session it was
/// computed for.
pub fn commands_configured() -> &'static [String] {
    static UNION: OnceLock<Vec<String>> = OnceLock::new();
    UNION.get_or_init(|| commands(&config().spec_dirs))
}

/// The names a name-laid-out-like-`specs/` directory answers: `<name>.json`
/// files and `<name>/index.json` directories, both stripped back to `name`.
/// An unreadable directory yields no names rather than an error.
fn loose_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if let Some(stem) = name.strip_suffix(".json") {
                Some(stem.to_string())
            } else if path.join("index.json").is_file() {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// The config this process was started with, read once and kept for the
/// rest of the run.
fn config() -> &'static crate::config::Config {
    static CONFIG: OnceLock<crate::config::Config> = OnceLock::new();
    CONFIG.get_or_init(crate::config::Config::load)
}

/// `SPEC_TABLE`'s position for `name` followed by `suffix`, compared without
/// building that concatenation. `Iterator::cmp` reads both sides lazily, so a
/// lookup allocates nothing beyond the output buffer `decompress` returns.
fn locate(name: &str, suffix: &str) -> Option<usize> {
    SPEC_TABLE
        .binary_search_by(|(key, ..)| key.bytes().cmp(name.bytes().chain(suffix.bytes())))
        .ok()
}

fn decompress(offset: u32, compressed_len: u32, plain_len: u32) -> Option<Vec<u8>> {
    let start = offset as usize;
    let end = start + compressed_len as usize;
    let mut buf = Vec::with_capacity(plain_len as usize);
    DeflateDecoder::new(&BLOB[start..end])
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    #[test]
    fn commands_are_727_sorted_and_unique_with_no_spec_dirs() {
        let list = commands(&[]);
        assert_eq!(list.len(), 727);
        assert!(list.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn git_decompresses_to_an_object() {
        let bytes = get("git", &[]).expect("git is a committed spec");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("git spec is JSON");
        assert!(value.is_object());
    }

    #[test]
    fn an_unknown_name_is_none() {
        assert!(get("not-a-real-command", &[]).is_none());
    }

    /// `az` is one of the six diff-versioned commands. Its spec lives at
    /// `specs/az/index.json`, keyed `az/index`, and `get("az")` only finds it
    /// through the `<name>/index` fallback.
    #[test]
    fn a_directory_backed_command_resolves_through_index() {
        let bytes = get("az", &[]).expect("az resolves through az/index");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("az spec is JSON");
        assert!(value.is_object());
    }

    /// The phase-1 exit criterion, at the byte level: every one of the 1481
    /// committed specs decompresses to exactly the length `build.rs` recorded
    /// for it and parses as JSON. The data is local and fixed, so this costs
    /// nothing beyond the run time it takes.
    #[test]
    fn every_committed_spec_decompresses_and_parses() {
        for &(key, _, _, plain_len) in SPEC_TABLE {
            let bytes = get(key, &[]).unwrap_or_else(|| panic!("{key} did not decompress"));
            assert_eq!(bytes.len(), plain_len as usize, "{key} plain length");
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap_or_else(|e| panic!("{key} did not parse: {e}"));
        }
    }

    #[test]
    fn a_loose_spec_wins_over_the_compiled_in_one_of_the_same_name() {
        let f = Fixture::new(&[]);
        std::fs::write(f.path().join("git.json"), br#"{"name": "loose"}"#).unwrap();
        let bytes = get("git", &[f.path().to_path_buf()]).unwrap();
        assert_eq!(bytes, br#"{"name": "loose"}"#);
    }

    #[test]
    fn a_loose_spec_wins_through_the_directory_form_too() {
        let f = Fixture::new(&["git"]);
        std::fs::write(f.path().join("git/index.json"), br#"{"name": "loose"}"#).unwrap();
        let bytes = get("git", &[f.path().to_path_buf()]).unwrap();
        assert_eq!(bytes, br#"{"name": "loose"}"#);
    }

    #[test]
    fn a_name_only_on_disk_resolves() {
        let f = Fixture::new(&[]);
        std::fs::write(f.path().join("private-tool.json"), br#"{"name": "mine"}"#).unwrap();
        let bytes = get("private-tool", &[f.path().to_path_buf()]).unwrap();
        assert_eq!(bytes, br#"{"name": "mine"}"#);
    }

    #[test]
    fn a_name_in_neither_place_is_none() {
        let f = Fixture::new(&[]);
        assert!(get("private-tool", &[f.path().to_path_buf()]).is_none());
    }

    #[test]
    fn disabled_commands_names_are_refused() {
        let disabled_commands = ["git".to_string(), "cd".to_string()];
        assert!(disabled("git", &disabled_commands));
        assert!(!disabled("hub", &disabled_commands));
        assert!(!disabled("git", &[]));
    }

    #[test]
    fn a_missing_spec_dir_is_skipped_rather_than_an_error() {
        let f = Fixture::new(&[]);
        let missing = f.path().join("does-not-exist");
        let bytes = get("git", &[missing]).expect("the compiled-in git still answers");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn commands_holds_the_disk_name() {
        let f = Fixture::new(&["also-a-dir"]);
        std::fs::write(f.path().join("private-tool.json"), b"{}").unwrap();
        std::fs::write(f.path().join("also-a-dir/index.json"), b"{}").unwrap();
        let list = commands(&[f.path().to_path_buf()]);
        assert!(list.iter().any(|c| c == "private-tool"));
        assert!(list.iter().any(|c| c == "also-a-dir"));
        assert!(list.iter().any(|c| c == "git"));
        assert!(list.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_name_holding_a_parent_component_cannot_read_outside_the_directory() {
        let f = Fixture::new(&["inner"]);
        // If `..` reached the filesystem, `inner/../passwd.json` would
        // resolve to this file.
        std::fs::write(f.path().join("passwd.json"), b"outside").unwrap();
        assert!(get("../passwd", &[f.path().join("inner")]).is_none());
    }

    #[test]
    fn an_absolute_name_cannot_read_outside_the_directory() {
        let f = Fixture::new(&["inner"]);
        let target = f.path().join("passwd.json");
        std::fs::write(&target, b"outside").unwrap();
        let absolute = f.path().join("passwd");
        assert!(get(absolute.to_str().unwrap(), &[f.path().join("inner")]).is_none());
    }
}
