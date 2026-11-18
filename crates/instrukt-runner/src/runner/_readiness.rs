//! The runtime's readiness, kept by the runner off every start path: one
//! refresh when the runner starts and one on each of its wakes, on a thread
//! of its own. A start reads the last snapshot a refresh published and never
//! waits on one in progress. The `readiness` row in `runner_workers` lives
//! as long as the worker: written by a failed refresh, removed by the next
//! success and when the worker ends, a stop by SIGINT or SIGTERM included,
//! which also revokes the kept token.

use std::sync::Arc;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;

use instrukt_core::home::Home;
use jiff::Timestamp;

use crate::runtime::Readiness;

/// The worker's name in `runner_workers`.
const READINESS: &str = "readiness";

/// Refreshes `readiness` at `now`. A refresh that fails keeps the last
/// snapshot published and is recorded in `runner_workers` until a refresh
/// succeeds; a successful one applies the snapshot's usage windows once.
/// `recorded` says whether this process left a failure row (or found one
/// it must clear on its first success).
pub(crate) fn refresh(
    home: &Home,
    readiness: &mut dyn Readiness,
    clock: &dyn Fn() -> Timestamp,
    recorded: &mut bool,
) {
    match readiness.refresh(home, clock) {
        Ok(observed) => {
            if let Err(error) = crate::agents::apply_usage(home, &observed) {
                tracing::warn!(event = "readiness_usage_unapplied", %error);
            }
            if std::mem::take(recorded) {
                crate::state::worker_state(home, READINESS, None);
            }
        }
        Err(error) => {
            tracing::warn!(event = "readiness_refresh_failed", %error);
            crate::state::worker_state(home, READINESS, Some(&error.to_string()));
            *recorded = true;
        }
    }
}

/// Ends the `readiness` worker's row: its worker is gone.
pub(crate) fn ended(home: &Home) {
    crate::state::worker_state(home, READINESS, None);
}

/// The runner's readiness thread.
pub(crate) struct Worker {
    wake: Option<SyncSender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts the thread: it refreshes at once, then on each [`Worker::wake`].
    /// It ends when the worker is dropped, revoking its token.
    pub(crate) fn start(
        home: &Home,
        mut readiness: Box<dyn Readiness>,
        clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    ) -> std::io::Result<Self> {
        let home = home.clone();
        // A stop by SIGINT or SIGTERM revokes the kept token and ends the
        // worker's row before the process ends.
        let stopper = readiness.stopper();
        let stopping_home = home.clone();
        instrukt_core::process::on_stop(Box::new(move || {
            stopper();
            ended(&stopping_home);
        }))?;
        let (wake, wakes) = mpsc::sync_channel::<()>(1);
        let handle = std::thread::Builder::new()
            .name("readiness".into())
            .spawn(move || {
                // An earlier process's failure row is cleared by the first
                // refresh that succeeds.
                let mut recorded = true;
                refresh(&home, readiness.as_mut(), &*clock, &mut recorded);
                while wakes.recv().is_ok() {
                    refresh(&home, readiness.as_mut(), &*clock, &mut recorded);
                }
                drop(readiness);
                ended(&home);
            })?;
        Ok(Self {
            wake: Some(wake),
            handle: Some(handle),
        })
    }

    /// Asks for a refresh; one already asked for and not begun covers it.
    pub(crate) fn wake(&self) {
        if let Some(wake) = &self.wake
            && let Err(TrySendError::Disconnected(())) = wake.try_send(())
        {
            tracing::warn!(event = "readiness_worker_gone");
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.wake.take());
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!(event = "readiness_worker_panicked");
        }
    }
}
