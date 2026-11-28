//! Retained scope assignments: capture new positions, then drain former
//! sources still configured before using them; abandon those no longer configured.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use instrukt_core::fsx::{FsError, LockGuard};
use instrukt_core::state_db::rusqlite::params;

use super::Consumer;
use crate::state::{self, RunnerState, StateError};
use crate::transport::{Endpoint, EventScope, ScopedTransport};

pub(super) const HANDOVER_WAIT: Duration = Duration::from_secs(1);
const STATUS_LOCK_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct BoundSource {
    pub endpoint: Arc<Endpoint>,
    pub retiring: bool,
}

pub(super) fn bind(
    state: &RunnerState,
    transport: &ScopedTransport,
    consumer: &Consumer,
) -> Result<Vec<BoundSource>, StateError> {
    let desired = transport.distinct(&consumer.scopes);
    let known = transport.distinct(&[EventScope::Local, EventScope::Cluster]);
    // Serialize binders before taking any per-source lock. A reader never
    // takes the binding lock, and neither side waits while holding SQLite.
    let transition = format!("{}{}", super::CONSUMER_TRANSITION_PREFIX, consumer.name);
    let started = Instant::now();
    let _binding = handover_lock(state, &transition, None, started)?;
    let former = state.read(|conn, path| {
        let mut stmt = conn.prepare("SELECT source FROM consumer_sources WHERE consumer = ?1 AND state != 'retired'").map_err(state::sql(path))?;
        let rows = stmt.query_map([&consumer.name], |row| row.get::<_, String>(0)).map_err(state::sql(path))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
    })?.unwrap_or_default();
    let locked: std::collections::BTreeSet<_> = former
        .iter()
        .map(String::as_str)
        .chain(desired.iter().map(|e| e.identity.as_str()))
        .collect();
    let _sources = locked
        .into_iter()
        .map(|source| handover_lock(state, &transition, Some(source), started))
        .collect::<Result<Vec<_>, _>>()?;
    let bound = state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let mut bound = Vec::new();
        for source in former.iter().filter(|source| !desired.iter().any(|endpoint| &endpoint.identity == *source)) {
            // A former source still configured is drained before it retires.
            // One no longer configured cannot be drained: it is abandoned, the
            // new source runs at once, and its pending events stay unread in
            // that Redis until it is configured again.
            let retired: bool = tx.query_row("SELECT state = 'retired' FROM consumer_sources WHERE consumer = ?1 AND source = ?2", params![consumer.name, source], |row| row.get(0)).map_err(state::sql(&path))?;
            if retired {
                continue;
            }
            if let Some(endpoint) = known.iter().find(|endpoint| &endpoint.identity == source) {
                tx.execute("UPDATE consumer_sources SET state = 'retiring' WHERE consumer = ?1 AND source = ?2", params![consumer.name, source]).map_err(state::sql(&path))?;
                bound.push(BoundSource { endpoint: Arc::clone(endpoint), retiring: true });
            } else {
                let abandoned = tx.execute("UPDATE consumer_sources SET state = 'abandoned' WHERE consumer = ?1 AND source = ?2 AND state != 'abandoned'", params![consumer.name, source]).map_err(state::sql(&path))?;
                if abandoned > 0 {
                    tracing::warn!(event = "consumer_source_abandoned", consumer = %consumer.name, source, "the former Redis source is no longer configured; its pending events stay unread there");
                }
            }
        }
        for endpoint in desired {
            tx.execute("INSERT INTO consumer_sources (consumer, source, state, prepared) VALUES (?1, ?2, 'active', 0) ON CONFLICT (consumer, source) DO UPDATE SET prepared = CASE WHEN consumer_sources.state = 'retired' THEN 0 ELSE consumer_sources.prepared END, state = 'active'", params![consumer.name, endpoint.identity]).map_err(state::sql(&path))?;
            bound.push(BoundSource { endpoint, retiring: false });
        }
        Ok(bound)
    })?;
    clear_transition_status(state, &consumer.name, &transition);
    Ok(bound)
}

fn clear_transition_status(state: &RunnerState, consumer: &str, transition: &str) {
    let status_lock = status_lock_path(state, consumer);
    let result = state::refuse_symlink(&status_lock)
        .and_then(|()| LockGuard::acquire(&status_lock, STATUS_LOCK_WAIT).map_err(StateError::from))
        .and_then(|_guard| {
            state::record_worker_failure_at(state.home(), transition, None, jiff::Timestamp::now())
        });
    if let Err(error) = result {
        tracing::warn!(
            event = "consumer_source_transition_clear_failed",
            consumer = %consumer,
            %error
        );
    }
}

