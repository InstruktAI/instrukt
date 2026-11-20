use std::collections::HashSet;

use instrukt_core::config::{Configuration, Env, Runtime};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};

const START_OPERATOR_BOUND: SignedDuration = SignedDuration::from_mins(5);
const PREFIX: &str = "keyed-start:";

#[derive(Serialize)]
pub(crate) struct Finding {
    runtime: String,
    key: String,
    failure: String,
    since: Timestamp,
}

pub(crate) fn note(
    state: &RunnerState,
    runtime: Runtime,
    key: &str,
    now: Timestamp,
    outcome: Result<(), &SessionsError>,
) {
    let owner_key = key;
    let key = match outcome {
        Err(
            SessionsError::ArchivedBound { key, .. }
            | SessionsError::DeletedBound { key, .. }
            | SessionsError::PendingReplacement { key, .. }
            | SessionsError::FirstStartAbsent { key, .. }
            | SessionsError::PendingPlacement { key, .. }
            | SessionsError::ProjectChanged { key, .. }
            | SessionsError::ProjectUnverified { key, .. }
            | SessionsError::ConflictingThreads { key, .. }
            | SessionsError::Unresolved { key }
            | SessionsError::Uncertain { key, .. },
        ) => key.as_str(),
        _ => key,
    };
    let worker = format!("{PREFIX}{}:{key}", runtime.name());
    let result = state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        if owner_key != key {
            tx.execute("DELETE FROM runner_workers WHERE worker=?1",
                [format!("{PREFIX}{}:{owner_key}", runtime.name())],
            ).map_err(state::sql(&path))?;
        }
        match outcome {
            Ok(()) => tx.execute("DELETE FROM runner_workers WHERE worker=?1", [&worker]),
            Err(error) => {
                let (failure, since) = match error {
                    SessionsError::Unresolved { .. } | SessionsError::Uncertain { .. } => {
                        let begun: Option<String> = tx.query_row(
                            "SELECT begun_at FROM runtime_starts WHERE runtime=?1 AND semantic_key=?2",
                            params![runtime.name(), key], |row| row.get(0),
                        ).optional().map_err(state::sql(&path))?;
                        (format!("start `{key}` remains unresolved; use {}", super::resolve_choices(key)), begun.unwrap_or_else(|| now.to_string()))
                    }
                    SessionsError::ArchivedBound { .. } | SessionsError::PendingPlacement { .. } | SessionsError::ProjectChanged { .. } | SessionsError::ProjectUnverified { .. } | SessionsError::DeletedBound { .. } | SessionsError::FirstStartAbsent { .. } | SessionsError::PendingReplacement { .. } | SessionsError::ConflictingThreads { .. } => (error.to_string(), now.to_string()),
                    SessionsError::KeyConflict(_) => (format!("{error}; archive or delete the extra thread"), now.to_string()),
                    _ => return Ok(()),
                };
                tx.execute("INSERT INTO runner_workers(worker,failure,since) VALUES(?1,?2,?3)
                    ON CONFLICT(worker) DO UPDATE SET failure=excluded.failure,
                    since=CASE WHEN failure=excluded.failure THEN since ELSE excluded.since END",
                    params![worker, failure, since])
            }
        }.map_err(state::sql(&path))?;
        Ok(())
    });
    if let Err(error) = result {
        tracing::warn!(event = "keyed_start_finding_not_recorded", key, %error);
    }
}

pub(crate) fn status(state: &RunnerState) -> Result<Vec<Finding>, StateError> {
    Ok(state.read(|conn, path| {
        let mut query = conn.prepare(
            "SELECT worker,failure,since FROM runner_workers WHERE worker GLOB 'keyed-start:*' ORDER BY worker",
        ).map_err(state::sql(path))?;
        let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(state::sql(path))?;
        rows.map(|row| {
            let (worker, failure, since) = row.map_err(state::sql(path))?;
            let (runtime, key) = worker.strip_prefix(PREFIX).and_then(|name| name.split_once(':'))
                .ok_or_else(|| state::invalid(path, "invalid keyed-start finding identity"))?;
            Ok(Finding {
                runtime: runtime.into(), key: key.into(), failure,
                since: since.parse().map_err(|err| state::invalid(path, err))?,
            })
        }).collect()
    })?.unwrap_or_default())
}

/// The runner calls this outside its dispatch guard, with one set per process.
pub(crate) fn notify_due(
    state: &RunnerState,
    config: &Configuration,
    env: &Env,
    now: Timestamp,
    notified: &mut HashSet<(String, String)>,
) {
    let findings = match status(state) {
        Ok(findings) => findings,
        Err(error) => {
            tracing::warn!(event = "keyed_start_findings_unreadable", %error);
            return;
        }
    };
    for finding in findings {
        if now.duration_since(finding.since) >= START_OPERATOR_BOUND
            && notified.insert((finding.runtime, finding.key.clone()))
        {
            crate::anomaly::notify(config, env, &finding.key, None);
        }
    }
}
