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

use instrukt_core::config::{ConfigError, Configuration, Env, Module};
use instrukt_core::fsx::{FsError, LockGuard};
use instrukt_core::github;
use instrukt_core::home::Home;
use instrukt_core::ids::CommitSha;
use instrukt_core::publication::PublishedRecord;
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

/// A runner invocation could not run.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error(transparent)]
    Links(#[from] crate::links::LinkError),
    #[error(transparent)]
    History(#[from] crate::history::HistoryError),
    #[error("the event worker panicked; retained entries remain recoverable")]
    EventWorkerPanicked,
    #[error("the activation wake channel failed; accepted work remains pending")]
    ActivationWakeFailed,
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

/// Every published project whose effective configuration enables `todos`
/// or `events`. A project whose checkout is gone or whose configuration
/// cannot be read is skipped with a warning.
pub fn considered_projects(home: &Home) -> Result<Vec<Considered>, RunnerError> {
    let mut considered = Vec::new();
    for record in PublishedRecord::load_all(home)? {
        if !record.checkout.is_dir() {
            tracing::warn!(event = "project_checkout_missing", project = %record.project, checkout = %record.checkout.display());
            continue;
        }
        let config = match Configuration::load(home, Some(&record.checkout)) {
            Ok(config) => config,
            Err(err) => {
                tracing::warn!(event = "project_config_unreadable", project = %record.project, error = %err);
                continue;
            }
        };
        let todos = config.module_enabled(Module::Todos)?;
        let events = config.module_enabled(Module::Events)?;
        if todos || events {
            considered.push(Considered {
                project: ProjectRef {
                    slug: record.project,
                    root: record.checkout,
                },
                config,
                todos,
                events,
            });
        }
    }
    Ok(considered)
}

/// Whether a tick did its work or found the runner busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Done,
    /// Another invocation holds the runner lock.
    Busy,
}

/// One wake-up at `now`; exits at once when the runner lock is held.
pub fn tick(cx: &RunnerContext<'_>, now: Timestamp) -> Result<TickOutcome, RunnerError> {
    let state = RunnerState::new(cx.home);
    let Some(_runner) = LockGuard::try_acquire(&cx.home.runner_lock_file())? else {
        return Ok(TickOutcome::Busy);
    };
    let Some(invocation) = crate::keyed::RuntimeInvocation::try_acquire(cx.home)? else {
        tracing::info!(event = "tick_runner_busy");
        return Ok(TickOutcome::Busy);
    };
    let considered = considered_projects(cx.home)?;
    let platform = event_platform(cx, &state, &considered)?;
    wake(
        cx,
        &state,
        now,
        &considered,
        platform.as_ref(),
        true,
        &invocation,
    )?;
    Ok(TickOutcome::Done)
}

/// The long-lived runner: a wake-up each `interval` until the process is
/// stopped, holding the runner lock throughout.
pub fn run(
    cx: &RunnerContext<'_>,
    clock: impl Fn() -> Timestamp + Send + Sync + 'static,
    interval: Duration,
) -> Result<(), RunnerError> {
    let state = RunnerState::new(cx.home);
    let _runner = LockGuard::acquire(&cx.home.runner_lock_file(), RUN_LOCK_WAIT)?;
    // Initialize the shared schema before independent workers open connections.
    drop(instrukt_core::state_db::StateDb::open(cx.home).map_err(StateError::from)?);
    let clock: Arc<dyn Fn() -> Timestamp + Send + Sync> = Arc::new(clock);
    let mut platform: Option<EventPlatform> = None;
    let mut _readers = None;
    let history = match Configuration::load_global(cx.home)?.runner_runtime()? {
        Some(runtime)
            if crate::runtime::adapter(runtime)
                .supports(crate::runtime::Capability::ConversationSource) =>
        {
            Some(crate::history::start(cx.home, cx.env)?)
        }
        Some(runtime) => {
            tracing::warn!(
                event = "history_source_unsupported",
                runtime = runtime.name()
            );
            None
        }
        None => None,
    };
    let mut relay = match Configuration::load_global(cx.home)?.runner_runtime()? {
        Some(_) => match crate::links::Worker::start(cx.home, cx.env) {
            Ok(worker) => Some(worker),
            Err(error) => {
                crate::links::record_failure(cx.home, &error);
                None
            }
        },
        None => None,
    };
    let mut history_search = None;
    tracing::info!(event = "runner_started");
    loop {
        if let Some(relay) = &mut relay
            && let Err(error) = relay.check()
        {
            crate::links::record_failure(cx.home, &error);
        }
        let considered = considered_projects(cx.home)?;
        if platform.is_none()
            && let Some(prepared) = event_platform(cx, &state, &considered)?
        {
            history_search = Some(crate::history::SearchWorker::start(
                cx.home,
                cx.env,
                &prepared.transport,
            )?);
            _readers = Some(prepared.runtime.start(Arc::clone(&clock))?);
            platform = Some(prepared);
        }
        let events_enabled = Configuration::load_global(cx.home)?.module_enabled(Module::Events)?
            || considered.iter().any(|project| project.events);
        let enabled_platform = platform.as_ref().filter(|_| events_enabled);
        let invocation = crate::keyed::RuntimeInvocation::acquire(cx.home, RUN_LOCK_WAIT)?;
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
        drop(invocation);
        if let Some(history) = &history {
            history.check()?;
        }
        if let Some(search) = &history_search {
            search.check()?;
        }
        if let Some(platform) = &platform {
            let receiver = platform
                .wakeups
                .lock()
                .map_err(|_| RunnerError::ActivationWakeFailed)?;
            match receiver.recv_timeout(interval) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(RunnerError::ActivationWakeFailed);
                }
            }
        } else {
            std::thread::sleep(interval);
        }
    }
}

