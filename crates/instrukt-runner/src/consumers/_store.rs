//! SQLite settlement and retry journal. Callers finish all network work before entering here.

use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, params};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{HANDLED_RETENTION_SECONDS, SettledOutcome};
use crate::events::{EventEnvelope, EventHome};
use crate::state::{self, RunnerState, StateError};
use crate::transport::TransportCoordinate;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Attempt {
    pub attempts: u32,
    pub first_attempt_at: String,
    pub last_attempt_at: String,
    pub failure: String,
}

pub(super) fn materialize(
    state: &RunnerState,
    event: &EventEnvelope,
    coordinate: &TransportCoordinate,
    now: Timestamp,
) -> Result<u64, StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let project = match &event.home { EventHome::Project(slug) => Some(slug.as_str()), _ => None };
        let envelope = event.encode().map_err(|e| state::invalid(&path, e))?;
        tx.execute("INSERT OR IGNORE INTO events (id, name, project, payload, envelope, logged_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![event.id, event.event_type, project, serde_json::Value::Object(event.data.clone()).to_string(), envelope, now.to_string()]).map_err(state::sql(&path))?;
        let seq: u64 = tx.query_row("SELECT seq FROM events WHERE id = ?1", [&event.id], |row| row.get(0)).map_err(state::sql(&path))?;
        tx.execute("INSERT OR IGNORE INTO event_coordinates (source, stream, entry_id, seq) VALUES (?1, ?2, ?3, ?4)", params![coordinate.source, coordinate.stream, coordinate.entry_id, seq]).map_err(state::sql(&path))?;
        Ok(seq)
    })
}

pub(super) fn handled(
    state: &RunnerState,
    consumer: &str,
    id: &str,
    now: Timestamp,
) -> Result<bool, StateError> {
    state
        .read(|conn, path| {
            let at: Option<String> = conn
                .query_row(
                    "SELECT handled_at FROM handled_events WHERE consumer = ?1 AND event_id = ?2",
                    [consumer, id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(state::sql(path))?;
            at.map(|at| {
                let at: Timestamp = at.parse().map_err(|e| state::invalid(path, e))?;
                Ok(now.duration_since(at)
                    < jiff::SignedDuration::from_secs(HANDLED_RETENTION_SECONDS))
            })
            .transpose()
            .map(|v| v.unwrap_or(false))
        })
        .map(|v| v.unwrap_or(false))
}

pub(super) fn attempt(
    state: &RunnerState,
    consumer: &str,
    coordinate: &TransportCoordinate,
) -> Result<Option<Attempt>, StateError> {
    state.read(|conn, path| {
        conn.query_row("SELECT attempts, first_attempt_at, last_attempt_at, failure FROM event_attempts WHERE consumer = ?1 AND source = ?2 AND stream = ?3 AND entry_id = ?4", params![consumer, coordinate.source, coordinate.stream, coordinate.entry_id], |row| Ok(Attempt { attempts: row.get(0)?, first_attempt_at: row.get(1)?, last_attempt_at: row.get(2)?, failure: row.get(3)? })).optional().map_err(state::sql(path))
    }).map(Option::flatten)
}

/// Record the invocation before the external effect so interrupted attempts have evidence.
pub(super) fn begin_attempt(
    state: &RunnerState,
    consumer: &str,
    coordinate: &TransportCoordinate,
    now: Timestamp,
) -> Result<Attempt, StateError> {
    let mut attempt = attempt(state, consumer, coordinate)?.unwrap_or_else(|| Attempt {
        attempts: 0,
        first_attempt_at: now.to_string(),
        last_attempt_at: now.to_string(),
        failure: String::new(),
    });
    attempt.attempts += 1;
    attempt.last_attempt_at = now.to_string();
    attempt.failure = "handling interrupted before its outcome was recorded".into();
    save_attempt(state, consumer, coordinate, &attempt)?;
    Ok(attempt)
}

pub(super) fn save_attempt(
    state: &RunnerState,
    consumer: &str,
    coordinate: &TransportCoordinate,
    attempt: &Attempt,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        tx.execute("INSERT INTO event_attempts (consumer, source, stream, entry_id, attempts, first_attempt_at, last_attempt_at, failure) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (consumer, source, stream, entry_id) DO UPDATE SET attempts = excluded.attempts, last_attempt_at = excluded.last_attempt_at, failure = excluded.failure", params![consumer, coordinate.source, coordinate.stream, coordinate.entry_id, attempt.attempts, attempt.first_attempt_at, attempt.last_attempt_at, attempt.failure]).map_err(state::sql(&path))?;
        Ok(())
    })
}

pub(super) struct Settlement<'a> {
    pub consumer: &'a str,
    pub coordinate: &'a TransportCoordinate,
    pub seq: Option<u64>,
    pub event_id: Option<&'a str>,
    pub outcome: &'a SettledOutcome,
    pub now: Timestamp,
}

