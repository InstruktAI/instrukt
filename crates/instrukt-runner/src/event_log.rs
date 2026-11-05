//! The event log in the state database (`runner` domain, "Domain events
//! emitted" and "Domain events consumed"; `event-activation`, "Delivery and
//! restart boundary"; `state-database`, `events`, `consumer_cursors`,
//! `handled_events`).
//!
//! Stable occurrence identity and publication obligations are separate from local
//! sequence and Redis coordinates. Work appends inside its source transaction.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::events::{
    AppendOutcome, EmissionContext, Event, EventEnvelope, EventHomeOwner, EventLog, EventLogError,
};
use crate::state::{self, RunnerState, StateError};
use crate::transport::{EventScope, TransportCoordinate};

/// A wake-up publishes a bounded batch from each scope independently.
const PUBLICATION_BATCH_SIZE: u64 = 50;

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

    /// Locally produced activations accepted for this procedure but still
    /// awaiting its runtime delivery, oldest first. Retained Redis readers
    /// never use this local scan as transport replay.
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
                     WHERE seq > COALESCE((SELECT seq FROM consumer_cursors WHERE consumer = ?1 AND source = 'local-log'), 0)
                       AND idempotency_key IS NOT NULL
                       AND EXISTS (SELECT 1 FROM procedure_activations a WHERE a.event_id = events.id AND a.procedure || '@' || substr(a.exact_home, 9) = ?1)
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
                    "INSERT OR IGNORE INTO handled_events (consumer, event_id, seq, handled_at)
                     SELECT ?1, id, ?2, ?3 FROM events WHERE seq = ?2",
                    params![consumer, seq, at],
                )
                .map_err(&sql)?;
            }
            tx.execute(
                "INSERT INTO consumer_cursors (consumer, source, stream, entry_id, seq) VALUES (?1, 'local-log', '', '', ?2)
                 ON CONFLICT (consumer, source, stream) DO UPDATE SET seq = MAX(seq, excluded.seq)",
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

/// One committed source fact still awaiting stream acceptance.
pub struct PendingPublication {
    pub scope: EventScope,
    pub envelope: EventEnvelope,
}

impl DbEventLog {
    /// Oldest unconfirmed facts, bounded independently for each publication scope.
    pub fn unconfirmed(&self) -> Result<Vec<PendingPublication>, EventLogError> {
        Ok(self.state.read(|conn, path| {
            let mut statement = conn.prepare("SELECT envelope FROM events WHERE publication_state = 'pending' AND publication_scope = ?1 ORDER BY seq LIMIT ?2").map_err(state::sql(path))?;
            let mut pending = Vec::new();
            for scope in [EventScope::Local, EventScope::Cluster] {
                let envelopes = statement.query_map(params![scope.as_str(), PUBLICATION_BATCH_SIZE], |row| row.get::<_, String>(0)).map_err(state::sql(path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))?;
                for envelope in envelopes {
                    pending.push(PendingPublication { scope, envelope: EventEnvelope::decode(&envelope).map_err(|error| state::invalid(path, error))? });
                }
            }
            Ok(pending)
        })?.unwrap_or_default())
    }

    pub fn confirm(&self, id: &str, coordinate: &TransportCoordinate) -> Result<(), EventLogError> {
        self.state.write(|tx| {
            let path = state::db_path(tx);
            tx.execute("INSERT OR IGNORE INTO event_coordinates (source, stream, entry_id, seq) SELECT ?1, ?2, ?3, seq FROM events WHERE id = ?4", params![coordinate.source, coordinate.stream, coordinate.entry_id, id]).map_err(state::sql(&path))?;
            tx.execute("UPDATE events SET publication_state = 'confirmed', publication_failure = NULL WHERE id = ?1", [id]).map_err(state::sql(&path))?;
            Ok(())
        })
    }

    pub fn publication_failed(&self, id: &str, failure: &str) -> Result<(), EventLogError> {
        self.state.write(|tx| {
            let path = state::db_path(tx);
            tx.execute("UPDATE events SET publication_failure = ?2 WHERE id = ?1 AND publication_state = 'pending'", params![id, failure]).map_err(state::sql(&path))?;
            Ok(())
        })
    }
}

impl EventLog for DbEventLog {
    fn append(&self, tx: &Transaction<'_>, event: &Event) -> Result<AppendOutcome, EventLogError> {
        let context = EmissionContext {
            source: instrukt_core::machine::machine_id(self.state.home())?.to_string(),
            time: self.now,
            project_owner: EventHomeOwner::Unresolved,
            caller_session: None,
        };
        append(tx, event, &context)
    }
}

/// Appends `event` inside `tx`, unless an equal event is already logged.
pub(crate) fn append(
    tx: &Connection,
    event: &Event,
    context: &EmissionContext,
) -> Result<AppendOutcome, EventLogError> {
    let path = state::db_path(tx);
    let sql = state::sql(&path);
    let key = event.occurrence_id();
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
    let envelope = event.envelope(context)?.encode()?;
    tx.execute(
        "INSERT INTO events (seq, id, name, project, payload, envelope, idempotency_key, logged_at, publication_scope, publication_state)
         VALUES (?1, ?5, ?2, ?3, ?4, ?7, ?5, ?6, 'local', 'pending')",
        params![
            seq,
            event.name(),
            event.project().as_str(),
            payload_of(&path, event)?,
            key,
            context.time.to_string(),
            envelope
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
