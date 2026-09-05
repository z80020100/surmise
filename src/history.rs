//! Successful directory changes. Each source directory has preferences of its own.
//!
//! SQLite keeps concurrent shells from losing each other's updates. The picker
//! reads once and keeps the resulting weights for the whole menu session.

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{DirBuilder, OpenOptions};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const DAY: i64 = 86_400;
const HALF_LIFE: i64 = 30 * DAY;
const RETENTION: i64 = 180 * DAY;
const WRITE_WAIT: Duration = Duration::from_millis(50);
/// The least a recorded visit can weigh. Six half-lives fit inside the
/// retention window and one visit still carries a sixty-fourth after them.
const FLOOR: f64 = 0.01;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}

fn location(data: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    data.filter(|p| p.is_absolute())
        .or_else(|| {
            home.filter(|p| p.is_absolute())
                .map(|p| p.join(".local/share"))
        })
        .map(|p| p.join("surmise/history.sqlite3"))
}

fn database() -> Option<PathBuf> {
    location(
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn decay(weight: f64, last: i64, at: i64) -> f64 {
    weight * 2.0_f64.powf(-(at.saturating_sub(last).max(0) as f64) / HALF_LIFE as f64)
}

fn directory(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(io::Error::other(
            "history requires an absolute directory path",
        ));
    }
    path.canonicalize()
}

/// Record the change the shell reported. A storage failure leaves the shell quiet.
pub fn record(source: &Path, target: &Path) -> bool {
    database().is_some_and(|db| record_at(&db, source, target, now()).is_ok())
}

fn record_at(db: &Path, source: &Path, target: &Path, at: i64) -> Result<()> {
    let source = directory(source)?;
    let target = directory(target)?;
    if source == target {
        return Ok(());
    }
    let parent = db.parent().ok_or("history has no parent directory")?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(db)
    {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(WRITE_WAIT)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS visits_v1 (
            source BLOB NOT NULL,
            target BLOB NOT NULL,
            visits INTEGER NOT NULL,
            last_used INTEGER NOT NULL,
            weight REAL NOT NULL,
            PRIMARY KEY (source, target)
        ) WITHOUT ROWID;",
    )?;
    let source = source.as_os_str().as_bytes();
    let target = target.as_os_str().as_bytes();
    let previous: Option<(f64, i64)> = tx
        .query_row(
            "SELECT weight, last_used FROM visits_v1 WHERE source = ?1 AND target = ?2",
            params![source, target],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    // A record's own time never moves back. The retention cursor below stays
    // the caller's time rather than this one. A record left in the future
    // would otherwise take every honest record with it.
    let (weight, last_used) = previous.map_or((1.0, at), |(weight, last)| {
        (decay(weight, last, at) + 1.0, at.max(last))
    });
    tx.execute(
        "INSERT INTO visits_v1 (source, target, visits, last_used, weight)
         VALUES (?1, ?2, 1, ?3, ?4)
         ON CONFLICT (source, target) DO UPDATE SET
         visits = MIN(visits, ?5) + 1,
         last_used = excluded.last_used, weight = excluded.weight",
        params![source, target, last_used, weight, i64::MAX - 1],
    )?;
    tx.execute(
        "DELETE FROM visits_v1 WHERE last_used < ?1",
        [at.saturating_sub(RETENTION)],
    )?;
    tx.commit()?;
    Ok(())
}

/// The preferences of the directory where the picker opened.
#[derive(Default)]
pub struct History(HashMap<PathBuf, f64>);

impl History {
    pub fn load(source: &Path) -> Self {
        database().map_or_else(Self::default, |db| {
            Self::read(&db, source, now()).unwrap_or_default()
        })
    }

    fn read(db: &Path, source: &Path, at: i64) -> Result<Self> {
        let source = directory(source)?;
        // Reading never creates a database. A locked writer must not hold a key.
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(Duration::ZERO)?;
        let mut stmt = conn.prepare(
            "SELECT target, weight, last_used FROM visits_v1
             WHERE source = ?1 AND last_used >= ?2",
        )?;
        let rows = stmt.query_map(
            params![source.as_os_str().as_bytes(), at.saturating_sub(RETENTION)],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        let mut weights = HashMap::new();
        for row in rows {
            let (target, weight, last) = row?;
            let weight = decay(weight, last, at);
            if weight.is_finite() && weight >= FLOOR {
                weights.insert(PathBuf::from(OsString::from_vec(target)), weight);
            }
        }
        Ok(Self(weights))
    }

    pub(crate) fn weight(&self, target: &Path) -> f64 {
        if self.0.is_empty() {
            return 0.0;
        }
        target
            .canonicalize()
            .ok()
            .and_then(|p| self.0.get(&p).copied())
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::candidates::{self, Kind};
    use crate::fixture::Fixture;
    use std::os::unix::fs::PermissionsExt;

    const AT: i64 = 1_800_000_000;

    fn count(db: &Path) -> i64 {
        Connection::open(db)
            .unwrap()
            .query_row("SELECT SUM(visits) FROM visits_v1", [], |row| row.get(0))
            .unwrap()
    }

    /// `powf` returns these weights exactly on every platform this builds for.
    /// It does not promise to and the room here is what covers the difference.
    fn weighs(history: &History, target: &Path, want: f64) {
        let got = history.weight(target);
        assert!((got - want).abs() < 1e-9, "{got} is not {want}");
    }

    fn names(history: &History, cwd: &Path, arg: &str) -> Vec<String> {
        candidates::generate_in(arg, cwd, history)
            .into_iter()
            .filter(|c| c.kind == Kind::Dir)
            .map(|c| c.display)
            .collect()
    }

    #[test]
    fn storage_uses_an_absolute_data_home_or_the_home_default() {
        let path = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            location(path("/data"), path("/home/demo")),
            path("/data/surmise/history.sqlite3")
        );
        for data in [None, path(""), path("relative")] {
            assert_eq!(
                location(data, path("/home/demo")),
                path("/home/demo/.local/share/surmise/history.sqlite3")
            );
        }
        assert!(location(None, None).is_none());
        assert!(location(path("relative"), path("relative")).is_none());
    }

    #[test]
    fn a_change_survives_reopening_and_belongs_to_its_source() {
        let f = Fixture::new(&["alpha", "beta", "other"]);
        let db = f.path().join("data/history.sqlite3");
        let target = f.path().join("beta");
        record_at(&db, f.path(), &target, AT).unwrap();
        record_at(&db, f.path(), &target, AT).unwrap();
        assert_eq!(count(&db), 2);
        let h = History::read(&db, f.path(), AT).unwrap();
        weighs(&h, &target, 2.0);
        assert_eq!(names(&h, f.path(), "")[0], "beta/");
        let other = History::read(&db, &f.path().join("other"), AT).unwrap();
        weighs(&other, &target, 0.0);
        assert_eq!(names(&other, f.path(), "")[0], "alpha/");
        // A umask can only take bits away. The claim is that nothing outside
        // the owner reaches the history rather than an exact mode.
        assert_eq!(
            std::fs::metadata(&db).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            std::fs::metadata(db.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }

    #[test]
    fn frequency_decays_and_a_new_visit_does_not_restore_old_weight() {
        let f = Fixture::new(&["alpha", "beta"]);
        let db = f.path().join("history.sqlite3");
        let alpha = f.path().join("alpha");
        for _ in 0..4 {
            record_at(&db, f.path(), &alpha, AT).unwrap();
        }
        record_at(&db, f.path(), &f.path().join("beta"), AT + 3 * HALF_LIFE).unwrap();
        let h = History::read(&db, f.path(), AT + 3 * HALF_LIFE).unwrap();
        weighs(&h, &alpha, 0.5);
        assert_eq!(names(&h, f.path(), "")[0], "beta/");
        record_at(&db, f.path(), &alpha, AT + 3 * HALF_LIFE).unwrap();
        let h = History::read(&db, f.path(), AT + 3 * HALF_LIFE).unwrap();
        weighs(&h, &alpha, 1.5);
        assert_eq!(count(&db), 6);
    }

    #[test]
    fn an_old_record_expires_on_read_and_is_removed_on_write() {
        let f = Fixture::new(&["alpha", "beta"]);
        let db = f.path().join("history.sqlite3");
        record_at(&db, f.path(), &f.path().join("alpha"), AT).unwrap();
        let later = AT + RETENTION + 1;
        assert!(History::read(&db, f.path(), later).unwrap().0.is_empty());
        record_at(&db, f.path(), &f.path().join("beta"), later).unwrap();
        assert_eq!(count(&db), 1);
    }

    #[test]
    fn a_clock_that_moves_back_does_not_multiply_the_weight() {
        let f = Fixture::new(&["alpha"]);
        let db = f.path().join("history.sqlite3");
        let target = f.path().join("alpha");
        record_at(&db, f.path(), &target, AT).unwrap();
        record_at(&db, f.path(), &target, AT - DAY).unwrap();
        weighs(&History::read(&db, f.path(), AT).unwrap(), &target, 2.0);
    }

    #[test]
    fn a_record_left_in_the_future_does_not_expire_the_others() {
        let f = Fixture::new(&["alpha", "beta"]);
        let db = f.path().join("history.sqlite3");
        let alpha = f.path().join("alpha");
        let beta = f.path().join("beta");
        // A clock that ran ahead dates one record past the retention window.
        // Visiting that record again must not carry the cursor with it.
        record_at(&db, f.path(), &beta, AT + 2 * RETENTION).unwrap();
        record_at(&db, f.path(), &alpha, AT).unwrap();
        record_at(&db, f.path(), &beta, AT).unwrap();
        weighs(&History::read(&db, f.path(), AT).unwrap(), &alpha, 1.0);
    }

    #[test]
    fn history_cannot_promote_a_fuzzy_match_above_a_prefix_or_exact_name() {
        let f = Fixture::new(&["wo", "work", "workshop", "a_wo"]);
        let db = f.path().join("history.sqlite3");
        for _ in 0..10 {
            record_at(&db, f.path(), &f.path().join("a_wo"), AT).unwrap();
        }
        record_at(&db, f.path(), &f.path().join("workshop"), AT).unwrap();
        let h = History::read(&db, f.path(), AT).unwrap();
        for prefix in [
            String::new(),
            "./".to_string(),
            format!("{}/", f.path().display()),
        ] {
            assert_eq!(
                names(&h, f.path(), &format!("{prefix}wo")),
                ["wo/", "workshop/", "work/", "a_wo/"]
            );
        }
        let items = candidates::generate_in("wo", f.path(), &h);
        assert_eq!(items[0].kind, Kind::Run);
        std::fs::remove_dir(f.path().join("workshop")).unwrap();
        assert!(!names(&h, f.path(), "wo").contains(&"workshop/".to_string()));
    }

    #[test]
    fn a_menu_keeps_its_snapshot_and_origin_while_the_argument_changes() {
        let f = Fixture::new(&["alpha", "beta", "nested/alpha", "nested/beta"]);
        let db = f.path().join("history.sqlite3");
        record_at(&db, f.path(), &f.path().join("beta"), AT).unwrap();
        record_at(&db, f.path(), &f.path().join("nested/beta"), AT).unwrap();
        let mut app =
            App::over(f.path(), "cd ").with_history(History::read(&db, f.path(), AT).unwrap());
        for _ in 0..4 {
            record_at(&db, f.path(), &f.path().join("alpha"), AT).unwrap();
        }
        app.refresh();
        assert_eq!(app.items[0].display, "beta/");
        app.line.insert("nested/");
        app.refresh();
        assert_eq!(app.items[1].display, "beta/");
        let fresh = History::read(&db, f.path(), AT).unwrap();
        assert_eq!(names(&fresh, f.path(), "")[0], "alpha/");
    }

    #[test]
    fn symlinks_share_physical_paths_and_path_bytes_survive() {
        let f = Fixture::new(&["source", "target"]);
        let db = f.path().join("history.sqlite3");
        let source = f.path().join("source");
        let alias = f.path().join("alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let target = source.join("目錄 'with spaces\nand quotes\"");
        std::fs::create_dir(&target).unwrap();
        record_at(&db, &alias, &target, AT).unwrap();
        let h = History::read(&db, &source, AT).unwrap();
        weighs(&h, &target, 1.0);
        record_at(&db, &source, &alias, AT).unwrap();
        assert_eq!(count(&db), 1);
    }

    #[test]
    fn missing_corrupt_or_locked_storage_leaves_the_text_order_available() {
        let f = Fixture::new(&["alpha", "beta", "file*"]);
        let db = f.path().join("missing/history.sqlite3");
        let h = History::read(&db, f.path(), AT).unwrap_or_default();
        assert!(!db.parent().unwrap().exists());
        assert_eq!(names(&h, f.path(), "")[0], "alpha/");
        assert!(record_at(&db, f.path(), &f.path().join("file"), AT).is_err());
        assert!(!db.exists());
        std::fs::write(f.path().join("corrupt"), b"not a database").unwrap();
        assert!(History::read(&f.path().join("corrupt"), f.path(), AT).is_err());
        record_at(&db, f.path(), &f.path().join("beta"), AT).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
        assert!(History::read(&db, f.path(), AT).is_err());
        assert!(record_at(&db, f.path(), &f.path().join("alpha"), AT).is_err());
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(count(&db), 1);
    }

    #[test]
    fn concurrent_writers_keep_every_successful_update() {
        let f = Fixture::new(&["alpha"]);
        let db = f.path().join("history.sqlite3");
        let target = f.path().join("alpha");
        let barrier = std::sync::Barrier::new(4);
        let successes: usize = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        (0..8)
                            .filter(|_| record_at(&db, f.path(), &target, AT).is_ok())
                            .count()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).sum()
        });
        assert!(successes > 0);
        assert_eq!(count(&db), successes as i64);
    }
}