/// Successful outcome, occurrence mark and settled source cursor commit together.
pub(super) fn settle(state: &RunnerState, settlement: &Settlement<'_>) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let s = settlement;
        let encoded = serde_json::to_string(s.outcome).map_err(|e| state::invalid(&path, e))?;
        tx.execute("INSERT OR IGNORE INTO consumer_outcomes (consumer, source, stream, entry_id, outcome, settled_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![s.consumer, s.coordinate.source, s.coordinate.stream, s.coordinate.entry_id, encoded, s.now.to_string()]).map_err(state::sql(&path))?;
        tx.execute("DELETE FROM handled_events WHERE CAST(strftime('%s', handled_at) AS INTEGER) < ?1", [s.now.as_second().saturating_sub(HANDLED_RETENTION_SECONDS)]).map_err(state::sql(&path))?;
        if let (SettledOutcome::Accepted(_), Some(id)) = (s.outcome, s.event_id) {
            tx.execute("INSERT INTO handled_events (consumer, event_id, seq, handled_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (consumer, event_id) DO UPDATE SET handled_at = excluded.handled_at, seq = excluded.seq", params![s.consumer, id, s.seq, s.now.to_string()]).map_err(state::sql(&path))?;
        }
        cursor(tx, s)?;
        tx.execute("DELETE FROM event_attempts WHERE consumer = ?1 AND source = ?2 AND stream = ?3 AND entry_id = ?4", params![s.consumer, s.coordinate.source, s.coordinate.stream, s.coordinate.entry_id]).map_err(state::sql(&path))?;
        Ok(())
    })
}

fn cursor(conn: &Connection, s: &Settlement<'_>) -> Result<(), StateError> {
    let path = state::db_path(conn);
    let prior: Option<String> = conn.query_row("SELECT entry_id FROM consumer_cursors WHERE consumer = ?1 AND source = ?2 AND stream = ?3", params![s.consumer, s.coordinate.source, s.coordinate.stream], |row| row.get(0)).optional().map_err(state::sql(&path))?;
    if prior
        .as_deref()
        .is_some_and(|prior| stream_position(prior) >= stream_position(&s.coordinate.entry_id))
    {
        return Ok(());
    }
    conn.execute("INSERT INTO consumer_cursors (consumer, source, stream, entry_id, seq) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (consumer, source, stream) DO UPDATE SET entry_id = excluded.entry_id, seq = excluded.seq", params![s.consumer, s.coordinate.source, s.coordinate.stream, s.coordinate.entry_id, s.seq]).map_err(state::sql(&path))?;
    Ok(())
}

fn stream_position(id: &str) -> Option<(u64, u64)> {
    let (millis, sequence) = id.split_once('-')?;
    Some((millis.parse().ok()?, sequence.parse().ok()?))
}

/// A parking receipt is durable before ACK; completion after a lost ACK can be
/// recovered even when Redis no longer has that entry in its pending list.
pub(super) fn parking_ready(state: &RunnerState, s: &Settlement<'_>) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let value = serde_json::to_string(s.outcome).map_err(|e| state::invalid(&path, e))?;
        tx.execute("INSERT OR IGNORE INTO event_parking (consumer, source, stream, entry_id, seq, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![s.consumer, s.coordinate.source, s.coordinate.stream, s.coordinate.entry_id, s.seq, value]).map_err(state::sql(&path))?;
        Ok(())
    })
}

pub(super) fn parked(
    state: &RunnerState,
    consumer: &str,
    source: &str,
) -> Result<Vec<(TransportCoordinate, Option<u64>, SettledOutcome)>, StateError> {
    Ok(state.read(|conn, path| {
        let mut stmt = conn.prepare("SELECT stream, entry_id, seq, outcome FROM event_parking WHERE consumer = ?1 AND source = ?2 ORDER BY rowid").map_err(state::sql(path))?;
        let rows = stmt.query_map([consumer, source], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<u64>>(2)?, r.get::<_, String>(3)?))).map_err(state::sql(path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))?;
        rows.into_iter().map(|(stream, entry_id, seq, outcome)| Ok((TransportCoordinate { source: source.into(), stream, entry_id }, seq, serde_json::from_str(&outcome).map_err(|e| state::invalid(path, e))?))).collect::<Result<Vec<_>, StateError>>()
    })?.unwrap_or_default())
}

