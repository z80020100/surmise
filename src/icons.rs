//! The glyph in front of a row and the colour it wears.
//!
//! Two sets answer that. [`Set::Text`] is what a menu draws unless the config
//! says otherwise: every glyph in it is plain text and a terminal draws each
//! one in whatever colour ANSI sets under it. [`Set::Nerd`] is what
//! `icons = "nerd"` asks for, and every glyph in it is a codepoint only a
//! patched font carries.
//!
//! That set is a person's to turn on rather than surmise's to guess at. Every
//! codepoint in it sits in the private use area, where the East Asian width
//! table says ambiguous. A terminal set for CJK draws such a character two
//! cells wide and [`crate::ui::cells`] reads that table and calls it one,
//! which slides every name in the panel a column right. A terminal with no
//! such font draws a box in place of each one. Neither question can be asked
//! of a terminal, so turning the set on is how a person answers both.
//!
//! Colour emoji are not a third set. The text set replaced one and the reason
//! rules a later one out: an emoji draws from the font's own colour table and
//! ignores the foreground the code sets under it, so the colour that says what
//! sort of row this is would never reach it.
//!
//! The name on a row is one colour whatever the set. What sort of row it is
//! is the glyph's to say, in a shape and a colour of its own, and a name that
//! changed colour with it would say the same thing twice.

use crate::candidates::{CURRENT_BRANCH, Candidate, FILE, FOLDER, Kind};
use crate::ui::cells;

/// Which glyphs a menu draws. `config.toml` is where it comes from and
/// `ui::menu` is what carries it to the rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Set {
    #[default]
    Text,
    Nerd,
}

/// The glyph on a directory row and on the row that goes up, in the text set.
/// The seven below it are the others.
///
/// Every one is one cell wide. None of them is a character whose East Asian
/// width is ambiguous, because a terminal set for CJK draws such a character
/// two cells wide where [`crate::ui::cells`] reads the table and calls it
/// one. That rules out the box-drawing and mathematical shapes an editor
/// reaches for first. [`widest`] still measures them, for a later glyph that
/// is not one cell.
const DIR: &str = "▸";
/// The home shortcut is a directory like any other and wears the same colour.
/// The shape is what says which one it is.
const HOME: &str = "~";
/// The row that runs the line rather than growing it. An ordinary character,
/// because the row names an action rather than a thing.
const RUN: &str = "\u{21b5}";
/// A subcommand row.
const CMD: &str = "$";
/// A branch row.
const BRANCH: &str = "|";
/// The branch the repository is on. A shape of its own on a row that is a
/// branch row like any other, because the name beside it says nothing about
/// where the repository already stands.
const CURRENT: &str = "*";
/// A file row, and a path row where the path is not a directory.
const PLAIN_FILE: &str = "=";
/// An option row.
const OPTION: &str = "-";

/// The same eight in the nerd set. Each one is named for the glyph a patched
/// font carries at that codepoint, because the escape says nothing to a
/// reader and the name is the only way to check one.
const NF_DIR: &str = "\u{f07b}"; // nf-fa-folder
const NF_HOME: &str = "\u{f015}"; // nf-fa-home
const NF_RUN: &str = "\u{f04b}"; // nf-fa-play
const NF_CMD: &str = "\u{f120}"; // nf-fa-terminal
const NF_BRANCH: &str = "\u{e0a0}"; // nf-pl-branch
const NF_CURRENT: &str = "\u{f005}"; // nf-fa-star
const NF_PLAIN_FILE: &str = "\u{f15b}"; // nf-fa-file
const NF_OPTION: &str = "\u{f024}"; // nf-fa-flag

