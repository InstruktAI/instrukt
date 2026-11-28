//! Hosts an adapter's source notifications in the existing runner.
use super::{HistoryError, extract};
use crate::runtime::{RuntimeRegistry, cold_text::SourceWake};
use instrukt_core::{
    config::{Current, Env},
    home::Home,
};
use std::{
    sync::mpsc::{self, SyncSender},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(crate) struct Worker {
    stop: SyncSender<()>,
    handle: Option<JoinHandle<()>>,
}

/// The worker's name in its failure file and `instrukt events status`.
const EXTRACTION: &str = "history-extraction";

/// The longest wait before T3's conversation source is acquired again.
const MOST_RETRY: Duration = Duration::from_secs(60);

impl Worker {
    /// Extraction from T3's conversation source. The source is acquired
    /// inside the worker's own thread; when it cannot be acquired, stops
    /// notifying or cannot be extracted from, the worker records why, waits
    /// a growing delay and acquires it again. Only a successful extraction
    /// clears what it recorded, an earlier process's record included. It
    /// ends only when stopped or when history is switched off, leaving no
    /// record.
    pub(super) fn start(
        home: &Home,
        env: &Env,
        current: Current,
        runtimes: RuntimeRegistry,
    ) -> Result<Self, HistoryError> {
        let home = home.clone();
        let env = env.clone();
        let (stop, stopping) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("history-extraction".into())
            .spawn(move || {
                let mut health = Health::default();
                let mut delay = Duration::from_secs(1);
                loop {
                    let error = match serve(
                        &home,
                        &env,
                        &current,
                        &runtimes,
                        &stopping,
                        &mut health,
                        &mut delay,
                    ) {
                        Ok(()) => {
                            crate::state::worker_state(&home, EXTRACTION, None);
                            return;
                        }
                        Err(error) => error,
                    };
                    health.failed(&home, &error);
                    if stopping.recv_timeout(delay).is_ok() {
                        crate::state::worker_state(&home, EXTRACTION, None);
                        return;
                    }
                    delay = (delay * 2).min(MOST_RETRY);
                }
            })
            .map_err(|source| HistoryError::Io {
                path: home_path(),
                source,
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    /// Whether the worker's thread has ended: stopped, history switched off,
    /// or unwound.
    pub(crate) fn ended(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

/// What the worker last recorded of itself: failing for a cause, or
/// healthy, which it has not shown yet when it starts.
#[derive(Default)]
struct Health {
    failing: Option<String>,
    shown_healthy: bool,
}

impl Health {
    fn failed(&mut self, home: &Home, error: &HistoryError) {
        let cause = error.to_string();
        self.shown_healthy = false;
        if self.failing.as_deref() != Some(cause.as_str()) {
            tracing::warn!(event = "history_worker_failing", worker = EXTRACTION, error = %error);
            if crate::state::worker_state(home, EXTRACTION, Some(&cause)) {
                self.failing = Some(cause);
            }
        }
    }

    fn extracted(&mut self, home: &Home) {
        if let Some(cause) = self.failing.take() {
            tracing::info!(event = "history_worker_recovered", worker = EXTRACTION, cause = %cause);
        }
        if !self.shown_healthy && crate::state::worker_state(home, EXTRACTION, None) {
            self.shown_healthy = true;
        }
    }
}

/// When history follows the source's requests to read again. Its read covers
/// the whole source, so it follows the first request after a change and then
/// only once the time since that change has doubled since the last request it
/// followed: the number of reads in a quiet period grows with the logarithm
/// of its length, and no request is ever the last one followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Following {
    /// When the source last changed.
    changed: Instant,
    /// How long after that change the last extraction for a request ended.
    followed: Option<Duration>,
}

impl Following {
    fn changed(now: Instant) -> Self {
        Self {
            changed: now,
            followed: None,
        }
    }
    /// Whether a request received at `now` is followed.
    fn follows(&self, now: Instant) -> bool {
        self.followed
            .is_none_or(|followed| now.duration_since(self.changed) >= followed * 2)
    }
    /// An extraction for a request ended at `now`.
    fn extracted(&mut self, now: Instant) {
        self.followed = Some(now.duration_since(self.changed));
    }
}

/// Acquires the source and extracts on every change, and on each request to
/// read again that it follows, until the worker stops
/// or history is switched off (`Ok`), or the source fails: it cannot be
/// acquired, stops notifying, or an extraction from it fails. A successful
/// extraction starts the next retry delay from its shortest.
fn serve(
    home: &Home,
    env: &Env,
    current: &Current,
    runtimes: &RuntimeRegistry,
    stopping: &mpsc::Receiver<()>,
    health: &mut Health,
    delay: &mut Duration,
) -> Result<(), HistoryError> {
    let runtime = current.get().global().runner_runtime()?;
    let adapter = runtimes
        .adapter(runtime.name(), env)
        .map_err(crate::sessions::SessionsError::from)?;
    let source = adapter
        .cold_text_source(home, env)
        .map_err(crate::sessions::SessionsError::from)?;
    let mut notifications = source.subscribe()?;
    let mut following = Following::changed(Instant::now());
    let mut requested = false;
    loop {
        if !current.get().history_enabled()? {
            return Ok(());
        }
        if !requested {
            following = Following::changed(Instant::now());
        }
        extract(home, env, runtimes)?;
        health.extracted(home);
        *delay = Duration::from_secs(1);
        if requested {
            following.extracted(Instant::now());
        }
        requested = loop {
            if stopping.try_recv().is_ok() {
                return Ok(());
            }
            match notifications.wait(Duration::from_millis(100))? {
                SourceWake::Changed => break false,
                SourceWake::ReadAgain if following.follows(Instant::now()) => break true,
                SourceWake::ReadAgain | SourceWake::Timeout => {}
            }
        };
    }
}
fn home_path() -> std::path::PathBuf {
    std::path::PathBuf::from("history worker")
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.stop.send(()) {
            tracing::debug!(event="history_worker_already_stopped",%error);
        }
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!(event = "history_worker_panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_request_after_a_source_change_is_followed() {
        let change = Instant::now();
        let following = Following::changed(change);
        assert!(following.follows(change + Duration::from_millis(250)));
    }

    #[test]
    fn a_later_request_is_followed_once_the_time_since_the_change_has_doubled() {
        let change = Instant::now();
        let mut following = Following::changed(change);
        following.extracted(change + Duration::from_secs(1));
        assert!(!following.follows(change + Duration::from_millis(1999)));
        assert!(following.follows(change + Duration::from_secs(2)));
    }
}
