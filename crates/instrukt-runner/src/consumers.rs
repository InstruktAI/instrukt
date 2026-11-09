//! Independent retained reactions. Redis owns replay; SQLite commits local outcomes before ACK.

mod _activation;
mod _parking;
mod _reader;
mod _sources;
mod _store;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{EventEnvelope, HomeKind, ReferenceKind, TypePattern};
use crate::state::RunnerState;
use crate::transport::{EventScope, ScopedTransport, TransportError};

pub(crate) use _activation::consumer as activation_consumer;
pub const MAX_HANDLING_ATTEMPTS: u32 = 3;
pub const STALE_PENDING_MS: u64 = 30_000;
pub const RECOVERY_SWEEP: Duration = Duration::from_millis(5_000);
pub const HANDLED_RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;
pub const TICK_ENTRY_BUDGET: usize = 50;

/// Selection reads envelope attributes only. Empty kind lists impose no kind filter.
#[derive(Debug, Clone)]
pub struct Interest {
    pub patterns: Vec<TypePattern>,
    pub own_computer_only: bool,
    pub subject_kinds: Vec<ReferenceKind>,
    pub home_kinds: Vec<HomeKind>,
}

impl Interest {
    pub fn matches(&self, event: &EventEnvelope, computer: &str) -> bool {
        (!self.own_computer_only || event.source == computer)
            && self
                .patterns
                .iter()
                .any(|pattern| pattern.matches(&event.event_type))
            && (self.subject_kinds.is_empty()
                || self
                    .subject_kinds
                    .iter()
                    .any(|kind| kind.as_str() == event.subject.kind()))
            && (self.home_kinds.is_empty()
                || event
                    .home
                    .kind()
                    .is_some_and(|kind| self.home_kinds.contains(&kind)))
    }
}

/// The reaction owns idempotent acceptance at its effect boundary. Its returned
/// local outcome commits with the handled mark and cursor in the runtime.
pub type Reaction = dyn Fn(&EventEnvelope, Timestamp) -> Result<Value, ReactionError> + Send + Sync;

#[derive(Debug, thiserror::Error)]
#[error("{stage}: {detail}")]
pub struct ReactionError {
    pub stage: String,
    pub detail: String,
}

#[derive(Clone)]
pub struct Consumer {
    pub name: String,
    pub interest: Interest,
    pub scopes: Vec<EventScope>,
    pub reaction: Arc<Reaction>,
}

