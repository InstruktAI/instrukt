//! Runner invocations (`work-dispatch`, "Execution contract"; `runner`
//! domain, "Runner invocation"): `instrukt tick` performs one wake-up's
//! work and `instrukt run` repeats it until stopped. One runner lock per
//! machine serializes them; a tick exits at once while it is held.
//!
//! A wake-up records itself, then considers every published project whose
//! effective configuration enables `events` or `todos`: with `events` on it
//! reads the event sources and delivers pending events to standing agents;
//! with `todos` on it dispatches eligible todos.

use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{
    AdmissionError, Admitted, ConfigError, Configuration, Current, Env, Module, Offer,
};
use instrukt_core::fsx::FsError;
use instrukt_core::github;
use instrukt_core::home::Home;
use instrukt_core::ids::CommitSha;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::ci::{self, CiClient, CiRead};
use crate::consumers::{self, ConsumerError, ConsumerRuntime};
use crate::dispatch::{self, DispatchContext};
use crate::event_log::DbEventLog;
use crate::events::Event;
use crate::interfaces::{ProjectRef, RunnerWiring};
use crate::sources;
use crate::standing;
use crate::state::{RunnerState, StateError};
use crate::ticks;
use crate::transport::{EventScope, ScopedTransport, TransportError};

/// How long `instrukt run` waits for a tick in progress to finish.
const RUN_LOCK_WAIT: Duration = Duration::from_secs(120);

mod _holder;
mod _readiness;

/// What a running `instrukt run` publishes about the connectors it hosts.
#[derive(serde::Serialize, serde::Deserialize)]
struct Hosting {
    pid: u32,
    started: String,
    /// Each hosted connector, as its company and connector key.
    connectors: Vec<(String, String)>,
}

/// Publishes the connectors this `instrukt run` hosts, with this process's
/// incarnation, for `instrukt events status`.
pub fn publish_hosted(home: &Home, connectors: &[(String, String)]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let record = Hosting {
        pid: std::process::id(),
        started: _holder::own_incarnation()?,
        connectors: connectors.to_vec(),
    };
    let bytes = serde_json::to_vec(&record).map_err(std::io::Error::other)?;
    instrukt_core::fsx::atomic_write_with_permissions(
        &home.runner_hosts_file(),
        &bytes,
        std::fs::Permissions::from_mode(0o600),
    )
    .map_err(std::io::Error::other)
}

/// The connectors the running `instrukt run` hosts now: what it published,
/// believed only while that same process incarnation holds the runner lock
/// as a run, the service's or one started by hand. Nothing is hosted
/// otherwise.
pub fn hosted(home: &Home) -> Vec<(String, String)> {
    let Some((holder, pid, started)) = _holder::live_holder(&home.runner_lock_file()) else {
        return Vec::new();
    };
    if !matches!(holder, _holder::Holder::Service | _holder::Holder::Run) {
        return Vec::new();
    }
    std::fs::read(home.runner_hosts_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Hosting>(&bytes).ok())
        .filter(|record| record.pid == pid && record.started == started)
        .map(|record| record.connectors)
        .unwrap_or_default()
}

/// The argument the owned runner service passes to `instrukt run`.
pub const SERVICE_FLAG: &str = "--service";