fn handover_lock(
    state: &RunnerState,
    transition: &str,
    source: Option<&str>,
    started: Instant,
) -> Result<LockGuard, FsError> {
    let consumer = transition
        .strip_prefix(super::CONSUMER_TRANSITION_PREFIX)
        .unwrap_or(transition);
    let path = lock_path(state, consumer, source);
    loop {
        if let Some(guard) = LockGuard::try_acquire(&path)? {
            return Ok(guard);
        }
        let waited = started.elapsed();
        let remaining = HANDOVER_WAIT.saturating_sub(waited);
        if remaining.is_zero() {
            let status_lock = status_lock_path(state, consumer);
            match state::refuse_symlink(&status_lock).and_then(|()| {
                LockGuard::acquire(&status_lock, STATUS_LOCK_WAIT).map_err(StateError::from)
            }) {
                Ok(_status_guard) => {
                    // Non-blocking recheck under the per-consumer status lock:
                    // if the holder just released `path`, take it cleanly
                    // without recording a phantom deferred transition.
                    if let Some(guard) = LockGuard::try_acquire(&path)? {
                        return Ok(guard);
                    }
                    let error = FsError::LockTimeout { path, waited };
                    tracing::warn!(event = "consumer_source_transition_deferred", %consumer, source, %error);
                    let cause = source.map_or_else(
                        || "waiting for another binder".into(),
                        |source| format!("waiting for reader on Redis source {source}"),
                    );
                    state::worker_state(state.home(), transition, Some(&cause));
                    return Err(error);
                }
                Err(status_error) => {
                    let error = FsError::LockTimeout { path, waited };
                    tracing::warn!(
                        event = "consumer_source_transition_deferred",
                        %consumer,
                        source,
                        %error,
                        status_error = %status_error
                    );
                    return Err(error);
                }
            }
        }
        // This lease covers real Redis I/O, so its handover uses wall time.
        std::thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}

fn lock_path(state: &RunnerState, consumer: &str, source: Option<&str>) -> PathBuf {
    let parts = [consumer, source.unwrap_or("")];
    state
        .home()
        .state_dir()
        .join("consumer-locks")
        .join(crate::events::occurrence_id(&parts))
}

fn status_lock_path(state: &RunnerState, consumer: &str) -> PathBuf {
    let parts = [consumer, ""];
    state
        .home()
        .state_dir()
        .join("consumer-locks")
        .join(format!("{}.status", crate::events::occurrence_id(&parts)))
}

/// A source operation holds this lease through its Redis effects. An old
/// runtime whose binding changed cannot read, acknowledge or prepare it.
pub(super) fn reader_lock(
    state: &RunnerState,
    consumer: &str,
    source: &str,
    retiring: bool,
) -> Result<ReaderLease, StateError> {
    let path = lock_path(state, consumer, Some(source));
    let Some(guard) = LockGuard::try_acquire(&path)? else {
        return Ok(ReaderLease::Busy);
    };
    let expected = if retiring { "retiring" } else { "active" };
    let bound = state.read(|conn, path| conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM consumer_sources WHERE consumer = ?1 AND source = ?2 AND state = ?3)",
        params![consumer, source, expected], |row| row.get(0),
    ).map_err(state::sql(path)))?.unwrap_or(false);
    Ok(if bound {
        ReaderLease::Held(guard)
    } else {
        ReaderLease::Ended
    })
}

pub(super) enum ReaderLease {
    Held(LockGuard),
    Busy,
    Ended,
}

pub(super) fn retiring(state: &RunnerState, consumer: &str) -> Result<bool, StateError> {
    Ok(state.read(|conn, path| conn.query_row("SELECT EXISTS(SELECT 1 FROM consumer_sources WHERE consumer = ?1 AND state = 'retiring')", [consumer], |row| row.get(0)).map_err(state::sql(path)))?.unwrap_or(false))
}

pub(super) fn prepared(
    state: &RunnerState,
    consumer: &str,
    source: &str,
) -> Result<bool, StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let known: bool = tx
            .query_row(
                "SELECT prepared FROM consumer_sources WHERE consumer = ?1 AND source = ?2",
                [consumer, source],
                |row| row.get(0),
            )
            .map_err(state::sql(&path))?;
        tx.execute(
            "UPDATE consumer_sources SET prepared = 1 WHERE consumer = ?1 AND source = ?2",
            [consumer, source],
        )
        .map_err(state::sql(&path))?;
        Ok(known)
    })
}

pub(super) fn retire(state: &RunnerState, consumer: &str, source: &str) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "UPDATE consumer_sources SET state = 'retired' WHERE consumer = ?1 AND source = ?2",
            [consumer, source],
        )
        .map_err(state::sql(&path))?;
        Ok(())
    })
}

/// The sources a consumer abandoned: each `(consumer, source)` no longer
/// configured, whose pending events stay unread in that Redis.
pub(crate) fn abandoned(state: &RunnerState) -> Result<Vec<(String, String)>, StateError> {
    Ok(state
        .read(|conn, path| {
            let mut stmt = conn
                .prepare("SELECT consumer, source FROM consumer_sources WHERE state = 'abandoned' ORDER BY consumer, source")
                .map_err(state::sql(path))?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(state::sql(path))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
        })?
        .unwrap_or_default())
}