struct EventPlatform {
    transport: ScopedTransport,
    runtime: ConsumerRuntime,
    wakeups: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

fn event_platform(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    projects: &[Considered],
) -> Result<Option<EventPlatform>, RunnerError> {
    if !Configuration::load_global(cx.home)?.module_enabled(Module::Events)?
        && !projects.iter().any(|project| project.events)
    {
        return Ok(None);
    }
    let transport =
        ScopedTransport::from_config(&Configuration::load_global(cx.home)?, cx.env, state)?;
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
    if let Some(runtime) = Configuration::load_global(cx.home)?.runner_runtime()? {
        let adapter = crate::runtime::adapter(runtime);
        if let Err(error) = crate::_attention::approvals(invocation, adapter.as_ref(), now) {
            tracing::warn!(event="approval_attention_failed",error=%error);
        }
        if let Err(error) = crate::inputs::pump_all(invocation, adapter.as_ref(), now) {
            tracing::warn!(event="pending_input_failed",error=%error);
        }
        if let Err(error) = crate::_attention::settle_approvals(state) {
            tracing::warn!(event="approval_settlement_failed",error=%error);
        }
        if let Err(error) = standing::settle(state) {
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

    // CI is read for the active candidate and for remote canonical main's
    // head, in one conditional pass.
    let candidate = observed.candidate.clone();
    let head = observed.remote_main.clone();
    if candidate.is_none() && head.is_none() {
        return;
    }
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
    let client = CiClient::new(
        &api_url,
        &repository,
        github::credentials(cx.env),
        state,
        &project.slug,
    );
    let commits: Vec<&CommitSha> = candidate.iter().chain(head.iter()).collect();
    match client.read(&commits, now) {
        Ok(CiRead::States(states)) => {
            let mut states = states.into_iter();
            let candidate_state = candidate.as_ref().and_then(|_| states.next());
            let head_state = head.as_ref().and_then(|_| states.next());
            for event in ci::events(
                &project.slug,
                candidate.as_ref(),
                candidate_state,
                head.as_ref(),
                head_state,
            ) {
                append(&event);
            }
        }
        Ok(CiRead::Deferred(until)) => {
            tracing::debug!(event = "ci_poll_deferred", project = %project.slug, until = %until);
        }
        Err(err) => tracing::warn!(event = "ci_unreadable", project = %project.slug, error = %err),
    }
}