pub(super) fn finish_parking(state: &RunnerState, s: &Settlement<'_>) -> Result<(), StateError> {
    // The writer lock is reentrant, but the cursor and parking removal must be
    // one transaction rather than nested transactions on separate connections.
    state.write(|tx| {
        let path = state::db_path(tx);
        let value = serde_json::to_string(s.outcome).map_err(|e| state::invalid(&path, e))?;
        tx.execute("INSERT OR IGNORE INTO consumer_outcomes (consumer, source, stream, entry_id, outcome, settled_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![s.consumer, s.coordinate.source, s.coordinate.stream, s.coordinate.entry_id, value, s.now.to_string()]).map_err(state::sql(&path))?;
        cursor(tx, s)?;
        for table in ["event_parking", "event_attempts"] {
            tx.execute(&format!("DELETE FROM {table} WHERE consumer = ?1 AND source = ?2 AND stream = ?3 AND entry_id = ?4"), params![s.consumer, s.coordinate.source, s.coordinate.stream, s.coordinate.entry_id]).map_err(state::sql(&path))?;
        }
        Ok(())
    })
}

pub(super) const ANNOUNCEMENT_BATCH: usize = 32;
const OPEN_ANNOUNCEMENT_LIMIT: u32 = 1024;

pub(super) struct AnnouncementIntent {
    pub rowid: i64,
    pub consumer: String,
    pub coordinate: TransportCoordinate,
    pub source: String,
    pub receipt: String,
    pub confirmed: bool,
    // Earlier outcomes recorded a receipt without a replayable payload.
    pub payload: Option<String>,
}

pub(super) fn record_announcement_intent(
    state: &RunnerState,
    consumer: &str,
    coordinate: &TransportCoordinate,
    source: &str,
    receipt: &str,
    payload: &str,
) -> Result<bool, StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let open: u32 = tx.query_row(
            "SELECT count(*) FROM consumer_outcomes WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL AND json_extract(outcome, '$.parked.announced') = 0",
            [], |row| row.get(0),
        ).map_err(state::sql(&path))?;
        if open >= OPEN_ANNOUNCEMENT_LIMIT {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE consumer_outcomes SET outcome = json_set(outcome, '$.parked.announcement_source', ?5, '$.parked.announcement_receipt', ?6, '$.parked.announcement_payload', ?7) WHERE consumer = ?1 AND source = ?2 AND stream = ?3 AND entry_id = ?4 AND json_extract(outcome, '$.parked.announced') = 0",
            params![consumer, coordinate.source, coordinate.stream, coordinate.entry_id, source, receipt, payload],
        ).map_err(state::sql(&path))?;
        if changed != 1 {
            return Err(state::invalid(&path, "no unannounced parking outcome for announcement intent"));
        }
        Ok(true)
    })
}

pub(super) fn announcement_intents(
    state: &RunnerState,
    after: i64,
) -> Result<Vec<AnnouncementIntent>, StateError> {
    Ok(state.read(|conn, path| {
        let mut statement = conn.prepare(
            "SELECT rowid, consumer, source, stream, entry_id, json_extract(outcome, '$.parked.announcement_source'), json_extract(outcome, '$.parked.announcement_receipt'), json_extract(outcome, '$.parked.announcement_payload'), json_extract(outcome, '$.parked.announced') FROM consumer_outcomes WHERE rowid > ?1 AND json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL ORDER BY rowid LIMIT ?2",
        ).map_err(state::sql(path))?;
        let rows = statement.query_map(params![after, ANNOUNCEMENT_BATCH], |row| Ok(AnnouncementIntent {
            rowid: row.get(0)?,
            consumer: row.get(1)?,
            coordinate: TransportCoordinate { source: row.get(2)?, stream: row.get(3)?, entry_id: row.get(4)? },
            source: row.get(5)?,
            receipt: row.get(6)?,
            payload: row.get(7)?,
            confirmed: row.get(8)?,
        })).map_err(state::sql(path))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
    })?.unwrap_or_default())
}

#[derive(Clone, Copy)]
pub(super) enum AnnouncementResult {
    Published,
    Abandoned,
    MissingLegacyPayload,
}

pub(super) fn finish_announcement(
    state: &RunnerState,
    consumer: &str,
    coordinate: &TransportCoordinate,
    result: AnnouncementResult,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let outcome = match result {
            AnnouncementResult::Published => "json_set(outcome, '$.parked.announced', json('true'))",
            AnnouncementResult::Abandoned => "json_set(outcome, '$.parked.announcement_abandoned', json('true'))",
            AnnouncementResult::MissingLegacyPayload => "outcome",
        };
        tx.execute(
            &format!("UPDATE consumer_outcomes SET outcome = json_remove({outcome}, '$.parked.announcement_source', '$.parked.announcement_receipt', '$.parked.announcement_payload') WHERE consumer = ?1 AND source = ?2 AND stream = ?3 AND entry_id = ?4"),
            params![consumer, coordinate.source, coordinate.stream, coordinate.entry_id],
        ).map_err(state::sql(&path))?;
        Ok(())
    })
}
