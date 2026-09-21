//! The person's own shell history, read for one thing: how often a
//! command's second word followed its first. `git status` teaches the pair
//! `("git", "status")`; nothing else about that line is kept and the line
//! itself never leaves [`read`]. `crate::candidates::rank` is the one reader
//! of what comes back, and it looks up one pair at a time through
//! [`Counts::count`].
//!
//! Nothing here writes a file, logs, or prints. A missing, unreadable or
//! empty history answers every lookup with zero, the same as a history that
//! was never asked for.
//!
//! An entry reads as [`crate::shellparse::parse_with_aliases`] reads it
//! rather than as a naive split on whitespace, so `cd sample && git status`
//! teaches both `("cd", "sample")` and `("git", "status")` and a quoted
//! argument never breaks across two words. That same read strips a leading
//! assignment and expands an alias on a command's own name, whole value and
//! all, before the two words it keeps are read off: `FOO=bar git status`
//! and, with `alias g='git -C sample'` configured, `g status` both teach
//! `git`'s own pair rather than the assignment's or the alias's own literal
//! text. Measured over a full [`READ_LIMIT`] window of invented entries,
//! this costs about 1.5 ms in a release build, against the 250 ms a Git
//! query already gets at the same prompt.

use crate::shellparse;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How many times a command's first word was followed by a given second
/// word, keyed by the first and then by the second. The first word is
/// already resolved through the shell's alias table; the second is counted
/// exactly as typed.
///
/// Two maps rather than one keyed by a pair. A lookup then borrows both
/// words rather than building an owned pair to hash, and
/// `crate::candidates::rank` makes one lookup per row on a keystroke the
/// person is waiting through.
#[derive(Clone, Default)]
pub(crate) struct Counts(pub(crate) HashMap<String, HashMap<String, u32>>);

impl Counts {
    /// How many times `first second` was typed. Zero for a pair that never
    /// was, which is also every pair's answer once [`read`] found nothing to
    /// count.
    pub(crate) fn count(&self, first: &str, second: &str) -> u32 {
        self.0
            .get(first)
            .and_then(|seconds| seconds.get(second))
            .copied()
            .unwrap_or(0)
    }
}

/// The most of a history file a read looks at, from its tail. `crate::native`
/// caps a file it reads at 64 KiB for the same reason this does: a prompt is
/// waiting on the read and what sits past the cap is worth nothing beside
/// making it wait longer.
const READ_LIMIT: u64 = 64 * 1024;

/// The most entries a read keeps, again from the tail. A history of ordinary
/// command lines never fills [`READ_LIMIT`] with this many; the cap only
/// bites on one of unusually short entries.
const ENTRY_LIMIT: usize = 5000;

/// `first` and `second` from every command in every entry in `path`, with
/// `aliases` already resolved through `shellparse::parse_with_aliases`. An
/// entry chaining more than one command with `;`, `&&`, `||`, `|` or `&`
/// teaches one pair per command rather than one for the whole line. `path`
/// empty, missing, unreadable or carrying nothing to count all answer with
/// the empty map. Nothing here may print at a prompt, so none of those
/// cases is an error.
pub(crate) fn read(path: &str, aliases: &HashMap<String, String>) -> Counts {
    if path.is_empty() {
        return Counts::default();
    }
    let mut counts: HashMap<String, HashMap<String, u32>> = HashMap::new();
    for entry in tail_entries(Path::new(path)) {
        for command in shellparse::parse_with_aliases(command_text(&entry), aliases).commands {
            let mut words = command.words.into_iter();
            let Some(first) = words.next() else { continue };
            let Some(second) = words.next() else { continue };
            *counts
                .entry(first.inner_text)
                .or_default()
                .entry(second.inner_text)
                .or_default() += 1;
        }
    }
    Counts(counts)
}

/// The last [`READ_LIMIT`] bytes of `path`, and whether the entry the
/// window opens on had already begun in front of it. An absent, unreadable
/// or irregular file gives neither.
fn tail_bytes(path: &Path) -> Option<(Vec<u8>, bool)> {
    // The `stat` goes in front of the open rather than after it, and a file
    // that is not a regular one is refused on what it says. Opening a FIFO
    // waits for somebody to write to it and the shell's own line editor
    // waits with it, and a device answers a length of nothing and then
    // reads for as long as anything asks.
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    let over = len > READ_LIMIT;
    let mut file = File::open(path).ok()?;
    if over {
        // One byte in front of the window as well. A window opening on the
        // byte after a newline opens on a whole entry, and nothing else in
        // the window tells that apart from opening halfway through one.
        file.seek(SeekFrom::Start(len - READ_LIMIT - 1)).ok()?;
    }
    let mut bytes = Vec::new();
    // The cap rather than the seek is what bounds the read. The `stat` and
    // this read are two moments, and a file that grew between them would
    // otherwise hand back more than one window of it.
    let window = if over { READ_LIMIT + 1 } else { READ_LIMIT };
    file.take(window).read_to_end(&mut bytes).ok()?;
    if !over {
        return Some((bytes, false));
    }
    let whole = bytes.first() == Some(&b'\n');
    if !bytes.is_empty() {
        bytes.remove(0);
    }
    Some((bytes, !whole))
}

