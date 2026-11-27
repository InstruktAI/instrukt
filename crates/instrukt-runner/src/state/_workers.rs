//! Per-worker failure files under `state/runner-workers/<id>.json`, outside
//! `instrukt.db` and `state/writer.lock` (`project/instrukt/spec/domain/runner`,
//! "Runner background state").

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use instrukt_core::fsx::{self, FsError};
use instrukt_core::home::Home;
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

fn unreadable_worker(stem: &str, error: String, since: Option<Timestamp>) -> WorkerFailure {
    WorkerFailure {
        worker: format!("runner-worker:{stem}"),
        failure: format!("runner worker failure state cannot be read: {error}"),
        since,
    }
}

/// Reads all current runner worker failure files without locking or creating
/// the directory. A missing directory is healthy (empty); unreadable or
/// symlink `.json` files are surfaced as worker failures rather than masked as
/// healthy.
pub(crate) fn read_worker_failures(home: &Home) -> Result<Vec<WorkerFailure>, StateError> {
    let dir = home.runner_workers_dir();
    super::refuse_symlink(&dir)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(StateError::Read { path: dir, source }),
    };
    let mut failures = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(source) => return Err(StateError::Read { path: dir, source }),
        };
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Some(stem) = name.strip_suffix(".json") else {
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
                Err(error) => failures.push(unreadable_worker(stem, error.to_string(), mtime)),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => failures.push(unreadable_worker(stem, error.to_string(), mtime)),
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
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use instrukt_core::fsx::{self, LockGuard};
    use instrukt_core::home::Home;
    use jiff::Timestamp;
    use tracing::field::{Field, Visit};

    use super::{
        WorkerRecord, read_worker_failures, record_worker_failure_at, worker_path, worker_state,
    };
    use crate::state::StateError;

    fn home() -> (tempfile::TempDir, Home) {
        let dir = fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        (dir, home)
    }

    fn seconds(n: i64) -> Timestamp {
        Timestamp::from_second(1_900_000_000 + n).unwrap()
    }

    struct CaptureEvents {
        events: Arc<Mutex<Vec<String>>>,
    }

    struct EventField(Option<String>);

    impl Visit for EventField {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "event" {
                self.0 = Some(value.to_owned());
            }
        }
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "event" {
                self.0 = Some(format!("{value:?}"));
            }
        }
    }

    impl tracing::Subscriber for CaptureEvents {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut visitor = EventField(None);
            event.record(&mut visitor);
            if let Some(name) = visitor.0 {
                self.events.lock().unwrap().push(name);
            }
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    #[test]
    fn failure_updates_preserve_since_and_recovery_removes_the_file() {
        let (_dir, home) = home();
        let worker = "history-extraction";
        record_worker_failure_at(&home, worker, Some("first cause"), seconds(0)).unwrap();
        record_worker_failure_at(&home, worker, Some("second cause"), seconds(60)).unwrap();
        let path = worker_path(&home, worker);
        let stored: WorkerRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored.worker, worker);
        assert_eq!(stored.failure, "second cause");
        assert_eq!(stored.since, seconds(0));

        record_worker_failure_at(&home, worker, None, seconds(120)).unwrap();
        assert!(!path.exists());
        // Removing an absent file succeeds idempotently.
        record_worker_failure_at(&home, worker, None, seconds(180)).unwrap();
    }

    #[test]
    fn writes_and_clears_succeed_while_writer_lock_is_held() {
        let (_dir, home) = home();
        let _writer = LockGuard::acquire(&home.writer_lock_file(), Duration::from_secs(1)).unwrap();
        assert!(worker_state(
            &home,
            "readiness",
            Some("provider unavailable")
        ));
        let path = worker_path(&home, "readiness");
        assert!(path.exists());
        assert!(!home.state_db_file().exists());
        assert!(worker_state(&home, "readiness", None));
        assert!(!path.exists());
    }

    #[test]
    fn symlinks_at_worker_file_or_directory_are_refused() {
        let (_dir, home) = home();
        let outside = home.state_dir().join("outside.json");
        fsx::atomic_write(&outside, b"keep").unwrap();
        let path = worker_path(&home, "history-search");
        fsx::create_dir_all(path.parent().unwrap()).unwrap();
        fsx::symlink(&outside, &path).unwrap();

        let write_err = record_worker_failure_at(&home, "history-search", Some("boom"), seconds(0));
        assert!(matches!(write_err, Err(StateError::Symlink { .. })));
        let clear_err = record_worker_failure_at(&home, "history-search", None, seconds(0));
        assert!(matches!(clear_err, Err(StateError::Symlink { .. })));
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    }

    #[test]
    fn overwriting_malformed_worker_file_warns_and_replaces_since() {
        let (_dir, home) = home();
        let worker = "runtime-start:t3:demo";
        let path = worker_path(&home, worker);
        fsx::atomic_write(&path, b"{not json").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = CaptureEvents {
            events: Arc::clone(&events),
        };
        tracing::subscriber::with_default(subscriber, || {
            record_worker_failure_at(&home, worker, Some("fresh cause"), seconds(42)).unwrap();
        });
        let stored: WorkerRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored.failure, "fresh cause");
        assert_eq!(stored.since, seconds(42));
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e == "runner_worker_state_unread")
        );
    }

    #[test]
    fn reading_worker_failures_ignores_dotfiles_and_surfaces_corrupt_and_symlink_files() {
        let (_dir, home) = home();
        assert!(read_worker_failures(&home).unwrap().is_empty());
        assert!(
            !home.runner_workers_dir().exists(),
            "reading must not create runner-workers/"
        );

        let _writer = LockGuard::acquire(&home.writer_lock_file(), Duration::from_secs(1)).unwrap();
        record_worker_failure_at(&home, "history-extraction", Some("bad shape"), seconds(10))
            .unwrap();
        // Atomic-write temp dotfiles are ignored.
        fsx::atomic_write(
            &home.runner_workers_dir().join(".tmp12345.json"),
            b"partial",
        )
        .unwrap();
        // Malformed JSON is surfaced as a worker failure.
        fsx::atomic_write(&home.runner_workers_dir().join("corrupt.json"), b"{bad").unwrap();
        // Symlinked JSON is surfaced as a worker failure rather than followed.
        let outside = home.state_dir().join("outside-read.json");
        fsx::atomic_write(
            &outside,
            br#"{"worker":"spoof","failure":"nope","since":"2026-11-27T00:00:00Z"}"#,
        )
        .unwrap();
        fsx::symlink(&outside, home.runner_workers_dir().join("linked.json")).unwrap();

        let failures = read_worker_failures(&home).unwrap();
        let workers: Vec<&str> = failures.iter().map(|entry| entry.worker.as_str()).collect();
        assert_eq!(
            workers,
            vec![
                "history-extraction",
                "runner-worker:corrupt",
                "runner-worker:linked",
            ]
        );
        assert_eq!(failures[0].since, Some(seconds(10)));
        assert!(!failures[1].failure.is_empty());
        assert!(failures[1].since.is_some());
        assert!(!failures[2].failure.is_empty());
        let json = failures[0].clone().into_json();
        assert_eq!(json["worker"], "history-extraction");
        assert_eq!(json["failure"], "bad shape");
        assert_eq!(json["since"], seconds(10).to_string());
    }
}
