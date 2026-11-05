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
use instrukt_core::index::GlobalIndex;
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
    #[error("the event worker panicked; retained entries remain recoverable")]
    EventWorkerPanicked,
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
    let Some(_lock) = LockGuard::try_acquire(&state.runner_lock())? else {
        tracing::info!(event = "tick_runner_busy");
        return Ok(TickOutcome::Busy);
    };
    let considered = considered_projects(cx.home)?;
    let platform = event_platform(cx, &state, &considered)?;
    wake(cx, &state, now, &considered, platform.as_ref(), true)?;
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
    let _lock = LockGuard::acquire(&state.runner_lock(), RUN_LOCK_WAIT)?;
    let clock: Arc<dyn Fn() -> Timestamp + Send + Sync> = Arc::new(clock);
    let mut platform: Option<EventPlatform> = None;
    let mut _readers = None;
    tracing::info!(event = "runner_started");
    loop {
        let considered = considered_projects(cx.home)?;
        if platform.is_none()
            && let Some(prepared) = event_platform(cx, &state, &considered)?
        {
            _readers = Some(prepared.runtime.start(Arc::clone(&clock))?);
            platform = Some(prepared);
        }
        let enabled_platform = platform
            .as_ref()
            .filter(|_| considered.iter().any(|project| project.events));
        if let Err(err) = wake(cx, &state, clock(), &considered, enabled_platform, false) {
            tracing::error!(event = "runner_wakeup_failed", error = %err);
        }
        std::thread::sleep(interval);
    }
}

struct EventPlatform {
    transport: ScopedTransport,
    runtime: ConsumerRuntime,
}

fn event_platform(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    projects: &[Considered],
) -> Result<Option<EventPlatform>, RunnerError> {
    if !projects.iter().any(|project| project.events) {
        return Ok(None);
    }
    let transport =
        ScopedTransport::from_config(&Configuration::load_global(cx.home)?, cx.env, state)?;
    let computer = instrukt_core::machine::machine_id(cx.home)?;
    let runtime = ConsumerRuntime::new(
        state.clone(),
        transport.clone(),
        computer.as_str(),
        vec![consumers::activation_consumer(cx.home, state)?],
    )?;
    Ok(Some(EventPlatform { transport, runtime }))
}

fn wake(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    now: Timestamp,
    considered: &[Considered],
    platform: Option<&EventPlatform>,
    drain: bool,
) -> Result<(), RunnerError> {
    let previous = ticks::last_wakeup(state)?;
    ticks::record_wakeup(state, now)?;
    let dispatch_cx = DispatchContext {
        home: cx.home,
        env: cx.env,
        now,
        tz: cx.tz,
        previous_wake: previous,
        state,
    };
    std::thread::scope(|threads| -> Result<(), RunnerError> {
        let events = threads.spawn(|| {
            EventWake {
                cx,
                state,
                now,
                considered,
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
    tracing::debug!(event = "runner_wakeup_done", at = %now);
    Ok(())
}

struct EventWake<'a, 'cx> {
    cx: &'a RunnerContext<'cx>,
    state: &'a RunnerState,
    now: Timestamp,
    considered: &'a [Considered],
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
            platform,
            drain,
        } = self;
        let log = DbEventLog::new(state.clone(), now);
        let index = match GlobalIndex::load(cx.home) {
            Ok(index) => index.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(event = "index_unreadable", error = %error);
                GlobalIndex::default()
            }
        };
        let ready =
            platform.is_none_or(|platform| platform.runtime.prepare_scope(EventScope::Local));
        for project in considered.iter().filter(|project| project.events) {
            read_sources(cx, state, &log, project, now);
        }
        if let Some(platform) = platform {
            if ready {
                platform.transport.publish_pending(&log)?;
            }
            if drain {
                platform.runtime.drain(now);
            }
        }
        for project in considered.iter().filter(|project| project.events) {
            for procedure in standing::subscribers(&index, &project.project.slug) {
                if let Err(error) = standing::deliver(
                    state,
                    &log,
                    &project.config,
                    cx.env,
                    procedure,
                    &project.project.slug,
                    &project.project.root,
                ) {
                    tracing::warn!(event = "standing_delivery_failed", project = %project.project.slug, procedure = %procedure.id, error = %error);
                }
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
    let observed = match sources::observe_main(&project.root) {
        Ok(observed) => observed,
        Err(err) => {
            tracing::debug!(event = "main_unobservable", project = %project.slug, error = %err);
            return;
        }
    };
    if let Some(event) = observed.main_ahead(&project.slug) {
        append(&event);
    }
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
