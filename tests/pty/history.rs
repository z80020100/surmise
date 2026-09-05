//! The directory hook and its storage through the installed command interface.

use rusqlite::Connection;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Command, Output};
use surmise::fixture::Fixture;

fn shell(root: &Path, script: &str) -> Output {
    Command::new("/bin/zsh")
        .args([
            "-fc",
            &format!("eval \"$($SURMISE_BIN init zsh)\"\n{script}"),
        ])
        .env_clear()
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("PATH", "/usr/bin:/bin")
        .env("SURMISE_BIN", env!("CARGO_BIN_EXE_surmise"))
        .current_dir(root)
        .output()
        .unwrap()
}

fn visits(root: &Path, source: &Path, target: &Path) -> i64 {
    let conn = Connection::open(root.join("data/surmise/history.sqlite3")).unwrap();
    conn.query_row(
        "SELECT COALESCE(SUM(visits), 0) FROM visits_v1 WHERE source = ?1 AND target = ?2",
        rusqlite::params![
            source.canonicalize().unwrap().as_os_str().as_bytes(),
            target.canonicalize().unwrap().as_os_str().as_bytes()
        ],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn manual_changes_record_once_and_keep_other_hooks_and_command_status() {
    let f = Fixture::new(&["alpha", "beta"]);
    let result = shell(
        f.path(),
        r#"
        after_move() { print OTHER }
        add-zsh-hook chpwd after_move
        cd alpha
        print STATUS:$?
        cd .
        cd ../missing 2>/dev/null
        print FAILED:$?
        cd ../beta
    "#,
    );
    assert!(result.status.success());
    assert!(result.stderr.is_empty(), "{:?}", result.stderr);
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("STATUS:0"), "{stdout}");
    assert!(stdout.contains("FAILED:1"), "{stdout}");
    assert_eq!(stdout.matches("OTHER").count(), 3);
    assert_eq!(visits(f.path(), f.path(), &f.path().join("alpha")), 1);
    assert_eq!(
        visits(f.path(), &f.path().join("alpha"), &f.path().join("beta")),
        1
    );
    assert_eq!(
        visits(f.path(), &f.path().join("alpha"), &f.path().join("alpha")),
        0
    );
}

#[test]
fn a_quiet_change_does_not_give_the_next_change_a_stale_source() {
    let f = Fixture::new(&["alpha", "beta"]);
    let result = shell(f.path(), "cd -q alpha\ncd ../beta");
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    assert_eq!(visits(f.path(), f.path(), &f.path().join("alpha")), 0);
    assert_eq!(
        visits(f.path(), &f.path().join("alpha"), &f.path().join("beta")),
        1
    );
}

#[test]
fn storage_failure_stays_quiet_and_does_not_stop_later_hooks() {
    let f = Fixture::new(&["alpha", "data*"]);
    let result = shell(
        f.path(),
        r#"
        after_move() { print OTHER }
        add-zsh-hook chpwd after_move
        cd alpha
        print STATUS:$?
    "#,
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"OTHER\nSTATUS:0\n");
    assert!(result.stderr.is_empty());
    assert_eq!(std::fs::read(f.path().join("data")).unwrap(), b"");
}

#[test]
fn the_record_command_preserves_spaces_quotes_and_unicode_paths() {
    let f = Fixture::new(&[]);
    let target = f.path().join("目錄 'with spaces\nand quotes\"");
    std::fs::create_dir(&target).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_surmise"))
        .arg("--record")
        .arg(f.path())
        .arg(&target)
        .env_clear()
        .env("XDG_DATA_HOME", f.path().join("data"))
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
    assert_eq!(visits(f.path(), f.path(), &target), 1);
}

#[test]
fn an_unresolvable_non_utf8_path_fails_without_a_panic_or_database() {
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new(&[]);
    let target = f
        .path()
        .join(std::ffi::OsString::from_vec(b"missing\xff".to_vec()));
    let result = Command::new(env!("CARGO_BIN_EXE_surmise"))
        .arg("--record")
        .arg(f.path())
        .arg(target)
        .env_clear()
        .env("XDG_DATA_HOME", f.path().join("data"))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
    assert!(!f.path().join("data").exists());
}