/// The colour a directory's glyph wears.
const DIR_FG: &str = "\x1b[38;5;75m";
/// The glyph on the row that runs the line. A colour apart from the folder
/// blue is what marks that row out now that it carries no name.
const RUN_FG: &str = "\x1b[38;5;167m";
/// The glyph on a subcommand row. A magenta of its own. The folder blue says
/// a directory and the red above says an action. A subcommand is neither and
/// it carries a name where the action row does not.
const CMD_FG: &str = "\x1b[38;5;169m";
/// The glyph on a branch row, worn by the branch itself and by the current
/// branch's own shape alike: the name beside either says nothing about where
/// the repository stands and the colour is not what would.
const BRANCH_FG: &str = "\x1b[38;5;114m";
/// The glyph on a file row, and on a path row where the path is not a
/// directory. In the nerd set a name the table below answers for takes its
/// role's colour instead and this is what the rest keep.
const FILE_FG: &str = "\x1b[38;5;180m";
/// The glyph on an option row.
const OPTION_FG: &str = "\x1b[38;5;221m";
/// The six glyphs above on the highlighted row. That row has a ground of its
/// own and every colour above is too close to it to read. A lighter tint of
/// the same hue clears it and still says which sort of row this is.
const DIR_FG_CHOSEN: &str = "\x1b[38;5;153m";
const RUN_FG_CHOSEN: &str = "\x1b[38;5;217m";
const CMD_FG_CHOSEN: &str = "\x1b[38;5;218m";
const BRANCH_FG_CHOSEN: &str = "\x1b[38;5;157m";
const FILE_FG_CHOSEN: &str = "\x1b[38;5;223m";
const OPTION_FG_CHOSEN: &str = "\x1b[38;5;229m";

/// What a file's name says it holds. The glyph says which sort of file it is
/// and the role says what that sort is for, so a `.rs` and a `.sh` read as
/// code together at a glance and still say which one they are.
///
/// A role rather than a colour for each name: the two tables below answer for
/// 92 names between them and 92 colours would be a palette nobody can hold in
/// their head, on a panel six rows tall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Code,
    Data,
    Doc,
    Media,
    Archive,
}

impl Role {
    /// The colour off the highlighted row and the colour on it. Every one
    /// differs from every other and from the six kind colours above, because
    /// a file row shares a menu with an option row, a directory row, a
    /// subcommand row and the row that runs the line.
    fn fg(self) -> (&'static str, &'static str) {
        match self {
            Role::Code => ("\x1b[38;5;209m", "\x1b[38;5;216m"),
            Role::Data => ("\x1b[38;5;107m", "\x1b[38;5;151m"),
            Role::Doc => ("\x1b[38;5;141m", "\x1b[38;5;183m"),
            Role::Media => ("\x1b[38;5;79m", "\x1b[38;5;158m"),
            Role::Archive => ("\x1b[38;5;131m", "\x1b[38;5;210m"),
        }
    }
}

/// What the part of a name after its last dot draws as, in the nerd set. Each
/// row names its glyph the way the eight above do. A name the table has no row
/// for keeps [`NF_PLAIN_FILE`] and [`FILE_FG`].
const EXTENSIONS: &[(&[&str], &str, Role)] = &[
    (&["rs"], "\u{e7a8}", Role::Code),               // nf-dev-rust
    (&["py"], "\u{e73c}", Role::Code),               // nf-dev-python
    (&["js", "mjs", "cjs"], "\u{e74e}", Role::Code), // nf-dev-javascript
    (&["ts", "tsx", "jsx"], "\u{e628}", Role::Code), // nf-seti-typescript
    (&["go"], "\u{e724}", Role::Code),               // nf-dev-go
    (&["c", "h"], "\u{e61e}", Role::Code),           // nf-custom-c
    (&["cc", "cpp", "cxx", "hpp"], "\u{e61d}", Role::Code), // nf-custom-cpp
    (&["rb"], "\u{e739}", Role::Code),               // nf-dev-ruby
    (&["java"], "\u{e738}", Role::Code),             // nf-dev-java
    (&["lua"], "\u{e620}", Role::Code),              // nf-seti-lua
    (&["swift"], "\u{e755}", Role::Code),            // nf-dev-swift
    (&["php"], "\u{e73d}", Role::Code),              // nf-dev-php
    (&["sh", "bash", "zsh", "fish"], "\u{f489}", Role::Code), // nf-oct-terminal
    (&["vim"], "\u{e62b}", Role::Code),              // nf-seti-vim
    (&["html", "htm"], "\u{e736}", Role::Code),      // nf-dev-html5
    (&["css", "scss", "sass"], "\u{e749}", Role::Code), // nf-dev-css3
    (&["json"], "\u{e60b}", Role::Data),             // nf-seti-json
    (
        &["toml", "ini", "cfg", "conf", "yml", "yaml"],
        "\u{e615}",
        Role::Data,
    ), // nf-seti-config
    (&["lock"], "\u{f023}", Role::Data),             // nf-fa-lock
    (&["xml"], "\u{e619}", Role::Data),              // nf-seti-xml
    (&["csv", "tsv"], "\u{f0ce}", Role::Data),       // nf-fa-table
    (&["db", "sql", "sqlite3"], "\u{f1c0}", Role::Data), // nf-fa-database
    (&["md", "markdown"], "\u{e73e}", Role::Doc),    // nf-dev-markdown
    (&["adoc", "rst", "txt"], "\u{f0f6}", Role::Doc), // nf-fa-file-text-o
    (&["pdf"], "\u{f1c1}", Role::Doc),               // nf-fa-file-pdf-o
    (
        &["bmp", "gif", "ico", "jpeg", "jpg", "png", "svg", "webp"],
        "\u{f1c5}",
        Role::Media,
    ), // nf-fa-file-image-o
    (&["mkv", "mov", "mp4", "webm"], "\u{f1c8}", Role::Media), // nf-fa-file-video-o
    (
        &["flac", "m4a", "mp3", "ogg", "wav"],
        "\u{f1c7}",
        Role::Media,
    ), // nf-fa-file-audio-o
    (
        &["7z", "bz2", "gz", "rar", "tar", "tgz", "xz", "zip", "zst"],
        "\u{f1c6}",
        Role::Archive,
    ), // nf-fa-file-archive-o
];

