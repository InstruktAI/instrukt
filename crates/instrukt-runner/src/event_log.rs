//! The event log in the state database (`runner` domain, "Domain events
//! emitted" and "Domain events consumed"; `event-activation`, "Delivery and
//! restart boundary"; `state-database`, `events`, `consumer_cursors`,
//! `handled_events`).
//!
//! Each event is one `events` row: its vocabulary name, project and payload
//! under a sequence number `seq`, starting at 1 and increasing by one per
//! appended event. An event's idempotency key is its whole payload, so
//! appending an event equal to a logged one changes nothing. A consumer's
//! cursor is the highest `seq` up to which every event it selects is
//! handled; each event handed to it carries a handled mark, committed with
//! the cursor in one transaction, and is never handed to it again.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::events::{AppendOutcome, Event, EventLog, EventLogError};
use crate::state::{self, RunnerState, StateError};

/// One logged event with its position in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
}

/// The event log of one home, stamping what it records at `now`.
#[derive(Debug, Clone)]
pub struct DbEventLog {
    state: RunnerState,
    now: Timestamp,
}

impl DbEventLog {
    pub fn new(state: RunnerState, now: Timestamp) -> Self {
        Self { state, now }
    }

    /// The time this log stamps what it records.
    pub fn now(&self) -> Timestamp {
        self.now
    }

    /// Every logged event after `consumer`'s cursor, not yet handled by it,
    /// that `wanted` selects, oldest first.
    pub fn pending(
        &self,
        consumer: &str,
        wanted: impl Fn(&Event) -> bool,
    ) -> Result<Vec<LoggedEvent>, EventLogError> {
        let logged = self.state.read(|conn, path| {
            let sql = state::sql(path);
            let mut statement = conn
                .prepare(
                    "SELECT seq, name, payload FROM events
                     WHERE seq > COALESCE((SELECT seq FROM consumer_cursors WHERE consumer = ?1), 0)
                       AND seq NOT IN (SELECT seq FROM handled_events WHERE consumer = ?1)
                     ORDER BY seq",
                )
                .map_err(&sql)?;
            let rows = statement
                .query_map(params![consumer], |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(&sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(&sql)?;
            rows.into_iter()
                .map(|(seq, name, payload)| {
                    Ok(LoggedEvent {
                        seq,
                        event: event_of(path, &name, &payload)?,
                    })
                })
                .collect::<Result<Vec<_>, StateError>>()
        })?;
        Ok(logged
            .unwrap_or_default()
            .into_iter()
            .filter(|logged| wanted(&logged.event))
            .collect())
    }

    /// Records that `consumer` was handed the events `seqs` (oldest first):
    /// a handled mark for each, and its cursor moved to the last; a cursor
    /// never moves back. One transaction.
    pub fn acknowledge(&self, consumer: &str, seqs: &[u64]) -> Result<(), EventLogError> {
        let Some(last) = seqs.iter().copied().max() else {
            return Ok(());
        };
        let at = self.now.to_string();
        self.state.write(|tx| {
            let path = state::db_path(tx);
            let sql = state::sql(&path);
            for seq in seqs {
                tx.execute(
                    "INSERT OR IGNORE INTO handled_events (consumer, seq, handled_at)
                     VALUES (?1, ?2, ?3)",
                    params![consumer, seq, at],
                )
                .map_err(&sql)?;
            }
            tx.execute(
                "INSERT INTO consumer_cursors (consumer, seq) VALUES (?1, ?2)
                 ON CONFLICT (consumer) DO UPDATE SET seq = MAX(seq, excluded.seq)",
                params![consumer, last],
            )
            .map_err(&sql)?;
            Ok::<_, EventLogError>(())
        })
    }

    /// Appends `event` in a transaction of its own.
    pub fn log(&self, event: &Event) -> Result<AppendOutcome, EventLogError> {
        self.state.write(|tx| self.append(tx, event))
    }
}

impl EventLog for DbEventLog {
    fn append(&self, tx: &Transaction<'_>, event: &Event) -> Result<AppendOutcome, EventLogError> {
        append(tx, event, self.now)
    }
}

/// Appends `event` inside `tx`, unless an equal event is already logged.
pub(crate) fn append(
    tx: &Connection,
    event: &Event,
    now: Timestamp,
) -> Result<AppendOutcome, EventLogError> {
    let path = state::db_path(tx);
    let sql = state::sql(&path);
    let key = serde_json::to_string(event).map_err(|e| state::invalid(&path, e))?;
    let known: Option<u64> = tx
        .query_row(
            "SELECT seq FROM events WHERE idempotency_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(&sql)?;
    if known.is_some() {
        tracing::debug!(event = "runner_event_already_logged", name = event.name(), project = %event.project());
        return Ok(AppendOutcome::AlreadyLogged);
    }
    let seq: u64 = tx
        .query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM events", [], |row| {
            row.get(0)
        })
        .map_err(&sql)?;
    tx.execute(
        "INSERT INTO events (seq, name, project, payload, idempotency_key, logged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            seq,
            event.name(),
            event.project().as_str(),
            payload_of(&path, event)?,
            key,
            now.to_string()
        ],
    )
    .map_err(&sql)?;
    tracing::info!(event = "runner_event_logged", name = event.name(), project = %event.project(), seq);
    Ok(AppendOutcome::Appended)
}

/// The event's fields without its vocabulary name.
fn payload_of(path: &std::path::Path, event: &Event) -> Result<String, StateError> {
    let mut value = serde_json::to_value(event).map_err(|e| state::invalid(path, e))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("event");
    }
    Ok(value.to_string())
}

fn event_of(path: &std::path::Path, name: &str, payload: &str) -> Result<Event, StateError> {
    let mut value: serde_json::Value =
        serde_json::from_str(payload).map_err(|e| state::invalid(path, e))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| state::invalid(path, "an event payload that is not an object"))?;
    object.insert("event".to_owned(), serde_json::Value::from(name));
    serde_json::from_value(value).map_err(|e| state::invalid(path, e))
}
