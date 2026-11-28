//! Isolated wake-loop state files stored at [`Home::runner_skipped_wake_file`]
//! and [`Home::runner_admission_file`], outside the state database and its
//! writer lock ([`Home::writer_lock_file`]; `project/instrukt/spec/domain/runner`,
//! "Runner background state").

use std::io::ErrorKind;
use std::path::Path;

use instrukt_core::fsx::{self, FsError};
use instrukt_core::home::Home;
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension as _};
use instrukt_core::state_db::{StateDb, StateRead};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::StateError;

/// What the running runner last did with the configuration it admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionOutcome {
    Replaced,
    Kept,
}

impl AdmissionOutcome {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "replaced" => Some(Self::Replaced),
            "kept" => Some(Self::Kept),
            _ => None,
        }
    }
}

/// The run loop's last skipped wake, persisted until the next unblocked wake
/// removes [`Home::runner_skipped_wake_file`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SkippedWake {
    pub at: Timestamp,
    pub cause: String,
}

impl SkippedWake {
    /// Renders this skipped wake in the `instrukt events status` JSON shape.
    pub(crate) fn into_json(self) -> serde_json::Value {
        serde_json::json!({
            "at": self.at.to_string(),
            "cause": self.cause,
        })
    }
}

/// The running runner's last configuration admission outcome, persisted at
/// [`Home::runner_admission_file`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunnerAdmission {
    pub at: Timestamp,
    pub outcome: AdmissionOutcome,
}

impl RunnerAdmission {
    /// Renders this admission outcome in the `instrukt events status` JSON
    /// shape (`admission.runner`).
    pub(crate) fn into_json(self) -> serde_json::Value {
        serde_json::json!({
            "at": self.at.to_string(),
            "outcome": self.outcome,
        })
    }
}

fn legacy_table_exists(conn: &Connection, path: &Path, table: &str) -> Result<bool, StateError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(super::sql(path))
}

fn read_legacy_skipped_wake(
    conn: &Connection,
    path: &Path,
) -> Result<Option<SkippedWake>, StateError> {
    if !legacy_table_exists(conn, path, "runner_skipped_wake")? {
        return Ok(None);
    }
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT at, cause FROM runner_skipped_wake WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(super::sql(path))?;
    match row {
        None => Ok(None),
        Some((at_raw, cause)) => {
            let at = at_raw.parse::<Timestamp>().map_err(|error| {
                super::invalid(path, format!("runner_skipped_wake.at: {error}"))
            })?;
            Ok(Some(SkippedWake { at, cause }))
        }
    }
}

