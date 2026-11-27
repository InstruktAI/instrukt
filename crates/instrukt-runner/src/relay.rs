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
use instrukt_outside::{
    Attempted, Extent, Gate, Key, Reason, Report, Settings, Tried, Unavailable,
};
use tokio::sync::{Mutex, Notify, mpsc, watch};

use crate::{interfaces::CompanyRelayPass, runtime::conversations::SourceWake};

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
    /// Source acquisition and reader failures are judged by `instrukt-outside::Gate`,
    /// which owns backoff pacing and outage reporting while projecting the
    /// current failure state onto `destinations.company`.
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
            let gate = Gate::new(
                Arc::new(jiff::Timestamp::now),
                Arc::new(std::time::Instant::now),
                Report::log(),
            );
            let key = Key::new("t3", ["conversation-source"]).item("relay");
            let settings = Settings::default();
            loop {
                if *stopping.borrow() {
                    return;
                }
                let acquired = gate
                    .attempt_async(&key, &settings, || async {
                        match tokio::task::block_in_place(|| {
                            let runtime = current
                                .get()
                                .global()
                                .runner_runtime()
                                .map_err(|error| error.to_string())?;
                            let adapter = crate::runtime::adapter(runtime);
                            let source = adapter
                                .conversation_source(&home, &env)
                                .map_err(|error| error.to_string())?;
                            let notifications =
                                source.subscribe().map_err(|error| error.to_string())?;
                            Ok::<_, String>((adapter, source, notifications))
                        }) {
                            Ok(ready) => Tried::Done(ready),
                            Err(cause) => Tried::Unavailable(
                                Unavailable {
                                    reason: Reason::Unreachable,
                                    extent: Extent::Scope,
                                    named: None,
                                },
                                cause,
                            ),
                        }
                    })
                    .await;
                let (adapter, source, notifications) = match acquired {
                    Attempted::Done(ready) => ready,
                    Attempted::Wait { wait, error } => {
                        let cause = error.unwrap_or_else(|| {
                            format!(
                                "T3 conversation source unavailable; waiting until {}",
                                wait.at()
                            )
                        });
                        tokio::task::block_in_place(|| {
                            destinations.company.relay_state(&home, Some(&cause));
                        });
                        tokio::select! {
                            changed = stopping.changed() => {
                                if changed.is_err() || *stopping.borrow() {
                                    return;
                                }
                            }
                            () = tokio::time::sleep_until(tokio::time::Instant::from_std(wait.until())) => {}
                        }
                        continue;
                    }
                    Attempted::Defect(cause) => {
                        tokio::task::block_in_place(|| {
                            destinations.company.relay_state(&home, Some(&cause));
                        });
                        let _ = stopping.changed().await;
                        return;
                    }
                };

                // The source is acquired: clear any previously recorded relay failure.
                tokio::task::block_in_place(|| {
                    destinations.company.relay_state(&home, None);
                });

                match serve(
                    &home,
                    &current,
                    &destinations,
                    adapter,
                    source,
                    notifications,
                    &mut stopping,
                )
                .await
                {
                    Ok(()) => return,
                    Err(cause) => {
                        let recorded = gate
                            .attempt_async(&key, &settings, || async {
                                Tried::<(), String>::Unavailable(
                                    Unavailable {
                                        reason: Reason::Unreachable,
                                        extent: Extent::Scope,
                                        named: None,
                                    },
                                    cause.clone(),
                                )
                            })
                            .await;
                        tokio::task::block_in_place(|| {
                            destinations.company.relay_state(&home, Some(&cause));
                        });
                        if let Attempted::Wait { wait, .. } = recorded {
                            tokio::select! {
                                changed = stopping.changed() => {
                                    if changed.is_err() || *stopping.borrow() {
                                        return;
                                    }
                                }
                                () = tokio::time::sleep_until(tokio::time::Instant::from_std(wait.until())) => {}
                            }
                        }
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

/// Serves company conversations on an acquired T3 conversation source until
/// shutdown (`Ok`) or a notification reader failure (`Err`).
async fn serve(
    home: &Home,
    current: &Current,
    destinations: &Destinations,
    adapter: Box<dyn crate::runtime::RuntimeAdapter>,
    source: Box<dyn crate::runtime::conversations::ConversationSource>,
    notifications: Box<dyn crate::runtime::conversations::ConversationNotifications>,
    stopping: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    if *stopping.borrow() {
        return Ok(());
    }

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
