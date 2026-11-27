//! Connector and relay status as reported to the operator.
use instrukt_core::config::Agents;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::_intake::demand;
use super::_store::Stage;
use super::{ConversationError, read_back};

/// A connector's connection state, as `instrukt events status` shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Connecting,
    Connected,
    Failed,
    Stopped,
}
impl ConnectionState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
    pub(super) fn parse(text: &str) -> Option<Self> {
        [
            Self::Connecting,
            Self::Connected,
            Self::Failed,
            Self::Stopped,
        ]
        .into_iter()
        .find(|state| state.as_str() == text)
    }
}

/// Record a connector's connection state for `instrukt events status`.
pub fn connector_state(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    status: ConnectionState,
    failure: Option<&str>,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "INSERT INTO connector_states VALUES (?1,?2,?3,?4,?5) ON CONFLICT(company,connector) DO UPDATE SET state=excluded.state,failure=excluded.failure,updated_at=excluded.updated_at",
            params![company.as_str(), connector, status.as_str(), failure, Timestamp::now().to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectorStatus {
    pub company: String,
    pub connector: String,
    pub state: ConnectionState,
    pub failure: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationStatus {
    pub company: String,
    pub connector: String,
    pub conversation: String,
    pub pending_messages: i64,
    pub pending_answers: i64,
    pub pending_refusals: i64,
    /// Every outstanding failure with its cause: of reading answers, of
    /// delivering or reading a message, of posting an answer or a refusal.
    pub failures: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    /// Why the finished-turn relay serving conversations is not running.
    pub relay_failure: Option<String>,
    pub connectors: Vec<ConnectorStatus>,
    pub conversations: Vec<ConversationStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missed_messages: Vec<read_back::Missed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_back_failures: Vec<read_back::Failure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_back_channel_gaps: Vec<read_back::ChannelGap>,
}

/// Connection states, conversations with pending messages, answers or
/// refusals, and their outstanding failures with their cause; no
/// conversation text or credentials. A connector the running runner does not
/// host is stopped, whatever an earlier run recorded for it, and a declared
/// connector no run ever hosted is listed as stopped too.
pub fn status(home: &Home) -> Result<Status, ConversationError> {
    status_at(home, Timestamp::now())
}

pub fn status_at(home: &Home, now: Timestamp) -> Result<Status, ConversationError> {
    let state = RunnerState::new(home);
    let hosted = instrukt_runner::runner::hosted(home);
    let mut connectors = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT company,connector,state,failure FROM connector_states ORDER BY company,connector")
                .map_err(state::sql(path))?;
            q.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default()
        .into_iter()
        .map(|(company, connector, state, failure)| {
            let state = ConnectionState::parse(&state)
                .ok_or_else(|| ConversationError::Refused(format!("unknown connection state `{state}`")))?;
            let live = hosted.contains(&(company.clone(), connector.clone()));
            Ok(ConnectorStatus {
                company,
                connector,
                state: if live { state } else { ConnectionState::Stopped },
                failure,
            })
        })
        .collect::<Result<Vec<_>, ConversationError>>()?;
    let agents = Agents::load(home).ok();
    for (company, declared) in demand(home)? {
        let failure = instrukt_core::config::connector_role_procedure_finding(
            home,
            &company,
            &declared.key,
            declared.role.as_str(),
            agents.as_ref(),
        )
        .map(|finding| finding.message);
        if let Some(known) = connectors
            .iter_mut()
            .find(|known| known.company == company.as_str() && known.connector == declared.key)
        {
            if failure.is_some() && known.state == ConnectionState::Stopped {
                known.failure = failure;
            }
        } else {
            connectors.push(ConnectorStatus {
                company: company.to_string(),
                connector: declared.key,
                state: ConnectionState::Stopped,
                failure,
            });
        }
    }
    connectors.sort_by(|a, b| (&a.company, &a.connector).cmp(&(&b.company, &b.connector)));
    let rows = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("WITH addresses AS (SELECT company,connector,conversation FROM connected_conversations UNION SELECT company,connector,conversation FROM conversation_messages UNION SELECT company,connector,conversation FROM conversation_answers UNION SELECT company,connector,conversation FROM conversation_refusals) SELECT a.company,a.connector,a.conversation,(SELECT count(*) FROM conversation_messages m WHERE m.company=a.company AND m.connector=a.connector AND m.conversation=a.conversation AND m.state NOT IN (?1,?2) AND m.superseded_at IS NULL AND m.send_unknown_at IS NULL),(SELECT count(*) FROM conversation_answers x WHERE x.company=a.company AND x.connector=a.connector AND x.conversation=a.conversation AND x.posted_at IS NULL),(SELECT count(*) FROM conversation_refusals r WHERE r.company=a.company AND r.connector=a.connector AND r.conversation=a.conversation AND r.reason IN ('not_own','not_admin') AND r.replied_at IS NULL),(SELECT json_group_array(f) FROM (SELECT failure AS f FROM connected_conversations c WHERE c.company=a.company AND c.connector=a.connector AND c.conversation=a.conversation AND failure IS NOT NULL UNION ALL SELECT failure FROM conversation_messages m WHERE m.company=a.company AND m.connector=a.connector AND m.conversation=a.conversation AND failure IS NOT NULL AND m.superseded_at IS NULL UNION ALL SELECT failure FROM conversation_answers x WHERE x.company=a.company AND x.connector=a.connector AND x.conversation=a.conversation AND failure IS NOT NULL UNION ALL SELECT failure FROM conversation_refusals r WHERE r.company=a.company AND r.connector=a.connector AND r.conversation=a.conversation AND failure IS NOT NULL)) FROM addresses a ORDER BY a.company,a.connector,a.conversation")
                .map_err(state::sql(path))?;
            q.query_map([Stage::Answered.as_str(), Stage::Refused.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    let mut conversations = Vec::new();
    for (
        company,
        connector,
        conversation,
        pending_messages,
        pending_answers,
        pending_refusals,
        failures,
    ) in rows
    {
        let failures: Vec<String> = serde_json::from_str(&failures)?;
        if pending_messages > 0
            || pending_answers > 0
            || pending_refusals > 0
            || !failures.is_empty()
        {
            conversations.push(ConversationStatus {
                company,
                connector,
                conversation,
                pending_messages,
                pending_answers,
                pending_refusals,
                failures,
            });
        }
    }
    let relay_failure = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT failure FROM conversation_relay WHERE id=1",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(state::sql(path))
        })?
        .flatten()
        .flatten();
    Ok(Status {
        relay_failure,
        connectors,
        conversations,
        missed_messages: read_back::missed(home, now)?,
        read_back_failures: read_back::failures(home)?,
        read_back_channel_gaps: read_back::channel_gaps(home, now)?,
    })
}
