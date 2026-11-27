//! Where a responder records its current outside failure. The computer
//! fleet's evaluation responder and the tick's fleet step keep theirs in
//! `state/computer-fleet.json` (`project/instrukt/spec/domain/runner`,
//! "Fleet query and membership"), and other runner workers keep theirs in
//! `state/runner-workers/<id>.json` (`project/instrukt/spec/domain/runner`,
//! "Runner background state") — each written whole beside its previous
//! version and renamed over it, outside `instrukt.db` and `state/writer.lock`,
//! so readers take no lock.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use instrukt_core::home::Home;

/// The name the computer fleet's failure is shown under, among the runner's
/// workers.
pub(crate) const WORKER: &str = "computer-fleet";

/// The operator clock the runner reads.
pub(crate) type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Where a responder records its current outside failure.
pub(crate) trait FailureRecorder: Send + 'static {
    /// Records `failure`, `None` for none; `true` once it is recorded, so the
    /// responder takes it as shown only then.
    fn record(&mut self, failure: Option<&str>) -> bool;
}

/// The file's document for `state/computer-fleet.json`.
#[derive(Serialize, Deserialize)]
struct Recorded {
    failure: String,
    since: Timestamp,
}

/// What a failure file's writer knows it has recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Note {
    Unknown,
    Healthy,
    Failing(String),
}

enum Target {
    ComputerFleet(PathBuf),
    Worker { home: Home, worker: &'static str },
}

/// An isolated failure file recorder. Its note changes only when a write or
/// removal succeeded, so a failed one is tried again with the next outcome
/// and never taken as recorded health.
pub(crate) struct FailureFile {
    target: Target,
    clock: Clock,
    note: Note,
}

impl FailureFile {
    pub(crate) fn new(home: &Home, clock: Clock) -> Self {
        Self {
            target: Target::ComputerFleet(path(home)),
            clock,
            note: Note::Unknown,
        }
    }

    /// A failure file for a runner worker under `state/runner-workers/`.
    pub(crate) fn worker(home: &Home, worker: &'static str, clock: Clock) -> Self {
        Self {
            target: Target::Worker {
                home: home.clone(),
                worker,
            },
            clock,
            note: Note::Unknown,
        }
    }

    /// When the failure already in `path` began; `None` when there is none,
    /// or it cannot be read.
    fn since(path: &Path) -> Result<Option<Timestamp>, String> {
        crate::state::refuse_symlink(path).map_err(|error| error.to_string())?;
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Recorded>(&bytes) {
                Ok(recorded) => Ok(Some(recorded.since)),
                Err(error) => {
                    tracing::warn!(event = "computer_fleet_failure_unread", path = %path.display(), %error);
                    Ok(None)
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => {
                tracing::warn!(event = "computer_fleet_failure_unread", path = %path.display(), %error);
                Ok(None)
            }
        }
    }
}

impl FailureRecorder for FailureFile {
    fn record(&mut self, failure: Option<&str>) -> bool {
        match failure {
            None if self.note == Note::Healthy => return true,
            Some(failure) if self.note == Note::Failing(failure.to_owned()) => return true,
            _ => {}
        }
        let now = (self.clock)();
        let written = match &self.target {
            Target::ComputerFleet(path) => match failure {
                None => crate::state::refuse_symlink(path)
                    .map_err(|error| error.to_string())
                    .and_then(|()| match instrukt_core::fsx::remove_file(path) {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(error.to_string()),
                    }),
                Some(failure) => Self::since(path).and_then(|existing| {
                    let recorded = Recorded {
                        failure: failure.to_owned(),
                        since: existing.unwrap_or(now),
                    };
                    serde_json::to_vec(&recorded)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            crate::state::write_bytes(path, &bytes)
                                .map_err(|error| error.to_string())
                        })
                }),
            }
            .map_err(|error| {
                tracing::warn!(event = "computer_fleet_failure_unrecorded", path = %path.display(), %error);
            }),
            Target::Worker { home, worker } => {
                crate::state::record_worker_failure_at(home, worker, failure, now).map_err(
                    |error| {
                        tracing::error!(event = "runner_worker_state_unrecorded", worker = %worker, error = %error);
                    },
                )
            }
        };
        match written {
            Ok(()) => {
                self.note =
                    failure.map_or(Note::Healthy, |failure| Note::Failing(failure.to_owned()));
                true
            }
            Err(()) => false,
        }
    }
}

