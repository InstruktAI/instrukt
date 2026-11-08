//! Ordered role/profile selection and expiring local capacity marks.
use crate::interfaces::{AgentPreference, Tier};
use instrukt_core::config::Configuration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChoice {
    pub agent: String,
    pub tier: Option<Tier>,
}

pub fn role_order(
    config: &Configuration,
    task: Option<&AgentPreference>,
    role: Option<&str>,
) -> Result<Vec<AgentChoice>, crate::sessions::SessionsError> {
    Ok(config
        .agents()?
        .order(
            role,
            task.map(|t| t.agent.as_str()),
            task.and_then(|t| t.tier),
        )?
        .into_iter()
        .map(|(agent, tier)| AgentChoice {
            agent,
            tier: Some(tier),
        })
        .collect())
}

/// Persisted agent/model marks, normalized at the command's controlled clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AvailabilityStatus {
    Available,
    Unavailable,
    Degraded,
}
impl AvailabilityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Degraded => "degraded",
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Availability {
    pub profile: String,
    pub model: Option<String>,
    pub status: AvailabilityStatus,
    pub until: Option<jiff::Timestamp>,
    pub reason: Option<String>,
}

pub fn availability(
    home: &instrukt_core::home::Home,
    now: jiff::Timestamp,
) -> Result<Vec<Availability>, crate::sessions::SessionsError> {
    use crate::state::{self, RunnerState};
    let state = RunnerState::new(home);
    let rows=state.read(|conn,path| {
        let mut q=conn.prepare("SELECT agent,model,status,status_until,reason FROM agent_availability ORDER BY agent,model").map_err(state::sql(path))?;
        q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?))).map_err(state::sql(path))?.collect::<Result<Vec<_>,_>>().map_err(state::sql(path))
    })?.unwrap_or_default();
    rows.into_iter()
        .map(|(profile, model, status, until, reason)| {
            let until = until
                .map(|v| v.parse::<jiff::Timestamp>())
                .transpose()
                .map_err(|e| crate::sessions::SessionsError::Refused(e.to_string()))?;
            let status: AvailabilityStatus =
                serde_json::from_value(serde_json::Value::String(status))
                    .map_err(|e| crate::sessions::SessionsError::Refused(e.to_string()))?;
            let expired = until.is_some_and(|until| until <= now);
            Ok(Availability {
                profile,
                model: (!model.is_empty()).then_some(model),
                status: if expired {
                    AvailabilityStatus::Available
                } else {
                    status
                },
                until: if expired { None } else { until },
                reason: if expired { None } else { reason },
            })
        })
        .collect()
}

pub fn mark_availability(
    home: &instrukt_core::home::Home,
    mark: &Availability,
    now: jiff::Timestamp,
) -> Result<(), crate::sessions::SessionsError> {
    use crate::state::{self, RunnerState, StateError};
    use instrukt_core::state_db::rusqlite::params;
    if !instrukt_core::config::Agents::load(home)?
        .profiles
        .contains_key(&mark.profile)
    {
        return Err(crate::sessions::SessionsError::Refused(format!(
            "unknown profile `{}`",
            mark.profile
        )));
    }
    if mark.model.as_ref().is_some_and(|m| m.trim().is_empty()) {
        return Err(crate::sessions::SessionsError::Refused(
            "model name must be nonempty".into(),
        ));
    }
    if mark.status == AvailabilityStatus::Available {
        RunnerState::new(home).write::<_, StateError>(|tx| {
            tx.execute(
                "DELETE FROM agent_availability WHERE agent=?1 AND model=?2",
                params![mark.profile, mark.model.as_deref().unwrap_or("")],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
            Ok(())
        })?;
        return Ok(());
    }
    if mark.status != AvailabilityStatus::Available && mark.model.is_some() && mark.until.is_none()
    {
        return Err(crate::sessions::SessionsError::Refused(
            "non-available model status requires --until <RFC3339>".into(),
        ));
    }
    if mark.until.is_some_and(|until| until <= now) {
        return Err(crate::sessions::SessionsError::Refused(
            "availability expiry must be in the future".into(),
        ));
    }
    if mark.status == AvailabilityStatus::Unavailable
        && mark
            .reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(crate::sessions::SessionsError::Refused(
            "unavailable status requires --reason".into(),
        ));
    }
    RunnerState::new(home).write::<_,StateError>(|tx| {
        tx.execute("INSERT INTO agent_availability(agent,model,status,status_until,reason) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(agent,model) DO UPDATE SET status=excluded.status,status_until=excluded.status_until,reason=excluded.reason",params![mark.profile,mark.model.as_deref().unwrap_or(""),mark.status.as_str(),mark.until.map(|v|v.to_string()),mark.reason]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
    })?;
    Ok(())
}

/// Walk each profile's model chain in order, reporting capacity restrictions.
pub fn selections(
    config: &Configuration,
    home: &instrukt_core::home::Home,
    order: &[AgentChoice],
    now: jiff::Timestamp,
) -> Result<Vec<instrukt_core::config::AgentSelection>, crate::sessions::SessionsError> {
    let marks = availability(home, now)?;
    let agents = config.agents()?;
    let restriction = |profile: &str, model: Option<&str>| {
        marks.iter().find(|mark| {
            mark.profile == profile
                && mark.model.as_deref() == model
                && mark.status != AvailabilityStatus::Available
        })
    };
    let mut result = Vec::new();
    for choice in order {
        if let Some(mark) = restriction(&choice.agent, None) {
            tracing::warn!(event="agent_selection_skip",profile=%choice.agent,status=%mark.status.as_str(),until=?mark.until,reason=%mark.reason.as_deref().unwrap_or("local profile capacity restriction"));
            continue;
        }
        let tier = choice.tier.or(agents.default.tier).ok_or_else(|| {
            crate::sessions::SessionsError::Refused("dispatch tier is missing".into())
        })?;
        for entry in agents.model_chain(&choice.agent, tier)? {
            if let Some(mark) = restriction(&choice.agent, Some(&entry.model)) {
                tracing::warn!(event="agent_selection_skip",profile=%choice.agent,model=%entry.model,status=%mark.status.as_str(),until=?mark.until,reason=%mark.reason.as_deref().unwrap_or("local model capacity restriction"));
            } else {
                result.push(entry);
            }
        }
    }
    if result.is_empty() {
        return Err(crate::sessions::SessionsError::OrderExhausted {
            tried: order.iter().map(|c| c.agent.clone()).collect(),
        });
    }
    Ok(result)
}
