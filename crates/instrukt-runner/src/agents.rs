//! Ordered role/profile selection and expiring local capacity marks.
use crate::interfaces::{AgentPreference, Tier};
use instrukt_core::config::Configuration;
use instrukt_core::ids::ProfileName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChoice {
    pub agent: ProfileName,
    pub tier: Option<Tier>,
}

pub fn role_order(
    config: &Configuration,
    task: Option<&AgentPreference>,
    role: Option<&str>,
) -> Result<Vec<AgentChoice>, crate::sessions::SessionsError> {
    Ok(config
        .agents()?
        .order(role, task.map(|t| &t.agent), task.and_then(|t| t.tier))?
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
    pub profile: ProfileName,
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
        let mut q=conn.prepare("SELECT agent,model,status,status_until,reason FROM agent_availability UNION ALL SELECT agent,model,status,status_until,reason FROM agent_quota ORDER BY agent,model").map_err(state::sql(path))?;
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
                profile: ProfileName::parse(&profile)
                    .map_err(|e| crate::sessions::SessionsError::Refused(e.to_string()))?,
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
                params![mark.profile.as_str(), mark.model.as_deref().unwrap_or("")],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
            tx.execute(
                "DELETE FROM agent_quota WHERE agent=?1 AND model=?2",
                params![mark.profile.as_str(), mark.model.as_deref().unwrap_or("")],
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
        tx.execute("INSERT INTO agent_availability(agent,model,status,status_until,reason) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(agent,model) DO UPDATE SET status=excluded.status,status_until=excluded.status_until,reason=excluded.reason",params![mark.profile.as_str(),mark.model.as_deref().unwrap_or(""),mark.status.as_str(),mark.until.map(|v|v.to_string()),mark.reason]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
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
    let restriction = |profile: &ProfileName, model: Option<&str>| {
        marks.iter().find(|mark| {
            mark.profile == *profile
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
            tried: order.iter().map(|c| c.agent.to_string()).collect(),
        });
    }
    Ok(result)
}

pub(crate) struct QuotaMark {
    pub profile: ProfileName,
    pub models: std::collections::BTreeSet<String>,
    pub until: jiff::Timestamp,
    pub cause: String,
}

pub(crate) fn mark_quota(
    home: &instrukt_core::home::Home,
    mark: &QuotaMark,
) -> Result<(), crate::sessions::SessionsError> {
    use crate::state::{self, RunnerState, StateError};
    use instrukt_core::state_db::rusqlite::params;
    RunnerState::new(home).write::<_,StateError>(|tx| {
        for model in &mark.models {
            tx.execute("INSERT INTO agent_quota(agent,model,status,status_until,reason) VALUES(?1,?2,'unavailable',?3,?4) ON CONFLICT(agent,model) DO UPDATE SET status_until=excluded.status_until,reason=excluded.reason",params![mark.profile.as_str(),model,mark.until.to_string(),mark.cause]).map_err(state::sql(&state::db_path(tx)))?;
        }
        Ok(())
    })?;
    tracing::warn!(event="agent_quota_marked",profile=%mark.profile,models=mark.models.len(),until=%mark.until,reason=%mark.cause);
    Ok(())
}

/// Apply capacity and ordered selection policy to adapter-translated readiness facts.
pub(crate) fn prepare_start(
    home: &instrukt_core::home::Home,
    adapter: &dyn crate::runtime::RuntimeAdapter,
    request: &crate::runtime::RuntimeStart,
    now: jiff::Timestamp,
) -> Result<crate::runtime::RuntimeStart, crate::sessions::SessionsError> {
    let snapshot = adapter.provider_snapshot(home)?;
    let config = Configuration::load(home, Some(&request.checkout))?;
    let agents = config.agents()?;
    const QUOTA_DEFAULT_SECONDS: i64 = 3600;
    for (name, profile) in &agents.profiles {
        let Some(provider) = snapshot
            .providers
            .iter()
            .find(|p| p.instance == profile.instance)
        else {
            continue;
        };
        let mut exhausted = None;
        for window in provider
            .usage
            .iter()
            .filter(|window| window.used_percent == 100.0)
        {
            let (until, defaulted) = match window.resets_at {
                Some(until) => (until, false),
                None => (
                    now.checked_add(jiff::SignedDuration::from_secs(QUOTA_DEFAULT_SECONDS))
                        .map_err(|_| {
                            crate::sessions::SessionsError::Refused(
                                "cannot compute quota expiry".into(),
                            )
                        })?,
                    true,
                ),
            };
            if until <= now {
                continue;
            }
            let cause = format!(
                "quota: window {} at 100%{}",
                window.id,
                if defaulted {
                    "; one-hour default expiry (resetsAt missing)"
                } else {
                    ""
                }
            );
            if exhausted
                .as_ref()
                .is_none_or(|(latest, _): &(jiff::Timestamp, String)| until > *latest)
            {
                exhausted = Some((until, cause));
            }
        }
        if let Some((until, cause)) = exhausted {
            let mut models = std::collections::BTreeSet::new();
            for tier in [Tier::Light, Tier::Standard, Tier::Strong, Tier::Premium] {
                for entry in agents.model_chain(name, tier)? {
                    models.insert(entry.model);
                }
            }
            mark_quota(
                home,
                &QuotaMark {
                    profile: name.clone(),
                    models,
                    until,
                    cause,
                },
            )?;
        }
    }
    let candidates = if request.preferences.is_empty() {
        request.selection.iter().cloned().collect()
    } else {
        selections(&config, home, &request.preferences, now)?
    };
    let mut tried = Vec::new();
    for selection in candidates {
        let cause = match snapshot
            .providers
            .iter()
            .find(|provider| provider.instance == selection.instance)
        {
            None => Some("instance missing".into()),
            Some(provider) if provider.kind != Some(selection.kind) => {
                Some("instance harness kind differs from configured kind".into())
            }
            Some(provider) if provider.unavailable_reason.is_some() => {
                provider.unavailable_reason.clone()
            }
            Some(provider)
                if !snapshot.native_picks.contains(&selection.model)
                    && !provider.models.iter().any(|model| {
                        model.name == selection.model || model.aliases.contains(&selection.model)
                    }) =>
            {
                Some("model not reported by instance".into())
            }
            Some(_) => None,
        };
        if let Some(cause) = cause {
            tried.push(format!(
                "{} / {}: {cause}",
                selection.profile, selection.model
            ));
            tracing::warn!(event="agent_readiness_skip",profile=%selection.profile,instance=%selection.instance,model=%selection.model,reason=%cause);
            continue;
        }
        let mut prepared = request.clone();
        prepared.selection = Some(selection);
        prepared.selection_evidence =
            Some(adapter.selection_realization(crate::runtime::SelectionOperation::ThreadStart));
        return Ok(prepared);
    }
    Err(crate::sessions::SessionsError::OrderExhausted { tried })
}
