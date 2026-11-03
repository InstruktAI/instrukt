//! Runner invocations (`work-dispatch`, "Execution contract"; `runner`
//! domain, "Runner invocation"): `instrukt tick` performs one wake-up's
//! work and `instrukt run` repeats it until stopped. One runner lock per
//! machine serializes them; a tick exits at once while it is held.
//!
//! A wake-up records itself, then considers every published project whose
//! effective configuration enables `events` or `todos`: with `events` on it
//! reads the event sources and delivers pending events to standing agents;
//! with `todos` on it dispatches eligible todos.

use std::time::Duration;

use instrukt_core::config::{ConfigError, Configuration, Env, Module};
use instrukt_core::fsx::{FsError, LockGuard};
use instrukt_core::github;
use instrukt_core::home::Home;
use instrukt_core::index::GlobalIndex;
use instrukt_core::publication::PublishedRecord;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::ci::{CiClient, CiRead};
use crate::dispatch::{self, DispatchContext};
use crate::event_log::FileEventLog;
use crate::events::{Event, EventLog};
use crate::interfaces::{ProjectRef, RunnerWiring};
use crate::sources;
use crate::standing;
use crate::state::{RunnerState, StateError};
use crate::ticks;

/// How long `instrukt run` waits for a tick in progress to finish.
const RUN_LOCK_WAIT: Duration = Duration::from_secs(120);

/// A runner invocation could not run.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
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
    wake(cx, &state, now)?;
    Ok(TickOutcome::Done)
}

/// The long-lived runner: a wake-up each `interval` until the process is
/// stopped, holding the runner lock throughout.
pub fn run(
    cx: &RunnerContext<'_>,
    clock: impl Fn() -> Timestamp,
    interval: Duration,
) -> Result<(), RunnerError> {
    let state = RunnerState::new(cx.home);
    let _lock = LockGuard::acquire(&state.runner_lock(), RUN_LOCK_WAIT)?;
    tracing::info!(event = "runner_started");
    loop {
        if let Err(err) = wake(cx, &state, clock()) {
            tracing::error!(event = "runner_wakeup_failed", error = %err);
        }
        std::thread::sleep(interval);
    }
}

fn wake(cx: &RunnerContext<'_>, state: &RunnerState, now: Timestamp) -> Result<(), RunnerError> {
    let previous = ticks::last_wakeup(state)?;
    ticks::record_wakeup(state, now)?;
    let log = FileEventLog::new(state.clone());
    let index = match GlobalIndex::load(cx.home) {
        Ok(index) => index.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(event = "index_unreadable", error = %err);
            GlobalIndex::default()
        }
    };
    let dispatch_cx = DispatchContext {
        home: cx.home,
        env: cx.env,
        now,
        tz: cx.tz,
        previous_wake: previous,
        state,
    };
    for considered in considered_projects(cx.home)? {
        let project = &considered.project;
        if considered.events {
            read_sources(cx, state, &log, &considered, now);
            for procedure in standing::subscribers(&index, &project.slug) {
                if let Err(err) = standing::deliver(
                    state,
                    &log,
                    &considered.config,
                    cx.env,
                    procedure,
                    &project.slug,
                    &project.root,
                ) {
                    tracing::warn!(event = "standing_delivery_failed", project = %project.slug, procedure = %procedure.id, error = %err);
                }
            }
        }
        if considered.todos {
            dispatch::dispatch_project(&dispatch_cx, &considered.config, cx.wiring, project);
        }
    }
    tracing::debug!(event = "runner_wakeup_done", at = %now);
    Ok(())
}

/// Reads the project's event sources and appends what they found.
fn read_sources(
    cx: &RunnerContext<'_>,
    state: &RunnerState,
    log: &FileEventLog,
    considered: &Considered,
    now: Timestamp,
) {
    let project = &considered.project;
    let mut append = |event: &Event| match log.append(event) {
        Ok(_) => true,
        Err(err) => {
            tracing::warn!(event = "event_not_logged", name = event.name(), project = %project.slug, error = %err);
            false
        }
    };
    match considered.config.events_watch() {
        Ok(watched) if !watched.is_empty() => {
            if let Err(err) = sources::upstream_changes(
                state,
                &project.slug,
                &project.root,
                &watched,
                &mut append,
            ) {
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
    let Some(candidate) = observed.candidate else {
        return;
    };
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
    match client.read(&candidate, now) {
        Ok(CiRead::State(ci_state)) => {
            if let Some(result) = ci_state.completed() {
                append(&Event::CiCompleted {
                    project: project.slug.clone(),
                    candidate,
                    result,
                });
            }
        }
        Ok(CiRead::Deferred(until)) => {
            tracing::debug!(event = "ci_poll_deferred", project = %project.slug, until = %until);
        }
        Err(err) => tracing::warn!(event = "ci_unreadable", project = %project.slug, error = %err),
    }
}