/// What a whole name draws as, in the nerd set. It is read before
/// [`EXTENSIONS`] and answers the two names that table cannot: a name with no
/// dot in it at all, and a name whose only dot is the one in front.
const NAMES: &[(&[&str], &str, Role)] = &[
    (&["gnumakefile", "makefile"], "\u{e673}", Role::Code), // nf-seti-makefile
    (&["containerfile", "dockerfile"], "\u{e7b0}", Role::Code), // nf-dev-docker
    (
        &[".gitattributes", ".gitconfig", ".gitignore", ".gitmodules"],
        "\u{e702}",
        Role::Data,
    ), // nf-dev-git
    (
        &[".bashrc", ".profile", ".zshenv", ".zshrc"],
        "\u{f489}",
        Role::Code,
    ), // nf-oct-terminal
    (
        &["licence", "license", "license-apache", "license-mit"],
        "\u{f24e}",
        Role::Doc,
    ), // nf-fa-balance-scale
];

/// Every kind a row can be. [`glyphs`] walks it and the tests below hold each
/// one to a look of its own.
const KINDS: [Kind; 9] = [
    Kind::Command,
    Kind::Branch,
    Kind::File,
    Kind::Option,
    Kind::Path,
    Kind::Dir,
    Kind::Parent,
    Kind::Special,
    Kind::Run,
];

/// The glyph `k` wears in `set`.
fn kind_glyph(set: Set, k: Kind) -> &'static str {
    // Every variant is named rather than swept into a catch-all. A new one
    // then fails the build here the way it does in `ui::tab_grows`. What a
    // kind looks like is one row of this table rather than two matches on the
    // same key. Two matches could disagree about a kind and the build would
    // not say so.
    match (set, k) {
        (Set::Text, Kind::Command) => CMD,
        (Set::Text, Kind::Branch) => BRANCH,
        (Set::Text, Kind::File | Kind::Path) => PLAIN_FILE,
        (Set::Text, Kind::Option) => OPTION,
        (Set::Text, Kind::Run) => RUN,
        (Set::Text, Kind::Special) => HOME,
        (Set::Text, Kind::Dir | Kind::Parent) => DIR,
        (Set::Nerd, Kind::Command) => NF_CMD,
        (Set::Nerd, Kind::Branch) => NF_BRANCH,
        (Set::Nerd, Kind::File | Kind::Path) => NF_PLAIN_FILE,
        (Set::Nerd, Kind::Option) => NF_OPTION,
        (Set::Nerd, Kind::Run) => NF_RUN,
        (Set::Nerd, Kind::Special) => NF_HOME,
        (Set::Nerd, Kind::Dir | Kind::Parent) => NF_DIR,
    }
}

