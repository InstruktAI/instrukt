//! Native database notifications drive the runner's company conversation relay.
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use instrukt_core::{
    config::{Current, Env},
    home::Home,
};
use tokio::sync::{Mutex, Notify, mpsc, watch};

use crate::{interfaces::CompanyRelayPass, runtime::conversations::SourceWake};

/// The longest wait before T3's conversation source is acquired again.
const MOST_RETRY: Duration = Duration::from_secs(60);

/// How often the relay checks for retained or out-of-process company work while idle.
const RETRY_CADENCE: Duration = Duration::from_secs(2);

/// How long the blocking notification bridge waits per poll before checking shutdown.
const NOTIFICATION_POLL: Duration = Duration::from_millis(200);

/// The company destination served by the relay and its intake wake-ups.
#[derive(Clone)]
pub(crate) struct Destinations {
    pub(crate) company: Arc<dyn crate::interfaces::CompanyPorts>,
    pub(crate) arrivals: Arc<Mutex<mpsc::Receiver<()>>>,
}

/// The long-lived async relay task serving company conversations from T3's
/// conversation source. Dropping it signals shutdown cleanly.
pub(crate) struct Worker {
    shutdown: watch::Sender<bool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

/// Signals the blocking notification bridge to exit when dropped.
struct BridgeStop(Arc<AtomicBool>);
impl Drop for BridgeStop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Lossless monotonic coalescer for conversation notifications so a stronger
/// `CatchUp` (`SourceWake::Changed`) or `Failed` event can never be dropped
/// behind queued `Retry` (`SourceWake::ReadAgain`) notifications.
#[derive(Default)]
enum PendingWake {
    #[default]
    None,
    Retry,
    CatchUp,
    Failed(String),
}

#[derive(Default)]
struct WakeSlot {
    state: StdMutex<PendingWake>,
    notify: Notify,
}

impl WakeSlot {
    fn record_wake(&self, wake: SourceWake) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match (&*guard, wake) {
            (PendingWake::Failed(_), _) | (_, SourceWake::Timeout) => return,
            (_, SourceWake::Changed) => *guard = PendingWake::CatchUp,
            (PendingWake::None, SourceWake::ReadAgain) => *guard = PendingWake::Retry,
            (PendingWake::Retry | PendingWake::CatchUp, SourceWake::ReadAgain) => return,
        }
        drop(guard);
        self.notify.notify_one();
    }

    fn record_error(&self, error: String) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(&*guard, PendingWake::Failed(_)) {
            *guard = PendingWake::Failed(error);
        }
        drop(guard);
        self.notify.notify_one();
    }

    async fn next(&self) -> Result<CompanyRelayPass, String> {
        loop {
            let notified = self.notify.notified();
            {
                let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                match std::mem::take(&mut *guard) {
                    PendingWake::None => {}
                    PendingWake::Retry => return Ok(CompanyRelayPass::Retry),
                    PendingWake::CatchUp => return Ok(CompanyRelayPass::CatchUp),
                    PendingWake::Failed(error) => return Err(error),
                }
            }
            notified.await;
        }
    }
}

impl Worker {
    /// Spawns the company conversation relay on the shared Tokio runtime.
    /// When T3's conversation source cannot be acquired or stops notifying, it
    /// records the failure on `destinations.company` and retries with
    /// exponential backoff up to [`MOST_RETRY`], cancellable by shutdown.
    pub(crate) fn start(
        home: &Home,
        env: &Env,
        current: Current,
        destinations: Destinations,
    ) -> std::io::Result<Self> {
        let rt = instrukt_outside::runtime::runtime()?;
        let home = home.clone();
        let env = env.clone();
        let (shutdown, mut stopping) = watch::channel(false);
        let handle = rt.spawn(async move {
            let mut failing: Option<String> = None;
            let mut delay = Duration::from_secs(1);
            loop {
                match serve(
                    &home,
                    &env,
                    &current,
                    &destinations,
                    &mut stopping,
                    &mut failing,
                    &mut delay,
                )
                .await
                {
                    Ok(()) => return,
                    Err(cause) => {
                        if failing.as_deref() != Some(cause.as_str()) {
                            tracing::error!(event = "conversation_source_failed", error = %cause);
                            tokio::task::block_in_place(|| {
                                destinations.company.relay_state(&home, Some(&cause));
                            });
                            failing = Some(cause);
                        }
                        tokio::select! {
                            changed = stopping.changed() => {
                                if changed.is_err() || *stopping.borrow() {
                                    return;
                                }
                            }
                            () = tokio::time::sleep(delay) => {}
                        }
                        delay = (delay * 2).min(MOST_RETRY);
                    }
                }
            }
        });
        Ok(Self {
            shutdown,
            handle: Some(handle),
        })
    }

