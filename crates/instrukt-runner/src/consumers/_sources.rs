//! Retained scope assignments: capture new positions, then drain former sources before using them.

use std::sync::Arc;

use instrukt_core::state_db::rusqlite::params;

use super::Consumer;
use crate::state::{self, RunnerState, StateError};
use crate::transport::{Endpoint, EventScope, ScopedTransport};

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
    state.write(|tx| {
        let path = state::db_path(tx);
        let mut stmt = tx.prepare("SELECT source FROM consumer_sources WHERE consumer = ?1 AND state != 'retired'").map_err(state::sql(&path))?;
        let former = stmt.query_map([&consumer.name], |row| row.get::<_, String>(0)).map_err(state::sql(&path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(&path))?;
        let mut bound = Vec::new();
        for source in former.iter().filter(|source| !desired.iter().any(|endpoint| &endpoint.identity == *source)) {
            tx.execute("UPDATE consumer_sources SET state = 'retiring' WHERE consumer = ?1 AND source = ?2", params![consumer.name, source]).map_err(state::sql(&path))?;
            if let Some(endpoint) = known.iter().find(|endpoint| &endpoint.identity == source) {
                bound.push(BoundSource { endpoint: Arc::clone(endpoint), retiring: true });
            } else {
                tracing::warn!(event = "consumer_former_source_unconfigured", consumer = %consumer.name, source, "restore the former Redis configuration to settle its retained group before the new source can run");
            }
        }
        for endpoint in desired {
            tx.execute("INSERT INTO consumer_sources (consumer, source, state, prepared) VALUES (?1, ?2, 'active', 0) ON CONFLICT (consumer, source) DO UPDATE SET prepared = CASE WHEN consumer_sources.state = 'retired' THEN 0 ELSE consumer_sources.prepared END, state = 'active'", params![consumer.name, endpoint.identity]).map_err(state::sql(&path))?;
            bound.push(BoundSource { endpoint, retiring: false });
        }
        Ok(bound)
    })
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
