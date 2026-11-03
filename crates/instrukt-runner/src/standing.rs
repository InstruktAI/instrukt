//! Standing agents (`event-activation`, "Subscription contract";
//! `runner` domain, "Standing-agent session binding"): a procedure whose
//! frontmatter subscribes to events is started or woken for a project's
//! pending events.
//!
//! The consumer of the event log is the pair (subscribing procedure,
//! project), and so is its session binding, kept in `standing.json`. A
//! published procedure subscribes for every project; a project's local
//! procedure only for its own project.
//!
//! Delivery: `event_delivery: sequential` (the default) hands over the
//! oldest pending event, `batched` every pending one. A bound session that
//! is `ended ok` or `ended failed` is woken with `send`; one that is
//! `running` or `needs-input` receives events only when the procedure is
//! `interruptible`, otherwise they wait; a `gone` session, or none, is
//! replaced by a new session started into the procedure. An unobservable
//! session is retried next tick. The cursor advances only after the start
//! or send succeeded.

use std::collections::BTreeMap;
use std::path::Path;

use instrukt_core::config::{Configuration, Env};
use instrukt_core::ids::{ProjectSlug, SessionRef};
use instrukt_core::index::{GlobalIndex, IndexEntry, IndexScope};
use serde::{Deserialize, Serialize};

use crate::agents::{agent_order, global_order};
use crate::event_log::{FileEventLog, LoggedEvent};
use crate::events::Event;
use crate::launch;
use crate::sessions::{Observation, SessionState, Sessions, StartRequest};
use crate::state::{self, RunnerState, StateError};

/// Whether a subscription pattern selects the event `name`: an exact name,
/// whole leading segments followed by `.*`, or `*`.
pub fn pattern_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.strip_suffix(".*") {
        Some(prefix) => name
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.')),
        None => pattern == name,
    }
}

/// The procedures subscribing for `project`.
pub fn subscribers<'i>(index: &'i GlobalIndex, project: &ProjectSlug) -> Vec<&'i IndexEntry> {
    index
        .snippets
        .iter()
        .filter(|entry| entry.taxonomy == "procedure" && !entry.subscriptions.is_empty())
        .filter(|entry| entry.scope == IndexScope::Global || &entry.source_project == project)
        .collect()
}

/// What a delivery attempt did for one consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Nothing,
    Started(SessionRef),
    Woken(SessionRef),
    /// Events wait for the session's next wake boundary.
    Waiting(SessionRef),
    /// The session could not be observed; retried next tick.
    Unobservable(SessionRef),
    /// An earlier start's result was never recorded; it is not repeated.
    StartUncertain,
}

/// A standing-agent start begun and not yet bound.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StartAttempt {
    /// The uncertain start was reported as an anomaly.
    #[serde(default)]
    reported: bool,
}

/// A delivery could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum StandingError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Log(#[from] crate::events::EventLogError),
    #[error(transparent)]
    Launch(#[from] launch::LaunchError),
    #[error(transparent)]
    Sessions(#[from] crate::sessions::SessionsError),
}

/// A standing agent's start could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum ResolveStartError {
    #[error("`{0}` is not `<procedure>@<project>`")]
    Consumer(String),
    #[error("{0} has no start awaiting resolution")]
    NothingUncertain(String),
    #[error("{session} belongs to no runtime of the runtime mapping")]
    UnknownRuntime { session: SessionRef },
    #[error(transparent)]
    State(#[from] StateError),
}

/// How an uncertain start is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartResolution {
    /// The runtime did create this session: bind it.
    Session(SessionRef),
    /// No session was created: allow a fresh start.
    None,
}

/// The standing agents' durable records: bound sessions and starts not
/// yet bound. Both files are read and written together under one lock,
/// each change applied to a fresh read, so a delivery and a
/// `resolve-start` running at once both keep their changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Records {
    pub bindings: BTreeMap<String, SessionRef>,
    starts: BTreeMap<String, StartAttempt>,
}

/// Whether a consumer may start a session now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartGate {
    /// The attempt is recorded; ask the runtime.
    Begin,
    /// An earlier start is uncertain and was just reported.
    ReportUncertain,
    /// An earlier start is uncertain and was reported before.
    Uncertain,
}

impl Records {
    /// Records a start attempt for `consumer`, unless one is uncertain.
    pub fn begin_start(&mut self, consumer: &str) -> StartGate {
        match self.starts.get_mut(consumer) {
            Some(attempt) if attempt.reported => StartGate::Uncertain,
            Some(attempt) => {
                attempt.reported = true;
                StartGate::ReportUncertain
            }
            None => {
                self.starts
                    .insert(consumer.to_owned(), StartAttempt::default());
                StartGate::Begin
            }
        }
    }