/// A runner invocation could not run.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error(transparent)]
    Links(#[from] crate::links::LinkError),
    #[error(transparent)]
    History(#[from] crate::history::HistoryError),
    #[error("conversations: {0}")]
    Conversations(String),
    #[error(
        "instrukt's runner service is already running `instrukt run`, which serves every enabled module; no second run is started"
    )]
    ServiceRunning,
    #[error(
        "the runner service timed out waiting {waited:?} for the runner lock; holder: {holder}"
    )]
    ServiceLockTimeout {
        waited: Duration,
        holder: &'static str,
    },
    #[error(
        "cannot publish the runner lock's holder in {}: {source}; restore write access to the state directory",
        path.display()
    )]
    HolderUnpublished {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("the event worker panicked; retained entries remain recoverable")]
    EventWorkerPanicked,
    #[error(transparent)]
    Admission(#[from] AdmissionError),
    #[error(transparent)]
    Consumer(#[from] ConsumerError),
    #[error(transparent)]
    Pattern(#[from] crate::events::TypePatternError),
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Lock(#[from] FsError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Publication(#[from] instrukt_core::publication::PublicationStateError),
}

/// What an invocation works with.
pub struct RunnerContext<'a> {
    pub home: &'a Home,
    pub env: &'a Env,
    pub tz: &'a TimeZone,
    pub wiring: &'a RunnerWiring,
}

/// A project the runner considers, with its effective configuration.
pub struct Considered {
    pub project: ProjectRef,
    pub config: Configuration,
    pub todos: bool,
    pub events: bool,
}

/// The published projects that switch on `todos` or `events`. A project
/// whose checkout is gone or whose layer could not be loaded is not among
/// them. A finding withholds only its module's own work: the project is
/// still considered, with that module off.
pub fn considered(admitted: &Admitted) -> Vec<Considered> {
    admitted
        .projects()
        .iter()
        .filter(|project| project.switched)
        .map(|project| Considered {
            project: ProjectRef {
                slug: project.slug.clone(),
                root: project.checkout.clone(),
            },
            config: project.config.clone(),
            todos: project.todos,
            events: project.events,
        })
        .collect()
}

/// [`considered`] in a generation admitted from the files now.
pub fn considered_projects(home: &Home) -> Result<Vec<Considered>, RunnerError> {
    let candidate = Admitted::load(home)?;
    Ok(considered(&candidate.generation))
}

/// Whether a tick did its work or found the runner busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Done,
    /// Another invocation holds the runner lock.
    Busy,
}

/// One wake-up at `now`; exits at once when the runner lock is held. Its
/// configuration is admitted at its start: an invalid machine layer or
/// `agents.yaml` refuses it, and what was not admitted is left out.
pub fn tick(
    cx: &RunnerContext<'_>,
    clock: &dyn Fn() -> Timestamp,
) -> Result<TickOutcome, RunnerError> {
    let now = clock();
    let state = RunnerState::new(cx.home);
    let Some(_runner) = _holder::tick(cx.home)? else {
        return Ok(TickOutcome::Busy);
    };
    let Some(invocation) = crate::keyed::RuntimeInvocation::try_acquire(cx.home)? else {
        tracing::info!(event = "tick_runner_busy");
        return Ok(TickOutcome::Busy);
    };
    let candidate = Admitted::load(cx.home)?;
    Admission::default().report(&candidate.findings);
    // The wake uses what it admitted: nothing in this process reads the
    // configuration files on its own.
    let current = Current::new(candidate.generation);
    instrukt_core::config::serve(cx.home, &current);
    let admitted = current.get();
    // Readiness is refreshed once, on entry, before any start; a failure
    // leaves the starts with the configured order. The token is revoked when
    // the tick ends.
    let mut readiness = admitted
        .global()
        .runner_runtime()
        .ok()
        .and_then(|runtime| crate::runtime::adapter(runtime).readiness());
    if let Some(readiness) = readiness.as_mut() {
        _readiness::refresh(cx.home, readiness.as_mut(), clock, &mut true);
    }
    let considered = considered(&admitted);
    let woke = event_platform(cx, &state, &admitted).and_then(|platform| {
        wake(
            cx,
            &state,
            now,
            &considered,
            platform.as_ref(),
            true,
            &invocation,
        )
    });
    // The tick's readiness ends with it: its token is revoked and its row,
    // if a failed refresh left one, removed.
    if readiness.take().is_some() {
        _readiness::ended(cx.home);
    }
    drop(invocation);
    crate::keyed::notify_due(
        &state,
        admitted.global(),
        cx.env,
        clock(),
        &mut Default::default(),
    );
    woke?;
    Ok(TickOutcome::Done)
}

/// What the run loop last reported of its admission, so a finding is
/// logged when it appears or changes, not on every wake.
#[derive(Default)]
struct Admission {
    reported: Vec<String>,
}

impl Admission {
    fn report(&mut self, findings: &[instrukt_core::config::Finding]) {
        let now: Vec<String> = findings.iter().map(ToString::to_string).collect();
        if now == self.reported {
            return;
        }
        for finding in &now {
            if !self.reported.contains(finding) {
                tracing::warn!(event = "configuration_not_admitted", finding = %finding);
            }
        }
        self.reported = now;
    }
}

/// The long-lived runner: a wake-up each `interval` until the process is
/// stopped, holding the runner lock throughout. Its configuration is one
/// admitted generation, built when it starts and again on every wake; what
/// depends on something outside instrukt retries in its own thread.
pub fn run(
    cx: &RunnerContext<'_>,
    clock: impl Fn() -> Timestamp + Send + Sync + 'static,
    interval: Duration,
    service: bool,
) -> Result<(), RunnerError> {
    let state = RunnerState::new(cx.home);
    if service {
        // An earlier release's service identity record: nothing reads it.
        let record = cx.home.runner_service_file();
        match instrukt_core::fsx::remove_file(&record) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                event = "runner_service_record_kept",
                path = %record.display(),
                %error,
                "the earlier release's runner service record could not be removed; startup continues, since nothing reads it"
            ),
        }
    }
    let _runner = _holder::run(cx.home, service)?;
    // Initialize the shared schema before independent workers open connections.
    drop(instrukt_core::state_db::StateDb::open(cx.home).map_err(StateError::from)?);
    let clock: Arc<dyn Fn() -> Timestamp + Send + Sync> = Arc::new(clock);
    // The gate: nothing starts on an invalid machine layer or agents.yaml.
    let mut admission = Admission::default();
    let candidate = Admitted::load(cx.home)?;
    admission.report(&candidate.findings);
    let current = Current::new(candidate.generation);
    // From here on every part of this runner uses the generation in force:
    // nothing in its process reads the configuration files on its own.
    instrukt_core::config::serve(cx.home, &current);
    crate::state::admission_outcome(cx.home, clock(), true);
    // Readiness is kept on a thread of its own: refreshed now and on each
    // wake, never by a start.
    let readiness = match current
        .get()
        .global()
        .runner_runtime()
        .ok()
        .and_then(|runtime| crate::runtime::adapter(runtime).readiness())
    {
        Some(readiness) => {
            match _readiness::Worker::start(cx.home, readiness, Arc::clone(&clock)) {
                Ok(worker) => Some(worker),
                Err(error) => {
                    tracing::error!(event = "readiness_worker_unstarted", %error);
                    None
                }
            }
        }
        None => None,
    };
    let mut platform: Option<EventPlatform> = None;
    // The transport settings the event platform was prepared on.
    let mut platform_transport: Option<serde_yaml_ng::Value> = None;
    let mut readers: Option<consumers::ConsumerWorkers> = None;
    let mut handover_pending = false;
    let mut history = crate::history::Readers::default();
    // The relay serves links and, while a company declares a connector and
    // conversations are on, the connector destination; the connections take
    // the intake. Both report their unexpected end to this loop at once.
    let (supervise, supervision) = std::sync::mpsc::sync_channel::<()>(8);
    if let Some(company) = &cx.wiring.company
        && let Err(error) = company.maintain_read_back(cx.home, clock())
    {
        tracing::warn!(event = "read_back_maintenance_failed", %error);
    }
    // The company is hosted when the admitted generation declares a
    // connector: its connections start first, building each connector kind
    // once; a setting a kind refuses stops the runner here. The relay then
    // serves its conversations beside links.
    let company = cx
        .wiring
        .company
        .as_ref()
        .filter(|company| company.demand(&current.get()));
    let (arrive, arrivals) = std::sync::mpsc::sync_channel::<()>(1);
    let mut connections = match company {
        Some(company) => Some(
            company
                .start(crate::interfaces::CompanyStart {
                    home: cx.home,
                    current: &current,
                    env: cx.env,
                    clock: Arc::clone(&clock),
                    wake: arrive,
                    supervise: supervise.clone(),
                })
                .map_err(|e| RunnerError::Conversations(e.0))?,
        ),
        None => None,
    };
    let destinations = company.map(|company| crate::links::Destinations {
        company: Arc::clone(company),
        arrivals: Arc::new(std::sync::Mutex::new(arrivals)),
    });
    // The relay retries T3 inside its own thread; it is started again here
    // only when its thread ended.
    let supervise_relay = |relay: &mut Option<crate::links::Worker>| {
        if let Some(current) = relay.as_mut() {
            match current.check() {
                Ok(()) => return,
                Err(error) => {
                    crate::links::record_failure(cx.home, &error);
                    *relay = None;
                }
            }
        }
        match crate::links::Worker::start(
            cx.home,
            cx.env,
            current.clone(),
            destinations.clone(),
            Some(supervise.clone()),
        ) {
            Ok(worker) => *relay = Some(worker),
            Err(error) => crate::links::record_failure(cx.home, &error),
        }
    };
    let mut relay = None;
    supervise_relay(&mut relay);
    tracing::info!(event = "runner_started");
    // A skipped wake stays in status until the next wake that is not skipped;
    // one left by an earlier runner is cleared by this runner's first.
    let mut skipped = true;
    let mut notified_starts = std::collections::HashSet::new();
    loop {
        // The interval on the operator's clock runs from when this wake
        // began, however long its work takes.
        let woke_at = clock();
        supervise_relay(&mut relay);
        if let Some(connections) = &mut connections {
            connections.check();
        }
        // Every wake admits the files again; an invalid change leaves the
        // current generation in force and is reported.
        match Admitted::load(cx.home) {
            Ok(candidate) => {
                admission.report(&candidate.findings);
                let offered = current.offer(candidate);
                if offered == Offer::Kept {
                    tracing::debug!(event = "configuration_kept");
                }
                crate::state::admission_outcome(cx.home, clock(), offered == Offer::Replaced);
            }
            Err(error) => {
                admission.report(&[]);
                tracing::warn!(event = "configuration_kept", error = %error);
                crate::state::admission_outcome(cx.home, clock(), false);
            }
        }
        let admitted = current.get();
        if let Some(readiness) = &readiness {
            readiness.wake();
        }
        // An event platform holds the transport it was prepared on: a
        // generation admitting other transport settings prepares it again.
        let transport_settings = admitted.global().get(Some("redis"), cx.env).ok();
        if platform.is_some() && (handover_pending || platform_transport != transport_settings) {
            handover_pending = true;
            if readers
                .as_ref()
                .is_none_or(consumers::ConsumerWorkers::stop_for_handover)
            {
                tracing::info!(
                    event = "event_platform_replaced",
                    cause = "transport settings changed"
                );
                readers = None;
                platform = None;
                handover_pending = false;
                crate::state::worker_state(cx.home, consumers::RUNNER_TRANSITION, None);
            } else {
                tracing::warn!(
                    event = "consumer_source_transition_deferred",
                    "previous readers are still stopping"
                );
                crate::state::worker_state(
                    cx.home,
                    consumers::RUNNER_TRANSITION,
                    Some("waiting for previous readers to stop"),
                );
            }
        }
        history.refresh(cx.home, cx.env, &current);
        let considered = considered(&admitted);
        let events_enabled = admitted.module_anywhere(Module::Events).unwrap_or(false);
        if platform.is_none() && events_enabled {
            match event_platform(cx, &state, &admitted) {
                Ok(Some(prepared)) => match prepared.runtime.start(Arc::clone(&clock)) {
                    Ok(started) => {
                        crate::state::worker_state(cx.home, consumers::RUNNER_TRANSITION, None);
                        readers = Some(started);
                        platform = Some(prepared);
                        platform_transport = transport_settings.clone();
                        handover_pending = false;
                    }
                    Err(error) => tracing::error!(event = "event_readers_unstarted", %error),
                },
                Ok(None) => {}
                Err(error) => tracing::error!(event = "event_platform_unprepared", %error),
            }
        }
        let enabled_platform = platform.as_ref().filter(|_| {
            events_enabled && !handover_pending && platform_transport == transport_settings
        });
        // A dispatch lock held past its wait skips this wake; the relay,
        // connections, readers and history go on in their own threads.
        match crate::keyed::RuntimeInvocation::acquire(cx.home, RUN_LOCK_WAIT) {
            Ok(invocation) => {
                if std::mem::take(&mut skipped) {
                    crate::state::skipped_wake(cx.home, None);
                }
                if let Err(err) = wake(
                    cx,
                    &state,
                    clock(),
                    &considered,
                    enabled_platform,
                    false,
                    &invocation,
                ) {
                    tracing::error!(event = "runner_wakeup_failed", error = %err);
                }
            }
            Err(error) => {
                tracing::warn!(event = "runner_wakeup_skipped", %error);
                crate::state::skipped_wake(cx.home, Some((clock(), &error.to_string())));
                skipped = true;
            }
        }
        crate::keyed::notify_due(
            &state,
            admitted.global(),
            cx.env,
            clock(),
            &mut notified_starts,
        );
        // Wait for the interval, an activation wake-up, or a supervised
        // worker's end, which is observed within a second.
        // The interval passes in real time, or on the operator's clock when
        // that has moved past it since the wake began.
        let until = std::time::Instant::now() + interval;
        let interval_on_clock =
            jiff::SignedDuration::try_from(interval).unwrap_or(jiff::SignedDuration::MAX);
        while std::time::Instant::now() < until
            && clock().duration_since(woke_at) < interval_on_clock
        {
            // Supervision runs each second, independently of the wake-up.
            if supervision.try_recv().is_ok() || relay.is_none() {
                supervise_relay(&mut relay);
                if let Some(connections) = &mut connections {
                    connections.check();
                }
            }
            let slice = (until - std::time::Instant::now()).min(Duration::from_secs(1));
            let woken = platform.as_ref().and_then(|platform| {
                platform
                    .wakeups
                    .lock()
                    .ok()
                    .map(|receiver| receiver.recv_timeout(slice))
            });
            match woken {
                Some(Ok(())) => break,
                Some(Err(std::sync::mpsc::RecvTimeoutError::Timeout)) => {}
                // No platform, or its wake-ups are gone: the interval alone
                // wakes the loop.
                Some(Err(std::sync::mpsc::RecvTimeoutError::Disconnected)) | None => {
                    std::thread::sleep(slice)
                }
            }
        }
    }
}