/// The colour `k` wears, off the highlighted row and on it. It is the kind's
/// alone and no set changes it: a folder is the same blue in both, so turning
/// the nerd set on changes the shapes and leaves the palette where the eye
/// last found it.
fn kind_fg(k: Kind) -> (&'static str, &'static str) {
    match k {
        Kind::Command => (CMD_FG, CMD_FG_CHOSEN),
        Kind::Branch => (BRANCH_FG, BRANCH_FG_CHOSEN),
        Kind::File | Kind::Path => (FILE_FG, FILE_FG_CHOSEN),
        Kind::Option => (OPTION_FG, OPTION_FG_CHOSEN),
        Kind::Run => (RUN_FG, RUN_FG_CHOSEN),
        // The shortcut reaches a directory and wears a directory's colour.
        Kind::Special | Kind::Dir | Kind::Parent => (DIR_FG, DIR_FG_CHOSEN),
    }
}

/// The glyph the branch the repository is on wears in `set`.
fn current_glyph(set: Set) -> &'static str {
    match set {
        Set::Text => CURRENT,
        Set::Nerd => NF_CURRENT,
    }
}

/// What the last part of `path` says it holds, or `None` for a name neither
/// table answers for. The whole name is read before the extension is, so
/// `.gitignore` is a Git file rather than a `gitignore` nobody has heard of.
fn file_look(path: &str) -> Option<(&'static str, Role)> {
    let name = path.rsplit('/').next()?;
    let found = |table: &[(&[&str], &'static str, Role)], key: &str| {
        table
            .iter()
            .find(|(names, _, _)| names.iter().any(|n| n.eq_ignore_ascii_case(key)))
            .map(|&(_, glyph, role)| (glyph, role))
    };
    found(NAMES, name).or_else(|| {
        // `rfind` rather than `split`: `foo.tar.gz` is a `gz`. A name whose
        // only dot is the one in front has no extension at all and the whole
        // name above is what answers for it.
        let dot = name.rfind('.').filter(|&i| i > 0)?;
        found(EXTENSIONS, &name[dot + 1..])
    })
}

/// The glyph in front of `c` and the colour it wears, on the highlighted row
/// or off it.
pub fn of(set: Set, c: &Candidate, chosen: bool) -> (&'static str, &'static str) {
    let plain = (kind_glyph(set, c.kind), kind_fg(c.kind));
    let (glyph, (fg, fg_chosen)) = match c.kind {
        // Git and a specification alike hand back a folder as a file row and
        // say so in the label. The kind says only who named the row.
        Kind::File | Kind::Path if c.label == FOLDER => {
            (kind_glyph(set, Kind::Dir), kind_fg(Kind::Dir))
        }
        // Only a row that says it is a file reads the two tables. A make
        // target and an SSH host are `Path` rows with a label of their own,
        // and neither is a path on disk for the tables to have anything true
        // to say about. `git add --pathspec-from-file` names real files and
        // still keeps the plain shape, because its rows carry the argument's
        // own sentence where a file row carries `FILE`. One overloaded field
        // is what says which, and reading it strictly is what keeps a make
        // target out.
        Kind::File | Kind::Path if set == Set::Nerd && c.label == FILE => {
            file_look(&c.display).map_or(plain, |(glyph, role)| (glyph, role.fg()))
        }
        Kind::Branch if c.label == CURRENT_BRANCH => (current_glyph(set), kind_fg(Kind::Branch)),
        _ => plain,
    };
    (glyph, if chosen { fg_chosen } else { fg })
}

/// Every glyph `set` can draw. [`widest`] measures the column the names start
/// in from this and the test below holds every one of them to a single cell.
fn glyphs(set: Set) -> Vec<&'static str> {
    let mut all: Vec<&'static str> = KINDS.iter().map(|&k| kind_glyph(set, k)).collect();
    all.push(current_glyph(set));
    if set == Set::Nerd {
        all.extend(EXTENSIONS.iter().map(|&(_, glyph, _)| glyph));
        all.extend(NAMES.iter().map(|&(_, glyph, _)| glyph));
    }
    all
}

