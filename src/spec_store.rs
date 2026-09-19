//! The compiled-in spec store. `build.rs` compresses every file under
//! `specs/` into one blob and a sorted table; this module is the byte-level
//! lookup over both. It does not parse a spec, follow a `versionedSpecPath`
//! pointer, or model anything — `plans/phase-2-spec-runtime.md` §1 is where
//! that lands, on top of the bytes this module hands back.

use flate2::read::DeflateDecoder;
use std::io::Read;

include!(concat!(env!("OUT_DIR"), "/spec_table.rs"));

static BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/specs.blob"));

/// A spec's decompressed JSON bytes, or `None` if no committed spec answers
/// `name`. `<name>` is tried first and `<name>/index` second, because a
/// directory-backed command such as `az` keeps its spec at `az/index.json`
/// rather than at `az.json`.
pub fn get(name: &str) -> Option<Vec<u8>> {
    let index = locate(name, "").or_else(|| locate(name, "/index"))?;
    let (_, offset, compressed_len, plain_len) = SPEC_TABLE[index];
    decompress(offset, compressed_len, plain_len)
}

/// The compiled-in command list `specs/index.json` recorded, already sorted
/// and already unique. `static` data, so this costs no allocation per call.
pub fn commands() -> &'static [&'static str] {
    COMMANDS
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

    #[test]
    fn commands_are_727_sorted_and_unique() {
        let list = commands();
        assert_eq!(list.len(), 727);
        assert!(list.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn git_decompresses_to_an_object() {
        let bytes = get("git").expect("git is a committed spec");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("git spec is JSON");
        assert!(value.is_object());
    }

    #[test]
    fn an_unknown_name_is_none() {
        assert!(get("not-a-real-command").is_none());
    }

    /// `az` is one of the six diff-versioned commands. Its spec lives at
    /// `specs/az/index.json`, keyed `az/index`, and `get("az")` only finds it
    /// through the `<name>/index` fallback.
    #[test]
    fn a_directory_backed_command_resolves_through_index() {
        let bytes = get("az").expect("az resolves through az/index");
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
            let bytes = get(key).unwrap_or_else(|| panic!("{key} did not decompress"));
            assert_eq!(bytes.len(), plain_len as usize, "{key} plain length");
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap_or_else(|e| panic!("{key} did not parse: {e}"));
        }
    }
}
