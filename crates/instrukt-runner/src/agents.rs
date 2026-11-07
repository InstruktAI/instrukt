//! The dispatch agent order (`work-dispatch`, "Agent selection and intent
//! checks"; `layered-configuration`, "Agent preferences and
//! availability"): a task's own preference first, then the procedure's
//! `dispatch`, or else the global `agents.order` with
//! `agents.tier`.

use instrukt_core::config::Configuration;
use instrukt_core::index::IndexDispatch;
use serde::{Deserialize, Serialize};

use crate::interfaces::{AgentPreference, Tier};

/// One agent to try, with the tier mode it starts in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChoice {
    pub agent: String,
    pub tier: Option<Tier>,
}

/// The global `agents.order` and its tier mode; a malformed value
/// (a validation finding) is logged and read as unset.
pub fn global_order(config: &Configuration) -> (Vec<&str>, Option<Tier>) {
    let order = config.agents_order().unwrap_or_else(|err| {
        tracing::warn!(event = "agents_order_unreadable", error = %err);
        Vec::new()
    });
    let tier = config.agents_tier().unwrap_or_else(|err| {
        tracing::warn!(event = "agents_tier_unreadable", error = %err);
        None
    });
    (order, tier)
}

/// The ordered agents to try; an agent appears once, at its first place.
pub fn agent_order(
    task: Option<&AgentPreference>,
    dispatch: Option<&IndexDispatch>,
    global_order: &[&str],
    global_tier: Option<Tier>,
) -> Vec<AgentChoice> {
    let mut order: Vec<AgentChoice> = Vec::new();
    let mut push = |agent: &str, tier: Option<Tier>| {
        if !order.iter().any(|choice| choice.agent == agent) {
            order.push(AgentChoice {
                agent: agent.to_owned(),
                tier,
            });
        }
    };
    if let Some(task) = task {
        push(
            &task.agent,
            task.tier
                .or_else(|| dispatch.and_then(|d| Tier::parse(&d.tier)))
                .or(global_tier),
        );
    }
    match dispatch {
        Some(dispatch) => {
            let tier = Tier::parse(&dispatch.tier);
            for agent in &dispatch.agent {
                push(agent, tier);
            }
        }
        None => {
            for agent in global_order {
                push(agent, global_tier);
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(agent: &str, tier: Tier) -> AgentChoice {
        AgentChoice {
            agent: agent.into(),
            tier: Some(tier),
        }
    }

    #[test]
    fn the_task_preference_precedes_the_procedures_dispatch_which_replaces_the_global_order() {
        let task = AgentPreference {
            agent: "b".into(),
            tier: Some(Tier::Light),
        };
        let dispatch = IndexDispatch {
            agent: vec!["a".into(), "b".into()],
            tier: "strong".into(),
        };
        assert_eq!(
            agent_order(Some(&task), Some(&dispatch), &["c"], Some(Tier::Standard)),
            [choice("b", Tier::Light), choice("a", Tier::Strong)]
        );
        assert_eq!(
            agent_order(None, None, &["c", "d"], Some(Tier::Standard)),
            [choice("c", Tier::Standard), choice("d", Tier::Standard)]
        );
        assert!(agent_order(None, None, &[], None).is_empty());
    }
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
    pub family: String,
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
        .map(|(family, model, status, until, reason)| {
            let until = until
                .map(|v| v.parse::<jiff::Timestamp>())
                .transpose()
                .map_err(|e| crate::sessions::SessionsError::Refused(e.to_string()))?;
            let status: AvailabilityStatus =
                serde_json::from_value(serde_json::Value::String(status))
                    .map_err(|e| crate::sessions::SessionsError::Refused(e.to_string()))?;
            let expired = until.is_some_and(|until| until <= now);
            Ok(Availability {
                family,
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
    let catalog = instrukt_core::config::agent_catalog()?;
    let agent = catalog.get(&mark.family).ok_or_else(|| {
        crate::sessions::SessionsError::Refused(format!("unknown agent family `{}`", mark.family))
    })?;
    if mark
        .model
        .as_ref()
        .is_some_and(|model| !agent.models.contains_key(model))
    {
        return Err(crate::sessions::SessionsError::Refused(
            "unknown model for this family".into(),
        ));
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
        tx.execute("INSERT INTO agent_availability(agent,model,status,status_until,reason) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(agent,model) DO UPDATE SET status=excluded.status,status_until=excluded.status_until,reason=excluded.reason",params![mark.family,mark.model.as_deref().unwrap_or(""),mark.status.as_str(),mark.until.map(|v|v.to_string()),mark.reason]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
    })?;
    Ok(())
}

/// Sequential family then chain selection. Degraded capacity is reserved for
/// human launches, as in TeleClaude; automated assignments skip it.
pub fn selections(
    config: &Configuration,
    home: &instrukt_core::home::Home,
    order: &[AgentChoice],
    now: jiff::Timestamp,
) -> Result<Vec<instrukt_core::config::AgentSelection>, crate::sessions::SessionsError> {
    let marks = availability(home, now)?;
    let eligible = |family: &str, model: Option<&str>| {
        !marks.iter().any(|mark| {
            mark.family == family
                && mark.model.as_deref() == model
                && mark.status != AvailabilityStatus::Available
        })
    };
    let mut result = Vec::new();
    for choice in order {
        if !eligible(&choice.agent, None) {
            continue;
        }
        let tier = choice.tier.or(config.agents_tier()?).ok_or_else(|| {
            crate::sessions::SessionsError::Refused("dispatch tier is missing".into())
        })?;
        for entry in config.model_chain(&choice.agent, tier)? {
            if eligible(&choice.agent, Some(&entry.model)) {
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
