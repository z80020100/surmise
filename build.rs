//! Compresses the committed spec corpus into one blob and a sorted lookup
//! table at build time. `spec_store` reads both back through `include_bytes!`
//! and `include!`, so the binary carries `specs/` and this script is the only
//! thing that reads the directory itself. `CLAUDE.md`'s "Completion spec
//! data" section says what `specs/` is and why it is committed rather than
//! fetched.

use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=specs");

    let specs_dir = Path::new("specs");
    let index = fs::read_to_string(specs_dir.join("index.json")).expect("read specs/index.json");
    let index: serde_json::Value = serde_json::from_str(&index).expect("parse specs/index.json");
    let commands = index["commands"]
        .as_array()
        .expect("specs/index.json has a commands array")
        .iter()
        .map(|name| name.as_str().expect("command name is a string"));

    let mut entries = Vec::new();
    collect(specs_dir, specs_dir, &mut entries);
    // Sorted once here rather than in `spec_store`, so a lookup there is a
    // binary search over data that is already in order.
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut blob = Vec::new();
    let mut table = String::from("static SPEC_TABLE: &[(&str, u32, u32, u32)] = &[\n");
    for (key, path) in &entries {
        let plain = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        // Each spec compresses on its own, so `spec_store` decompresses the
        // one it needs without touching its neighbours in the blob.
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&plain).expect("compress spec");
        let compressed = encoder.finish().expect("finish compressing spec");
        let offset = blob.len();
        let compressed_len = compressed.len();
        let plain_len = plain.len();
        blob.extend_from_slice(&compressed);
        table.push_str(&format!(
            "    ({key:?}, {offset}, {compressed_len}, {plain_len}),\n"
        ));
    }
    table.push_str("];\n\nstatic COMMANDS: &[&str] = &[\n");
    for command in commands {
        table.push_str(&format!("    {command:?},\n"));
    }
    table.push_str("];\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out_dir.join("specs.blob"), &blob).expect("write specs.blob");
    fs::write(out_dir.join("spec_table.rs"), table).expect("write spec_table.rs");
}

/// Every `*.json` under `dir`, keyed by its path relative to `base` with the
/// extension dropped. `specs/index.json` is the command list rather than a
/// spec, so it is the one file this walk leaves out; a nested `index.json`
/// such as `specs/az/index.json` is a spec like any other and goes in keyed
/// `az/index`.
fn collect(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("read a directory entry").path();
        if path.is_dir() {
            collect(&path, base, out);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "json") || path == base.join("index.json") {
            continue;
        }
        let key = path
            .strip_prefix(base)
            .expect("spec path is under specs/")
            .with_extension("")
            .to_str()
            .expect("spec path is UTF-8")
            .to_owned();
        out.push((key, path));
    }
}
