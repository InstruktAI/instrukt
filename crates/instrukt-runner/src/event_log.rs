//! The event log in `state/runner/` (`runner` domain, "Domain events
//! emitted" and "Domain events consumed"; `event-activation`, "Delivery and
//! restart boundary").
//!
//! The log is `events.jsonl`: one JSON object per line, the event's fields
//! under its vocabulary name (`"event": "runner.upstream_changed"`) plus a
//! sequence number `seq`, starting at 1 and increasing by one per appended
//! event. An event's idempotency key is its whole payload, so appending an
//! event equal to a logged one changes nothing. `cursors.json` maps each
//! consumer to the highest `seq` it has acknowledged; a consumer without a
//! cursor has acknowledged nothing. Every write takes `events.lock` and
//! replaces the file atomically.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::events::{AppendOutcome, Event, EventLog, EventLogError};
use crate::state::{self, RunnerState, StateError};

/// One logged event with its position in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
}

/// The file-backed event log of one home.
#[derive(Debug, Clone)]
pub struct FileEventLog {
    state: RunnerState,
}

impl FileEventLog {
    pub fn new(state: RunnerState) -> Self {
        Self { state }
    }

    /// Every logged event after `consumer`'s cursor that `wanted` selects,
    /// oldest first.
    pub fn pending(
        &self,
        consumer: &str,
        wanted: impl Fn(&Event) -> bool,
    ) -> Result<Vec<LoggedEvent>, EventLogError> {
        let cursor = self.cursors()?.get(consumer).copied().unwrap_or(0);
        Ok(self
            .read()?
            .into_iter()
            .filter(|logged| logged.seq > cursor && wanted(&logged.event))
            .collect())
    }

    /// Moves `consumer`'s cursor to `seq`; a cursor never moves back.
    pub fn acknowledge(&self, consumer: &str, seq: u64) -> Result<(), EventLogError> {
        let _guard = state::lock(&self.state.events_lock()).map_err(|e| self.wrap(e))?;
        let mut cursors = self.cursors()?;
        let cursor = cursors.entry(consumer.to_owned()).or_insert(0);
        if seq > *cursor {
            *cursor = seq;
            state::write_json(&self.state.cursors(), &cursors).map_err(|e| self.wrap(e))?;
        }
        Ok(())
    }

    fn cursors(&self) -> Result<BTreeMap<String, u64>, EventLogError> {
        Ok(state::read_json(&self.state.cursors())
            .map_err(|e| self.wrap(e))?
            .unwrap_or_default())
    }

    fn read(&self) -> Result<Vec<LoggedEvent>, EventLogError> {
        let path = self.state.events_log();
        let Some(text) = state::read_text(&path).map_err(|e| self.wrap(e))? else {
            return Ok(Vec::new());
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
            .map(|(index, line)| {
                serde_json::from_str(line).map_err(|err| EventLogError::Corrupt {
                    path: path.clone(),
                    message: format!("line {}: {err}", index + 1),
                })
            })
            .collect()
    }

    fn wrap(&self, err: StateError) -> EventLogError {
        match err {
            StateError::Read { path, source } => EventLogError::Io { path, source },
            StateError::Corrupt { path, message } => EventLogError::Corrupt { path, message },
            other => EventLogError::Io {
                path: self.state.dir().to_path_buf(),
                source: std::io::Error::other(other.to_string()),
            },
        }
    }
}

impl EventLog for FileEventLog {
    fn append(&self, event: &Event) -> Result<AppendOutcome, EventLogError> {
        let _guard = state::lock(&self.state.events_lock()).map_err(|e| self.wrap(e))?;
        let mut logged = self.read()?;
        if logged.iter().any(|entry| &entry.event == event) {
            tracing::debug!(event = "runner_event_already_logged", name = event.name(), project = %event.project());
            return Ok(AppendOutcome::AlreadyLogged);
        }
        let seq = logged.last().map_or(1, |last| last.seq + 1);
        logged.push(LoggedEvent {
            seq,
            event: event.clone(),
        });
        let mut text = String::new();
        for entry in &logged {
            let line = serde_json::to_string(entry).map_err(|err| EventLogError::Corrupt {
                path: self.state.events_log(),
                message: err.to_string(),
            })?;
            text.push_str(&line);
            text.push('\n');
        }
        state::write_bytes(&self.state.events_log(), text.as_bytes()).map_err(|e| self.wrap(e))?;
        tracing::info!(event = "runner_event_logged", name = event.name(), project = %event.project(), seq);
        Ok(AppendOutcome::Appended)
    }
}