impl std::fmt::Debug for Consumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Consumer")
            .field("name", &self.name)
            .field("interest", &self.interest)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error("retained consumer names must be nonempty and unique: `{0}`")]
    Name(String),
    #[error("local consumer positions are not prepared; publication remains pending")]
    NotPrepared,
    #[error("cannot start retained consumer reader: {0}")]
    Thread(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettledOutcome {
    Accepted(Value),
    Duplicate,
    Uninterested,
    Parked {
        destination: String,
        entry_id: String,
    },
    PayloadLost,
}

/// One registration set for a runner invocation. Every source has its own reader;
/// a single reaction gate serializes each logical consumer across its sources.
#[derive(Clone)]
pub struct ConsumerRuntime {
    state: RunnerState,
    transport: ScopedTransport,
    computer: String,
    consumers: Vec<Consumer>,
    gates: Vec<Arc<Mutex<()>>>,
    sources: Vec<Vec<_sources::BoundSource>>,
}

impl ConsumerRuntime {
    pub fn new(
        state: RunnerState,
        transport: ScopedTransport,
        computer: &str,
        consumers: Vec<Consumer>,
    ) -> Result<Self, ConsumerError> {
        let mut names = HashSet::new();
        for consumer in &consumers {
            if consumer.name.trim().is_empty() || !names.insert(&consumer.name) {
                return Err(ConsumerError::Name(consumer.name.clone()));
            }
        }
        let sources = consumers
            .iter()
            .map(|consumer| _sources::bind(&state, &transport, consumer))
            .collect::<Result<Vec<_>, _>>()?;
        let gates = consumers.iter().map(|_| Arc::new(Mutex::new(()))).collect();
        Ok(Self {
            state,
            transport,
            computer: computer.into(),
            consumers,
            gates,
            sources,
        })
    }

    /// Prepare just the source admitting this invocation's local producers.
    pub fn prepare_scope(&self, scope: EventScope) -> bool {
        let mut ready = true;
        let endpoint = self.transport.endpoint(scope);
        for (consumer, gate) in self.consumers.iter().zip(&self.gates) {
            if consumer
                .scopes
                .iter()
                .any(|selected| self.transport.endpoint(*selected).identity == endpoint.identity)
            {
                let reader = _reader::Reader::new(
                    self,
                    consumer,
                    gate,
                    _sources::BoundSource {
                        endpoint: Arc::clone(endpoint),
                        retiring: false,
                    },
                );
                if let Err(error) = reader.prepare() {
                    ready = false;
                    tracing::warn!(event = "consumer_prepare_failed", consumer = %consumer.name, error = %error);
                }
            }
        }
        ready
    }

    /// One bounded pending-first opportunity for every independent reader.
    pub fn drain(&self, now: Timestamp) {
        std::thread::scope(|threads| {
            for ((consumer, gate), sources) in
                self.consumers.iter().zip(&self.gates).zip(&self.sources)
            {
                for source in sources {
                    let reader = _reader::Reader::new(self, consumer, gate, source.clone());
                    threads.spawn(move || { if let Err(error) = reader.drain(now, TICK_ENTRY_BUDGET) { tracing::warn!(event = "consumer_drain_failed", consumer = %consumer.name, error = %error); } });
                }
            }
        });
    }

    /// Hosted by instrukt run, independent of its source-acquisition wake-ups.
    pub fn start(
        &self,
        clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    ) -> Result<ConsumerWorkers, ConsumerError> {
        let stop = Arc::new(Stop {
            requested: AtomicBool::new(false),
            mutex: Mutex::new(()),
            wake: Condvar::new(),
        });
        let mut workers = ConsumerWorkers {
            stop,
            threads: Vec::new(),
        };
        for ((consumer, gate), sources) in self.consumers.iter().zip(&self.gates).zip(&self.sources)
        {
            for source in sources {
                let source = source.clone();
                let runtime = self.clone();
                let consumer = consumer.clone();
                let gate = Arc::clone(gate);
                let stop = Arc::clone(&workers.stop);
                let clock = Arc::clone(&clock);
                let thread = std::thread::Builder::new().name(format!("event-{}", consumer.name)).spawn(move || {
                    let reader = _reader::Reader::new(&runtime, &consumer, &gate, source);
                    while !stop.requested.load(Ordering::Acquire) {
                        match reader.drain(clock(), TICK_ENTRY_BUDGET) {
                            Ok(_reader::DrainState::BudgetUsed) => continue,
                            Ok(_reader::DrainState::Retired) => break,
                            Ok(_reader::DrainState::Waiting) => {},
                            Err(error) => tracing::warn!(event = "consumer_reader_failed", consumer = %consumer.name, error = %error),
                        }
                        let guard = stop.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        let _wait = stop.wake.wait_timeout_while(guard, RECOVERY_SWEEP, |_| !stop.requested.load(Ordering::Acquire));
                    }
                })?;
                workers.threads.push(thread);
            }
        }
        Ok(workers)
    }
}

struct Stop {
    requested: AtomicBool,
    mutex: Mutex<()>,
    wake: Condvar,
}

/// Dropping the host requests shutdown and joins its readers. In-flight reaction
/// acceptance may finish; unresolved stream entries remain pending on interruption.
pub struct ConsumerWorkers {
    stop: Arc<Stop>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for ConsumerWorkers {
    fn drop(&mut self) {
        self.stop.requested.store(true, Ordering::Release);
        self.stop.wake.notify_all();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!(event = "consumer_reader_panicked");
            }
        }
    }
}

/// Prepare the runner's retained readers before installation admits producers.
/// Uses the same registrations and source handover as ordinary runner startup.
pub(crate) fn prepare_installation(
    home: &instrukt_core::home::Home,
    env: &instrukt_core::config::Env,
) -> anyhow::Result<()> {
    let state = RunnerState::new(home);
    let config = instrukt_core::config::Configuration::load_global(home)?;
    let transport = ScopedTransport::from_config(&config, env, &state)?;
    let computer = instrukt_core::machine::machine_id(home)?;
    let (wakeup, _) = std::sync::mpsc::sync_channel(1);
    let runtime = ConsumerRuntime::new(
        state.clone(),
        transport,
        computer.as_str(),
        vec![activation_consumer(home, &state, wakeup)?],
    )?;
    let local = runtime.prepare_scope(EventScope::Local);
    let cluster = runtime.prepare_scope(EventScope::Cluster);
    anyhow::ensure!(
        local && cluster,
        "retained reader preparation failed; restore Redis and rerun instrukt install"
    );
    Ok(())
}
