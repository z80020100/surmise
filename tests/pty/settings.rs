//! `surmise settings path` over the installed command interface.

use surmise::fixture::Fixture;

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
