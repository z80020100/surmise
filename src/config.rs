//! Settings a person changes. `$XDG_CONFIG_HOME/surmise/config.toml` and the
//! defaults everywhere it says nothing. `history.rs` resolves
//! `$XDG_DATA_HOME` the same way and this follows it so the two agree.
//!
//! A parse error must never reach the prompt: nothing surmise does may write
//! to a person's terminal outside the menu. `load` keeps the complaint as a
//! `warning` for a later `doctor` command instead, and runs with defaults
//! meanwhile. An unknown key is a warning of the same kind rather than an
//! error, because a file written for a later surmise should still work with
//! this one. So is a value a key does not know, and that key keeps its
//! default while the rest of the file stands.
//!
//! [`edit`] is the writing end. This file is a person's own and nothing
//! surmise decides for itself may touch it: a menu reads it and leaves it
//! where it found it. `surmise settings set` is a person naming the change
//! themselves, which is the one thing that writes here. `state.rs` is what
//! surmise writes without being asked and it is kept somewhere else for
//! exactly that reason.

use crate::icons::Set;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Value};

/// The effective settings: the file's own values where it has them, and the
/// default otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `false` answers every keystroke with `PASS` and the shell's own
    /// completion runs instead.
    pub enabled: bool,
    /// First words that skip the spec lookup. `cd` and Git do not go through
    /// a spec, so this reaches neither.
    pub disabled_commands: Vec<String>,
    /// Directories laid out like `specs/` and holding the same JSON,
    /// searched in order before the compiled-in data.
    pub spec_dirs: Vec<PathBuf>,
    /// Which glyphs the menu draws in front of a name. `"nerd"` is a person
    /// saying their terminal has a font that carries them and `icons` is
    /// where that answer is spent.
    pub icons: Set,
    /// What went wrong reading the file, if anything did. Nothing here
    /// prints it; a later `doctor` command is what a person sees this
    /// through.
    pub warning: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            disabled_commands: Vec::new(),
            spec_dirs: Vec::new(),
            icons: Set::default(),
            warning: None,
        }
    }
}

/// The keys this build reads. Every field is optional so a file naming one
/// key keeps the defaults for the rest, and `extra` catches whatever key the
/// file names that this build does not read yet. `plans/phase-6-settings-cli.md`
/// names the rest of the schema; a key with no reader stays out of `Config`
/// until the phase that reads it lands.
#[derive(Deserialize, Default)]
struct Schema {
    enabled: Option<bool>,
    disabled_commands: Option<Vec<String>>,
    spec_dirs: Option<Vec<PathBuf>>,
    icons: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Config {
    /// The effective config for this process: the file at `path()`, or
    /// defaults where there is no file, no home to place one under, or
    /// nothing this build can parse.
    pub fn load() -> Config {
        path().map_or_else(Config::default, |p| Self::load_from(&p))
    }

    /// `load`, over a path this build already resolved. A missing file means
    /// defaults, quietly: only a file that exists and fails to parse, or
    /// names a key this build does not know, leaves a `warning`.
    fn load_from(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Config::default(),
        }
    }

    fn parse(text: &str) -> Config {
        match toml::from_str::<Schema>(text) {
            Ok(schema) => {
                // Every complaint the file earns, rather than the first of
                // them. A person who names an unknown key and misspells a
                // value has two things to fix and a `doctor` that reported
                // one would send them back for the other.
                let mut warnings: Vec<String> = Vec::new();
                if !schema.extra.is_empty() {
                    let mut keys: Vec<&str> = schema.extra.keys().map(String::as_str).collect();
                    keys.sort_unstable();
                    warnings.push(format!("unknown config key(s): {}", keys.join(", ")));
                }
                // A value this build does not know keeps the default rather
                // than turning the whole file away. The rest of the file is
                // still what the person meant.
                let icons = match schema.icons.as_deref() {
                    None => Set::default(),
                    Some("text") => Set::Text,
                    Some("nerd") => Set::Nerd,
                    Some(other) => {
                        warnings.push(format!("icons is not \"text\" or \"nerd\": {other:?}"));
                        Set::default()
                    }
                };
                Config {
                    enabled: schema.enabled.unwrap_or(true),
                    disabled_commands: schema.disabled_commands.unwrap_or_default(),
                    spec_dirs: schema.spec_dirs.unwrap_or_default(),
                    icons,
                    warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
                }
            }
            Err(e) => Config {
                warning: Some(format!("config file did not parse: {e}")),
                ..Config::default()
            },
        }
    }
}

