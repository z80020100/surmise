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

use crate::icons::Set;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