fn path(home: &Home) -> PathBuf {
    home.computer_fleet_file()
}

/// The computer fleet's failure as a row of the runner's workers, for
/// `instrukt events status`: none while the file is absent, and a file that
/// cannot be read or parsed shown as that worker's failure, never as
/// health. The read takes no lock.
pub(crate) fn status(home: &Home) -> Option<serde_json::Value> {
    let path = path(home);
    let unreadable = |error: String| {
        let since = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| Timestamp::try_from(modified).ok())
            .map(|at| at.to_string());
        serde_json::json!({
            "worker": WORKER,
            "failure": format!("the computer fleet's failure state cannot be read: {error}"),
            "since": since,
        })
    };
    if let Err(error) = crate::state::refuse_symlink(&path) {
        return Some(unreadable(error.to_string()));
    }
    match std::fs::read(&path) {
        Ok(bytes) => Some(match serde_json::from_slice::<Recorded>(&bytes) {
            Ok(recorded) => serde_json::json!({
                "worker": WORKER,
                "failure": recorded.failure,
                "since": recorded.since.to_string(),
            }),
            Err(error) => unreadable(error.to_string()),
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => Some(unreadable(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use jiff::Timestamp;

    use instrukt_core::home::Home;

    use super::{Clock, FailureFile, FailureRecorder, WORKER, path, status};

    fn home() -> (tempfile::TempDir, Home) {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        (dir, home)
    }

    /// A clock the test moves.
    fn clock(at: &Arc<Mutex<Timestamp>>) -> Clock {
        let at = Arc::clone(at);
        Arc::new(move || *at.lock().unwrap())
    }

    fn seconds(n: i64) -> Timestamp {
        Timestamp::from_second(1_900_000_000 + n).unwrap()
    }

    fn since(home: &Home) -> Timestamp {
        status(home).unwrap()["since"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn a_changed_failure_keeps_when_the_failure_began() {
        let (_dir, home) = home();
        let now = Arc::new(Mutex::new(seconds(0)));
        let mut file = FailureFile::new(&home, clock(&now));
        assert!(file.record(Some("first cause")));
        *now.lock().unwrap() = seconds(60);
        assert!(file.record(Some("second cause")));
        assert_eq!(since(&home), seconds(0));
        // A later writer, as a later tick is, keeps it too.
        *now.lock().unwrap() = seconds(120);
        assert!(FailureFile::new(&home, clock(&now)).record(Some("third cause")));
        assert_eq!(since(&home), seconds(0));
    }

    #[test]
    fn success_removes_the_file() {
        let (_dir, home) = home();
        let now = Arc::new(Mutex::new(seconds(0)));
        let mut file = FailureFile::new(&home, clock(&now));
        assert!(file.record(Some("cause")));
        assert!(file.record(None));
        assert!(!path(&home).exists());
        assert!(status(&home).is_none());
        // Removing a file that is not there is a success.
        assert!(FailureFile::new(&home, clock(&now)).record(None));
    }

    #[test]
    fn an_unreadable_file_is_shown_as_the_workers_failure() {
        let (_dir, home) = home();
        instrukt_core::fsx::atomic_write(&path(&home), b"not json").unwrap();
        let row = status(&home).unwrap();
        assert_eq!(row["worker"], WORKER);
        assert!(!row["failure"].as_str().unwrap().is_empty());
    }

    #[test]
    fn an_absent_file_shows_no_worker() {
        let (_dir, home) = home();
        assert!(status(&home).is_none());
    }

    #[test]
    fn worker_failure_file_preserves_since_and_clears_under_held_writer_lock() {
        let (_dir, home) = home();
        let _writer = instrukt_core::fsx::LockGuard::acquire(
            &home.writer_lock_file(),
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        let now = Arc::new(Mutex::new(seconds(10)));
        let mut recorder = FailureFile::worker(&home, "history-search", clock(&now));
        assert!(recorder.record(Some("redis down")));
        *now.lock().unwrap() = seconds(70);
        assert!(recorder.record(Some("still down")));
        let entries = crate::state::read_worker_failures(&home).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].worker, "history-search");
        assert_eq!(entries[0].failure, "still down");
        assert_eq!(entries[0].since, Some(seconds(10)));

        assert!(recorder.record(None));
        assert!(
            crate::state::read_worker_failures(&home)
                .unwrap()
                .is_empty()
        );
    }
}
