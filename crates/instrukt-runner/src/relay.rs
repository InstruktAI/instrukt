//! Notification-free company conversation relay backed by live runtime polling
//! and `instrukt-outside::Gate` backoff.
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::{
    config::{Current, Env},
    home::Home,
};
use instrukt_outside::{
    Attempted, Extent, Gate, Key, Reason, Report, Settings, Tried, Unavailable,
};
use tokio::sync::{Mutex, mpsc, watch};

use crate::interfaces::CompanyRelayPass;
use crate::runtime::{RuntimeAdapter, RuntimeRegistry};

/// How often the relay checks for active turns, retries, or out-of-process
/// `instrukt company reply` outbox entries.
const RETRY_CADENCE: Duration = Duration::from_secs(2);

/// The company destination served by the relay and its intake wake-ups.
#[derive(Clone)]
pub(crate) struct Destinations {
    pub(crate) company: Arc<dyn crate::interfaces::CompanyPorts>,
    pub(crate) arrivals: Arc<Mutex<mpsc::Receiver<()>>>,
}

/// The long-lived async relay task serving company conversations over a shared
/// long-lived `RuntimeAdapter`. Dropping it signals shutdown cleanly.
pub(crate) struct Worker {
    shutdown: watch::Sender<bool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

struct AdapterShutdown(Arc<dyn RuntimeAdapter>);
impl Drop for AdapterShutdown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

fn format_relay_wait(wait: &instrukt_outside::Wait, error: Option<&str>) -> String {
    match error {
        Some(cause) => format!("{wait} ({cause})"),
        None => wait.to_string(),
    }
}

impl Worker {
    /// Spawns the company conversation relay on the shared Tokio runtime.
    /// Runtime resolution, `relay()`, and `outstanding()` failures are judged
    /// by `instrukt-outside::Gate` and projected onto `destinations.company.relay_state`.
    pub(crate) fn start(
        home: &Home,
        env: &Env,
        current: Current,
        runtimes: RuntimeRegistry,
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
            let key = Key::new("runtime", ["company-relay"]).item("relay");
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
                            let adapter = runtimes
                                .long_lived(runtime.name(), &env)
                                .map_err(|error| error.to_string())?;
                            Ok::<_, String>((runtime.name(), adapter))
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
                let (runtime_name, adapter) = match acquired {
                    Attempted::Done(ready) => ready,
                    Attempted::Wait { wait, error } => {
                        let status = format_relay_wait(&wait, error.as_deref());
                        tokio::task::block_in_place(|| {
                            destinations.company.relay_state(&home, Some(&status));
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

                let _shutdown = AdapterShutdown(Arc::clone(&adapter));

                match serve(
                    ServeArgs {
                        home: &home,
                        current: &current,
                        destinations: &destinations,
                        runtime_name,
                        adapter,
                        gate: &gate,
                        key: &key,
                        settings: &settings,
                    },
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
                        let status = match &recorded {
                            Attempted::Wait { wait, error } => {
                                format_relay_wait(wait, error.as_deref().or(Some(cause.as_str())))
                            }
                            Attempted::Defect(detail) => detail.clone(),
                            Attempted::Done(()) => cause,
                        };
                        tokio::task::block_in_place(|| {
                            destinations.company.relay_state(&home, Some(&status));
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

    pub(crate) fn is_running(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

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

struct ServeArgs<'a> {
    home: &'a Home,
    current: &'a Current,
    destinations: &'a Destinations,
    runtime_name: &'static str,
    adapter: Arc<dyn RuntimeAdapter>,
    gate: &'a Gate,
    key: &'a Key,
    settings: &'a Settings<'a>,
}

async fn serve(args: ServeArgs<'_>, stopping: &mut watch::Receiver<bool>) -> Result<(), String> {
    let ServeArgs {
        home,
        current,
        destinations,
        runtime_name,
        adapter,
        gate,
        key,
        settings,
    } = args;
    if *stopping.borrow() {
        return Ok(());
    }
    let mut arrivals = destinations.arrivals.lock().await;
    let mut pass = CompanyRelayPass::CatchUp;
    let mut failing = false;
    loop {
        if *stopping.borrow() {
            return Ok(());
        }
        let admitted = current.get();
        let selected = admitted
            .global()
            .runner_runtime()
            .map_err(|error| error.to_string())?;
        if selected.name() != runtime_name {
            return Err(format!(
                "configured runtime changed from `{runtime_name}` to `{}`",
                selected.name()
            ));
        }

        let pass_outcome: Result<bool, String> = tokio::task::block_in_place(|| {
            destinations
                .company
                .relay(home, &admitted, Arc::clone(&adapter), pass)
                .map_err(|error| error.to_string())?;
            if matches!(pass, CompanyRelayPass::CatchUp) {
                tracing::debug!(event = "conversation_relay_source_processed");
            }
            destinations
                .company
                .outstanding(home)
                .map_err(|error| error.to_string())
        });

        let retry = match pass_outcome {
            Ok(outstanding) => {
                if failing {
                    let _ = gate
                        .attempt_async(key, settings, || async { Tried::<(), String>::Done(()) })
                        .await;
                    tokio::task::block_in_place(|| {
                        destinations.company.relay_state(home, None);
                    });
                    failing = false;
                }
                outstanding
            }
            Err(cause) => {
                tracing::warn!(event = "conversation_relay_failed", error = %cause);
                let recorded = gate
                    .attempt_async(key, settings, || async {
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
                let status = match &recorded {
                    Attempted::Wait { wait, error } => {
                        format_relay_wait(wait, error.as_deref().or(Some(cause.as_str())))
                    }
                    Attempted::Defect(detail) => detail.clone(),
                    Attempted::Done(()) => cause,
                };
                tokio::task::block_in_place(|| {
                    destinations.company.relay_state(home, Some(&status));
                });
                failing = true;
                if let Attempted::Wait { wait, .. } = recorded {
                    tokio::select! {
                        changed = stopping.changed() => {
                            if changed.is_err() || *stopping.borrow() {
                                return Ok(());
                            }
                        }
                        arrived = arrivals.recv() => {
                            match arrived {
                                Some(()) => {
                                    pass = CompanyRelayPass::Retry;
                                    continue;
                                }
                                None => return Ok(()),
                            }
                        }
                        () = tokio::time::sleep_until(tokio::time::Instant::from_std(wait.until())) => {}
                    }
                    pass = CompanyRelayPass::Retry;
                    continue;
                }
                true
            }
        };

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
                () = tokio::time::sleep_until(retry_deadline) => {
                    let check: Result<bool, String> = if retry {
                        Ok(true)
                    } else {
                        tokio::task::block_in_place(|| {
                            destinations
                                .company
                                .outstanding(home)
                                .map_err(|error| error.to_string())
                        })
                    };
                    match check {
                        Ok(true) => break CompanyRelayPass::Retry,
                        Ok(false) => {
                            retry_deadline = tokio::time::Instant::now() + RETRY_CADENCE;
                        }
                        Err(cause) => {
                            tracing::warn!(event = "conversation_relay_outstanding_failed", error = %cause);
                            return Err(cause);
                        }
                    }
                }
            }
        };
    }
}
