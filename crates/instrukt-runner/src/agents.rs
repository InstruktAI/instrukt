//! Ordered role/profile selection and expiring local capacity marks.
use crate::interfaces::{AgentPreference, Tier};
pub use instrukt_core::config::AgentPreference as AgentChoice;
use instrukt_core::config::Configuration;
use instrukt_core::ids::ProfileName;
use serde::{Deserialize, Serialize};

pub fn role_order(
    config: &Configuration,
    task: Option<&AgentPreference>,
    role: Option<&str>,
    home_order: Option<&[instrukt_core::ids::ProfileName]>,
) -> Result<Vec<AgentChoice>, crate::sessions::SessionsError> {
    Ok(config
        .agents()?
        .order(
            role,
            task.map(|t| &t.agent),
            task.and_then(|t| t.tier),
            home_order,
        )?
        .into_iter()
        .map(|(agent, tier)| AgentChoice {
            agent,
            tier: Some(tier),
        })
        .collect())
}

/// The order a session that runs in `exact_home` is started on in place of
/// its role mapping's: a company home's `profiles`, when it names them.
pub fn home_order(
    home: &instrukt_core::home::Home,
    exact_home: &str,
) -> Result<Option<Vec<instrukt_core::ids::ProfileName>>, crate::sessions::SessionsError> {
    let Some(company) = exact_home.strip_prefix("company:") else {
        return Ok(None);
    };
    let company = instrukt_core::ids::ProjectSlug::parse(company)
        .map_err(|e| crate::sessions::SessionsError::Refused(e.to_string()))?;
    Ok(Configuration::load_company(home, &company)?.profiles()?)
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
    selections_with(&config.agents()?, home, order, now)
}

