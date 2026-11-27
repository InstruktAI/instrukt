//! Where a responder records its current outside failure. The computer
//! fleet's evaluation responder and the tick's fleet step keep theirs in
//! `state/computer-fleet.json` (`project/instrukt/spec/domain/runner`,
//! "Fleet query and membership"), a file of its own outside the state
//! database, written whole beside its previous version and renamed over
//! it, so readers take no lock. History's responder keeps its row in
//! `runner_workers`.

use std::io::ErrorKind;
use std::path::PathBuf;
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

/// A worker's row in `runner_workers`. The row's own write failure is
/// logged where it is written, and the responder goes on as if recorded.
pub(crate) struct WorkerRow {
    pub home: Home,
    pub worker: &'static str,
}

impl FailureRecorder for WorkerRow {
    fn record(&mut self, failure: Option<&str>) -> bool {
        crate::state::worker_state(&self.home, self.worker, failure)
    }
}

/// The file's document.
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

/// The computer fleet's failure file. Its note changes only when a write or
/// removal succeeded, so a failed one is tried again with the next outcome
/// and never taken as recorded health.
pub(crate) struct FailureFile {
    path: PathBuf,
    clock: Clock,
    note: Note,
}

impl FailureFile {
    pub(crate) fn new(home: &Home, clock: Clock) -> Self {
        Self {
            path: path(home),
            clock,
            note: Note::Unknown,
        }
    }

    /// When the failure already in the file began; `None` when there is
    /// none, or it cannot be read.
    fn since(&self) -> Result<Option<Timestamp>, String> {
        crate::state::refuse_symlink(&self.path).map_err(|error| error.to_string())?;
        match std::fs::read(&self.path) {
            Ok(bytes) => match serde_json::from_slice::<Recorded>(&bytes) {
                Ok(recorded) => Ok(Some(recorded.since)),
                Err(error) => {
                    tracing::warn!(event = "computer_fleet_failure_unread", path = %self.path.display(), %error);
                    Ok(None)
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => {
                tracing::warn!(event = "computer_fleet_failure_unread", path = %self.path.display(), %error);
                Ok(None)
            }
        }
    }
}

impl FailureRecorder for FailureFile {
    fn record(&mut self, failure: Option<&str>) -> bool {
        let written = match failure {
            None if self.note == Note::Healthy => return true,
            Some(failure) if self.note == Note::Failing(failure.to_owned()) => return true,
            None => crate::state::refuse_symlink(&self.path)
                .map_err(|error| error.to_string())
                .and_then(|()| match instrukt_core::fsx::remove_file(&self.path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                }),
            Some(failure) => self.since().and_then(|existing| {
                let recorded = Recorded {
                    failure: failure.to_owned(),
                    since: existing.unwrap_or_else(|| (self.clock)()),
                };
                serde_json::to_vec(&recorded)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        crate::state::write_bytes(&self.path, &bytes)
                            .map_err(|error| error.to_string())
                    })
            }),
        };
        match written {
            Ok(()) => {
                self.note =
                    failure.map_or(Note::Healthy, |failure| Note::Failing(failure.to_owned()));
                true
            }
            Err(error) => {
                tracing::warn!(event = "computer_fleet_failure_unrecorded", path = %self.path.display(), %error);
                false
            }
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
}