/// Where the config file would be, whether or not it exists. `None` means
/// there is no absolute directory to place one under and the defaults stand.
pub fn path() -> Option<PathBuf> {
    location(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn location(config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    config_home
        .filter(|p| p.is_absolute())
        .or_else(|| home.filter(|p| p.is_absolute()).map(|p| p.join(".config")))
        .map(|p| p.join("surmise/config.toml"))
}

/// The file a write lands in.
///
/// A `config.toml` that is a symbolic link is a person pointing at a file
/// they keep somewhere else, a dotfiles repository most often. Writing beside
/// the link and renaming over it would leave them a plain file where their
/// link was, and the file they keep would go stale without a word. The link
/// is followed instead and the file it names is the one written.
///
/// `atomic::replace` still refuses to write through a link and `state.toml`
/// needs that: it is surmise's own file and nothing should be able to point
/// it somewhere else. Following one is this module's choice for this file
/// rather than that one's for every file.
///
/// A link with no target yet names a file this makes. A link this cannot
/// read at all is left to `replace`, which will fail on it and say so.
fn through_links(path: &Path) -> PathBuf {
    if !std::fs::symlink_metadata(path).is_ok_and(|at| at.is_symlink()) {
        return path.to_path_buf();
    }
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    let Ok(target) = std::fs::read_link(path) else {
        return path.to_path_buf();
    };
    if target.is_absolute() {
        target
    } else {
        // A relative target is relative to the directory the link sits in.
        path.parent().unwrap_or(Path::new("")).join(target)
    }
}

/// What a key's value looks like. [`edit`] reads this to turn the words a
/// person typed into a value [`Config`] above will accept, and to refuse a
/// word it would not.
enum Shape {
    Bool,
    /// One of a fixed set of words and nothing else.
    Word(&'static [&'static str]),
    /// A list of strings, changed one entry at a time.
    List,
}

/// Every key a person may write, in the order a complaint names them. A key
/// with no row here is one nothing reads, and writing it would leave a person
/// a line in their file that does nothing. `Schema` above is the reading end
/// of this same list and the two have to agree.
const KEYS: &[(&str, Shape)] = &[
    ("disabled_commands", Shape::List),
    ("enabled", Shape::Bool),
    ("icons", Shape::Word(&["nerd", "text"])),
    ("spec_dirs", Shape::List),
];

/// What a write asks for. The key and the value arrive as the words a person
/// typed and this module is what says whether they mean anything.
pub enum Edit<'a> {
    /// Give a key this value, whatever it held before.
    Set { key: &'a str, value: &'a str },
    /// Put one entry into a list key.
    Add { key: &'a str, value: &'a str },
    /// Take one entry back out of a list key.
    Remove { key: &'a str, value: &'a str },
    /// Drop a key so its default stands again.
    Unset { key: &'a str },
}

impl Edit<'_> {
    fn key(&self) -> &str {
        match *self {
            Edit::Set { key, .. }
            | Edit::Add { key, .. }
            | Edit::Remove { key, .. }
            | Edit::Unset { key } => key,
        }
    }
}

/// Carry out `what` against the file at [`path`] and say what happened, or
/// say why nothing did.
///
/// Every refusal here comes back whole rather than printed, because this
/// module may not write to a terminal and `main` is what owns the one this
/// ran on.
pub fn edit(what: Edit) -> Result<String, String> {
    let path = path().ok_or("no home directory to place a config under")?;
    edit_at(&path, what)
}

