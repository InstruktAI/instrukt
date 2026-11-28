//! Per-worker failure files stored under [`Home::runner_workers_dir`],
//! isolated from the state database and its writer lock
//! ([`Home::writer_lock_file`]; `project/instrukt/spec/domain/runner`,
//! "Runner background state").

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use instrukt_core::fsx::{self, FsError};
use instrukt_core::home::Home;
use instrukt_core::state_db::rusqlite::Connection;
use instrukt_core::state_db::{StateDb, StateRead};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::StateError;

/// On-disk failure document for one runner worker.
#[derive(Debug, Serialize, Deserialize)]
struct WorkerRecord {
    worker: String,
    failure: String,
    since: Timestamp,
}

/// One runner worker's current failure, as shown in `instrukt events status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerFailure {
    pub worker: String,
    pub failure: String,
    pub since: Option<Timestamp>,
}

impl WorkerFailure {
    /// Renders this failure in the `instrukt events status` JSON shape.
    pub(crate) fn into_json(self) -> serde_json::Value {
        serde_json::json!({
            "worker": self.worker,
            "failure": self.failure,
            "since": self.since.map(|at| at.to_string()),
        })
    }
}

fn worker_entry_stem(name: &str) -> Option<&str> {
    if name.starts_with('.') {
        return None;
    }
    name.strip_suffix(".json")
}

fn unreadable_worker(stem: &str, error: String, since: Option<Timestamp>) -> WorkerFailure {
    WorkerFailure {
        worker: format!("runner-worker:{stem}"),
        failure: format!("runner worker failure state cannot be read: {error}"),
        since,
    }
}

fn legacy_workers_table_exists(conn: &Connection, path: &Path) -> Result<bool, StateError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runner_workers')",
        [],
        |row| row.get(0),
    )
    .map_err(super::sql(path))
}

fn read_legacy_worker_rows(
    conn: &Connection,
    path: &Path,
) -> Result<Vec<(String, String, String)>, StateError> {
    let mut statement = conn
        .prepare("SELECT worker, failure, since FROM runner_workers")
        .map_err(super::sql(path))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(super::sql(path))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(super::sql(path))
}

/// Transfers any active legacy `runner_workers` rows from the state database
/// into per-worker files under [`Home::runner_workers_dir`] without overwriting
/// newer files, then drops the legacy table. Lock-free no-op when read-only,
/// when no state database exists, or when the legacy table is already gone.
pub(crate) fn ensure_workers_migrated(home: &Home) -> Result<(), StateError> {
    if home.is_state_read_only() {
        return Ok(());
    }
    let rows = match StateRead::open(home)? {
        Some(read) => {
            if !legacy_workers_table_exists(read.conn(), read.path())? {
                return Ok(());
            }
            read_legacy_worker_rows(read.conn(), read.path())?
        }
        None => return Ok(()),
    };
    super::refuse_symlink(&home.runner_workers_dir())?;
    for (worker, failure, since_raw) in rows {
        let path = worker_path(home, &worker);
        super::refuse_symlink(&path)?;
        if path.exists() {
            continue;
        }
        let since = match since_raw.parse::<Timestamp>() {
            Ok(ts) => ts,
            Err(error) => {
                tracing::warn!(
                    event = "runner_worker_state_unread",
                    worker = %worker,
                    path = %path.display(),
                    %error
                );
                Timestamp::now()
            }
        };
        let record = WorkerRecord {
            worker,
            failure,
            since,
        };
        let bytes = serde_json::to_vec(&record).map_err(|error| {
            StateError::Fs(FsError::Write {
                path: path.clone(),
                source: std::io::Error::other(error),
            })
        })?;
        super::write_bytes(&path, &bytes)?;
    }
    let mut db = StateDb::open(home)?;
    db.remove_runner_workers()?;
    Ok(())
}

