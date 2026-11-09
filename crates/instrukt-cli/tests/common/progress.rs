//! Bounded filesystem synchronization, with the watcher armed before the check.
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

pub struct Progress {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<Event>>,
    deadline: Instant,
}

impl Progress {
    pub fn new(root: &Path, timeout: Duration) -> Self {
        let (send, events) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if !event.as_ref().is_ok_and(|event| event.kind.is_access()) {
                // The watcher can deliver one final event during teardown.
                let _ = send.send(event);
            }
        })
        .unwrap();
        std::fs::create_dir_all(root).unwrap();
        watcher.watch(root, RecursiveMode::NonRecursive).unwrap();
        Self {
            watcher,
            events,
            deadline: Instant::now() + timeout,
        }
    }

    /// Directory events cover creation; a file watch also covers log appends.
    /// Arm it before checking readiness, including after the file first appears.
    pub fn watch_file(&mut self, file: &Path) {
        if !file.try_exists().unwrap() {
            return; // The directory watch observes the file's creation.
        }
        if let Err(error) = self.watcher.watch(file, RecursiveMode::NonRecursive) {
            // A file removed during registration is observed through its parent.
            assert!(
                !file.try_exists().unwrap(),
                "cannot watch the test's log file: {error}"
            );
        }
    }

    pub fn wait(&self) {
        if let Err(error) = self
            .events
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
            .expect("no filesystem progress before the test deadline")
        {
            // kqueue can discover a temporary file after an atomic rename has
            // removed it. That deletion is progress too: recheck the condition.
            assert!(
                matches!(error.kind, notify::ErrorKind::Io(ref error)
                if error.kind() == std::io::ErrorKind::NotFound),
                "test filesystem watcher failed: {error}"
            );
        }
    }
}