/// [`selections`] over the profiles `agents`.
fn selections_with(
    agents: &instrukt_core::config::Agents,
    home: &instrukt_core::home::Home,
    order: &[AgentChoice],
    now: jiff::Timestamp,
) -> Result<Vec<instrukt_core::config::AgentSelection>, crate::sessions::SessionsError> {
    let marks = availability(home, now)?;
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

/// One profile's automatic quota marks as a snapshot supports them: each
/// exhausted model with its expiry and cause.
struct ProfileQuota {
    profile: ProfileName,
    exhausted: std::collections::BTreeMap<String, (jiff::Timestamp, String)>,
}

/// Makes `agent_quota` match a snapshot for every profile it covers, in one
/// transaction: each exhausted model gets its mark, and the profile's other
/// automatic marks are removed. Manual restrictions (`agent_availability`)
/// are never touched.
fn match_quota(
    home: &instrukt_core::home::Home,
    profiles: &[ProfileQuota],
) -> Result<(), crate::sessions::SessionsError> {
    use crate::state::{self, RunnerState, StateError};
    use instrukt_core::state_db::rusqlite::params;
    let mut cleared = Vec::new();
    RunnerState::new(home).write::<_, StateError>(|tx| {
        cleared.clear();
        for quota in profiles {
            let marked = {
                let mut query = tx
                    .prepare("SELECT model FROM agent_quota WHERE agent=?1")
                    .map_err(state::sql(&state::db_path(tx)))?;
                query
                    .query_map(params![quota.profile.as_str()], |row| row.get::<_, String>(0))
                    .and_then(Iterator::collect::<Result<Vec<_>, _>>)
                    .map_err(state::sql(&state::db_path(tx)))?
            };
            for model in marked
                .into_iter()
                .filter(|model| !quota.exhausted.contains_key(model))
            {
                tx.execute(
                    "DELETE FROM agent_quota WHERE agent=?1 AND model=?2",
                    params![quota.profile.as_str(), model],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                cleared.push((quota.profile.clone(), model));
            }
            for (model, (until, cause)) in &quota.exhausted {
                tx.execute("INSERT INTO agent_quota(agent,model,status,status_until,reason) VALUES(?1,?2,'unavailable',?3,?4) ON CONFLICT(agent,model) DO UPDATE SET status_until=excluded.status_until,reason=excluded.reason",params![quota.profile.as_str(),model,until.to_string(),cause]).map_err(state::sql(&state::db_path(tx)))?;
            }
        }
        Ok(())
    })?;
    for quota in profiles {
        let mut marks: std::collections::BTreeMap<&(jiff::Timestamp, String), usize> =
            std::collections::BTreeMap::new();
        for key in quota.exhausted.values() {
            *marks.entry(key).or_default() += 1;
        }
        for ((until, cause), models) in marks {
            tracing::warn!(event="agent_quota_marked",profile=%quota.profile,models,until=%until,reason=%cause);
        }
    }
    for (profile, model) in cleared {
        tracing::info!(event = "agent_quota_cleared", profile = %profile, model = %model);
    }
    Ok(())
}

/// Marks the models of each profile whose provider a snapshot reports at a
/// full usage window, until the window resets, and removes that profile's
/// automatic marks the snapshot no longer supports: once, by the refresh that
/// read the snapshot. A missing reset defaults to one hour after the
/// snapshot was observed, so a snapshot read again never moves the expiry.
pub(crate) fn apply_usage(
    home: &instrukt_core::home::Home,
    observed: &crate::runtime::ObservedSnapshot,
) -> Result<(), crate::sessions::SessionsError> {
    apply_windows(home, &observed.snapshot, observed.at)
}

/// The family a configured model id names: its second `-`-separated
/// segment, after the provider prefix (`claude-fable-5-1` is `fable`), or
/// the whole id when it has one segment (`opus`), lowercase.
fn model_family(model: &str) -> String {
    let mut segments = model.split('-');
    let first = segments.next().unwrap_or(model);
    segments.next().unwrap_or(first).to_lowercase()
}

/// Matches the automatic quota marks to the full usage windows of
/// `snapshot`, observed at `observed_at`.
fn apply_windows(
    home: &instrukt_core::home::Home,
    snapshot: &crate::runtime::ProviderSnapshot,
    observed_at: jiff::Timestamp,
) -> Result<(), crate::sessions::SessionsError> {
    const QUOTA_DEFAULT_SECONDS: i64 = 3600;
    let agents = instrukt_core::config::Agents::load(home)?;
    let mut covered = Vec::new();
    for (name, profile) in &agents.profiles {
        let Some(provider) = snapshot
            .providers
            .iter()
            .find(|p| p.instance == profile.instance)
        else {
            continue;
        };
        let mut models = std::collections::BTreeSet::new();
        for tier in [Tier::Light, Tier::Standard, Tier::Strong, Tier::Premium] {
            for entry in agents.model_chain(name, tier)? {
                models.insert(entry.model);
            }
        }
        // Each model keeps the latest active reset among the full windows
        // that apply to it: a window scoped to a model family applies to
        // that family's models only.
        let mut exhausted: std::collections::BTreeMap<String, (jiff::Timestamp, String)> =
            std::collections::BTreeMap::new();
        for window in provider
            .usage
            .iter()
            .filter(|window| window.used_percent == 100.0)
        {
            let (until, defaulted) = match window.resets_at {
                Some(until) => (until, false),
                None => (
                    observed_at
                        .checked_add(jiff::SignedDuration::from_secs(QUOTA_DEFAULT_SECONDS))
                        .map_err(|_| {
                            crate::sessions::SessionsError::Refused(
                                "cannot compute quota expiry".into(),
                            )
                        })?,
                    true,
                ),
            };
            if until <= observed_at {
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
            for model in models.iter().filter(|model| {
                window
                    .model_family
                    .as_deref()
                    .is_none_or(|family| model_family(model) == family)
            }) {
                if exhausted
                    .get(model)
                    .is_none_or(|(latest, _)| until > *latest)
                {
                    exhausted.insert(model.clone(), (until, cause.clone()));
                }
            }
        }
        covered.push(ProfileQuota {
            profile: name.clone(),
            exhausted,
        });
    }
    match_quota(home, &covered)
}

/// Apply capacity and ordered selection policy to the last readiness facts
/// the runner observed. A start never acquires readiness and is never
/// refused for it: with no snapshot known yet it proceeds in the configured
/// order, and T3's own refusal of the start is the signal.
pub(crate) fn prepare_start(
    home: &instrukt_core::home::Home,
    adapter: &dyn crate::runtime::RuntimeAdapter,
    request: &crate::runtime::RuntimeStart,
    now: jiff::Timestamp,
) -> Result<crate::runtime::RuntimeStart, crate::sessions::SessionsError> {
    let observed = adapter.last_snapshot();
    // A start prepared in an admitted generation carries its profiles; one
    // read back from its record or made by a command reads them here.
    let agents = match &request.profiles.0 {
        Some(profiles) => (**profiles).clone(),
        None => Configuration::load(home, Some(&request.checkout))?.agents()?,
    };
    let candidates = if request.preferences.is_empty() {
        request.selection.iter().cloned().collect()
    } else {
        selections_with(&agents, home, &request.preferences, now)?
    };
    let mut tried = Vec::new();
    for selection in candidates {
        let Some(snapshot) = observed.as_ref().map(|observed| &observed.snapshot) else {
            let mut prepared = request.clone();
            prepared.selection = Some(selection);
            prepared.selection_evidence = Some(
                adapter.selection_realization(crate::runtime::SelectionOperation::ThreadStart),
            );
            return Ok(prepared);
        };
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