struct EventPlatform {
    transport: ScopedTransport,
    runtime: ConsumerRuntime,
    wakeups: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

/// The event platform of an admitted generation: none while no event
/// responses are admitted on the machine or any considered project. The
/// gate guarantees the transport is configured whenever one is.
fn event_platform(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    admitted: &Admitted,
) -> Result<Option<EventPlatform>, RunnerError> {
    if !admitted.module_anywhere(Module::Events)? {
        return Ok(None);
    }
    let transport = ScopedTransport::from_config(admitted.global(), cx.env, state)?;
    let computer = instrukt_core::machine::machine_id(cx.home)?;
    let (wakeup, wakeups) = std::sync::mpsc::sync_channel(1);
    let runtime = ConsumerRuntime::new(
        state.clone(),
        transport.clone(),
        computer.as_str(),
        vec![consumers::activation_consumer(cx.home, state, wakeup)?],
    )?;
    Ok(Some(EventPlatform {
        transport,
        runtime,
        wakeups: std::sync::Mutex::new(wakeups),
    }))
}

fn wake(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    now: Timestamp,
    considered: &[Considered],
    platform: Option<&EventPlatform>,
    drain: bool,
    invocation: &crate::keyed::RuntimeInvocation,
) -> Result<(), RunnerError> {
    let previous = ticks::last_wakeup(state)?;
    ticks::record_wakeup(state, now)?;
    let dispatch_cx = DispatchContext {
        invocation,
        home: cx.home,
        env: cx.env,
        now,
        tz: cx.tz,
        previous_wake: previous,
        state,
    };
    // Observe once before dispatch so all local readers see an integrated revision.
    // Synchronization is independent of event-module enablement and emits no main-ahead event.
    let mut mains = std::collections::BTreeMap::new();
    for considered in considered {
        let project = &considered.project;
        match sources::observe_main(&project.root) {
            Ok(observed) => {
                if let Err(error) = sources::synchronize_main(&project.root, &observed) {
                    tracing::warn!(event = "canonical_checkout_sync_failed", project = %project.slug, error = %error);
                }
                mains.insert(project.slug.clone(), observed);
            }
            Err(error) => {
                tracing::warn!(event = "main_unobservable", project = %project.slug, error = %error)
            }
        }
    }
    std::thread::scope(|threads| -> Result<(), RunnerError> {
        let events = threads.spawn(|| {
            EventWake {
                cx,
                state,
                now,
                considered,
                mains: &mains,
                platform,
                drain,
            }
            .run()
        });
        for considered in considered {
            let project = &considered.project;
            if considered.todos && code_on_github(cx.home, considered) {
                dispatch::dispatch_project(&dispatch_cx, &considered.config, cx.wiring, project);
            }
        }
        events
            .join()
            .map_err(|_| RunnerError::EventWorkerPanicked)?
    })?;
    if let Err(error) = standing::deliver_pending(invocation, now) {
        tracing::warn!(event="standing_delivery_failed",error=%error);
    }
    {
        let adapter =
            crate::runtime::adapter(Configuration::load_global(cx.home)?.runner_runtime()?);
        if let Err(error) = crate::_attention::approvals(invocation, adapter.as_ref(), now) {
            tracing::warn!(event="approval_attention_failed",error=%error);
        }
        if let Err(error) = crate::inputs::pump_all(invocation, adapter.as_ref(), now) {
            tracing::warn!(event="pending_input_failed",error=%error);
        }
        if let Err(error) = crate::_attention::settle_approvals(state) {
            tracing::warn!(event="approval_settlement_failed",error=%error);
        }
        if let Err(error) = standing::settle(state, now) {
            tracing::warn!(event="standing_settlement_failed",error=%error);
        }
    }
    tracing::debug!(event = "runner_wakeup_done", at = %now);
    Ok(())
}

struct EventWake<'a, 'cx> {
    cx: &'a RunnerContext<'cx>,
    state: &'a RunnerState,
    now: Timestamp,
    considered: &'a [Considered],
    mains:
        &'a std::collections::BTreeMap<instrukt_core::ids::ProjectSlug, sources::MainObservation>,
    platform: Option<&'a EventPlatform>,
    drain: bool,
}

impl EventWake<'_, '_> {
    fn run(self) -> Result<(), RunnerError> {
        let Self {
            cx,
            state,
            now,
            considered,
            mains,
            platform,
            drain,
        } = self;
        let log = DbEventLog::new(state.clone(), now);
        let ready =
            platform.is_none_or(|platform| platform.runtime.prepare_scope(EventScope::Local));
        for project in considered.iter().filter(|project| project.events) {
            read_sources(
                cx,
                state,
                &log,
                project,
                mains.get(&project.project.slug),
                now,
            );
        }
        if let Some(platform) = platform {
            if ready {
                platform.transport.publish_pending(&log)?;
            }
            if drain {
                platform.runtime.drain(now);
            }
        }
        Ok(())
    }
}

/// Whether the considered project's code is on GitHub: todos need it
/// there, and a project whose code is not gets no todo pickup
/// (`work-dispatch`, "Execution contract").
fn code_on_github(home: &Home, considered: &Considered) -> bool {
    let project = &considered.project;
    let host = github::api_url(home)
        .map_err(|err| err.to_string())
        .and_then(|api_url| {
            github::code_host(&considered.config, &api_url, &project.root)
                .map_err(|err| err.to_string())
        });
    match host {
        Ok(github::CodeHost::GitHub { .. }) => true,
        Ok(github::CodeHost::Elsewhere { reason }) => {
            tracing::info!(
                event = "todos_need_github",
                project = %project.slug,
                reason = %reason,
                "the project's code is not on GitHub; no todo pickup"
            );
            false
        }
        Err(err) => {
            tracing::warn!(event = "code_host_unresolved", project = %project.slug, error = %err);
            false
        }
    }
}

/// Reads the project's event sources and appends what they found.
fn read_sources(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    log: &DbEventLog,
    considered: &Considered,
    observed: Option<&sources::MainObservation>,
    now: Timestamp,
) {
    let project = &considered.project;
    let append = |event: &Event| match log.log(event) {
        Ok(_) => true,
        Err(err) => {
            tracing::warn!(event = "event_not_logged", name = event.name(), project = %project.slug, error = %err);
            false
        }
    };
    match considered.config.events_watch() {
        Ok(watched) if !watched.is_empty() => {
            if let Err(err) =
                sources::upstream_changes(state, log, &project.slug, &project.root, &watched, now)
            {
                tracing::warn!(event = "upstream_watch_failed", project = %project.slug, error = %err);
            }
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(event = "events_watch_unreadable", project = %project.slug, error = %err)
        }
    }
    let Some(observed) = observed else {
        return;
    };

    // Canonical, candidate and selected dependency heads share one conditional
    // pass and the same source deferral and credential boundary.
    let candidate = observed.candidate.clone();
    let head = observed.remote_main.clone();
    let api_url = match github::api_url(cx.home) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(event = "github_api_url_unreadable", error = %err);
            return;
        }
    };
    let repository = match github::repository(&considered.config, &api_url, &project.root) {
        Ok(repository) => repository,
        Err(err) => {
            tracing::debug!(event = "ci_repository_unresolved", project = %project.slug, error = %err);
            return;
        }
    };
    match ci::credential_deferral(state, &project.slug, now) {
        Ok(Some(until)) => {
            tracing::debug!(event="ci_app_deferred",project=%project.slug,until=%until);
            return;
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(event="ci_observation_state_failed",error=%error);
            return;
        }
    }
    let dependencies = match considered
        .config
        .dependency_branch_prefixes()
        .map_err(|error| error.to_string())
        .and_then(|prefixes| {
            sources::dependency_heads(&project.root, &prefixes).map_err(|error| error.to_string())
        }) {
        Ok(heads) => heads,
        Err(error) => {
            tracing::warn!(event="dependency_heads_unreadable", project=%project.slug, error=%error);
            // Canonical/candidate CI still progresses when dependency discovery fails.
            Vec::new()
        }
    };
    if candidate.is_none() && head.is_none() && dependencies.is_empty() {
        return;
    }
    let Some(credentials) = &cx.wiring.github_credentials else {
        tracing::warn!(event="ci_app_unavailable",project=%project.slug,cause="work credential capability unavailable");
        return;
    };
    let credential = match credentials.credential(&repository, github::Operation::Observe, now) {
        Ok(credential) => credential,
        Err(error) => {
            if let Err(save) = ci::record_credential_failure(state, &project.slug, &error, now) {
                tracing::warn!(event="ci_observation_state_failed",error=%save);
            }
            tracing::warn!(event="ci_app_unavailable",project=%project.slug,error=%error);
            return;
        }
    };
    let client = CiClient::new(
        &api_url,
        &repository,
        credential.token().to_owned(),
        state,
        &project.slug,
    );
    let commits: Vec<&CommitSha> = candidate
        .iter()
        .chain(head.iter())
        .chain(dependencies.iter().map(|(_, sha)| sha))
        .collect();
    match client.read(&commits, now) {
        Ok(CiRead::States(states)) => {
            let mut states = states.into_iter();
            let candidate_state = candidate
                .as_ref()
                .and_then(|_| states.next())
                .map(|read| read.state);
            let head_state = head
                .as_ref()
                .and_then(|_| states.next())
                .map(|read| read.state);
            for event in ci::events(
                &project.slug,
                candidate.as_ref(),
                candidate_state,
                head.as_ref(),
                head_state,
            ) {
                append(&event);
            }
            for ((branch, head), observed) in dependencies.into_iter().zip(states) {
                for check in observed.failed_checks {
                    append(&Event::DependencyFailing {
                        project: project.slug.clone(),
                        branch: branch.clone(),
                        head: head.clone(),
                        check_run_id: check.id,
                        check_name: check.name,
                        conclusion: check.conclusion,
                    });
                }
            }
        }
        Ok(CiRead::Deferred(until)) => {
            tracing::debug!(event = "ci_poll_deferred", project = %project.slug, until = %until);
        }
        Err(err) => tracing::warn!(event = "ci_unreadable", project = %project.slug, error = %err),
    }
}