/// [`edit`], over a path this build already resolved. The tests reach this
/// one, the same way they reach `load_from` rather than `load`.
fn edit_at(path: &Path, what: Edit) -> Result<String, String> {
    let path = &through_links(path);
    let key = what.key();
    let shape = KEYS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, shape)| shape)
        .ok_or_else(|| {
            let names: Vec<&str> = KEYS.iter().map(|(name, _)| *name).collect();
            format!(
                "no config key named {}. this build reads {}",
                crate::ui::printable(key),
                names.join(", ")
            )
        })?;
    // A file that will not parse is one a person wrote and is still owed. A
    // document this cannot read is a document it cannot put back either, and
    // writing anyway would hand them an empty file where their own work was.
    //
    // Only a file that is not there starts this from nothing. Every other
    // reason a read fails is a file that exists and holds something, and
    // `unwrap_or_default` on one of those is the whole of that content
    // replaced by whatever this write was. `Config::load_from` treats the two
    // alike and can afford to: it reads, and a read that fails leaves the
    // defaults standing rather than taking anything away.
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{} did not parse and was left alone: {e}", path.display()))?;
    let Some(done) = apply(&mut doc, &what, shape)? else {
        return Ok(format!("nothing to change in {}", path.display()));
    };
    // The file was a person's before surmise ever wrote to it and keeps
    // whatever permissions they gave it. A file this write is making is the
    // owner's alone, the same as every other file surmise creates.
    let mode = std::fs::metadata(path).map_or(0o600, |m| m.permissions().mode() & 0o777);
    crate::atomic::replace(path, &doc.to_string(), mode)
        .map_err(|e| format!("{} was not written: {e}", path.display()))?;
    Ok(format!("{done} in {}", path.display()))
}

/// Carry `what` into `doc`. `None` means the document already said that and
/// nothing needs writing.
fn apply(doc: &mut DocumentMut, what: &Edit, shape: &Shape) -> Result<Option<String>, String> {
    let key = what.key();
    match *what {
        Edit::Set { value, .. } => {
            if matches!(shape, Shape::List) {
                return Err(format!(
                    "{key} holds a list. `settings add` and `settings remove` are what change one"
                ));
            }
            let new = read(value, shape, key)?;
            let said = format!("{key} = {}", bare(&new));
            match doc.get_mut(key).and_then(Item::as_value_mut) {
                Some(old) => {
                    if bare(old) == bare(&new) {
                        return Ok(None);
                    }
                    // The decor is the space in front of a value and whatever
                    // follows it on the line, a trailing comment included.
                    // Assigning a fresh item would drop both and hand a person
                    // their own file with a comment missing.
                    let decor = old.decor().clone();
                    *old = new;
                    *old.decor_mut() = decor;
                }
                None => doc[key] = Item::Value(new),
            }
            Ok(Some(said))
        }
        Edit::Add { value, .. } => {
            let array = list(doc, key, shape)?;
            if holds(array, value) {
                return Ok(None);
            }
            array.push(value);
            tidy(array);
            Ok(Some(format!("{key} = {}", entries(array))))
        }
        Edit::Remove { value, .. } => {
            let array = list(doc, key, shape)?;
            let Some(at) = array.iter().position(|v| v.as_str() == Some(value)) else {
                return Ok(None);
            };
            array.remove(at);
            tidy(array);
            Ok(Some(format!("{key} = {}", entries(array))))
        }
        Edit::Unset { .. } => match doc.remove(key) {
            Some(_) => Ok(Some(format!("{key} dropped and its default stands"))),
            None => Ok(None),
        },
    }
}

/// The array `key` holds, made empty where the file has no such key. An
/// entry holding anything but a list is a person's own line and this refuses
/// rather than replaces it.
fn list<'a>(doc: &'a mut DocumentMut, key: &str, shape: &Shape) -> Result<&'a mut Array, String> {
    if !matches!(shape, Shape::List) {
        return Err(format!(
            "{key} holds one value. `settings set` is what changes it"
        ));
    }
    doc.entry(key)
        .or_insert_with(|| Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .ok_or_else(|| format!("{key} in the file is not a list. edit it by hand"))
}