    /// `consumer`'s start created `session`.
    pub fn started(&mut self, consumer: &str, session: &SessionRef) {
        self.bindings.insert(consumer.to_owned(), session.clone());
        self.starts.remove(consumer);
    }

    /// `consumer`'s start certainly created no session.
    pub fn start_failed(&mut self, consumer: &str) {
        self.starts.remove(consumer);
    }

    /// Settles `consumer`'s uncertain start.
    pub fn resolve(
        &mut self,
        consumer: &str,
        resolution: &StartResolution,
    ) -> Result<(), ResolveStartError> {
        if self.starts.remove(consumer).is_none() {
            return Err(ResolveStartError::NothingUncertain(consumer.to_owned()));
        }
        if let StartResolution::Session(session) = resolution {
            self.bindings.insert(consumer.to_owned(), session.clone());
        }
        Ok(())
    }
}

/// Applies `change` to the standing records under their lock, on a fresh
/// read, and writes them back.
fn update<T, E: From<StateError>>(
    state: &RunnerState,
    change: impl FnOnce(&mut Records) -> Result<T, E>,
) -> Result<T, E> {
    let _guard = state::lock(&state.standing_lock())?;
    let mut records = Records {
        bindings: state::read_json(&state.standing())?.unwrap_or_default(),
        starts: state::read_json(&state.standing_starts())?.unwrap_or_default(),
    };
    let before = records.clone();
    let answer = change(&mut records)?;
    if records.bindings != before.bindings {
        state::write_json(&state.standing(), &records.bindings)?;
    }
    if records.starts != before.starts {
        state::write_json(&state.standing_starts(), &records.starts)?;
    }
    Ok(answer)
}

/// The command line that resolves `consumer`'s uncertain start, as its
/// anomaly names it.
pub fn resolve_start_command(consumer: &str) -> String {
    format!("instrukt sessions resolve-start {consumer} --session <runtime:id> | --none")
}

/// Resolves the uncertain start of the standing agent `consumer`
/// (`<procedure>@<project>`): binds the session the runtime created, or
/// clears the attempt so the next tick starts afresh. Pending events stay
/// pending; the next tick delivers them to the resolved agent.
pub fn resolve_start(
    state: &RunnerState,
    config: &Configuration,
    consumer: &str,
    resolution: &StartResolution,
) -> Result<(), ResolveStartError> {
    let well_formed = consumer
        .rsplit_once('@')
        .is_some_and(|(procedure, project)| {
            instrukt_core::ids::SnippetId::parse(procedure).is_ok()
                && ProjectSlug::parse(project).is_ok()
        });
    if !well_formed {
        return Err(ResolveStartError::Consumer(consumer.to_owned()));
    }
    if let StartResolution::Session(session) = resolution
        && !config
            .runtimes()
            .iter()
            .any(|entry| entry.name == session.runtime.as_str())
    {
        return Err(ResolveStartError::UnknownRuntime {
            session: session.clone(),
        });
    }
    update(state, |records| records.resolve(consumer, resolution))?;
    tracing::info!(event = "standing_start_resolved", consumer, resolution = ?resolution);
    Ok(())
}