/// The history file's own entries, oldest first among what a read kept, and
/// no more than [`ENTRY_LIMIT`] of them. A missing or unreadable file gives
/// none. Bytes that are not UTF-8 do not cost the entry they sit in:
/// `String::from_utf8_lossy` puts U+FFFD in their place, so the entry
/// survives mangled rather than missing, and the words on either side of
/// the bad byte still read as themselves.
fn tail_entries(path: &Path) -> Vec<String> {
    let Some((bytes, partial)) = tail_bytes(path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if partial && !lines.is_empty() {
        // The window opened halfway through an entry. Its first line is the
        // tail of one that began in front of the window and half an entry
        // teaches the wrong pair.
        lines.remove(0);
    }
    let mut entries = join_continuations(&lines);
    let keep = entries.len().saturating_sub(ENTRY_LIMIT);
    entries.split_off(keep)
}

/// `lines` with a trailing backslash read as zsh reads it: the entry
/// continues on the line after it rather than ending on it.
fn join_continuations(lines: &[&str]) -> Vec<String> {
    let mut entries = Vec::new();
    let mut buf = String::new();
    for line in lines {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(line);
        if let Some(head) = buf.strip_suffix('\\') {
            buf = head.to_string();
            continue;
        }
        entries.push(std::mem::take(&mut buf));
    }
    if !buf.is_empty() {
        entries.push(buf);
    }
    entries
}

/// `entry` with zsh's extended prefix, `: <epoch>:<elapsed>;`, taken off it.
/// An entry the prefix does not fit is the plain format already and comes
/// back untouched.
fn command_text(entry: &str) -> &str {
    if let Some(rest) = entry.strip_prefix(": ")
        && let Some((epoch, rest)) = rest.split_once(':')
        && !epoch.is_empty()
        && epoch.bytes().all(|b| b.is_ascii_digit())
        && let Some((elapsed, cmd)) = rest.split_once(';')
        && !elapsed.is_empty()
        && elapsed.bytes().all(|b| b.is_ascii_digit())
    {
        return cmd;
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    /// A history file holding `bytes`, in a directory the test owns. The
    /// fixture outlives the path so the file is still there when `read` opens
    /// it.
    fn histfile(bytes: &[u8]) -> (String, Fixture) {
        let f = Fixture::new(&["histfile*"]);
        let path = f.path().join("histfile");
        std::fs::write(&path, bytes).unwrap();
        (path.to_str().unwrap().to_string(), f)
    }

    #[test]
    fn the_extended_format_teaches_its_pair() {
        let (path, _f) = histfile(b": 1700000000:0;git status\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
    }

    #[test]
    fn the_plain_format_teaches_its_pair() {
        let (path, _f) = histfile(b"git status\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
    }

    #[test]
    fn a_mixed_file_reads_both_formats() {
        let (path, _f) = histfile(b": 1700000000:0;git status\nnpm run build\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
        assert_eq!(counts.count("npm", "run"), 1);
    }

    #[test]
    fn a_continued_line_joins_before_the_words_are_read() {
        let (path, _f) = histfile(b": 1700000000:0;git commit \\\n--message sample\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "commit"), 1);
    }

    #[test]
    fn one_word_teaches_nothing() {
        let (path, _f) = histfile(b"fg\n");
        let counts = read(&path, &HashMap::new());
        assert!(counts.0.is_empty());
    }

    #[test]
    fn an_empty_file_teaches_nothing() {
        let (path, _f) = histfile(b"");
        let counts = read(&path, &HashMap::new());
        assert!(counts.0.is_empty());
    }

    #[test]
    fn a_missing_file_teaches_nothing() {
        let counts = read("/no-such-histfile-here", &HashMap::new());
        assert!(counts.0.is_empty());
    }

    #[test]
    fn an_empty_path_is_read_as_no_history_at_all() {
        let counts = read("", &HashMap::new());
        assert!(counts.0.is_empty());
    }

    #[test]
    fn bytes_that_are_not_utf8_survive_mangled_rather_than_missing() {
        // `caf\xe9` is not valid UTF-8. The word beside it and the entries
        // on either side still count normally.
        let (path, _f) = histfile(b"git status\ncaf\xe9 sample\nnpm run build\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
        assert_eq!(counts.count("npm", "run"), 1);
        assert_eq!(counts.count("caf\u{fffd}", "sample"), 1);
    }

    #[test]
    fn a_file_past_the_entry_cap_keeps_only_the_tail() {
        // Short lines, so the file stays well inside `READ_LIMIT` and the
        // entry cap is the one thing trimming it.
        let mut text = "cd x\n".repeat(ENTRY_LIMIT + 500);
        text.push_str("git status\n");
        assert!((text.len() as u64) < READ_LIMIT);
        let (path, _f) = histfile(text.as_bytes());
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
        assert_eq!(
            counts.count("cd", "x") + counts.count("git", "status"),
            ENTRY_LIMIT as u32
        );
    }

    #[test]
    fn a_file_past_the_byte_cap_reads_only_its_tail() {
        let mut text = "x".repeat(READ_LIMIT as usize * 2);
        text.push_str("\ngit status\n");
        let (path, _f) = histfile(text.as_bytes());
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
    }

    /// Exactly [`READ_LIMIT`] bytes of whole entries, the first of them
    /// `npm run build`. A file made of this plus something in front of it
    /// puts the window's own opening byte wherever that something ends.
    fn one_window() -> String {
        let pad = "cd sample00000\n";
        let mut window = String::from("npm run build\n");
        while window.len() + pad.len() <= READ_LIMIT as usize {
            window.push_str(pad);
        }
        // One last name, as long as it takes to come to the byte.
        let short = READ_LIMIT as usize - window.len();
        if short > 0 {
            window.push_str(&"z".repeat(short - 1));
            window.push('\n');
        }
        assert_eq!(window.len(), READ_LIMIT as usize);
        window
    }

    #[test]
    fn a_window_opening_on_a_whole_entry_keeps_it() {
        // The byte in front of the window is the newline ending what came
        // before, so the entry the window opens on is a whole one.
        let (path, _f) = histfile(format!("cd head\n{}", one_window()).as_bytes());
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("npm", "run"), 1);
    }

    #[test]
    fn a_window_opening_inside_an_entry_drops_it() {
        // One byte more in front, so the window opens one byte into a line
        // rather than on it. Half an entry teaches the wrong pair and this
        // one teaches none.
        let (path, _f) = histfile(format!("cd head\nz{}", one_window()).as_bytes());
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("npm", "run"), 0);
    }

    #[test]
    fn a_path_that_is_not_a_regular_file_teaches_nothing() {
        // A directory here stands for a FIFO and a device, which a test
        // cannot open without risking a read that never returns. All three
        // fail the same check and none of them is ever opened.
        let f = Fixture::new(&[]);
        let counts = read(f.path().to_str().unwrap(), &HashMap::new());
        assert!(counts.0.is_empty());
    }

    #[test]
    fn an_empty_elapsed_field_is_not_the_extended_format() {
        // `: 123:;` looks like the extended prefix and carries no elapsed
        // time, so it is a command of its own rather than a prefix. Reading
        // it as one would throw away the `: 123:` in front of it.
        let (path, _f) = histfile(b": 123:;npm run build\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count(":", "123:"), 1);
        assert_eq!(counts.count("npm", "run"), 1);
    }

    #[test]
    fn an_alias_teaches_the_name_it_expands_to() {
        let (path, _f) = histfile(b"g status\n");
        let mut aliases = HashMap::new();
        aliases.insert("g".to_string(), "git".to_string());
        let counts = read(&path, &aliases);
        assert_eq!(counts.count("git", "status"), 1);
    }

    #[test]
    fn a_chained_line_teaches_a_pair_for_each_command() {
        let (path, _f) = histfile(b"cd sample && git status\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("cd", "sample"), 1);
        assert_eq!(counts.count("git", "status"), 1);
    }

    #[test]
    fn a_leading_assignment_is_not_the_command_name() {
        let (path, _f) = histfile(b"FOO=bar git status\n");
        let counts = read(&path, &HashMap::new());
        assert_eq!(counts.count("git", "status"), 1);
        assert_eq!(counts.count("FOO=bar", "git"), 0);
    }

    #[test]
    fn a_multi_word_alias_expands_whole_before_the_words_are_counted() {
        // `g status` really runs `git -C sample status`: the alias's own
        // second word, not the one typed after `g`, is `git`'s pair here.
        let (path, _f) = histfile(b"g status\n");
        let mut aliases = HashMap::new();
        aliases.insert("g".to_string(), "git -C sample".to_string());
        let counts = read(&path, &aliases);
        assert_eq!(counts.count("git", "-C"), 1);
    }
}