    /// Whether the relay task is still alive.
    pub(crate) fn is_running(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

    /// Resolves as soon as the relay task exits (normal return or unwind).
    /// Cancellation-safe: dropping the returned future keeps the task running.
    pub(crate) async fn exited(&mut self) {
        if let Some(handle) = self.handle.as_mut() {
            let _ = handle.await;
            self.handle = None;
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Acquires T3's conversation source and serves company conversations until
/// shutdown (`Ok`) or a source failure (`Err`). A successful acquisition
/// resets the retry delay to its shortest interval.
async fn serve(
    home: &Home,
    env: &Env,
    current: &Current,
    destinations: &Destinations,
    stopping: &mut watch::Receiver<bool>,
    failing: &mut Option<String>,
    delay: &mut Duration,
) -> Result<(), String> {
    if *stopping.borrow() {
        return Ok(());
    }
    let (adapter, source, notifications) = tokio::task::block_in_place(|| {
        let runtime = current
            .get()
            .global()
            .runner_runtime()
            .map_err(|error| error.to_string())?;
        let adapter = crate::runtime::adapter(runtime);
        let source = adapter
            .conversation_source(home, env)
            .map_err(|error| error.to_string())?;
        let notifications = source.subscribe().map_err(|error| error.to_string())?;
        Ok::<_, String>((adapter, source, notifications))
    })?;
    *delay = Duration::from_secs(1);
    if let Some(cause) = failing.take() {
        tracing::info!(event = "conversation_relay_recovered", cause = %cause);
    }
    // The source is acquired: clear any previously recorded relay failure.
    tokio::task::block_in_place(|| {
        destinations.company.relay_state(home, None);
    });

    // Bridge blocking `ConversationNotifications::wait` onto a monotonic
    // coalescing slot so `Changed` can never be dropped behind `ReadAgain`.
    let wakes = Arc::new(WakeSlot::default());
    let bridge_wakes = Arc::clone(&wakes);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let _bridge_stop = BridgeStop(Arc::clone(&stop_flag));
    let bridge_flag = Arc::clone(&stop_flag);
    let _bridge = tokio::task::spawn_blocking(move || {
        let mut notifications = notifications;
        while !bridge_flag.load(Ordering::Acquire) {
            match notifications.wait(NOTIFICATION_POLL) {
                Ok(SourceWake::Timeout) => {}
                Ok(wake) => bridge_wakes.record_wake(wake),
                Err(error) => {
                    bridge_wakes.record_error(error.to_string());
                    return;
                }
            }
        }
    });

    let mut arrivals = destinations.arrivals.lock().await;
    let mut pass = CompanyRelayPass::CatchUp;
    loop {
        if *stopping.borrow() {
            return Ok(());
        }
        let admitted = current.get();
        let retry = tokio::task::block_in_place(|| {
            if let Err(error) =
                destinations
                    .company
                    .relay(home, &admitted, source.as_ref(), adapter.as_ref(), pass)
            {
                tracing::warn!(event = "conversation_relay_failed", error = %error);
            }
            if matches!(pass, CompanyRelayPass::CatchUp) {
                tracing::debug!(event = "conversation_relay_source_processed");
            }
            // Retained connector work that is not yet done is retried within a
            // bound; an out-of-process `instrukt company reply` can also
            // enqueue pending answers while idle, so check read-only SQLite on
            // the same 2-second cadence without touching the native source.
            destinations.company.outstanding(home).unwrap_or(true)
        });

        let mut retry_deadline = tokio::time::Instant::now() + RETRY_CADENCE;
        pass = loop {
            tokio::select! {
                changed = stopping.changed() => {
                    if changed.is_err() || *stopping.borrow() {
                        return Ok(());
                    }
                }
                arrived = arrivals.recv() => {
                    match arrived {
                        Some(()) => break CompanyRelayPass::Retry,
                        None => return Ok(()),
                    }
                }
                next = wakes.next() => {
                    break next?;
                }
                () = tokio::time::sleep_until(retry_deadline) => {
                    let has_work = retry
                        || tokio::task::block_in_place(|| {
                            destinations.company.outstanding(home).unwrap_or(false)
                        });
                    if has_work {
                        break CompanyRelayPass::Retry;
                    }
                    retry_deadline = tokio::time::Instant::now() + RETRY_CADENCE;
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_slot_preserves_changed_over_queued_read_again() {
        let slot = WakeSlot::default();
        slot.record_wake(SourceWake::ReadAgain);
        slot.record_wake(SourceWake::ReadAgain);
        slot.record_wake(SourceWake::Changed);
        slot.record_wake(SourceWake::ReadAgain);
        let pass = instrukt_outside::runtime::block_on(slot.next())
            .ok()
            .and_then(Result::ok);
        assert!(matches!(pass, Some(CompanyRelayPass::CatchUp)));
    }
}