/// Delivers `project`'s pending events to `procedure`.
pub fn deliver(
    state: &RunnerState,
    log: &FileEventLog,
    config: &Configuration,
    env: &Env,
    procedure: &IndexEntry,
    project: &ProjectSlug,
    root: &Path,
) -> Result<Delivery, StandingError> {
    let consumer = format!("{}@{project}", procedure.id);
    let pending = log.pending(&consumer, |event| {
        event.project() == project
            && procedure
                .subscriptions
                .iter()
                .any(|pattern| pattern_matches(pattern, event.name()))
    })?;
    let batched = procedure.event_delivery.as_deref() == Some("batched");
    let batch: Vec<&LoggedEvent> = if batched {
        pending.iter().collect()
    } else {
        pending.iter().take(1).collect()
    };
    let Some(last) = batch.last().map(|logged| logged.seq) else {
        return Ok(Delivery::Nothing);
    };
    let text = events_text(batch.iter().map(|logged| &logged.event));
    let sessions = Sessions::new(config, env);
    let bindings: BTreeMap<String, SessionRef> =
        state::read_json(&state.standing())?.unwrap_or_default();
    if let Some(bound) = bindings.get(&consumer).cloned() {
        match sessions.alive(&bound)? {
            Observation::Unobservable(reason) => {
                tracing::warn!(event = "standing_session_unobservable", consumer, session = %bound, reason);
                return Ok(Delivery::Unobservable(bound));
            }
            Observation::State(SessionState::Running | SessionState::NeedsInput)
                if !procedure.interruptible =>
            {
                return Ok(Delivery::Waiting(bound));
            }
            Observation::State(SessionState::Gone) => {}
            Observation::State(_) => {
                sessions.send(&bound, &text)?;
                log.acknowledge(&consumer, last)?;
                tracing::info!(event = "standing_agent_woken", consumer, session = %bound, through = last);
                return Ok(Delivery::Woken(bound));
            }
        }
    }
    let prepared = launch::prepare(procedure, None, Some(&text))?;
    let (global, thinking) = global_order(config);
    let order = agent_order(None, prepared.dispatch.as_ref(), &global, thinking);
    // The start is recorded before the runtime is asked: a start whose
    // result was never recorded (the tick died in between) may have created
    // a session, so it is never repeated; it is reported once instead.
    match update::<_, StateError>(state, |records| Ok(records.begin_start(&consumer)))? {
        StartGate::Begin => {}
        StartGate::ReportUncertain => {
            let door = resolve_start_command(&consumer);
            tracing::warn!(event = "standing_start_uncertain", consumer, resolve = %door);
            crate::anomaly::notify(config, env, &door, None);
            return Ok(Delivery::StartUncertain);
        }
        StartGate::Uncertain => return Ok(Delivery::StartUncertain),
    }
    let started = sessions.start(
        &order,
        &StartRequest {
            message: &prepared.message,
            worktree: None,
            cwd: Some(root),
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(err) => {
            // Only a start that certainly created no session clears its
            // attempt; an uncertain one stays until `resolve-start`.
            if err.start_certainly_failed() {
                update::<_, StateError>(state, |records| {
                    records.start_failed(&consumer);
                    Ok(())
                })?;
            }
            return Err(err.into());
        }
    };
    update::<_, StateError>(state, |records| {
        records.started(&consumer, &started.session);
        Ok(())
    })?;
    log.acknowledge(&consumer, last)?;
    tracing::info!(event = "standing_agent_started", consumer, session = %started.session, through = last);
    Ok(Delivery::Started(started.session))
}

/// The events handed to a standing agent: one JSON object per line.
fn events_text<'e>(events: impl Iterator<Item = &'e Event>) -> String {
    let mut text = String::from("instrukt events:\n");
    for event in events {
        match serde_json::to_string(event) {
            Ok(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            Err(err) => tracing::error!(event = "event_unrenderable", error = %err),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_select_exact_names_whole_segment_prefixes_or_everything() {
        assert!(pattern_matches(
            "runner.ci_completed",
            "runner.ci_completed"
        ));
        assert!(pattern_matches("runner.*", "runner.ci_completed"));
        assert!(pattern_matches("*", "workflow.finalized"));
        assert!(!pattern_matches("run.*", "runner.ci_completed"));
        assert!(!pattern_matches("runner.ci", "runner.ci_completed"));
        assert!(!pattern_matches(
            "runner.ci_completed.*",
            "runner.ci_completed"
        ));
    }

    fn session(id: &str) -> SessionRef {
        SessionRef::parse(&format!("rt:{id}")).unwrap()
    }

    #[test]
    fn an_uncertain_start_is_reported_once_and_never_begun_again_until_resolved() {
        let mut records = Records::default();
        assert_eq!(records.begin_start("a"), StartGate::Begin);
        assert_eq!(records.begin_start("a"), StartGate::ReportUncertain);
        assert_eq!(records.begin_start("a"), StartGate::Uncertain);
        records.resolve("a", &StartResolution::None).unwrap();
        assert_eq!(records.begin_start("a"), StartGate::Begin);
        assert!(matches!(
            records.clone().resolve("b", &StartResolution::None),
            Err(ResolveStartError::NothingUncertain(_))
        ));
    }

    #[test]
    fn one_consumers_start_keeps_anothers_resolution() {
        // A's start is uncertain; B begins a start of its own.
        let mut records = Records::default();
        records.begin_start("a");
        records.begin_start("b");
        // `resolve-start` binds A's session while B's start is under way;
        // B's start then completes on the same records.
        records
            .resolve("a", &StartResolution::Session(session("s1")))
            .unwrap();
        records.started("b", &session("s2"));
        assert_eq!(records.bindings.get("a"), Some(&session("s1")));
        assert_eq!(records.bindings.get("b"), Some(&session("s2")));
        assert_eq!(records.begin_start("a"), StartGate::Begin);
    }
}