fn read_legacy_runner_admission(
    conn: &Connection,
    path: &Path,
) -> Result<Option<RunnerAdmission>, StateError> {
    if !legacy_table_exists(conn, path, "runner_admission")? {
        return Ok(None);
    }
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT at, outcome FROM runner_admission WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(super::sql(path))?;
    match row {
        None => Ok(None),
        Some((at_raw, outcome_raw)) => {
            let at = at_raw
                .parse::<Timestamp>()
                .map_err(|error| super::invalid(path, format!("runner_admission.at: {error}")))?;
            let outcome = AdmissionOutcome::parse(&outcome_raw).ok_or_else(|| {
                super::invalid(
                    path,
                    format!("runner_admission.outcome: unexpected value {outcome_raw:?}"),
                )
            })?;
            Ok(Some(RunnerAdmission { at, outcome }))
        }
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StateError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        StateError::Fs(FsError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(error),
        })
    })?;
    super::write_bytes(path, &bytes)
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, StateError> {
    super::refuse_symlink(path)?;
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| StateError::InvalidFile {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StateError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Transfers any legacy `runner_skipped_wake` and `runner_admission` rows from
/// the state database into [`Home::runner_skipped_wake_file`] and
/// [`Home::runner_admission_file`] without overwriting newer files, then drops
/// both legacy tables in one transaction. Lock-free no-op when read-only, when
/// no state database exists, or when neither legacy table exists.
pub(crate) fn ensure_wake_loop_migrated(home: &Home) -> Result<(), StateError> {
    if home.is_state_read_only() {
        return Ok(());
    }
    let (legacy_skipped, legacy_admission) = match StateRead::open(home)? {
        Some(read) => {
            let has_skipped = legacy_table_exists(read.conn(), read.path(), "runner_skipped_wake")?;
            let has_admission = legacy_table_exists(read.conn(), read.path(), "runner_admission")?;
            if !has_skipped && !has_admission {
                return Ok(());
            }
            let skipped = if has_skipped {
                read_legacy_skipped_wake(read.conn(), read.path())?
            } else {
                None
            };
            let admission = if has_admission {
                read_legacy_runner_admission(read.conn(), read.path())?
            } else {
                None
            };
            (skipped, admission)
        }
        None => return Ok(()),
    };

    let skipped_path = home.runner_skipped_wake_file();
    let admission_path = home.runner_admission_file();
    super::refuse_symlink(&skipped_path)?;
    super::refuse_symlink(&admission_path)?;

    if let Some(skipped) = &legacy_skipped
        && !skipped_path.exists()
    {
        write_json(&skipped_path, skipped)?;
    }
    if let Some(admission) = &legacy_admission
        && !admission_path.exists()
    {
        write_json(&admission_path, admission)?;
    }

    let mut db = StateDb::open(home)?;
    db.remove_runner_wake_loop_state()?;
    Ok(())
}

/// Persists or clears the run loop's last skipped wake in
/// [`Home::runner_skipped_wake_file`]. Migrates any remaining legacy tables
/// first so clearing a skipped wake can never be resurrected by a stale
/// database row.
pub(crate) fn record_skipped_wake(
    home: &Home,
    skipped: Option<(Timestamp, &str)>,
) -> Result<(), StateError> {
    ensure_wake_loop_migrated(home)?;
    let path = home.runner_skipped_wake_file();
    match skipped {
        Some((at, cause)) => write_json(
            &path,
            &SkippedWake {
                at,
                cause: cause.to_owned(),
            },
        ),
        None => {
            super::refuse_symlink(&path)?;
            match fsx::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
                Err(source) => Err(StateError::Fs(FsError::Write { path, source })),
            }
        }
    }
}

/// Persists the running runner's last configuration admission outcome in
/// [`Home::runner_admission_file`]. Migrates any remaining legacy tables first.
pub(crate) fn record_admission_outcome(
    home: &Home,
    at: Timestamp,
    outcome: AdmissionOutcome,
) -> Result<(), StateError> {
    ensure_wake_loop_migrated(home)?;
    write_json(
        &home.runner_admission_file(),
        &RunnerAdmission { at, outcome },
    )
}

/// Records the run loop's last skipped wake, at `at` for `cause`; `None`
/// removes it, as a wake that is not skipped does.
pub(crate) fn skipped_wake(home: &Home, skipped: Option<(Timestamp, &str)>) {
    if let Err(error) = record_skipped_wake(home, skipped) {
        tracing::error!(event = "runner_skipped_wake_unrecorded", error = %error);
    }
}

/// Records what the run loop did with the configuration it admitted at
/// `at`: replaced the generation in force, or kept it.
pub(crate) fn admission_outcome(home: &Home, at: Timestamp, outcome: AdmissionOutcome) {
    if let Err(error) = record_admission_outcome(home, at, outcome) {
        tracing::error!(event = "runner_admission_unrecorded", error = %error);
    }
}

/// Reads the run loop's last skipped wake without locking or writing disk.
/// Falls back to an un-migrated legacy `runner_skipped_wake` row via
/// [`StateRead`] when the JSON file is absent.
pub(crate) fn read_skipped_wake(home: &Home) -> Result<Option<SkippedWake>, StateError> {
    if let Some(record) = read_json_file(&home.runner_skipped_wake_file())? {
        return Ok(Some(record));
    }
    match StateRead::open(home)? {
        Some(read) => read_legacy_skipped_wake(read.conn(), read.path()),
        None => Ok(None),
    }
}

/// Reads the runner's last configuration admission outcome without locking or
/// writing disk. Falls back to an un-migrated legacy `runner_admission` row
/// via [`StateRead`] when the JSON file is absent.
pub(crate) fn read_runner_admission(home: &Home) -> Result<Option<RunnerAdmission>, StateError> {
    if let Some(record) = read_json_file(&home.runner_admission_file())? {
        return Ok(Some(record));
    }
    match StateRead::open(home)? {
        Some(read) => read_legacy_runner_admission(read.conn(), read.path()),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use instrukt_core::fsx;
    use instrukt_core::home::Home;
    use instrukt_core::state_db::StateDb;
    use jiff::Timestamp;

    use super::{
        AdmissionOutcome, RunnerAdmission, SkippedWake, ensure_wake_loop_migrated,
        read_runner_admission, read_skipped_wake, record_admission_outcome, record_skipped_wake,
    };
    use crate::state::StateError;

    fn temp_home() -> (tempfile::TempDir, Home) {
        let dir = fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        (dir, home)
    }

    fn seed_legacy_tables(
        home: &Home,
        skipped: Option<(&str, &str)>,
        admission: Option<(&str, &str)>,
    ) {
        let db = StateDb::open(home).unwrap();
        db.conn()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS runner_skipped_wake (
                    id    INTEGER PRIMARY KEY CHECK (id = 1),
                    at    TEXT NOT NULL,
                    cause TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS runner_admission (
                    id      INTEGER PRIMARY KEY CHECK (id = 1),
                    at      TEXT NOT NULL,
                    outcome TEXT NOT NULL CHECK (outcome IN ('replaced', 'kept'))
                );",
            )
            .unwrap();
        if let Some((at, cause)) = skipped {
            db.conn()
                .execute(
                    "INSERT INTO runner_skipped_wake(id, at, cause) VALUES(1, ?1, ?2)",
                    [at, cause],
                )
                .unwrap();
        }
        if let Some((at, outcome)) = admission {
            db.conn()
                .execute(
                    "INSERT INTO runner_admission(id, at, outcome) VALUES(1, ?1, ?2)",
                    [at, outcome],
                )
                .unwrap();
        }
    }

    #[test]
    fn skipped_wake_and_admission_round_trip_overwrite_and_clear() {
        let (_dir, home) = temp_home();
        let t1: Timestamp = "2026-11-28T01:00:00Z".parse().unwrap();
        let t2: Timestamp = "2026-11-28T01:05:00Z".parse().unwrap();

        assert_eq!(read_skipped_wake(&home).unwrap(), None);
        assert_eq!(read_runner_admission(&home).unwrap(), None);
        // Clearing an absent skipped-wake file is a no-op and creates no files.
        record_skipped_wake(&home, None).unwrap();
        assert!(!home.runner_skipped_wake_file().exists());

        record_skipped_wake(&home, Some((t1, "dispatch lock held"))).unwrap();
        let loaded_skipped = read_skipped_wake(&home).unwrap().unwrap();
        assert_eq!(
            loaded_skipped,
            SkippedWake {
                at: t1,
                cause: "dispatch lock held".into(),
            }
        );
        assert_eq!(
            loaded_skipped.into_json(),
            serde_json::json!({"at": "2026-11-28T01:00:00Z", "cause": "dispatch lock held"})
        );

        record_skipped_wake(&home, Some((t2, "still busy"))).unwrap();
        assert_eq!(
            read_skipped_wake(&home).unwrap(),
            Some(SkippedWake {
                at: t2,
                cause: "still busy".into(),
            })
        );

        record_skipped_wake(&home, None).unwrap();
        assert_eq!(read_skipped_wake(&home).unwrap(), None);
        assert!(!home.runner_skipped_wake_file().exists());

        record_admission_outcome(&home, t1, AdmissionOutcome::Kept).unwrap();
        let kept = read_runner_admission(&home).unwrap().unwrap();
        assert_eq!(
            kept,
            RunnerAdmission {
                at: t1,
                outcome: AdmissionOutcome::Kept,
            }
        );
        assert_eq!(
            kept.into_json(),
            serde_json::json!({"at": "2026-11-28T01:00:00Z", "outcome": "kept"})
        );

        record_admission_outcome(&home, t2, AdmissionOutcome::Replaced).unwrap();
        let replaced = read_runner_admission(&home).unwrap().unwrap();
        assert_eq!(
            replaced.into_json(),
            serde_json::json!({"at": "2026-11-28T01:05:00Z", "outcome": "replaced"})
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_at_wake_loop_paths_are_refused_on_read_write_and_clear() {
        let (_dir, home) = temp_home();
        fsx::create_dir_all(home.state_dir()).unwrap();
        let target = home.root().join("outside.json");
        std::fs::write(&target, b"{}").unwrap();
        std::os::unix::fs::symlink(&target, home.runner_skipped_wake_file()).unwrap();
        std::os::unix::fs::symlink(&target, home.runner_admission_file()).unwrap();
        let now: Timestamp = "2026-11-28T02:00:00Z".parse().unwrap();

        assert!(matches!(
            read_skipped_wake(&home),
            Err(StateError::Symlink { path }) if path == home.runner_skipped_wake_file()
        ));
        assert!(matches!(
            record_skipped_wake(&home, Some((now, "busy"))),
            Err(StateError::Symlink { path }) if path == home.runner_skipped_wake_file()
        ));
        assert!(matches!(
            record_skipped_wake(&home, None),
            Err(StateError::Symlink { path }) if path == home.runner_skipped_wake_file()
        ));
        assert!(matches!(
            read_runner_admission(&home),
            Err(StateError::Symlink { path }) if path == home.runner_admission_file()
        ));
        assert!(matches!(
            record_admission_outcome(&home, now, AdmissionOutcome::Replaced),
            Err(StateError::Symlink { path }) if path == home.runner_admission_file()
        ));
        assert!(target.exists(), "target of refused symlink stays intact");
    }

    #[test]
    fn corrupt_json_returns_invalid_file_naming_its_path() {
        let (_dir, home) = temp_home();
        fsx::create_dir_all(home.state_dir()).unwrap();
        std::fs::write(home.runner_skipped_wake_file(), b"{not-json").unwrap();
        std::fs::write(
            home.runner_admission_file(),
            b"{\"at\":\"bad\",\"outcome\":\"kept\"}",
        )
        .unwrap();

        assert!(matches!(
            read_skipped_wake(&home),
            Err(StateError::InvalidFile { path, .. }) if path == home.runner_skipped_wake_file()
        ));
        assert!(matches!(
            read_runner_admission(&home),
            Err(StateError::InvalidFile { path, .. }) if path == home.runner_admission_file()
        ));
    }

    #[test]
    fn legacy_migration_preserves_newer_files_and_drops_tables_idempotently() {
        let (_dir, home) = temp_home();
        let legacy_at: Timestamp = "2026-11-28T01:00:00Z".parse().unwrap();
        let newer_at: Timestamp = "2026-11-28T03:00:00Z".parse().unwrap();
        seed_legacy_tables(
            &home,
            Some(("2026-11-28T01:00:00Z", "legacy lock")),
            Some(("2026-11-28T01:00:00Z", "kept")),
        );

        // Read-only inspection sees legacy rows without mutating disk or creating files.
        let readonly = home.for_inspection();
        ensure_wake_loop_migrated(&readonly).unwrap();
        assert!(!home.runner_skipped_wake_file().exists());
        assert!(!home.runner_admission_file().exists());
        assert_eq!(
            read_skipped_wake(&readonly).unwrap(),
            Some(SkippedWake {
                at: legacy_at,
                cause: "legacy lock".into(),
            })
        );
        assert_eq!(
            read_runner_admission(&readonly).unwrap(),
            Some(RunnerAdmission {
                at: legacy_at,
                outcome: AdmissionOutcome::Kept,
            })
        );

        // Pre-write a newer admission file to prove migration never overwrites it.
        super::write_json(
            &home.runner_admission_file(),
            &RunnerAdmission {
                at: newer_at,
                outcome: AdmissionOutcome::Replaced,
            },
        )
        .unwrap();

        ensure_wake_loop_migrated(&home).unwrap();
        assert_eq!(
            read_skipped_wake(&home).unwrap(),
            Some(SkippedWake {
                at: legacy_at,
                cause: "legacy lock".into(),
            })
        );
        assert_eq!(
            read_runner_admission(&home).unwrap(),
            Some(RunnerAdmission {
                at: newer_at,
                outcome: AdmissionOutcome::Replaced,
            })
        );

        // Both legacy tables are gone and a second migration call is a lock-free no-op.
        let db = StateDb::open(&home).unwrap();
        let remaining: u32 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('runner_skipped_wake', 'runner_admission')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        drop(db);
        ensure_wake_loop_migrated(&home).unwrap();
    }

    #[test]
    fn clearing_skipped_wake_before_reconcile_migrates_first_so_legacy_row_cannot_resurrect() {
        let (_dir, home) = temp_home();
        seed_legacy_tables(&home, Some(("2026-11-28T01:00:00Z", "stale skip")), None);

        record_skipped_wake(&home, None).unwrap();
        assert_eq!(read_skipped_wake(&home).unwrap(), None);
        assert!(!home.runner_skipped_wake_file().exists());
    }
}
