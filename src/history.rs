//! Successful directory changes. Each source directory has preferences of its
//! own and an absolute argument reads the preferences of every source.
//!
//! SQLite keeps concurrent shells from losing each other's updates. The picker
//! reads once and keeps the resulting weights for the whole menu session.

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeMap;
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

/// Where the database would be, whether or not it exists. `None` means there
/// is no absolute directory to keep one in and the history is unavailable.
pub(crate) fn database() -> Option<PathBuf> {
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
pub struct History {
    source: PathBuf,
    /// The visits from `source`.
    here: BTreeMap<PathBuf, f64>,
    /// The visits from every source added up for each target.
    everywhere: BTreeMap<PathBuf, f64>,
}

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
            "SELECT source, target, weight, last_used FROM visits_v1
             WHERE last_used >= ?1",
        )?;
        let rows = stmt.query_map(params![at.saturating_sub(RETENTION)], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut here = BTreeMap::new();
        let mut everywhere = BTreeMap::new();
        for row in rows {
            let (from, target, weight, last) = row?;
            let weight = decay(weight, last, at);
            if weight.is_finite() && weight >= FLOOR {
                let target = PathBuf::from(OsString::from_vec(target));
                if from == source.as_os_str().as_bytes() {
                    here.insert(target.clone(), weight);
                }
                *everywhere.entry(target).or_insert(0.0) += weight;
            }
        }
        Ok(Self {
            source,
            here,
            everywhere,
        })
    }

    /// What `target` weighs when the argument reached it through `prefix`:
    /// its own visits and those of every directory beneath it. A relative
    /// prefix names another directory from each source and counts the visits
    /// from here. An absolute one names the same directory from every source
    /// and counts the visits from all of them. The rows under it therefore
    /// rank the same wherever the menu opened. A menu reaches a deep
    /// directory one level at a time and each level on the way is a row a
    /// person picks. A row that leads back up keeps its own visits alone.
    /// Under a relative prefix those rows are the directory the menu opened
    /// in and every directory above it. Every visit from here lies beneath
    /// one of them and none of them is the way down to it. Under an absolute
    /// prefix they are the links back to the listed directory or above it. A
    /// link to `/` is one.
    pub(crate) fn weight(&self, prefix: &str, target: &Path) -> f64 {
        let listed = crate::path::expand(prefix);
        let absolute = listed.is_absolute();
        let weights = if absolute {
            &self.everywhere
        } else {
            &self.here
        };
        if weights.is_empty() {
            return 0.0;
        }
        let Ok(target) = target.canonicalize() else {
            return 0.0;
        };
        let leads_back_up = if absolute {
            // A row physically beneath the listed directory cannot lead above it.
            !target.starts_with(&listed)
                && listed
                    .canonicalize()
                    .is_ok_and(|physical| physical.starts_with(&target))
        } else {
            self.source.starts_with(&target)
        };
        if leads_back_up {
            return weights.get(&target).copied().unwrap_or(0.0);
        }
        // A path sorts by component and everything beneath it follows it in one run.
        weights
            .range::<PathBuf, _>(&target..)
            .take_while(|(path, _)| path.starts_with(&target))
            // `sum` starts at -0.0 and `total_cmp` sorts that below a row that weighs nothing.
            .fold(0.0, |total, (_, weight)| total + weight)
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
        let got = history.weight("", target);
        assert!((got - want).abs() < 1e-9, "{got} is not {want}");
    }

    fn names(history: &History, cwd: &Path, arg: &str) -> Vec<String> {
        candidates::generate_in(arg, cwd, history, &mut candidates::Scan::default())
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
        assert!(
            History::read(&db, f.path(), later)
                .unwrap()
                .everywhere
                .is_empty()
        );
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
        let items = candidates::generate_in("wo", f.path(), &h, &mut candidates::Scan::default());
        assert_eq!(items[0].kind, Kind::Run);
        std::fs::remove_dir(f.path().join("workshop")).unwrap();
        assert!(!names(&h, f.path(), "wo").contains(&"workshop/".to_string()));
    }

    #[test]
    fn history_outranks_the_case_that_was_typed() {
        // `dist` and `Docs` tie on everything else and `dist` is in the case
        // that was typed.
        let f = Fixture::new(&["dist", "Docs"]);
        let db = f.path().join("history.sqlite3");
        assert_eq!(
            names(&History::default(), f.path(), "d"),
            ["dist/", "Docs/"]
        );
        record_at(&db, f.path(), &f.path().join("Docs"), AT).unwrap();
        let h = History::read(&db, f.path(), AT).unwrap();
        assert_eq!(names(&h, f.path(), "d"), ["Docs/", "dist/"]);
    }

    #[test]
    fn a_menu_keeps_its_snapshot_and_origin_while_the_argument_changes() {
        let f = Fixture::new(&["alpha", "beta", "nested/alpha", "nested/beta"]);
        let db = f.path().join("history.sqlite3");
        record_at(&db, f.path(), &f.path().join("beta"), AT).unwrap();
        record_at(&db, f.path(), &f.path().join("nested/beta"), AT).unwrap();
        let mut app = App::over(f.path(), "cd ");
        app.seed_history(
            History::read(&db, f.path(), AT).unwrap(),
            crate::histfile::Counts::default(),
            crate::candidates::Sort::Recent,
        );
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
    fn a_directory_carries_the_visits_beneath_it() {
        let f = Fixture::new(&["alpha", "zul", "zulu/alpha", "zulu/middle/leaf"]);
        let db = f.path().join("history.sqlite3");
        let zulu = f.path().join("zulu");
        let leaf = zulu.join("middle/leaf");
        record_at(&db, f.path(), &leaf, AT).unwrap();
        record_at(&db, f.path(), &leaf, AT).unwrap();
        record_at(&db, f.path(), &zulu, AT).unwrap();
        let h = History::read(&db, f.path(), AT).unwrap();
        weighs(&h, &zulu, 3.0);
        weighs(&h, &zulu.join("middle"), 2.0);
        weighs(&h, &leaf, 2.0);
        weighs(&h, &zulu.join("alpha"), 0.0);
        // `zul` shares letters with `zulu` and no component.
        weighs(&h, &f.path().join("zul"), 0.0);
        assert_eq!(names(&h, f.path(), "")[0], "zulu/");
        assert_eq!(names(&h, f.path(), "zulu/")[0], "middle/");
    }

    #[test]
    fn the_way_back_up_keeps_its_own_visits() {
        let f = Fixture::new(&["level/child", "sibling"]);
        let db = f.path().join("history.sqlite3");
        let level = f.path().join("level");
        record_at(&db, &level, &level.join("child"), AT).unwrap();
        record_at(&db, &level, &level.join("child"), AT).unwrap();
        record_at(&db, &level, &f.path().join("sibling"), AT).unwrap();
        let up = f.path().join("up");
        std::os::unix::fs::symlink(f.path(), &up).unwrap();
        let h = History::read(&db, &level, AT).unwrap();
        weighs(&h, &level, 0.0);
        weighs(&h, f.path(), 0.0);
        weighs(&h, &up, 0.0);
        assert_eq!(names(&h, &level, "../")[0], "sibling/");
    }

    #[test]
    fn an_absolute_argument_ranks_the_same_from_every_directory() {
        let f = Fixture::new(&["level/child", "sibling", "elsewhere"]);
        let db = f.path().join("history.sqlite3");
        let level = f.path().join("level");
        let elsewhere = f.path().join("elsewhere");
        record_at(&db, &level, &level.join("child"), AT).unwrap();
        record_at(&db, &elsewhere, &level.join("child"), AT).unwrap();
        record_at(&db, &elsewhere, &f.path().join("sibling"), AT).unwrap();
        let arg = format!("{}/", f.path().display());
        for cwd in [
            f.path().to_path_buf(),
            elsewhere.clone(),
            level.clone(),
            level.join("child"),
        ] {
            let h = History::read(&db, &cwd, AT).unwrap();
            assert_eq!(names(&h, &cwd, &arg), ["level/", "sibling/", "elsewhere/"]);
        }
    }

    #[test]
    fn a_link_back_up_keeps_its_own_visits_under_an_absolute_argument() {
        let f = Fixture::new(&["place/alpha", "elsewhere"]);
        let db = f.path().join("history.sqlite3");
        let place = f.path().join("place");
        let elsewhere = f.path().join("elsewhere");
        std::os::unix::fs::symlink(f.path(), place.join("up")).unwrap();
        record_at(&db, &elsewhere, &place.join("alpha"), AT).unwrap();
        record_at(&db, &elsewhere, &place.join("alpha"), AT).unwrap();
        record_at(&db, &place, &elsewhere, AT).unwrap();
        let h = History::read(&db, &elsewhere, AT).unwrap();
        let arg = format!("{}/", place.display());
        assert_eq!(names(&h, &elsewhere, &arg), ["alpha/", "up/"]);
    }

    #[test]
    fn an_absolute_argument_counts_the_visits_from_every_source() {
        let f = Fixture::new(&["place/alpha", "place/beta", "here", "elsewhere"]);
        let db = f.path().join("history.sqlite3");
        let place = f.path().join("place");
        let here = f.path().join("here");
        let elsewhere = f.path().join("elsewhere");
        record_at(&db, &here, &place.join("alpha"), AT).unwrap();
        record_at(&db, &elsewhere, &place.join("beta"), AT).unwrap();
        record_at(&db, &elsewhere, &place.join("beta"), AT).unwrap();
        let h = History::read(&db, &here, AT).unwrap();
        assert_eq!(names(&h, &here, "../place/"), ["alpha/", "beta/"]);
        assert_eq!(
            names(&h, &here, &format!("{}/", place.display())),
            ["beta/", "alpha/"]
        );
        let mut app = App::over(&here, "ls ");
        app.seed_history(
            h,
            crate::histfile::Counts::default(),
            crate::candidates::Sort::Recent,
        );
        app.line.insert(&format!("{}/", place.display()));
        app.refresh();
        let rows: Vec<_> = app
            .items
            .iter()
            .filter(|c| c.kind == Kind::Path)
            .map(|c| c.display.as_str())
            .collect();
        assert_eq!(rows[..2], ["beta/", "alpha/"]);
    }

    #[test]
    fn parent_visits_do_not_move_navigation_above_child_directories() {
        let f = Fixture::new(&["level/child", "level/peer"]);
        let db = f.path().join("history.sqlite3");
        let source = f.path().join("level");
        record_at(&db, &source, f.path(), AT).unwrap();
        let h = History::read(&db, &source, AT).unwrap();
        for arg in ["", "./"] {
            let items = candidates::generate_in(arg, &source, &h, &mut candidates::Scan::default());
            let rows: Vec<_> = items.iter().filter(|c| c.kind != Kind::Run).collect();
            assert_eq!(rows[0].display, "child/");
            assert_eq!(rows[1].display, "peer/");
            assert_eq!(rows[2].kind, Kind::Parent);
        }
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
