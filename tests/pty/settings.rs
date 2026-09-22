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
