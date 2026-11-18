//! Hosts an adapter's source notifications in the existing runner.
use super::{HistoryError, extract};
use instrukt_core::{
    config::{Current, Env},
    home::Home,
};
use std::{
    sync::mpsc::{self, SyncSender},
    thread::JoinHandle,
    time::Duration,
};

pub(crate) struct Worker {
    stop: SyncSender<()>,
    handle: Option<JoinHandle<()>>,
}

/// The worker's name in `runner_workers`.
const EXTRACTION: &str = "history-extraction";

/// The longest wait before T3's conversation source is acquired again.
const MOST_RETRY: Duration = Duration::from_secs(60);

impl Worker {
    /// Extraction from T3's conversation source. The source is acquired
    /// inside the worker's own thread; when it cannot be acquired or stops
    /// notifying, the worker records why, waits a growing delay and acquires
    /// it again. It ends only when stopped or when history is switched off.
    pub(super) fn start(home: &Home, env: &Env, current: Current) -> Result<Self, HistoryError> {
        let home = home.clone();
        let env = env.clone();
        let (stop, stopping) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("history-extraction".into())
            .spawn(move || {
                let mut failing: Option<String> = None;
                let mut delay = Duration::from_secs(1);
                loop {
                    let error = match serve(&home, &env, &current, &stopping, &mut failing, &mut delay) {
                        Ok(()) => return,
                        Err(error) => error,
                    };
                    let cause = error.to_string();
                    if failing.as_deref() != Some(cause.as_str()) {
                        tracing::warn!(event = "history_worker_failing", worker = EXTRACTION, error = %error);
                        crate::state::worker_state(&home, EXTRACTION, Some(&cause));
                        failing = Some(cause);
                    }
                    if stopping.recv_timeout(delay).is_ok() {
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

/// Acquires the source and extracts on every change until the worker stops
/// or history is switched off (`Ok`), or the source fails. A successful
/// acquisition starts the next retry delay from its shortest.
fn serve(
    home: &Home,
    env: &Env,
    current: &Current,
    stopping: &mpsc::Receiver<()>,
    failing: &mut Option<String>,
    delay: &mut Duration,
) -> Result<(), HistoryError> {
    let adapter = crate::runtime::adapter(current.get().global().runner_runtime()?);
    let source = adapter.conversation_source(home, env)?;
    let mut notifications = source.subscribe()?;
    *delay = Duration::from_secs(1);
    if let Some(cause) = failing.take() {
        tracing::info!(event = "history_worker_recovered", worker = EXTRACTION, cause = %cause);
        crate::state::worker_state(home, EXTRACTION, None);
    }
    loop {
        if !current.get().history_enabled()? {
            return Ok(());
        }
        if let Err(error) = extract(home, env) {
            tracing::error!(event="history_extract_failed",error=%error);
        }
        loop {
            if stopping.try_recv().is_ok() {
                return Ok(());
            }
            if notifications.wait(Duration::from_millis(100))? {
                break;
            }
        }
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
