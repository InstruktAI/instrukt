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
    let mut bindings: BTreeMap<String, SessionRef> =
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
    let started = sessions.start(
        &order,
        &StartRequest {
            message: &prepared.message,
            worktree: None,
            cwd: Some(root),
        },
    )?;
    bindings.insert(consumer.clone(), started.session.clone());
    state::write_json(&state.standing(), &bindings)?;
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
}