/// The cells the widest glyph in `set` takes. `ui` pads every glyph out to it
/// and a set whose glyphs are not all one cell therefore still draws straight.
pub fn widest(set: Set) -> usize {
    glyphs(set).into_iter().map(cells).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::collections::HashSet;

    const SETS: [Set; 2] = [Set::Text, Set::Nerd];

    fn row(kind: Kind, display: &str, label: &'static str) -> Candidate {
        Candidate {
            display: display.to_string(),
            insert: display.to_string(),
            label: Cow::Borrowed(label),
            hint: Vec::new(),
            kind,
            score: 0,
            priority: crate::candidates::DEFAULT_PRIORITY,
        }
    }

    #[test]
    fn every_glyph_is_one_cell_wide() {
        // `ui` measures rather than assumes, so a wider glyph would still
        // draw straight. What it would take is a cell off every name, and
        // `tests/pty/pick.rs` reads a drawn row by stripping one character.
        // This is the decision both of those rest on.
        for set in SETS {
            for glyph in glyphs(set) {
                assert_eq!(cells(glyph), 1, "{set:?} {glyph:?}");
            }
        }
    }

    #[test]
    fn no_two_kinds_share_both_a_glyph_and_a_colour() {
        // `Path` draws as a `File` does and `Parent` draws as a `Dir` does.
        // Every other kind has to differ from every other in the glyph or the
        // colour or both, or `of` stops being a map from a kind to a look of
        // its own. `Special` wears a `Dir`'s colour and is in here for the
        // shape, which is all it has to tell the two apart by.
        for set in SETS {
            for chosen in [false, true] {
                let mut seen = HashSet::new();
                for kind in [
                    Kind::Run,
                    Kind::Command,
                    Kind::Branch,
                    Kind::File,
                    Kind::Option,
                    Kind::Dir,
                    Kind::Special,
                ] {
                    let look = of(set, &row(kind, "name", "label"), chosen);
                    assert!(seen.insert(look), "{set:?} {kind:?} chosen={chosen}");
                }
            }
        }
    }

    #[test]
    fn the_two_sets_share_no_glyph() {
        // A test that reads a drawn row tells the sets apart by the glyph
        // alone. One shape in both would leave it reading either.
        let text: HashSet<&str> = glyphs(Set::Text).into_iter().collect();
        for glyph in glyphs(Set::Nerd) {
            assert!(!text.contains(glyph), "{glyph:?}");
        }
    }

    #[test]
    fn the_row_that_runs_the_line_is_told_apart_by_its_glyph() {
        // The row carries no name and the glyph is therefore the only thing
        // left to tell it apart from a directory. That holds on the
        // highlighted row as well. White there would read as a directory and
        // would take the row's sort with it.
        for set in SETS {
            for chosen in [false, true] {
                let dir = of(set, &row(Kind::Dir, "alpha/", "directory"), chosen);
                let run = of(set, &row(Kind::Run, "", "run"), chosen);
                assert_eq!(of(set, &row(Kind::Parent, "../", "parent"), chosen), dir);
                assert_ne!(run.0, dir.0);
                assert_ne!(run.1, dir.1);
            }
        }
    }

    #[test]
    fn the_home_shortcut_is_told_apart_by_its_glyph() {
        // The shortcut reaches a directory and the glyph therefore wears the
        // directory's own colour. The shape is what says which directory it
        // is. A colour there would say the row is another sort of thing.
        for set in SETS {
            for chosen in [false, true] {
                let home = of(set, &row(Kind::Special, "~/", "home"), chosen);
                let dir = of(set, &row(Kind::Dir, "alpha/", "directory"), chosen);
                assert_eq!(home.1, dir.1);
                assert_ne!(home.0, dir.0);
            }
        }
    }

    #[test]
    fn a_subcommand_row_is_told_apart_by_its_glyph() {
        // A subcommand and a directory both carry a name. The glyph is what
        // says which sort of row it is and its colour therefore has to differ
        // from the folder blue and from the action row's red alike.
        for set in SETS {
            for chosen in [false, true] {
                let cmd = of(set, &row(Kind::Command, "switch", "command"), chosen);
                let dir = of(set, &row(Kind::Dir, "alpha/", "directory"), chosen);
                let run = of(set, &row(Kind::Run, "", "run"), chosen);
                assert_ne!(cmd.0, dir.0);
                assert_ne!(cmd.0, run.0);
                assert_ne!(cmd.1, dir.1);
                assert_ne!(cmd.1, run.1);
            }
        }
    }

    #[test]
    fn a_folder_handed_back_as_a_file_row_draws_as_a_directory() {
        for set in SETS {
            for kind in [Kind::File, Kind::Path] {
                assert_eq!(
                    of(set, &row(kind, "assets/", FOLDER), false),
                    of(set, &row(Kind::Dir, "assets/", "directory"), false),
                );
            }
        }
    }

    #[test]
    fn the_branch_the_repository_is_on_wears_a_shape_of_its_own() {
        for set in SETS {
            let current = of(set, &row(Kind::Branch, "main", CURRENT_BRANCH), false);
            let other = of(set, &row(Kind::Branch, "work", "branch"), false);
            assert_ne!(current.0, other.0);
            // The name says nothing about where the repository stands and the
            // colour is not what would.
            assert_eq!(current.1, other.1);
        }
    }

    #[test]
    fn a_file_name_reaches_its_own_glyph_in_the_nerd_set() {
        // The extension is read off the last part of the path and the case it
        // is written in makes no difference.
        for name in ["build.rs", "src/build.rs", "./SRC/BUILD.RS"] {
            let look = of(Set::Nerd, &row(Kind::File, name, FILE), false);
            assert_eq!(look, ("\u{e7a8}", Role::Code.fg().0), "{name:?}");
        }
        // The last dot is what the extension starts at.
        let look = of(Set::Nerd, &row(Kind::File, "src.tar.gz", FILE), false);
        assert_eq!(look, ("\u{f1c6}", Role::Archive.fg().0));
        // A whole name is read before an extension is.
        let look = of(Set::Nerd, &row(Kind::File, ".gitignore", FILE), false);
        assert_eq!(look, ("\u{e702}", Role::Data.fg().0));
        // And a name neither table answers for keeps the plain file look.
        let plain = of(Set::Nerd, &row(Kind::File, "notes.xyz", FILE), false);
        assert_eq!(plain, (NF_PLAIN_FILE, FILE_FG));
        assert_eq!(
            of(Set::Nerd, &row(Kind::File, "COPYING", FILE), false),
            plain
        );
    }

    #[test]
    fn only_a_row_that_names_a_file_reaches_the_file_tables() {
        // A make target and an SSH host are `Path` rows with a label of their
        // own. A target named `release.tar` is not an archive.
        let target = of(
            Set::Nerd,
            &row(Kind::Path, "release.tar", "make target"),
            false,
        );
        assert_eq!(target, (NF_PLAIN_FILE, FILE_FG));
        // And the text set reads neither table at all.
        assert_eq!(
            of(Set::Text, &row(Kind::File, "build.rs", FILE), false),
            (PLAIN_FILE, FILE_FG),
        );
    }

    #[test]
    fn no_two_roles_share_a_colour() {
        // A file row shares a menu with an option row, a directory row and
        // the row that runs the line, so the roles have to clear the kinds as
        // well as each other.
        let mut seen: HashSet<&str> = KINDS.iter().map(|&k| kind_fg(k).0).collect();
        for role in [
            Role::Code,
            Role::Data,
            Role::Doc,
            Role::Media,
            Role::Archive,
        ] {
            assert!(seen.insert(role.fg().0), "{role:?}");
        }
    }

    #[test]
    fn no_name_is_in_two_rows_of_a_table() {
        // A name in two rows would draw as whichever came first and the other
        // row would be a glyph nothing reaches.
        for table in [EXTENSIONS, NAMES] {
            let mut seen = HashSet::new();
            for (names, _, _) in table {
                for name in *names {
                    assert!(seen.insert(*name), "{name:?}");
                    assert_eq!(*name, name.to_lowercase(), "{name:?}");
                }
            }
        }
    }

    #[test]
    fn the_file_tables_hold_what_claude_md_says_they_hold() {
        // `CLAUDE.md` gives a person these three numbers under "Use". A row
        // added here without a word there leaves the document saying
        // something the build no longer does.
        let keys = |table: &[(&[&str], &str, Role)]| {
            table.iter().map(|(names, _, _)| names.len()).sum::<usize>()
        };
        assert_eq!(keys(EXTENSIONS), 76);
        assert_eq!(keys(NAMES), 16);
        let shapes: HashSet<&str> = EXTENSIONS
            .iter()
            .chain(NAMES)
            .map(|&(_, glyph, _)| glyph)
            .collect();
        assert_eq!(shapes.len(), 33);
    }

    #[test]
    fn the_widest_glyph_is_what_the_names_are_padded_out_to() {
        for set in SETS {
            assert_eq!(widest(set), 1);
        }
    }
}