fn holds(array: &Array, value: &str) -> bool {
    array.iter().any(|v| v.as_str() == Some(value))
}

/// A value as the file would hold it, without the space in front of it or the
/// comment behind it. What a person is told is what changed rather than what
/// else happens to be on that line, and what one write compares against the
/// last is the value rather than the line it sits on.
fn bare(value: &Value) -> String {
    let mut bare = value.clone();
    bare.decor_mut().clear();
    bare.to_string().trim().to_string()
}

/// One space between entries and none inside the brackets, whatever the entry
/// that was added or taken out left behind. A push arrives with no space of
/// its own and a removal can leave the one that separated it from the entry
/// before, so the list is spaced here rather than at either edit.
fn tidy(array: &mut Array) {
    for (at, value) in array.iter_mut().enumerate() {
        value.decor_mut().set_prefix(if at == 0 { "" } else { " " });
    }
    array.set_trailing_comma(false);
}

/// The same for a whole list.
fn entries(array: &Array) -> String {
    let items: Vec<String> = array.iter().map(bare).collect();
    format!("[{}]", items.join(", "))
}

/// The word a person typed, as the value its key takes.
fn read(word: &str, shape: &Shape, key: &str) -> Result<Value, String> {
    match shape {
        Shape::Bool => match word {
            "true" => Ok(Value::from(true)),
            "false" => Ok(Value::from(false)),
            other => Err(format!(
                "{key} takes true or false, not {}",
                crate::ui::printable(other)
            )),
        },
        Shape::Word(words) => {
            if words.contains(&word) {
                Ok(Value::from(word))
            } else {
                Err(format!(
                    "{key} takes {}, not {}",
                    words.join(" or "),
                    crate::ui::printable(word)
                ))
            }
        }
        // `list` above is what changes one of these and it never reaches here.
        Shape::List => Err(format!("{key} holds a list")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    #[test]
    fn a_missing_file_gives_defaults() {
        let f = Fixture::new(&[]);
        let config = Config::load_from(&f.path().join("missing/config.toml"));
        assert_eq!(config, Config::default());
    }

    #[test]
    fn each_of_the_four_keys_parses_from_a_real_file() {
        let f = Fixture::new(&[]);
        let path = f.path().join("config.toml");
        std::fs::write(
            &path,
            "enabled = false\n\
             disabled_commands = [\"ls\", \"cd\"]\n\
             spec_dirs = [\"/opt/specs\", \"/home/demo/specs\"]\n\
             icons = \"nerd\"\n",
        )
        .unwrap();
        let config = Config::load_from(&path);
        assert!(!config.enabled);
        assert_eq!(config.disabled_commands, ["ls", "cd"]);
        assert_eq!(
            config.spec_dirs,
            [
                PathBuf::from("/opt/specs"),
                PathBuf::from("/home/demo/specs")
            ]
        );
        assert_eq!(config.icons, Set::Nerd);
        assert_eq!(config.warning, None);
    }

    #[test]
    fn the_glyph_set_is_the_text_one_unless_the_file_names_the_other() {
        assert_eq!(Config::parse("").icons, Set::Text);
        assert_eq!(Config::parse("icons = \"text\"").icons, Set::Text);
        assert_eq!(Config::parse("icons = \"nerd\"").icons, Set::Nerd);
    }

    #[test]
    fn a_glyph_set_this_build_does_not_know_is_a_warning_and_nothing_more() {
        // The rest of the file is still what the person meant.
        let config = Config::parse("enabled = false\nicons = \"emoji\"\n");
        assert!(!config.enabled);
        assert_eq!(config.icons, Set::Text);
        assert_eq!(
            config.warning,
            Some("icons is not \"text\" or \"nerd\": \"emoji\"".to_string())
        );
    }

    #[test]
    fn a_file_that_earns_two_complaints_keeps_both() {
        let config = Config::parse("verbose_names = false\nicons = \"emoji\"\n");
        let warning = config.warning.expect("a warning");
        assert!(warning.contains("verbose_names"), "{warning:?}");
        assert!(warning.contains("emoji"), "{warning:?}");
    }

    #[test]
    fn an_unknown_key_is_ignored_and_kept_as_a_warning() {
        let config = Config::parse("enabled = true\nverbose_names = false\n");
        assert!(config.enabled);
        assert_eq!(
            config.warning,
            Some("unknown config key(s): verbose_names".to_string())
        );
    }

    #[test]
    fn a_malformed_file_gives_defaults_and_a_warning() {
        let config = Config::parse("enabled = [this is not toml");
        assert_eq!(config.enabled, Config::default().enabled);
        assert_eq!(
            config.disabled_commands,
            Config::default().disabled_commands
        );
        assert_eq!(config.spec_dirs, Config::default().spec_dirs);
        assert_eq!(config.icons, Config::default().icons);
        assert!(config.warning.is_some());
    }

    /// A config file holding `text`, and the path to it.
    fn written(f: &Fixture, text: &str) -> PathBuf {
        let path = f.path().join(".config/surmise/config.toml");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(&path, text).expect("a config file");
        path
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("the file back")
    }

    #[test]
    fn a_write_keeps_every_comment_and_blank_line_the_file_had() {
        // The whole reason this reads a document rather than a value. A
        // person's own file comes back with one value changed and nothing
        // else touched.
        let f = Fixture::new(&[]);
        let path = written(
            &f,
            "# the top of my file\nenabled = true  # why\nicons = \"text\"  # which\n\ndisabled_commands = [\"kubectl\"]  # slow\n",
        );
        edit_at(
            &path,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect("a write");
        edit_at(
            &path,
            Edit::Add {
                key: "disabled_commands",
                value: "helm",
            },
        )
        .expect("a write");
        assert_eq!(
            read(&path),
            "# the top of my file\nenabled = true  # why\nicons = \"nerd\"  # which\n\ndisabled_commands = [\"kubectl\", \"helm\"]  # slow\n",
        );
    }

    #[test]
    fn a_write_with_no_file_under_it_makes_one() {
        let f = Fixture::new(&[]);
        let path = f.path().join(".config/surmise/config.toml");
        let said = edit_at(
            &path,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect("a write");
        assert!(said.starts_with("icons = \"nerd\" in "), "{said:?}");
        assert_eq!(read(&path), "icons = \"nerd\"\n");
        assert_eq!(Config::load_from(&path).icons, Set::Nerd);
    }

    #[test]
    fn a_write_that_changes_nothing_says_so() {
        let f = Fixture::new(&[]);
        let path = written(&f, "icons = \"nerd\"\n");
        for what in [
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
            Edit::Unset { key: "enabled" },
            Edit::Remove {
                key: "spec_dirs",
                value: "/opt/specs",
            },
        ] {
            let said = edit_at(&path, what).expect("an answer");
            assert!(said.starts_with("nothing to change in "), "{said:?}");
        }
        assert_eq!(read(&path), "icons = \"nerd\"\n");
    }

    #[test]
    fn a_list_grows_and_shrinks_one_entry_at_a_time() {
        let f = Fixture::new(&[]);
        let path = written(&f, "");
        for value in ["kubectl", "vault", "helm"] {
            edit_at(
                &path,
                Edit::Add {
                    key: "disabled_commands",
                    value,
                },
            )
            .expect("a write");
        }
        // The first entry is what a removal leaves a space in front of unless
        // the list is spaced again afterwards.
        let said = edit_at(
            &path,
            Edit::Remove {
                key: "disabled_commands",
                value: "kubectl",
            },
        )
        .expect("a write");
        assert!(
            said.starts_with("disabled_commands = [\"vault\", \"helm\"] in "),
            "{said:?}"
        );
        assert_eq!(read(&path), "disabled_commands = [\"vault\", \"helm\"]\n");
        assert_eq!(
            Config::load_from(&path).disabled_commands,
            ["vault", "helm"]
        );
    }

    #[test]
    fn unset_drops_the_key_and_leaves_the_rest() {
        let f = Fixture::new(&[]);
        let path = written(&f, "enabled = false\nicons = \"nerd\"\n");
        edit_at(&path, Edit::Unset { key: "enabled" }).expect("a write");
        assert_eq!(read(&path), "icons = \"nerd\"\n");
        assert!(Config::load_from(&path).enabled);
    }

    #[test]
    fn a_file_that_will_not_parse_is_left_exactly_as_it_was() {
        // A document this cannot read is one it cannot put back either.
        // Writing anyway would hand a person an empty file where their own
        // work was.
        let f = Fixture::new(&[]);
        let broken = "enabled = [this is not toml\n";
        let path = written(&f, broken);
        let complaint = edit_at(
            &path,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect_err("a complaint");
        assert!(
            complaint.contains("did not parse and was left alone"),
            "{complaint:?}"
        );
        assert_eq!(read(&path), broken);
    }

    #[test]
    fn a_key_or_a_value_this_build_does_not_read_writes_nothing() {
        let f = Fixture::new(&[]);
        let path = written(&f, "icons = \"text\"\n");
        let refused = |what| edit_at(&path, what).expect_err("a complaint");
        // A key with no reader would be a line in the file that does nothing.
        let said = refused(Edit::Set {
            key: "verbose_names",
            value: "false",
        });
        assert!(
            said.starts_with("no config key named verbose_names."),
            "{said:?}"
        );
        assert!(
            said.contains("disabled_commands, enabled, icons, spec_dirs"),
            "{said:?}"
        );
        // A value with no reader would be one the picker silently ignores,
        // which is the one thing a person typing a command must not get.
        assert_eq!(
            refused(Edit::Set {
                key: "icons",
                value: "emoji"
            }),
            "icons takes nerd or text, not emoji"
        );
        assert_eq!(
            refused(Edit::Set {
                key: "enabled",
                value: "yes"
            }),
            "enabled takes true or false, not yes"
        );
        // And each shape names the verb that does reach it.
        assert!(
            refused(Edit::Set {
                key: "spec_dirs",
                value: "/opt/specs"
            })
            .contains("`settings add` and `settings remove`"),
        );
        assert!(
            refused(Edit::Add {
                key: "icons",
                value: "nerd"
            })
            .contains("`settings set`"),
        );
        assert_eq!(read(&path), "icons = \"text\"\n");
    }

    #[test]
    fn a_list_key_holding_something_else_is_refused_rather_than_replaced() {
        let f = Fixture::new(&[]);
        let path = written(&f, "disabled_commands = \"kubectl\"\n");
        let complaint = edit_at(
            &path,
            Edit::Add {
                key: "disabled_commands",
                value: "helm",
            },
        )
        .expect_err("a complaint");
        assert!(complaint.contains("is not a list"), "{complaint:?}");
        assert_eq!(read(&path), "disabled_commands = \"kubectl\"\n");
    }

    #[test]
    fn every_key_the_reader_knows_is_one_a_person_can_write() {
        // `Schema` above and `KEYS` are two lists of the same thing. A key
        // added to one and not the other is a key a person can set and
        // nothing reads, or one nothing can set.
        let names: Vec<&str> = KEYS.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            ["disabled_commands", "enabled", "icons", "spec_dirs"]
        );
        let f = Fixture::new(&[]);
        let path = written(&f, "");
        for (name, shape) in KEYS {
            let what = match shape {
                Shape::List => Edit::Add {
                    key: name,
                    value: "sample",
                },
                Shape::Bool => Edit::Set {
                    key: name,
                    value: "false",
                },
                Shape::Word(words) => Edit::Set {
                    key: name,
                    value: words[0],
                },
            };
            edit_at(&path, what).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        // Nothing written here is a key the reader calls unknown.
        assert_eq!(Config::load_from(&path).warning, None);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_left_exactly_as_it_was() {
        // Only a file that is not there starts a write from nothing. A file
        // this cannot read is one that exists and holds something, and
        // treating it as absent would replace the whole of that with this
        // one line.
        let f = Fixture::new(&[]);
        let mine = "# mine\nenabled = false\ndisabled_commands = [\"kubectl\"]\n";
        let path = written(&f, mine);
        let locked = std::fs::Permissions::from_mode(0o000);
        std::fs::set_permissions(&path, locked).expect("a locked file");
        let complaint = edit_at(
            &path,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect_err("a complaint");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("it back");
        assert!(complaint.contains("could not be read"), "{complaint:?}");
        assert_eq!(read(&path), mine);
    }

    #[test]
    fn a_write_keeps_the_permissions_the_file_already_had() {
        // The file was a person's before surmise ever wrote to it. A write
        // that tightened it would be surmise deciding something nobody asked
        // it to.
        let f = Fixture::new(&[]);
        let path = written(&f, "enabled = true\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("a readable file");
        edit_at(
            &path,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect("a write");
        let got = std::fs::metadata(&path).expect("it back").permissions();
        assert_eq!(got.mode() & 0o777, 0o644);
        // And a file this write is making is the owner's alone.
        let fresh = f.path().join("fresh/config.toml");
        edit_at(
            &fresh,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect("a write");
        let got = std::fs::metadata(&fresh).expect("it back").permissions();
        assert_eq!(got.mode() & 0o777, 0o600);
    }

    #[test]
    fn a_config_that_is_a_link_is_followed_rather_than_replaced() {
        // The dotfiles case. A person who points this at a file they keep
        // elsewhere gets that file written, and the link is still a link
        // afterwards.
        let f = Fixture::new(&[]);
        let kept = f.path().join("dotfiles/surmise.toml");
        std::fs::create_dir_all(kept.parent().expect("a parent")).expect("a directory");
        std::fs::write(&kept, "# kept somewhere else\nenabled = false\n").expect("a file");
        let link = f.path().join(".config/surmise/config.toml");
        std::fs::create_dir_all(link.parent().expect("a parent")).expect("a directory");
        std::os::unix::fs::symlink(&kept, &link).expect("a link");

        let said = edit_at(
            &link,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect("a write");
        // What it says is where the bytes went. `canonicalize` is what the
        // write resolved the link with and macOS answers that with the real
        // `/private` path, so the name is read back the same way rather than
        // assumed.
        let real = std::fs::canonicalize(&kept).expect("the file");
        assert!(
            said.ends_with(&format!("in {}", real.display())),
            "{said:?}"
        );
        assert_eq!(
            read(&kept),
            "# kept somewhere else\nenabled = false\nicons = \"nerd\"\n"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the link")
                .is_symlink()
        );
    }

    #[test]
    fn a_link_with_no_target_yet_names_the_file_a_write_makes() {
        let f = Fixture::new(&[]);
        let kept = f.path().join("dotfiles/surmise.toml");
        std::fs::create_dir_all(kept.parent().expect("a parent")).expect("a directory");
        let link = f.path().join(".config/surmise/config.toml");
        std::fs::create_dir_all(link.parent().expect("a parent")).expect("a directory");
        std::os::unix::fs::symlink(&kept, &link).expect("a link");
        edit_at(
            &link,
            Edit::Set {
                key: "icons",
                value: "nerd",
            },
        )
        .expect("a write");
        assert_eq!(read(&kept), "icons = \"nerd\"\n");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the link")
                .is_symlink()
        );
    }

    #[test]
    fn the_path_falls_back_from_xdg_config_home_to_home_config() {
        let path = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            location(path("/config"), path("/home/demo")),
            path("/config/surmise/config.toml")
        );
        for config_home in [None, path(""), path("relative")] {
            assert_eq!(
                location(config_home, path("/home/demo")),
                path("/home/demo/.config/surmise/config.toml")
            );
        }
        assert!(location(None, None).is_none());
        assert!(location(path("relative"), path("relative")).is_none());
    }
}
