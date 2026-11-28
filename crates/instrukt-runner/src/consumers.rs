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
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{EventEnvelope, HomeKind, ReferenceKind, TypePattern};
use crate::state::RunnerState;
use crate::transport::{EventScope, ScopedTransport, TransportError};

pub(crate) use _activation::consumer as activation_consumer;
pub(crate) use _activation::{ACTIVATION_CONSUMER, REPLACEABLE, end_replaceable};
pub const MAX_HANDLING_ATTEMPTS: u32 = 3;
pub const STALE_PENDING_MS: u64 = 30_000;
pub const RECOVERY_SWEEP: Duration = Duration::from_millis(5_000);
pub const HANDLED_RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;
pub const TICK_ENTRY_BUDGET: usize = 50;

/// Selection reads envelope attributes only. Empty kind lists impose no kind filter.
#[derive(Debug, Clone)]
pub struct Interest {
    pub patterns: Vec<TypePattern>,
    /// For an event that is not an external fact: only one this computer
    /// emitted.
    pub own_computer_only: bool,
    /// An external fact, recognised by its inbound-source subject, is
    /// selected only on the computer that reacts to external facts.
    pub external_on_reacting: bool,
    pub subject_kinds: Vec<ReferenceKind>,
    pub home_kinds: Vec<HomeKind>,
}

impl Interest {
    /// Whether `event` is selected on `computer`, where `reacting` is the
    /// computer that reacts to external facts, if any.
    pub fn matches(&self, event: &EventEnvelope, computer: &str, reacting: Option<&str>) -> bool {
        let reach = if self.external_on_reacting && is_external(event) {
            reacting == Some(computer)
        } else {
            !self.own_computer_only || event.source == computer
        };
        reach
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

/// Whether `event` is an external fact: one the ingress receiver verified and
/// published, recognised by its inbound-source subject.
pub fn is_external(event: &EventEnvelope) -> bool {
    event.subject.kind() == crate::events::ReferenceKind::InboundSource.as_str()
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
    #[error("cannot read which computer reacts to external facts: {0}")]
    Reacting(#[from] instrukt_core::config::ConfigError),
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
        /// Whether `instrukt.system.event.dead_lettered` was announced. A
        /// failed announcement is counted in status and never holds the
        /// entries after it; a settlement recorded before this was kept was
        /// announced.
        #[serde(default = "announced")]
        announced: bool,
    },
    PayloadLost,
}

fn announced() -> bool {
    true
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
                if let Err(error) = reader.prepare_bound() {
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
                    let source = source.clone();
                    threads.spawn(move || { let reader = _reader::Reader::new(self, consumer, gate, source); if let Err(error) = reader.drain(now, TICK_ENTRY_BUDGET) { tracing::warn!(event = "consumer_drain_failed", consumer = %consumer.name, error = %error); } });
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

impl ConsumerWorkers {
    fn request_stop(&self) {
        let _guard = self
            .stop
            .mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.stop.requested.store(true, Ordering::Release);
        self.stop.wake.notify_all();
    }

    /// Signal once and wait only through the same deadline as source binding.
    /// The caller keeps ownership of unfinished threads for the next wake.
    pub(crate) fn stop_for_handover(&self) -> bool {
        self.request_stop();
        let deadline = Instant::now() + _sources::HANDOVER_WAIT;
        loop {
            if self.threads.iter().all(JoinHandle::is_finished) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        }
    }
}

impl Drop for ConsumerWorkers {
    fn drop(&mut self) {
        self.request_stop();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!(event = "consumer_reader_panicked");
            }
        }
    }
}

const TRANSITION_PREFIX: &str = "consumer-source-transition:";
const CONSUMER_TRANSITION_PREFIX: &str = "consumer-source-transition:consumer:";
pub(crate) const RUNNER_TRANSITION: &str = "consumer-source-transition:runner";

/// What `instrukt events status` shows of the retained consumers: the
/// published subscriptions the activation matcher drops, each named with why
/// (or why the index cannot be read); how many events settled with no
/// activation; how many dead letters could not be announced; and each source
/// abandoned when it left the configuration, whose pending events stay unread
/// in that Redis.
pub(crate) fn status(
    home: &instrukt_core::home::Home,
    state: &RunnerState,
) -> Result<Value, crate::state::StateError> {
    let subscriptions = match _activation::subscriptions(home) {
        Ok(subscriptions) => serde_json::json!({"dropped": subscriptions.dropped}),
        Err(error) => serde_json::json!({"error": error.to_string()}),
    };
    let count = |sql: &str| -> Result<u64, crate::state::StateError> {
        Ok(state
            .read(|conn, path| {
                conn.query_row(sql, [_activation::ACTIVATION_CONSUMER], |row| row.get(0))
                    .map_err(crate::state::sql(path))
            })?
            .unwrap_or(0))
    };
    let settled_without_activation = count(
        "SELECT count(*) FROM consumer_outcomes WHERE consumer = ?1 AND json_extract(outcome, '$.accepted.activations') = 0",
    )?;
    let unannounced = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT count(*) FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announced') = 0",
                [],
                |row| row.get::<_, u64>(0),
            )
            .map_err(crate::state::sql(path))
        })?
        .unwrap_or(0);
    let now = Timestamp::now();
    let mut transitions: Vec<_> = crate::state::read_worker_failures(home)?
        .into_iter()
        .filter(|entry| entry.worker.starts_with(TRANSITION_PREFIX))
        .collect();
    transitions.sort_by(|a, b| a.worker.cmp(&b.worker));
    let transitions = transitions
        .into_iter()
        .map(|entry| {
            let transition = entry
                .worker
                .strip_prefix(TRANSITION_PREFIX)
                .unwrap_or(&entry.worker)
                .to_owned();
            let age_seconds = entry
                .since
                .map_or(0, |started| now.duration_since(started).as_secs().max(0));
            serde_json::json!({
                "transition": transition,
                "cause": entry.failure,
                "since": entry.since.map(|at| at.to_string()),
                "age_seconds": age_seconds,
            })
        })
        .collect::<Vec<_>>();
    let abandoned = _sources::abandoned(state)?
        .into_iter()
        .map(|(consumer, source)| {
            serde_json::json!({
                "consumer": consumer,
                "source": source,
                "pending": "its pending events stay unread in that Redis until it is configured again",
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "subscriptions": subscriptions,
        "settled_without_activation": settled_without_activation,
        "unannounced_dead_letters": unannounced,
        "abandoned_sources": abandoned,
        "pending_source_transitions": transitions,
    }))
}

/// Prepare the runner's retained readers before installation admits producers.
/// Uses the same registrations and source handover as ordinary runner startup.
pub(crate) fn prepare_installation(
    home: &instrukt_core::home::Home,
    env: &instrukt_core::config::Env,
) -> anyhow::Result<()> {
    let state = RunnerState::new(home);
    let config = instrukt_core::config::Configuration::load_global(home)?;
    let transport = ScopedTransport::from_config_with(
        &config,
        env,
        &state,
        crate::transport::Reachability::Unrecorded,
    )?;
    let computer = instrukt_core::machine::machine_id(home)?;
    let (wakeup, _) = tokio::sync::mpsc::channel(1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_letter_settled_before_its_announcement_was_kept_reads_as_announced() {
        let stored = r#"{"parked":{"destination":"instrukt:events:dlq","entry_id":"1-0"}}"#;
        assert!(matches!(
            serde_json::from_str::<SettledOutcome>(stored).unwrap(),
            SettledOutcome::Parked {
                announced: true,
                ..
            }
        ));
    }
}
