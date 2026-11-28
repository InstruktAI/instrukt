//! Execution evidence and its watermark are independent of snapshot/manual marks.
use super::QUOTA_DEFAULT_SECONDS;
use crate::runtime::{CapacityOutcome, CapacityResult, RuntimeAdapter};
use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};
use instrukt_core::config::{Agents, Configuration};
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;

/// Observe one managed execution through its runtime's normalized contract.
pub fn observe(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    session: &SessionRef,
) -> Result<(), SessionsError> {
    let Some(outcome) = adapter.capacity_outcome(session)? else {
        return Ok(());
    };
    record(home, adapter.runtime().name(), &outcome)
}

fn restriction(
    outcome: &CapacityOutcome,
) -> Result<(Option<Timestamp>, Option<String>), SessionsError> {
    match &outcome.result {
        CapacityResult::Succeeded => Ok((None, None)),
        CapacityResult::QuotaExhausted { resets_at } => {
            let (until, reason) = match resets_at {
                Some(until) => (*until, "runtime-reported quota exhaustion"),
                None => (
                    outcome
                        .at
                        .checked_add(jiff::SignedDuration::from_secs(QUOTA_DEFAULT_SECONDS))
                        .map_err(|_| {
                            SessionsError::Refused("cannot compute quota retry time".into())
                        })?,
                    "runtime-reported quota exhaustion; local retry cooldown (reset missing)",
                ),
            };
            Ok((Some(until), Some(reason.into())))
        }
    }
}

fn record(home: &Home, runtime: &str, outcome: &CapacityOutcome) -> Result<(), SessionsError> {
    let (until, reason) = restriction(outcome)?;
    RunnerState::new(home).write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let previous: Option<String> = tx.query_row(
            "SELECT event_at FROM agent_execution_capacity WHERE runtime=?1 AND instance=?2 AND model=?3",
            params![runtime, outcome.instance.as_str(), outcome.model], |row| row.get(0),
        ).optional().map_err(state::sql(&path))?;
        if let Some(previous) = previous {
            let previous: Timestamp = previous.parse().map_err(|error| state::invalid(&path, error))?;
            if outcome.at <= previous {
                return Ok(());
            }
        }
        tx.execute(
            "INSERT INTO agent_execution_capacity(runtime,instance,model,run_id,event_at,status_until,reason)
             VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(runtime,instance,model) DO UPDATE SET
             run_id=excluded.run_id,event_at=excluded.event_at,status_until=excluded.status_until,reason=excluded.reason",
            params![runtime, outcome.instance.as_str(), outcome.model, outcome.run, outcome.at.to_string(), until.map(|at| at.to_string()), reason],
        ).map_err(state::sql(&path))?;
        Ok(())
    })?;
    Ok(())
}

type Mark = (String, String, String, Option<String>, Option<String>);

/// Project instance/model restrictions onto the configured profile names.
pub(super) fn marks(home: &Home) -> Result<Vec<Mark>, SessionsError> {
    let profiles = Agents::load(home)?;
    let runtime = Configuration::load(home, None)?.runner_runtime()?;
    let rows: Vec<(String, String, String, Option<String>)> = RunnerState::new(home).read(|conn, path| {
        let mut query = conn.prepare(
            "SELECT instance,model,status_until,reason FROM agent_execution_capacity WHERE runtime=?1 AND status_until IS NOT NULL",
        ).map_err(state::sql(path))?;
        query.query_map([runtime.name()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .and_then(Iterator::collect::<Result<Vec<_>, _>>).map_err(state::sql(path))
    })?.unwrap_or_default();
    Ok(rows
        .into_iter()
        .flat_map(|(instance, model, until, reason)| {
            profiles
                .profiles
                .iter()
                .filter(move |(_, profile)| profile.instance.as_str() == instance)
                .map(move |(name, _)| {
                    (
                        name.to_string(),
                        model.clone(),
                        "unavailable".into(),
                        Some(until.clone()),
                        reason.clone(),
                    )
                })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::ids::ProviderInstanceId;

    #[test]
    fn missing_reset_anchors_the_retry_to_the_failure_instead_of_each_observation() {
        let outcome = CapacityOutcome {
            run: "run-1".into(),
            instance: ProviderInstanceId::parse("antigravity").unwrap(),
            model: "gemini-4-neon".into(),
            at: "2026-11-28T14:00:00Z".parse().unwrap(),
            result: CapacityResult::QuotaExhausted { resets_at: None },
        };
        let until = restriction(&outcome).unwrap().0.unwrap();
        assert!(until > outcome.at);
        let shift = jiff::SignedDuration::from_secs(17);
        let shifted = CapacityOutcome {
            at: outcome.at.checked_add(shift).unwrap(),
            ..outcome
        };
        let shifted_until = restriction(&shifted).unwrap().0.unwrap();
        assert_eq!(shifted_until, until.checked_add(shift).unwrap());
    }
}