/// Reads all current runner worker failures without locking or creating
/// [`Home::runner_workers_dir`]. Unreadable or symlink entries surface as
/// worker failures rather than healthy state; any un-moved legacy
/// `runner_workers` rows visible through [`StateRead`] (including read-only
/// older-schema inspection snapshots) are merged without touching disk.
pub(crate) fn read_worker_failures(home: &Home) -> Result<Vec<WorkerFailure>, StateError> {
    let dir = home.runner_workers_dir();
    super::refuse_symlink(&dir)?;
    let mut failures = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) if error.kind() == ErrorKind::NotFound => continue,
                    Err(source) => return Err(StateError::Read { path: dir, source }),
                };
                let file_name = entry.file_name();
                let Some(stem) = file_name.to_str().and_then(worker_entry_stem) else {
                    continue;
                };
                let path = entry.path();
                let meta = match std::fs::symlink_metadata(&path) {
                    Ok(meta) => meta,
                    Err(error) if error.kind() == ErrorKind::NotFound => continue,
                    Err(error) => {
                        failures.push(unreadable_worker(stem, error.to_string(), None));
                        continue;
                    }
                };
                if meta.file_type().is_symlink() {
                    failures.push(unreadable_worker(
                        stem,
                        format!("{} is a symbolic link", path.display()),
                        None,
                    ));
                    continue;
                }
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|modified| Timestamp::try_from(modified).ok());
                match std::fs::read(&path) {
                    Ok(bytes) => match serde_json::from_slice::<WorkerRecord>(&bytes) {
                        Ok(record) => failures.push(WorkerFailure {
                            worker: record.worker,
                            failure: record.failure,
                            since: Some(record.since),
                        }),
                        Err(error) => {
                            failures.push(unreadable_worker(stem, error.to_string(), mtime))
                        }
                    },
                    Err(error) if error.kind() == ErrorKind::NotFound => continue,
                    Err(error) => failures.push(unreadable_worker(stem, error.to_string(), mtime)),
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => return Err(StateError::Read { path: dir, source }),
    }
    if let Some(read) = StateRead::open(home)?
        && legacy_workers_table_exists(read.conn(), read.path())?
    {
        for (worker, failure, since_raw) in read_legacy_worker_rows(read.conn(), read.path())? {
            if failures.iter().any(|entry| entry.worker == worker) {
                continue;
            }
            let since = match since_raw.parse::<Timestamp>() {
                Ok(ts) => Some(ts),
                Err(error) => {
                    tracing::warn!(
                        event = "runner_worker_state_unread",
                        worker = %worker,
                        path = %read.path().display(),
                        %error
                    );
                    None
                }
            };
            failures.push(WorkerFailure {
                worker,
                failure,
                since,
            });
        }
    }
    failures.sort_by(|a, b| a.worker.cmp(&b.worker));
    Ok(failures)
}

fn worker_path(home: &Home, worker: &str) -> PathBuf {
    home.runner_workers_dir()
        .join(format!("{}.json", crate::events::occurrence_id(&[worker])))
}

/// When `worker`'s existing failure file began, if readable and matching
/// `worker`. Symlinks are refused; other read/parse failures warn before
/// returning `None` so the caller replaces the corrupt file cleanly.
fn prior_since(path: &Path, worker: &str) -> Result<Option<Timestamp>, StateError> {
    super::refuse_symlink(path)?;
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<WorkerRecord>(&bytes) {
            Ok(record) if record.worker == worker => Ok(Some(record.since)),
            Ok(_) => {
                tracing::warn!(
                    event = "runner_worker_state_unread",
                    worker,
                    path = %path.display(),
                    error = "stored worker name does not match"
                );
                Ok(None)
            }
            Err(error) => {
                tracing::warn!(
                    event = "runner_worker_state_unread",
                    worker,
                    path = %path.display(),
                    %error
                );
                Ok(None)
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => {
            tracing::warn!(
                event = "runner_worker_state_unread",
                worker,
                path = %path.display(),
                %error
            );
            Ok(None)
        }
    }
}

/// Records `worker`'s current failure at `now`, preserving `since` from an
/// existing failure file; `None` removes the worker's file.
pub(crate) fn record_worker_failure_at(
    home: &Home,
    worker: &str,
    failure: Option<&str>,
    now: Timestamp,
) -> Result<(), StateError> {
    ensure_workers_migrated(home)?;
    super::refuse_symlink(&home.runner_workers_dir())?;
    let path = worker_path(home, worker);
    match failure {
        None => {
            super::refuse_symlink(&path)?;
            match fsx::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
                Err(source) => Err(StateError::Fs(FsError::Write { path, source })),
            }
        }
        Some(failure) => {
            let since = prior_since(&path, worker)?.unwrap_or(now);
            let record = WorkerRecord {
                worker: worker.to_owned(),
                failure: failure.to_owned(),
                since,
            };
            let bytes = serde_json::to_vec(&record).map_err(|error| {
                StateError::Fs(FsError::Write {
                    path: path.clone(),
                    source: std::io::Error::other(error),
                })
            })?;
            super::write_bytes(&path, &bytes)
        }
    }
}

/// Records a runner worker's current failure in its isolated state file,
/// keeping when it started; `None` removes the file. Returns `true` iff the
/// state change was persisted.
pub(crate) fn worker_state(home: &Home, worker: &str, failure: Option<&str>) -> bool {
    match record_worker_failure_at(home, worker, failure, Timestamp::now()) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(event = "runner_worker_state_unrecorded", worker, error = %error);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::worker_entry_stem;

    #[test]
    fn worker_entry_stem_accepts_only_non_dotfile_json_names() {
        assert_eq!(
            worker_entry_stem("0123456789abcdef.json"),
            Some("0123456789abcdef")
        );
        assert_eq!(worker_entry_stem(".tmp12345.json"), None);
        assert_eq!(worker_entry_stem(".hidden"), None);
        assert_eq!(worker_entry_stem("worker.lock"), None);
    }
}
