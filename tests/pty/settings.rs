//! `surmise settings` over the installed command interface.
//!
//! `config`'s own tests reach `edit_at` with a path in hand. These run the
//! command a person types, so the words they typed, the path the binary
//! resolves for itself and the file that comes out are all one claim.

use std::path::Path;
use std::process::{Command, Output};
use surmise::fixture::Fixture;

/// `surmise settings <words>` in a home of its own. The three tests above
/// build this themselves because each one names a different environment.
fn settings(home: &Path, words: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_surmise"))
        .arg("settings")
        .args(words)
        .env_clear()
        .env("HOME", home)
        .output()
        .unwrap()
}

/// Every key this build reads with the value it has where nothing set one.
const DEFAULTS: &str = "disabled_commands = []\nenabled = true\nicons = \"text\"\nspec_dirs = []\n";

fn config(home: &Path) -> String {
    std::fs::read_to_string(home.join(".config/surmise/config.toml")).expect("a config file")
}

#[test]
fn settings_path_prints_the_resolved_path_under_home() {
    let f = Fixture::new(&["home"]);
    let home = f.path().join("home");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_surmise"))
        .args(["settings", "path"])
        .env_clear()
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success());
    let printed = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        printed.trim_end(),
        home.join(".config/surmise/config.toml").to_str().unwrap()
    );
}

#[test]
fn settings_path_prefers_xdg_config_home_over_the_home_default() {
    let f = Fixture::new(&["home", "config"]);
    let home = f.path().join("home");
    let config_home = f.path().join("config");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_surmise"))
        .args(["settings", "path"])
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .unwrap();
    assert!(output.status.success());
    let printed = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        printed.trim_end(),
        config_home.join("surmise/config.toml").to_str().unwrap()
    );
}

#[test]
fn an_unknown_settings_word_is_refused_rather_than_the_picker() {
    let f = Fixture::new(&["home"]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_surmise"))
        .args(["settings", "nope"])
        .env_clear()
        .env("HOME", f.path().join("home"))
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn a_write_reaches_the_file_the_path_command_names() {
    // The seam `config`'s own tests cannot reach: the binary resolving its
    // own path and writing there.
    let f = Fixture::new(&["home"]);
    let home = f.path().join("home");
    let said = settings(&home, &["set", "icons", "nerd"]);
    assert!(said.status.success());
    let printed = String::from_utf8(said.stdout).unwrap();
    let path = home.join(".config/surmise/config.toml");
    assert!(printed.contains(path.to_str().unwrap()), "{printed}");
    assert_eq!(config(&home), "icons = \"nerd\"\n");
    // And what `settings path` names is the file that was written.
    let named = settings(&home, &["path"]);
    assert_eq!(
        String::from_utf8(named.stdout).unwrap().trim_end(),
        path.to_str().unwrap()
    );
}

#[test]
fn a_list_verb_and_a_scalar_verb_both_reach_that_file() {
    let f = Fixture::new(&["home"]);
    let home = f.path().join("home");
    assert!(
        settings(&home, &["set", "enabled", "false"])
            .status
            .success()
    );
    assert!(
        settings(&home, &["add", "disabled_commands", "kubectl"])
            .status
            .success()
    );
    assert!(
        settings(&home, &["add", "disabled_commands", "helm"])
            .status
            .success()
    );
    assert!(
        settings(&home, &["remove", "disabled_commands", "kubectl"])
            .status
            .success()
    );
    assert_eq!(
        config(&home),
        "enabled = false\ndisabled_commands = [\"helm\"]\n"
    );
    assert!(settings(&home, &["unset", "enabled"]).status.success());
    assert_eq!(config(&home), "disabled_commands = [\"helm\"]\n");
}

#[test]
fn a_value_the_picker_would_ignore_is_refused_and_writes_nothing() {
    // The trap this command exists to close. A file holding `icons = "emoji"`
    // reads as a warning nothing prints, so the person sees a menu that did
    // not change and no reason why.
    let f = Fixture::new(&["home"]);
    let home = f.path().join("home");
    let said = settings(&home, &["set", "icons", "emoji"]);
    assert!(!said.status.success());
    let complaint = String::from_utf8(said.stderr).unwrap();
    assert!(
        complaint.contains("icons takes nerd or text"),
        "{complaint}"
    );
    assert!(!home.join(".config/surmise/config.toml").exists());
}

#[test]
fn settings_show_prints_what_the_writes_before_it_made() {
    // The other end of `set`. A person who has just written a value asks for
    // the file back and reads the value the picker will use, whether or not
    // the file names the key at all.
    let f = Fixture::new(&["home"]);
    let home = f.path().join("home");
    let before = settings(&home, &["show"]);
    assert!(before.status.success());
    // No file under this home yet, so every key here is a default.
    assert_eq!(String::from_utf8(before.stdout).unwrap(), DEFAULTS);
    assert!(!home.join(".config/surmise/config.toml").exists());

    assert!(settings(&home, &["set", "icons", "nerd"]).status.success());
    assert!(
        settings(&home, &["add", "disabled_commands", "kubectl"])
            .status
            .success()
    );
    let after = settings(&home, &["show"]);
    assert_eq!(
        String::from_utf8(after.stdout).unwrap(),
        "disabled_commands = [\"kubectl\"]\nenabled = true\nicons = \"nerd\"\nspec_dirs = []\n"
    );
}

#[test]
fn settings_show_leads_with_a_comment_when_the_file_will_not_parse() {
    // The picker runs on defaults and prints nothing about it. This is where
    // a person finds out their file is not being read.
    let f = Fixture::new(&["home"]);
    let home = f.path().join("home");
    let path = home.join(".config/surmise/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "enabled = [this is not toml").unwrap();
    let said = settings(&home, &["show"]);
    assert!(said.status.success());
    let printed = String::from_utf8(said.stdout).unwrap();
    assert!(
        printed.starts_with("# config file did not parse"),
        "{printed}"
    );
    assert!(printed.contains("icons = \"text\""), "{printed}");
    // The file it complained about is the file it left alone.
    assert_eq!(config(&home), "enabled = [this is not toml");
}

#[test]
fn settings_show_takes_nothing_after_it() {
    let f = Fixture::new(&["home"]);
    let said = settings(&f.path().join("home"), &["show", "icons"]);
    assert!(!said.status.success());
    let complaint = String::from_utf8(said.stderr).unwrap();
    assert!(complaint.contains("takes nothing after it"), "{complaint}");
}

#[test]
fn settings_show_answers_where_there_is_no_home_to_put_a_config_under() {
    // `path` has nothing to name here and says so. `show` has the defaults
    // to name and they are what the picker would run on, so the two answer
    // differently on purpose.
    let show = Command::new(env!("CARGO_BIN_EXE_surmise"))
        .args(["settings", "show"])
        .env_clear()
        .output()
        .unwrap();
    assert!(show.status.success());
    assert_eq!(String::from_utf8(show.stdout).unwrap(), DEFAULTS);
    let path = Command::new(env!("CARGO_BIN_EXE_surmise"))
        .args(["settings", "path"])
        .env_clear()
        .output()
        .unwrap();
    assert!(!path.status.success());
    assert!(
        String::from_utf8(path.stderr).unwrap().contains("no home"),
        "the path complaint changed"
    );
}
