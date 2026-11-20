//! Conversations (`conversations` spec): a person addresses a company's
//! agent through a connector; each conversation is bound to one plain agent
//! session in the company's home, every addressed message is its own turn,
//! and the answer of that turn is posted back. No event is emitted and no
//! subscribed procedure is activated on this path.
//!
//! The record moves each addressed message through its stages: received
//! (retained, with its sender's connector identity, before the connector
//! acknowledges it), refused (its provider's record shows its sender is not
//! one of the company's own), pending (its sender admitted and its
//! accompanying messages captured), sent (accepted by the runtime as its own
//! turn) and answered (its turn finished). Each answer piece stays pending
//! until the connector accepted its post.
use std::path::PathBuf;

use instrukt_connector::{Accompanying, Incoming, Sender};
use instrukt_core::config::{Admitted, COMPANY_AGENT_ROLE, ConfigError, DeclaredConnector};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, SnippetId};
use instrukt_core::index::{GlobalIndex, IndexEntry, RoleProcedure};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::connector_host::Connectors;
use instrukt_runner::envelopes::{self, ContextEntry, Control, Span};
use instrukt_runner::interfaces::CompanyRelayPass;
use instrukt_runner::keyed::{self, RuntimeInvocation};
use instrukt_runner::runtime::conversations::{
    AuthorKind, ConversationSource, Scope, SourcePosition,
};
use instrukt_runner::runtime::{CreationPermit, RuntimeAdapter, RuntimeAssignment, RuntimeStart};
use instrukt_runner::sessions::{SessionState, SessionsError};
use instrukt_runner::state::{self, RunnerState, StateError};

/// The target every log statement here carries: the module path it logged
/// under before the company context became its own crate, so the same
/// settings select it.
const LOG_TARGET: &str = "instrukt_runner::conversations";

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
    fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
    fn parse(text: &str) -> Option<Self> {
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

/// Where an addressed message is on its way to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Retained before the connector acknowledged it.
    Received,
    /// Its sender is not one of the company's own; a reply is owed to them.
    Refused,
    /// Its accompanying messages captured; waiting for its turn.
    Pending,
    /// Accepted by the runtime as its own turn.
    Sent,
    /// Its turn finished; its answer, if any, is retained for posting.
    Answered,
}
impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Refused => "refused",
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Answered => "answered",
        }
    }
    fn parse(text: &str) -> Option<Self> {
        [
            Self::Received,
            Self::Refused,
            Self::Pending,
            Self::Sent,
            Self::Answered,
        ]
        .into_iter()
        .find(|stage| stage.as_str() == text)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Sessions(#[from] SessionsError),
    #[error(transparent)]
    Source(#[from] instrukt_runner::runtime::conversations::SourceError),
    #[error(transparent)]
    Lock(#[from] instrukt_core::fsx::FsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{}", .0.cause)]
    Connector(#[from] instrukt_connector::ConnectorError),
    #[error(transparent)]
    Index(#[from] instrukt_core::index::IndexError),
    #[error(
        "conversation: no published procedure declares the role `{role}`; a conversation starts once one does"
    )]
    NoProcedure { role: &'static str },
    #[error("conversation: {0}")]
    Refused(String),
}

/// What became of an incoming addressed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intake {
    /// Retained, waiting for its sender's admission.
    Retained,
    /// Already retained.
    Repeated,
}

/// The semantic start key of a conversation's session.
pub fn key(company: &ProjectSlug, connector: &str, conversation: &str) -> String {
    format!(
        "instrukt:conversation:{}:{connector}:{}",
        keyed::encode(company.as_str()),
        keyed::encode(conversation)
    )
}

fn micros(at: Timestamp) -> i64 {
    at.as_microsecond()
}

/// Retain one incoming addressed message, with its sender's connector
/// identity, before the connector acknowledges it. Nothing here waits on the
/// network and nothing here decides who is answered.
pub fn retain(
    home: &Home,
    company: &ProjectSlug,
    incoming: &Incoming,
    now: Timestamp,
) -> Result<Intake, ConversationError> {
    let state = RunnerState::new(home);
    let semantic = key(company, &incoming.connector, &incoming.conversation);
    let inserted = state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key) VALUES (?1,?2,?3,?4) ON CONFLICT DO NOTHING",
            params![
                company.as_str(),
                incoming.connector,
                incoming.conversation,
                semantic
            ],
        )
        .map_err(state::sql(&path))?;
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,written_us,written_at,arrived_at,state,text) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT DO NOTHING",
            params![
                company.as_str(),
                incoming.connector,
                incoming.conversation,
                incoming.message,
                incoming.sender,
                micros(incoming.written_at),
                incoming.written_at.to_string(),
                now.to_string(),
                Stage::Received.as_str(),
                incoming.text
            ],
        )
        .map_err(state::sql(&path))
    })?;
    if inserted == 0 {
        return Ok(Intake::Repeated);
    }
    tracing::info!(target: LOG_TARGET, event = "conversation_message_accepted", company = %company, connector = %incoming.connector, conversation = %incoming.conversation);
    Ok(Intake::Retained)
}

/// A received message: its sender still to be admitted, or what
/// accompanies it still to be read.
#[derive(Debug, Clone)]
pub struct Unread {
    pub conversation: String,
    pub message: String,
    /// The previous addressed message of the conversation in written order,
    /// whether or not it was answered, if any: the lower bound of this one's
    /// range.
    pub previous: Option<String>,
    /// The sender's connector identity; `None` for a message stored before
    /// senders were kept, which was admitted when it was stored.
    pub sender: Option<String>,
    /// Whether the sender was admitted.
    pub admitted: bool,
    /// The message's text as written.
    pub text: String,
}

/// The received messages of one company's connector, oldest written first.
pub fn unread(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<Unread>, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT m.conversation,m.message,(SELECT p.message FROM conversation_messages p WHERE p.company=m.company AND p.connector=m.connector AND p.conversation=m.conversation AND p.written_us<m.written_us ORDER BY p.written_us DESC, p.message DESC LIMIT 1),m.sender,m.sender_name IS NOT NULL,m.text FROM conversation_messages m WHERE m.company=?1 AND m.connector=?2 AND m.state=?3 ORDER BY m.written_us, m.message")
                .map_err(state::sql(path))?;
            q.query_map(params![company.as_str(), connector, Stage::Received.as_str()], |r| {
                Ok(Unread {
                    conversation: r.get(0)?,
                    message: r.get(1)?,
                    previous: r.get(2)?,
                    sender: r.get(3)?,
                    admitted: r.get(4)?,
                    text: r.get(5)?,
                })
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default())
}

/// Record the sender of a received message as admitted at `now`, with the
/// name and email address the provider gave; once written they do not
/// change.
pub fn admitted(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    sender: &Sender,
    now: Timestamp,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET sender_name=?5,sender_email=?6,admitted_at=?8,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?7 AND sender_name IS NULL",
            params![company.as_str(), connector, unread.conversation, unread.message, sender.name, sender.email, Stage::Received.as_str(), now.to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// Record that the provider's record shows a received message's sender is
/// not one of the company's own: the message is refused and the one reply
/// owed to that sender in that conversation is retained, in one
/// transaction.
pub fn refused(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    sender: &str,
    now: Timestamp,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let changed = tx
            .execute(
                "UPDATE conversation_messages SET state=?5,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?6",
                params![company.as_str(), connector, unread.conversation, unread.message, Stage::Refused.as_str(), Stage::Received.as_str()],
            )
            .map_err(state::sql(&path))?;
        if changed > 0 {
            tx.execute(
                "INSERT INTO conversation_refusals(company,connector,conversation,sender,received_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
                params![company.as_str(), connector, unread.conversation, sender, now.to_string()],
            )
            .map_err(state::sql(&path))?;
        }
        Ok(())
    })?;
    tracing::info!(target: LOG_TARGET, event = "conversation_sender_refused", company = %company, connector, conversation = %unread.conversation);
    Ok(())
}

/// Record why a received message is still waiting: its sender could not be
/// looked up, or what accompanies it could not be read.
pub fn waiting(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    cause: &str,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET failure=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?6",
            params![company.as_str(), connector, unread.conversation, unread.message, cause, Stage::Received.as_str()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// When a message's no-work interval opened, as the status and the notice
/// both read it: its own interval, or, while the relay cannot acquire the
/// conversation source, that failure's start clamped at the message's
/// admission, whichever is earlier. `None` while neither is open.
pub fn effective_since(
    own: Option<Timestamp>,
    admitted: Option<Timestamp>,
    relay: Option<Timestamp>,
) -> Option<Timestamp> {
    let relay = relay.map(|since| admitted.map_or(since, |admitted| since.max(admitted)));
    match (own, relay) {
        (Some(own), Some(relay)) => Some(own.min(relay)),
        (own, relay) => own.or(relay),
    }
}

/// An unfinished admitted message, and what its status shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfinished {
    pub conversation: String,
    pub message: String,
    pub status: instrukt_connector::Status,
}

/// The admitted messages of one company's connector that are not complete,
/// oldest written first, each with what its status shows. A message is
/// complete once its answer is posted, or its turn completed with nothing to
/// post; one whose answer is not yet posted stays unfinished. It shows the
/// unavailable text while its effective interval ([`effective_since`]) is
/// open, the newest message the agent has written in its turn once there is
/// one, until its answer is posted, and the working text otherwise.
pub fn unfinished(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<Unfinished>, ConversationError> {
    type Row = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let read = RunnerState::new(home).read(|conn, path| {
        let relay: Option<Option<String>> = conn
            .query_row("SELECT updated_at FROM conversation_relay WHERE id=1 AND failure IS NOT NULL", [], |r| r.get(0))
            .optional()
            .map_err(state::sql(path))?;
        let mut q = conn
            .prepare("SELECT m.conversation,m.message,m.unavailable_since,m.admitted_at,m.turn_update FROM conversation_messages m WHERE m.company=?1 AND m.connector=?2 AND ((m.state=?3 AND m.sender_name IS NOT NULL) OR m.state IN (?4,?5) OR (m.state=?6 AND EXISTS(SELECT 1 FROM conversation_answers x WHERE x.company=m.company AND x.connector=m.connector AND x.conversation=m.conversation AND x.turn=m.turn AND x.posted_at IS NULL))) ORDER BY m.written_us, m.message")
            .map_err(state::sql(path))?;
        let rows: Vec<Row> = q
            .query_map(
                params![company.as_str(), connector, Stage::Received.as_str(), Stage::Pending.as_str(), Stage::Sent.as_str(), Stage::Answered.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))?;
        Ok((relay.flatten(), rows))
    })?;
    let Some((relay, rows)) = read else {
        return Ok(Vec::new());
    };
    let at = |text: Option<String>| -> Result<Option<Timestamp>, ConversationError> {
        text.map(|text| {
            text.parse::<Timestamp>().map_err(|e| {
                ConversationError::Refused(format!("unreadable interval time `{text}`: {e}"))
            })
        })
        .transpose()
    };
    let relay = at(relay)?;
    rows.into_iter()
        .map(|(conversation, message, own, admitted, update)| {
            let open = effective_since(at(own)?, at(admitted)?, relay).is_some();
            let status = match update {
                _ if open => instrukt_connector::Status::Unavailable,
                Some(update) => instrukt_connector::Status::Update(update),
                _ => instrukt_connector::Status::Working,
            };
            Ok(Unfinished {
                conversation,
                message,
                status,
            })
        })
        .collect()
}

/// Record what accompanies an admitted received message and frame it for
/// its turn with `text`, its text as the connector supplies it; or record
/// why what accompanies it could not be read.
pub fn captured(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    text: &str,
    accompanying: Result<Accompanying, String>,
) -> Result<(), ConversationError> {
    let state = RunnerState::new(home);
    let accompanying = match accompanying {
        Ok(accompanying) => accompanying,
        Err(failure) => {
            state.write::<_, StateError>(|tx| {
                tx.execute(
                    "UPDATE conversation_messages SET failure=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?6",
                    params![company.as_str(), connector, unread.conversation, unread.message, failure, Stage::Received.as_str()],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })?;
            return Ok(());
        }
    };
    let row = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT sender_name,written_at FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                params![company.as_str(), connector, unread.conversation, unread.message],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(state::sql(path))
        })?
        .ok_or_else(|| ConversationError::Refused("the received message is gone".into()))?;
    let (name, written_at) = row;
    let name = name
        .ok_or_else(|| ConversationError::Refused("the message's sender is not admitted".into()))?;
    let written_at: Timestamp = written_at
        .parse()
        .map_err(|e: jiff::Error| ConversationError::Refused(e.to_string()))?;
    let (token, content) = framed(
        connector,
        &unread.conversation,
        &name,
        written_at,
        text,
        &accompanying,
    )?;
    let context = serde_json::to_string(&accompanying)?;
    state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET state=?5,context_json=?6,boundary=?7,content=?8,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?9",
            params![company.as_str(), connector, unread.conversation, unread.message, Stage::Pending.as_str(), context, token, content, Stage::Received.as_str()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// Retain why a company's intake could not even begin — its Slack access
/// could not be prepared — on its outstanding refusals and received messages.
pub fn intake_failed(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    cause: &str,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "UPDATE conversation_refusals SET failure=?3 WHERE company=?1 AND connector=?2 AND replied_at IS NULL",
            params![company.as_str(), connector, cause],
        )
        .map_err(state::sql(&path))?;
        tx.execute(
            "UPDATE conversation_messages SET failure=?3 WHERE company=?1 AND connector=?2 AND state=?4",
            params![company.as_str(), connector, cause, Stage::Received.as_str()],
        )
        .map_err(state::sql(&path))?;
        Ok(())
    })?;
    Ok(())
}

/// A refusal still to be posted.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub conversation: String,
    pub sender: String,
}

/// The refusals of one company's connector, oldest first, each with whether
/// its reply was posted.
pub fn refusals(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<(Refusal, bool)>, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT conversation,sender,replied_at IS NOT NULL FROM conversation_refusals WHERE company=?1 AND connector=?2 ORDER BY rowid")
                .map_err(state::sql(path))?;
            q.query_map(params![company.as_str(), connector], |r| {
                Ok((
                    Refusal {
                        conversation: r.get(0)?,
                        sender: r.get(1)?,
                    },
                    r.get(2)?,
                ))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default())
}

/// Record that a refusal's post was accepted, or why it failed.
pub fn refusal_posted(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    refusal: &Refusal,
    outcome: Result<(), String>,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let failure = outcome.err();
    RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_refusals SET replied_at=CASE WHEN ?5 IS NULL THEN ?6 END,failure=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND sender=?4",
            params![company.as_str(), connector, refusal.conversation, refusal.sender, failure, now.to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// The company homes whose declared connector `instrukt run` hosts: every
/// valid company home declaring one. Declaring a connector is what gives a
/// company conversations.
pub fn demand(home: &Home) -> Result<Vec<(ProjectSlug, DeclaredConnector)>, ConversationError> {
    let mut hosted = Vec::new();
    for (name, config) in instrukt_core::config::company_homes(home)? {
        match config.and_then(|config| config.connectors()) {
            Ok(declared) => hosted.extend(
                declared
                    .into_iter()
                    .map(|connector| (name.clone(), connector)),
            ),
            Err(ConfigError::NoLayer { .. }) => {}
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "connector_declaration_unreadable", company = %name, error = %error);
            }
        }
    }
    Ok(hosted)
}

/// The connectors the runner hosts: each admitted company's declared
/// connectors, by key.
pub fn admitted_demand(admitted: &Admitted) -> Vec<(ProjectSlug, String)> {
    admitted
        .companies()
        .iter()
        .flat_map(|company| {
            company
                .connectors
                .iter()
                .map(|declared| (company.name.clone(), declared.key.clone()))
        })
        .collect()
}

struct Conversation {
    company: ProjectSlug,
    connector: String,
    conversation: String,
    semantic_key: String,
    opening: Option<String>,
    request: Option<String>,
    session: Option<SessionRef>,
    source_identity: Option<String>,
    position: Option<SourcePosition>,
}

struct Message {
    message: String,
    stage: Stage,
    /// Present from `pending` on.
    boundary: Option<String>,
    /// The framed input, present from `pending` on.
    content: Option<String>,
    /// A send to the runtime was begun; its acceptance may be unrecorded.
    attempted: bool,
    /// The cause retained from its last failed delivery.
    failure: Option<String>,
}

const CONVERSATION_COLUMNS: &str = "company,connector,conversation,semantic_key,opening,request,session,source_identity,position_json";

const RELAY_WORK: &str = "SELECT company,connector,conversation FROM conversation_messages WHERE state IN (?1,?2,?3)
    UNION ALL SELECT company,connector,conversation FROM conversation_answers WHERE posted_at IS NULL";

fn relay_stages() -> [&'static str; 3] {
    [
        Stage::Received.as_str(),
        Stage::Pending.as_str(),
        Stage::Sent.as_str(),
    ]
}

fn conversations(
    state: &RunnerState,
    restriction: &str,
    values: impl instrukt_core::state_db::rusqlite::Params,
) -> Result<Vec<Conversation>, ConversationError> {
    let sql = format!(
        "SELECT {CONVERSATION_COLUMNS} FROM connected_conversations {restriction} ORDER BY rowid"
    );
    let rows = state
        .read(|conn, path| {
            let mut q = conn.prepare(&sql).map_err(state::sql(path))?;
            q.query_map(values, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    rows.into_iter()
        .map(
            |(
                company,
                connector,
                conversation,
                semantic_key,
                opening,
                request,
                session,
                source_identity,
                position,
            )| {
                Ok(Conversation {
                    company: ProjectSlug::parse(&company)
                        .map_err(|e| ConversationError::Refused(e.to_string()))?,
                    connector,
                    conversation,
                    semantic_key,
                    opening,
                    request,
                    session: session
                        .map(|s| SessionRef::parse(&s))
                        .transpose()
                        .map_err(|e| ConversationError::Refused(e.to_string()))?,
                    source_identity,
                    position: position.map(|p| serde_json::from_str(&p)).transpose()?,
                })
            },
        )
        .collect()
}

fn reread(
    state: &RunnerState,
    c: &Conversation,
) -> Result<Option<Conversation>, ConversationError> {
    Ok(conversations(
        state,
        "WHERE company=?1 AND connector=?2 AND conversation=?3",
        params![c.company.as_str(), c.connector, c.conversation],
    )?
    .pop())
}

/// The conversation's messages neither answered nor refused, oldest written
/// first.
fn open_messages(state: &RunnerState, c: &Conversation) -> Result<Vec<Message>, ConversationError> {
    let rows = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT message,state,boundary,content,attempted_at IS NOT NULL,failure FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND state NOT IN (?4,?5) ORDER BY written_us, message")
                .map_err(state::sql(path))?;
            q.query_map(params![c.company.as_str(), c.connector, c.conversation, Stage::Answered.as_str(), Stage::Refused.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    rows.into_iter()
        .map(|(message, stage, boundary, content, attempted, failure)| {
            Ok(Message {
                message,
                stage: Stage::parse(&stage).ok_or_else(|| {
                    ConversationError::Refused(format!("unknown message stage `{stage}`"))
                })?,
                boundary,
                content,
                attempted,
                failure,
            })
        })
        .collect()
}

fn lock(home: &Home, c: &Conversation) -> Result<instrukt_core::fsx::LockGuard, ConversationError> {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(&Sha256::digest(c.semantic_key.as_bytes())[..8]);
    Ok(instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!("conversation-{digest}.lock")),
        std::time::Duration::from_secs(120),
    )?)
}

/// One addressed message framed for its own turn: the context envelope and
/// the member envelope naming its sender, sharing one fresh boundary token
/// that occurs nowhere in what they frame.
fn framed(
    connector: &str,
    conversation: &str,
    sender: &str,
    written_at: Timestamp,
    text: &str,
    accompanying: &Accompanying,
) -> Result<(String, String), ConversationError> {
    let mut scanned: Vec<&str> = vec![text, connector, conversation, sender];
    for entry in &accompanying.messages {
        scanned.push(&entry.text);
        scanned.push(&entry.speaker);
    }
    let token = envelopes::delivery_boundary(&scanned)?;
    let entries: Vec<ContextEntry<'_>> = accompanying
        .messages
        .iter()
        .map(|e| ContextEntry {
            speaker: &e.speaker,
            own: e.own,
            at: e.at,
            text: &e.text,
        })
        .collect();
    let input = format!(
        "{}\n{}",
        envelopes::context(&token, &entries, accompanying.left_out),
        envelopes::member(
            connector,
            conversation,
            sender,
            Some(written_at),
            &token,
            text
        )
    );
    Ok((token, input))
}

/// Whether a failed delivery is logged. A message waiting for its procedure
/// is logged once, when its cause starts or changes: each retry finds the
/// same cause already retained with the message, which status shows. Every
/// other failure is logged as it happens.
fn reports(error: &ConversationError, retained: Option<&str>, cause: &str) -> bool {
    !matches!(error, ConversationError::NoProcedure { .. }) || retained != Some(cause)
}

/// The procedure a conversation's session starts from: the delivered
/// procedure declaring the role. A pending one, like none at all, leaves the
/// conversation without one, and its first message waits.
fn opening_procedure(
    declared: Option<RoleProcedure<'_>>,
) -> Result<&IndexEntry, ConversationError> {
    match declared {
        Some(RoleProcedure::Delivered(entry)) => Ok(entry),
        Some(RoleProcedure::Pending(_)) | None => Err(ConversationError::NoProcedure {
            role: COMPANY_AGENT_ROLE,
        }),
    }
}

/// The runner-authored opening assignment of a conversation session: the
/// loader of its procedure, with the facts it is assigned.
fn assignment(
    admitted: &Admitted,
    c: &Conversation,
    procedure: &SnippetId,
) -> Result<String, ConversationError> {
    let capabilities = admitted
        .agents()
        .capabilities(COMPANY_AGENT_ROLE)?
        .describe();
    Ok(Control::Assignment.frame(&format!(
        "instrukt conversation assignment\nProcedure: {procedure}\nRole: {COMPANY_AGENT_ROLE}\nResolved role capabilities: {capabilities}\nCompany: {}\nConnector: {}\nConversation: {}\nLoad `instrukt docs get {procedure} --closure` before handling the member's message.\n",
        c.company, c.connector, c.conversation
    )))
}

/// The position a new session's answers are read from: one over no
/// conversation yet, so everything its own thread comes to hold is read after
/// it. Capturing it reads no native history.
fn opening_position(source: &dyn ConversationSource) -> Result<SourcePosition, ConversationError> {
    Ok(source.changed_since(None, Scope::Only(&[]))?.position)
}

fn set_failure(
    state: &RunnerState,
    c: &Conversation,
    failure: Option<&str>,
) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            "UPDATE connected_conversations SET failure=?4 WHERE company=?1 AND connector=?2 AND conversation=?3",
            params![c.company.as_str(), c.connector, c.conversation, failure],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

fn set_message_failure(
    state: &RunnerState,
    c: &Conversation,
    message: &str,
    failure: Option<&str>,
) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET failure=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
            params![c.company.as_str(), c.connector, c.conversation, message, failure],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// Open the no-work interval of each of `messages` at `at`, where none is
/// open: a confirmed failure keeps the agent from the work they wait on.
fn unavailable(
    state: &RunnerState,
    c: &Conversation,
    messages: &[String],
    at: Timestamp,
) -> Result<(), StateError> {
    if messages.is_empty() {
        return Ok(());
    }
    state.write(|tx| {
        for message in messages {
            tx.execute(
                "UPDATE conversation_messages SET unavailable_since=coalesce(unavailable_since,?5) WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                params![c.company.as_str(), c.connector, c.conversation, message, at.to_string()],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
        }
        Ok(())
    })
}

/// Close the no-work interval of each of `messages`: the work on them is
/// shown to go on.
fn available(state: &RunnerState, c: &Conversation, messages: &[String]) -> Result<(), StateError> {
    if messages.is_empty() {
        return Ok(());
    }
    state.write(|tx| {
        for message in messages {
            tx.execute(
                "UPDATE conversation_messages SET unavailable_since=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND unavailable_since IS NOT NULL",
                params![c.company.as_str(), c.connector, c.conversation, message],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
        }
        Ok(())
    })
}

/// Keep `update`, the newest message the agent has written in the running
/// turn of `message`, as what its status shows.
fn turn_update(
    state: &RunnerState,
    c: &Conversation,
    message: &str,
    update: Option<&str>,
) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET turn_update=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND turn_update IS NOT ?5",
            params![c.company.as_str(), c.connector, c.conversation, message, update],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// The runtime accepted `message` as its own turn, which ends its no-work
/// interval; a first message's acceptance also binds the conversation's
/// session.
fn accepted(
    state: &RunnerState,
    c: &Conversation,
    message: &str,
    session: Option<&SessionRef>,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "UPDATE conversation_messages SET state=?5,failure=NULL,unavailable_since=NULL,turn_update=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
            params![c.company.as_str(), c.connector, c.conversation, message, Stage::Sent.as_str()],
        )
        .map_err(state::sql(&path))?;
        if let Some(session) = session {
            tx.execute(
                "UPDATE connected_conversations SET session=?4 WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![c.company.as_str(), c.connector, c.conversation, session.to_string()],
            )
            .map_err(state::sql(&path))?;
        }
        Ok(())
    })
}

/// The message the conversation delivers next, if one is ready: none while a
/// turn it handed over has not finished. Until its session is bound, a
/// captured opening request belongs to its own message alone.
fn next_message<'a>(open: &'a [Message], c: &Conversation) -> Option<&'a Message> {
    if open.iter().any(|m| m.stage == Stage::Sent) {
        return None;
    }
    let next = match (&c.session, &c.opening) {
        (None, Some(opening)) => open.iter().find(|m| &m.message == opening),
        _ => open.first(),
    }?;
    (next.stage == Stage::Pending).then_some(next)
}

/// Deliver the conversation's next addressed message. The first starts its
/// session by key in the company's home, with an opening request owned by
/// that one message; a later one waits until the session's current turn has
/// finished. Failures stay with the message until it is delivered.
fn deliver_one(
    invocation: &RuntimeInvocation,
    admitted: &Admitted,
    adapter: &dyn RuntimeAdapter,
    source: &dyn ConversationSource,
    state: &RunnerState,
    c: &Conversation,
    clock: &dyn Fn() -> Timestamp,
) -> Result<(), ConversationError> {
    let now = clock();
    let open = open_messages(state, c)?;
    let Some(next) = next_message(&open, c) else {
        return Ok(());
    };
    let result = match &c.session {
        None => start(invocation, admitted, adapter, source, c, next, now),
        Some(session) => send(adapter, state, c, session, next, now),
    };
    // A runtime failure is retained through the adapter's safe diagnostic,
    // never its raw native output. An unresolved start says so, with its key
    // and the command that resolves it.
    let failure = result.as_ref().err().map(|error| match error {
        ConversationError::Sessions(
            instrukt_runner::sessions::SessionsError::Unresolved { key }
            | instrukt_runner::sessions::SessionsError::Uncertain { key, .. },
        ) => instrukt_runner::sessions::SessionsError::Unresolved { key: key.clone() }.to_string(),
        ConversationError::Sessions(native) => adapter
            .diagnostic("conversation delivery", native)
            .to_string(),
        other => other.to_string(),
    });
    if let (Err(error), Some(cause)) = (&result, &failure)
        && reports(error, next.failure.as_deref(), cause)
    {
        tracing::warn!(target: LOG_TARGET, event = "conversation_delivery_failed", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %cause);
    }
    set_message_failure(state, c, &next.message, failure.as_deref())?;
    // A start, send or listing that is refused or fails keeps the agent from
    // the message: its no-work interval opens when the failure is
    // established, read on the clock then.
    if result.is_err() {
        unavailable(state, c, std::slice::from_ref(&next.message), clock())?;
    }
    result
}

fn start(
    invocation: &RuntimeInvocation,
    admitted: &Admitted,
    adapter: &dyn RuntimeAdapter,
    source: &dyn ConversationSource,
    c: &Conversation,
    next: &Message,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let home = invocation.home();
    let state = &RunnerState::new(home);
    let request = match &c.request {
        Some(request) => serde_json::from_str::<RuntimeStart>(request)?,
        None => {
            let index = GlobalIndex::load(home)?.unwrap_or_default();
            let role = RoleName::parse(COMPANY_AGENT_ROLE)
                .map_err(|e| ConversationError::Refused(e.to_string()))?;
            let procedure =
                opening_procedure(index.procedures_declaring(&role).into_iter().next())?
                    .id
                    .clone();
            let checkout: PathBuf = home.company_dir(&c.company);
            let company = &admitted
                .company(&c.company)
                .ok_or_else(|| {
                    ConversationError::Refused(format!(
                        "company `{}` is not admitted; `instrukt config validate --company {}` names why",
                        c.company, c.company
                    ))
                })?
                .config;
            let config = admitted.global();
            let preferences = instrukt_runner::agents::role_order(
                config,
                None,
                Some(COMPANY_AGENT_ROLE),
                company.profiles()?.as_deref(),
            )?;
            let selection = instrukt_runner::agents::selections(config, home, &preferences, now)?
                .into_iter()
                .next();
            let input = next.content.as_deref().ok_or_else(|| {
                ConversationError::Refused("the message is not framed yet".into())
            })?;
            let request = RuntimeStart {
                preferences,
                selection,
                selection_evidence: Some(adapter.selection_realization(
                    instrukt_runner::runtime::SelectionOperation::ThreadStart,
                )),
                checkout,
                title: instrukt_runner::runtime::thread_title(
                    &format!("conversation · {}", c.connector),
                    &c.semantic_key,
                ),
                key: c.semantic_key.clone(),
                brief: format!("{}\n{input}", assignment(admitted, c, &procedure)?),
                worktree: None,
                profiles: instrukt_runner::runtime::Profiles(Some(admitted.shared_agents())),
            };
            let position = opening_position(source)?;
            state.write::<_, StateError>(|tx| {
                let path = state::db_path(tx);
                tx.execute(
                    "UPDATE connected_conversations SET opening=?4,request=?5,source_identity=?6,position_json=?7 WHERE company=?1 AND connector=?2 AND conversation=?3",
                    params![
                        c.company.as_str(),
                        c.connector,
                        c.conversation,
                        next.message,
                        serde_json::to_string(&request).map_err(|e| state::invalid(&path, e))?,
                        source.identity(),
                        serde_json::to_string(&position).map_err(|e| state::invalid(&path, e))?
                    ],
                )
                .map_err(state::sql(&path))?;
                Ok(())
            })?;
            request
        }
    };
    let thread = adapter.start_or_wake(&RuntimeAssignment {
        creation: CreationPermit::Initial,
        invocation,
        project: None,
        request: &request,
        now,
    })?;
    accepted(state, c, &next.message, Some(&thread.session))?;
    tracing::info!(target: LOG_TARGET, event = "conversation_session_started", company = %c.company, connector = %c.connector, key = %c.semantic_key, session = %thread.session);
    Ok(())
}

fn send(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    session: &SessionRef,
    next: &Message,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let content = next
        .content
        .as_deref()
        .ok_or_else(|| ConversationError::Refused("the message is not framed yet".into()))?;
    // A send whose acceptance was lost is observed before sending again.
    if next.attempted && reconcile(adapter, state, c, session, next, now)? {
        return Ok(());
    }
    let thread = adapter.read_state(session)?;
    if !matches!(
        thread.state,
        SessionState::InputReady | SessionState::EndedFailed
    ) {
        return Ok(());
    }
    // The attempt is recorded before the runtime can accept it, so a lost
    // acceptance is reconciled before any answer is read past it.
    state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET attempted_at=coalesce(attempted_at,?5) WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
            params![c.company.as_str(), c.connector, c.conversation, next.message, now.to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    adapter.send(session, content)?;
    accepted(state, c, &next.message, None)?;
    Ok(())
}

/// Whether an attempted send was accepted by the runtime after all; if so,
/// its acceptance is recorded.
fn reconcile(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    session: &SessionRef,
    message: &Message,
    now: Timestamp,
) -> Result<bool, ConversationError> {
    let Some(content) = message.content.as_deref() else {
        return Ok(false);
    };
    if adapter.observe_input(session, content, now)?.state
        == instrukt_runner::runtime::InputState::Unknown
    {
        return Ok(false);
    }
    accepted(state, c, &message.message, None)?;
    Ok(true)
}

/// Split `text` into parts of at most `limit` characters, changing nothing
/// in it. While more than `limit` characters remain, a part takes the first
/// `limit` of them and ends after the last line break among them when that
/// line break lies after the first half, and after the `limit`th character
/// otherwise. The remainder is the last part, unchanged; an empty text has
/// no part.
pub fn split(text: &str, limit: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text;
    while rest.chars().count() > limit {
        let mut end = 0;
        let mut after_break = None;
        for (index, (at, character)) in rest.char_indices().take(limit).enumerate() {
            end = at + character.len_utf8();
            if character == '\n' && index >= limit / 2 {
                after_break = Some(end);
            }
        }
        let at = after_break.unwrap_or(end);
        parts.push(rest[..at].to_owned());
        rest = &rest[at..];
    }
    if !rest.is_empty() {
        parts.push(rest.to_owned());
    }
    parts
}

/// Read finished turns of a conversation session from its own retained
/// source position: the conversational output of a turn its addressed
/// message started becomes that message's answer. A turn started any other
/// way is not posted. Missing native content leaves the delivery unresolved
/// with its cause; the position does not move past it.
fn harvest(
    state: &RunnerState,
    source: &dyn ConversationSource,
    c: &Conversation,
    message_chars: usize,
    clock: &dyn Fn() -> Timestamp,
) -> Result<(), ConversationError> {
    let (Some(session), Some(mut position)) = (c.session.clone(), c.position.clone()) else {
        return Ok(());
    };
    if c.source_identity.as_deref() != Some(source.identity()) {
        return Err(ConversationError::Refused(
            "the selected conversation source changed; restore the recorded source".into(),
        ));
    }
    // Only the session's own thread is read: other threads, changing or
    // not, neither delay nor fail its answers.
    let own = std::slice::from_ref(&session.id);
    loop {
        let batch = source.changed_since(Some(&position), Scope::Only(own))?;
        if batch.missing.iter().any(|id| id == &session.id) {
            return Err(ConversationError::Refused(
                "the session's native conversation content is missing; restore it".into(),
            ));
        }
        // Its answers stay unread, with this cause, until a read is coherent.
        if batch.unsettled.iter().any(|id| id == &session.id) {
            return Err(ConversationError::Refused(
                "the session's native conversation kept changing while it was read; the next catch-up reads it again".into(),
            ));
        }
        if let Some(failure) = batch
            .turn_failures
            .iter()
            .find(|failure| failure.conversation == session.id)
        {
            return Err(ConversationError::Refused(format!(
                "completed-turn evidence unavailable: {}",
                failure.cause
            )));
        }
        let mut turns = batch
            .completed_turns
            .iter()
            .filter(|turn| turn.conversation == session.id)
            .collect::<Vec<_>>();
        turns.sort_by_key(|turn| turn.order);
        // A completion belongs to the message its turn carried: one accepted,
        // or one whose send was begun with its acceptance still unrecorded.
        let sent = open_messages(state, c)?
            .into_iter()
            .filter(|m| m.stage == Stage::Sent || (m.stage == Stage::Pending && m.attempted))
            .collect::<Vec<_>>();
        let mut answers = Vec::new();
        for turn in turns {
            let started = turn
                .contributions
                .iter()
                .filter(|contribution| contribution.author.kind == AuthorKind::Human)
                .flat_map(|contribution| envelopes::segment(&contribution.text))
                .find_map(|span| match span {
                    Span::Member { boundary, .. } => sent
                        .iter()
                        .find(|m| m.boundary.as_deref() == Some(boundary)),
                    _ => None,
                });
            let Some(message) = started else {
                continue;
            };
            let output = turn
                .contributions
                .iter()
                .filter(|contribution| contribution.author.kind == AuthorKind::Agent)
                .map(|contribution| contribution.text.as_str())
                // Only whether it says anything is judged on its trimmed
                // form; what is posted is the text as the agent wrote it.
                .filter(|text| {
                    let said = text.trim();
                    !said.is_empty() && said != "NOOP"
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            answers.push((
                message.message.clone(),
                turn.native_turn.to_string(),
                output,
            ));
        }
        position = batch.position;
        let ended = clock();
        state.write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            for (message, turn, output) in &answers {
                for (piece, text) in split(output, message_chars).iter().enumerate() {
                    tx.execute(
                        "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING",
                        params![c.company.as_str(), c.connector, c.conversation, turn, i64::try_from(piece).unwrap_or(i64::MAX), text],
                    )
                    .map_err(state::sql(&path))?;
                }
                tx.execute(
                    // The finished turn proves the message was accepted: its
                    // resolved delivery cause is retired with it. Its turn's
                    // end is kept before any post is tried: a post that then
                    // fails opens its interval from that end. A turn with an
                    // answer to post opens nothing now, and its status keeps
                    // its newest message while the answer is posted; one with
                    // nothing to post completes the message.
                    "UPDATE conversation_messages SET state=?5,turn=?6,failure=NULL,turn_ended_at=?8,turn_update=CASE WHEN ?7 THEN turn_update END,unavailable_since=CASE WHEN ?7 THEN unavailable_since END WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                    params![c.company.as_str(), c.connector, c.conversation, message, Stage::Answered.as_str(), turn, !output.is_empty(), ended.to_string()],
                )
                .map_err(state::sql(&path))?;
            }
            tx.execute(
                "UPDATE connected_conversations SET position_json=?4 WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![c.company.as_str(), c.connector, c.conversation, serde_json::to_string(&position).map_err(|e| state::invalid(&path, e))?],
            )
            .map_err(state::sql(&path))?;
            Ok(())
        })?;
        if !batch.has_more {
            break;
        }
    }
    Ok(())
}

/// The answer pieces this process saw the connector accept whose acceptance
/// it has not yet recorded. Such a piece is never posted again: only its
/// record is retried, on every pass, until it is written.
#[derive(Debug, Default)]
pub struct Accepted(std::sync::Mutex<std::collections::BTreeSet<AcceptedPiece>>);

/// One accepted piece: its conversation's address, its turn and its index.
type AcceptedPiece = (String, String, String, String, i64);

impl Accepted {
    fn key(c: &Conversation, turn: &str, piece: i64) -> AcceptedPiece {
        (
            c.company.to_string(),
            c.connector.clone(),
            c.conversation.clone(),
            turn.to_owned(),
            piece,
        )
    }
    fn holds(&self, c: &Conversation, turn: &str, piece: i64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&Self::key(c, turn, piece))
    }
    fn keep(&self, c: &Conversation, turn: &str, piece: i64) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(Self::key(c, turn, piece));
    }
    fn release(&self, c: &Conversation, turn: &str, piece: i64) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&Self::key(c, turn, piece));
    }
}

/// One outstanding answer piece: its turn, its index, its text and when its
/// post was last begun.
struct Pending {
    turn: String,
    piece: i64,
    text: String,
    attempted: bool,
}

/// Post the conversation's retained answer pieces in order, each once. A
/// piece is recorded as being posted before its post is sent. One whose
/// acceptance this process saw is never posted again; only its record is
/// retried. One left being posted with no acceptance recorded is first
/// looked for among the connector's own posts. A failed piece keeps its
/// cause, and the pieces after it wait.
fn post(
    admitted: &Admitted,
    state: &RunnerState,
    connectors: &dyn Connectors,
    accepted: &Accepted,
    c: &Conversation,
) -> Result<(), ConversationError> {
    let pending = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT turn,piece,text,attempted_at IS NOT NULL FROM conversation_answers WHERE company=?1 AND connector=?2 AND conversation=?3 AND posted_at IS NULL ORDER BY rowid")
                .map_err(state::sql(path))?;
            q.query_map(params![c.company.as_str(), c.connector, c.conversation], |r| {
                Ok(Pending {
                    turn: r.get(0)?,
                    piece: r.get(1)?,
                    text: r.get(2)?,
                    attempted: r.get(3)?,
                })
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    if pending.is_empty() {
        return Ok(());
    }
    // A failure before any request is the cause of the first outstanding
    // piece, kept until that same post succeeds.
    let connector = match connectors.open(admitted, &c.company, &c.connector) {
        Ok(connector) => connector,
        Err(error) => {
            piece_failed(state, c, &pending[0], &error.cause)?;
            return Err(error.into());
        }
    };
    for piece in &pending {
        if accepted.holds(c, &piece.turn, piece.piece) {
            posted(state, c, piece)?;
            accepted.release(c, &piece.turn, piece.piece);
            continue;
        }
        if piece.attempted && landed(state, connector.as_ref(), c, piece)? {
            posted(state, c, piece)?;
            continue;
        }
        state.write::<_, StateError>(|tx| {
            tx.execute(
                "UPDATE conversation_answers SET attempted_at=?6 WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5",
                params![c.company.as_str(), c.connector, c.conversation, piece.turn, piece.piece, Timestamp::now().to_string()],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
            Ok(())
        })?;
        if let Err(error) = connector.answer(&c.conversation, &piece.text) {
            piece_failed(state, c, piece, &error.cause)?;
            return Err(ConversationError::Refused(format!(
                "post failed and stays pending: {}",
                error.cause
            )));
        }
        if let Err(error) = posted(state, c, piece) {
            accepted.keep(c, &piece.turn, piece.piece);
            return Err(error);
        }
    }
    Ok(())
}

/// Whether the post of `piece`, begun earlier with no acceptance recorded,
/// landed: more of the connector's own posts in the conversation since its
/// first addressed message carry its text than pieces with that text are
/// recorded posted.
fn landed(
    state: &RunnerState,
    connector: &dyn instrukt_connector::Connector,
    c: &Conversation,
    piece: &Pending,
) -> Result<bool, ConversationError> {
    let counted = state
        .read(|conn, path| {
            let first: Option<String> = conn
                .query_row(
                    "SELECT message FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 ORDER BY written_us, message LIMIT 1",
                    params![c.company.as_str(), c.connector, c.conversation],
                    |r| r.get(0),
                )
                .optional()
                .map_err(state::sql(path))?;
            let recorded: i64 = conn
                .query_row(
                    "SELECT count(*) FROM conversation_answers WHERE company=?1 AND connector=?2 AND conversation=?3 AND posted_at IS NOT NULL AND text=?4",
                    params![c.company.as_str(), c.connector, c.conversation, piece.text],
                    |r| r.get(0),
                )
                .map_err(state::sql(path))?;
            Ok((first, recorded))
        })?;
    let Some((Some(first), recorded)) = counted else {
        return Ok(false);
    };
    let own = match connector.own_posts(&c.conversation, &first) {
        Ok(own) => own,
        Err(error) => {
            piece_failed(state, c, piece, &error.cause)?;
            return Err(error.into());
        }
    };
    let carried = own.iter().filter(|text| **text == piece.text).count();
    Ok(i64::try_from(carried).unwrap_or(i64::MAX) > recorded)
}

/// Record that the connector accepted `piece`.
fn posted(state: &RunnerState, c: &Conversation, piece: &Pending) -> Result<(), ConversationError> {
    state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_answers SET posted_at=?6,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5",
            params![c.company.as_str(), c.connector, c.conversation, piece.turn, piece.piece, Timestamp::now().to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// Keep `cause` as the last failure of `piece`.
fn piece_failed(
    state: &RunnerState,
    c: &Conversation,
    piece: &Pending,
    cause: &str,
) -> Result<(), ConversationError> {
    state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_answers SET failure=?6 WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5",
            params![c.company.as_str(), c.connector, c.conversation, piece.turn, piece.piece, cause],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// How this relay pass stands with the runtime's dispatch lock: not asked
/// for yet, held, or not had within its wait.
enum Dispatch {
    Untried,
    Held(RuntimeInvocation),
    TimedOut,
}

/// How long the relay waits for a conversation's lock or the runtime's
/// dispatch lock before it leaves that work for its next pass.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The connector destination of the finished-turn relay `instrukt run`
/// already hosts: for every conversation, read its finished turns into
/// answers, post what is retained, observe the turn its session runs, then
/// deliver its next addressed message. A conversation whose lock is not had
/// within its wait is left for the next pass, as is delivery while the
/// runtime's dispatch lock is not had; the messages that waited on either
/// have their no-work interval opened.
pub fn relay(
    home: &Home,
    admitted: &Admitted,
    source: &dyn ConversationSource,
    adapter: &dyn RuntimeAdapter,
    connectors: &dyn Connectors,
    accepted: &Accepted,
    pass: CompanyRelayPass,
) -> Result<(), ConversationError> {
    let state = RunnerState::new(home);
    let mut dispatch = Dispatch::Untried;
    let work = match pass {
        CompanyRelayPass::CatchUp => conversations(&state, "", [])?,
        CompanyRelayPass::Retry => conversations(
            &state,
            &format!("WHERE (company,connector,conversation) IN ({RELAY_WORK})"),
            relay_stages(),
        )?,
    };
    for initial in work {
        let _lock = match lock(home, &initial) {
            Ok(lock) => lock,
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "conversation_lock_unavailable", company = %initial.company, connector = %initial.connector, key = %initial.semantic_key, error = %error);
                let waited = open_messages(&state, &initial)?
                    .into_iter()
                    .filter(|m| matches!(m.stage, Stage::Pending | Stage::Sent))
                    .map(|m| m.message)
                    .collect::<Vec<_>>();
                unavailable(&state, &initial, &waited, connectors.now())?;
                continue;
            }
        };
        let Some(c) = reread(&state, &initial)? else {
            continue;
        };
        // Attempted sends whose acceptance went unrecorded are reconciled
        // before any answer is read past them.
        if let Some(session) = &c.session {
            for message in open_messages(&state, &c)?
                .into_iter()
                .filter(|m| m.stage == Stage::Pending && m.attempted)
            {
                if let Err(error) =
                    reconcile(adapter, &state, &c, session, &message, connectors.now())
                {
                    let cause = match &error {
                        ConversationError::Sessions(native) => adapter
                            .diagnostic("conversation delivery", native)
                            .to_string(),
                        other => other.to_string(),
                    };
                    set_message_failure(&state, &c, &message.message, Some(&cause))?;
                }
            }
        }
        if !answers(admitted, &state, source, connectors, accepted, &c)? {
            continue;
        }
        let Some(c) = reread(&state, &c)? else {
            continue;
        };
        observe(adapter, source, &state, &c, &|| connectors.now())?;
        let ready = next_message(&open_messages(&state, &c)?, &c).map(|m| m.message.clone());
        let Some(ready) = ready else {
            continue;
        };
        if matches!(dispatch, Dispatch::Untried) {
            dispatch = match RuntimeInvocation::acquire(home, LOCK_WAIT) {
                Ok(invocation) => Dispatch::Held(invocation),
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "conversation_dispatch_unavailable", error = %error);
                    Dispatch::TimedOut
                }
            };
        }
        let invocation = match &dispatch {
            Dispatch::Held(invocation) => invocation,
            // The message waited on the dispatch lock it did not get.
            Dispatch::TimedOut | Dispatch::Untried => {
                unavailable(&state, &c, std::slice::from_ref(&ready), connectors.now())?;
                continue;
            }
        };
        // Its failure stays with the message it concerns.
        let _ = deliver_one(
            invocation,
            admitted,
            adapter,
            source,
            &state,
            &c,
            // Restrictions are judged on the runner's clock, as dispatch
            // and `agents availability` judge them.
            &|| connectors.now(),
        );
        // A session bound just now may be one a recovery found with its
        // opening turn already finished while the runner was down: its answer
        // is read now, not left for an unrelated native change.
        if c.session.is_none()
            && let Some(bound) = reread(&state, &c)?
            && bound.session.is_some()
        {
            answers(admitted, &state, source, connectors, accepted, &bound)?;
        }
    }
    clear_idle_answer_failures(&state)?;
    Ok(())
}

/// A conversation without remaining answer work has no answer-read failure
/// to report. Clear it without locking the conversation or reading T3, even
/// when all other work is also finished.
fn clear_idle_answer_failures(state: &RunnerState) -> Result<(), StateError> {
    let idle_failure =
        format!("failure IS NOT NULL AND (company,connector,conversation) NOT IN ({RELAY_WORK})");
    let due = state.read(|conn, path| {
        conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM connected_conversations WHERE {idle_failure})"),
            relay_stages(),
            |row| row.get::<_, bool>(0),
        )
        .map_err(state::sql(path))
    })?;
    if !due.unwrap_or(false) {
        return Ok(());
    }
    state.write(|tx| {
        tx.execute(
            &format!("UPDATE connected_conversations SET failure=NULL WHERE {idle_failure}"),
            relay_stages(),
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// What the session's running turn shows of the messages handed to it. A
/// running turn shows the work going on for every message waiting behind it,
/// and for its own message, whose status then shows the newest message the
/// agent has written in it. A session whose turn cannot be observed opens the
/// no-work interval of what was handed to it; one showing no running turn
/// changes nothing.
fn observe(
    adapter: &dyn RuntimeAdapter,
    source: &dyn ConversationSource,
    state: &RunnerState,
    c: &Conversation,
    clock: &dyn Fn() -> Timestamp,
) -> Result<(), ConversationError> {
    let Some(session) = &c.session else {
        return Ok(());
    };
    let open = open_messages(state, c)?;
    let names = |stage: Stage| {
        open.iter()
            .filter(|m| m.stage == stage)
            .map(|m| m.message.clone())
            .collect::<Vec<_>>()
    };
    let (sent, waiting) = (names(Stage::Sent), names(Stage::Pending));
    if sent.is_empty() && waiting.is_empty() {
        return Ok(());
    }
    let opening = match adapter.active_turn_input(session) {
        Ok(Some(opening)) => opening,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, event = "conversation_turn_unobserved", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
            unavailable(state, c, &sent, clock())?;
            return Ok(());
        }
    };
    available(state, c, &waiting)?;
    let own = envelopes::segment(&opening)
        .into_iter()
        .find_map(|span| match span {
            Span::Member { boundary, .. } => open
                .iter()
                .find(|m| m.stage == Stage::Sent && m.boundary.as_deref() == Some(boundary)),
            _ => None,
        });
    let Some(own) = own else {
        return Ok(());
    };
    available(state, c, std::slice::from_ref(&own.message))?;
    match source.latest_update(&session.id) {
        Ok(update) => turn_update(state, c, &own.message, update.as_deref())?,
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, event = "conversation_turn_unobserved", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
            unavailable(state, c, std::slice::from_ref(&own.message), clock())?;
        }
    }
    Ok(())
}

/// Read the conversation's finished turns into answers and post what is
/// retained. Returns whether the read succeeded; its cause is kept on the
/// conversation until a later read recovers. A failed read opens the no-work
/// interval of the turns handed over; a turn read as ended whose answer is
/// not posted opens its message's, and a posted answer closes it.
fn answers(
    admitted: &Admitted,
    state: &RunnerState,
    source: &dyn ConversationSource,
    connectors: &dyn Connectors,
    accepted: &Accepted,
    c: &Conversation,
) -> Result<bool, ConversationError> {
    // An answer tried earlier and not confirmed is kept from the person
    // whatever this pass manages: its interval opens before anything here
    // can fail.
    unconfirmed(state, c, connectors.now())?;
    // Answers are split to the connector's own limit, so reading them waits
    // until the connector can be opened.
    let read = connectors
        .open(admitted, &c.company, &c.connector)
        .map_err(ConversationError::from)
        .and_then(|connector| {
            harvest(state, source, c, connector.limits().message_chars, &|| {
                connectors.now()
            })
        });
    set_failure(
        state,
        c,
        read.as_ref().err().map(ToString::to_string).as_deref(),
    )?;
    if let Err(error) = read {
        tracing::warn!(target: LOG_TARGET, event = "conversation_answer_unread", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
        let sent = open_messages(state, c)?
            .into_iter()
            .filter(|m| m.stage == Stage::Sent)
            .map(|m| m.message)
            .collect::<Vec<_>>();
        unavailable(state, c, &sent, connectors.now())?;
        return Ok(false);
    }
    if let Err(error) = post(admitted, state, connectors, accepted, c) {
        tracing::warn!(target: LOG_TARGET, event = "conversation_answer_unposted", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
    }
    ended(state, c, connectors.now())?;
    Ok(true)
}

const UNPOSTED: &str = "SELECT 1 FROM conversation_answers x WHERE x.company=conversation_messages.company AND x.connector=conversation_messages.connector AND x.conversation=conversation_messages.conversation AND x.turn=conversation_messages.turn AND x.posted_at IS NULL";

/// After the posts of a pass: a message whose answer is posted is complete,
/// its interval closed and its update gone. One whose answer is still not
/// posted although a post of it was tried has its interval opened, as
/// [`unconfirmed`] does.
fn ended(state: &RunnerState, c: &Conversation, now: Timestamp) -> Result<(), StateError> {
    state.write::<_, StateError>(|tx| {
        tx.execute(
            &format!("UPDATE conversation_messages SET unavailable_since=NULL,turn_update=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND state=?4 AND (unavailable_since IS NOT NULL OR turn_update IS NOT NULL) AND NOT EXISTS({UNPOSTED})"),
            params![c.company.as_str(), c.connector, c.conversation, Stage::Answered.as_str()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    unconfirmed(state, c, now)
}

/// A message whose answer is not posted although a post of it was tried —
/// it failed, or its outcome was lost, a restart included — has its interval
/// opened from its turn's end (`now` for a turn ended before that end was
/// kept). A post that was not tried yet opens nothing: the healthy path never
/// shows the unavailable text.
fn unconfirmed(state: &RunnerState, c: &Conversation, now: Timestamp) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            &format!("UPDATE conversation_messages SET unavailable_since=coalesce(turn_ended_at,?5) WHERE company=?1 AND connector=?2 AND conversation=?3 AND state=?4 AND unavailable_since IS NULL AND EXISTS({UNPOSTED} AND (x.attempted_at IS NOT NULL OR x.failure IS NOT NULL))"),
            params![c.company.as_str(), c.connector, c.conversation, Stage::Answered.as_str(), now.to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// Whether any conversation work is outstanding. A sent message stays
/// outstanding until its turn is harvested: a native wake can precede
/// readable completion evidence, and that read needs another chance even
/// when the source makes no further write.
pub fn outstanding(home: &Home) -> Result<bool, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            conn.query_row(
                &format!("SELECT EXISTS({RELAY_WORK}) OR EXISTS(SELECT 1 FROM conversation_refusals WHERE replied_at IS NULL)"),
                relay_stages(),
                |r| r.get::<_, bool>(0),
            )
            .map_err(state::sql(path))
        })?
        .unwrap_or(false))
}

/// Record at `now` whether the finished-turn relay serving conversations
/// runs: the cause of its failure to start or of its end, kept until it runs
/// again, with the start of that failure kept across its retries.
pub fn relay_state(home: &Home, failure: Option<&str>, now: Timestamp) {
    let result = RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "INSERT INTO conversation_relay(id,failure,updated_at) VALUES (1,?1,?2) ON CONFLICT(id) DO UPDATE SET failure=excluded.failure,updated_at=CASE WHEN excluded.failure IS NOT NULL AND conversation_relay.failure IS NOT NULL THEN conversation_relay.updated_at ELSE excluded.updated_at END",
            params![failure, now.to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    });
    if let Err(error) = result {
        tracing::error!(target: LOG_TARGET, event = "conversation_relay_state_unrecorded", error = %error);
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
}

/// Connection states, conversations with pending messages, answers or
/// refusals, and their outstanding failures with their cause; no
/// conversation text or credentials. A connector the running runner does not
/// host is stopped, whatever an earlier run recorded for it, and a declared
/// connector no run ever hosted is listed as stopped too.
pub fn status(home: &Home) -> Result<Status, ConversationError> {
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
    for (company, DeclaredConnector { key, .. }) in demand(home)? {
        let listed = connectors
            .iter()
            .any(|known| known.company == company.as_str() && known.connector == key);
        if !listed {
            connectors.push(ConnectorStatus {
                company: company.to_string(),
                connector: key,
                state: ConnectionState::Stopped,
                failure: None,
            });
        }
    }
    connectors.sort_by(|a, b| (&a.company, &a.connector).cmp(&(&b.company, &b.connector)));
    let rows = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("WITH addresses AS (SELECT company,connector,conversation FROM connected_conversations UNION SELECT company,connector,conversation FROM conversation_refusals) SELECT a.company,a.connector,a.conversation,(SELECT count(*) FROM conversation_messages m WHERE m.company=a.company AND m.connector=a.connector AND m.conversation=a.conversation AND m.state NOT IN (?1,?2)),(SELECT count(*) FROM conversation_answers x WHERE x.company=a.company AND x.connector=a.connector AND x.conversation=a.conversation AND x.posted_at IS NULL),(SELECT count(*) FROM conversation_refusals r WHERE r.company=a.company AND r.connector=a.connector AND r.conversation=a.conversation AND r.replied_at IS NULL),(SELECT json_group_array(f) FROM (SELECT failure AS f FROM connected_conversations c WHERE c.company=a.company AND c.connector=a.connector AND c.conversation=a.conversation AND failure IS NOT NULL UNION ALL SELECT failure FROM conversation_messages m WHERE m.company=a.company AND m.connector=a.connector AND m.conversation=a.conversation AND failure IS NOT NULL UNION ALL SELECT failure FROM conversation_answers x WHERE x.company=a.company AND x.connector=a.connector AND x.conversation=a.conversation AND failure IS NOT NULL UNION ALL SELECT failure FROM conversation_refusals r WHERE r.company=a.company AND r.connector=a.connector AND r.conversation=a.conversation AND failure IS NOT NULL)) FROM addresses a ORDER BY a.company,a.connector,a.conversation")
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
    })
}

/// A conversation session's assignment: the `company-agent` role the runner
/// gives it, in its company's home; the record names no procedure.
pub fn conversation_assignment(
    home: &Home,
    caller: &SessionRef,
) -> Result<Option<instrukt_core::capabilities::Assignment>, instrukt_core::state_db::StateDbError>
{
    let Some(db) = instrukt_core::state_db::StateRead::open(home)? else {
        return Ok(None);
    };
    let row = db
        .conn()
        .query_row(
            "SELECT company,semantic_key FROM connected_conversations WHERE session=?1",
            [caller.to_string()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let Some((company, key)) = row else {
        return Ok(None);
    };
    let company = ProjectSlug::parse(&company)
        .map_err(|e| instrukt_core::state_db::StateDbError::invalid(db.path(), e.to_string()))?;
    Ok(Some(instrukt_core::capabilities::Assignment {
        worker: false,
        role: instrukt_core::ids::RoleName::parse(instrukt_core::config::COMPANY_AGENT_ROLE)
            .map_err(|e| {
                instrukt_core::state_db::StateDbError::invalid(db.path(), e.to_string())
            })?,
        project: None,
        procedure: None,
        todo: None,
        workspace: Some(home.company_dir(&company)),
        key,
        platform: false,
        company: Some(company),
        dispatcher: None,
    }))
}

/// The speaker of `session`'s current turn: the sender recorded with the
/// addressed message that opened the turn the runtime has in progress. The
/// turn is correlated with its stored delivery by the delivery's boundary
/// token, which the member envelope of the turn's opening input carries. A
/// session bound to no conversation, and a turn whose opening input carries
/// no member envelope, have no speaker. Any other uncertainty refuses.
pub fn speaker(
    home: &Home,
    session: &SessionRef,
    adapter: &dyn RuntimeAdapter,
) -> Result<Option<instrukt_runner::mcp::Speaker>, instrukt_runner::mcp::McpError> {
    let Some(db) = instrukt_core::state_db::StateRead::open(home)? else {
        return Ok(None);
    };
    let conversation = db
        .conn()
        .query_row(
            "SELECT company,connector,conversation FROM connected_conversations WHERE session=?1",
            [session.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let Some((company, connector, conversation)) = conversation else {
        return Ok(None);
    };
    let opening = adapter
        .active_turn_input(session)
        .map_err(|error| instrukt_runner::mcp::McpError::Turn {
            cause: error.to_string(),
        })?
        .ok_or_else(|| instrukt_runner::mcp::McpError::Turn {
            cause: "the runtime has no turn of this session in progress".into(),
        })?;
    let boundary = envelopes::segment(&opening)
        .into_iter()
        .find_map(|span| match span {
            Span::Member { boundary, .. } => Some(boundary.to_owned()),
            _ => None,
        });
    let Some(boundary) = boundary else {
        return Ok(None);
    };
    let row = db
        .conn()
        .query_row(
            "SELECT sender, sender_name, sender_email FROM conversation_messages
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND boundary=?4",
            [&company, &connector, &conversation, &boundary],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let (identity, name, email) = row.ok_or_else(|| instrukt_runner::mcp::McpError::Turn {
        cause: "the turn's addressed message is not one this conversation delivered".into(),
    })?;
    Ok(Some(instrukt_runner::mcp::Speaker {
        name: name
            .or_else(|| identity.clone())
            .unwrap_or_else(|| "an unnamed sender".to_owned()),
        identity,
        email: email.filter(|email| !email.trim().is_empty()),
    }))
}

/// The company's ports as the runner hosts it: built by the CLI with its
/// registry's connector builders, it builds the kinds when the runner starts
/// hosting and keeps the directory they make, and the answer pieces accepted
/// but not yet recorded, for its own relay.
pub struct Ports {
    builders: Vec<crate::BuildKind>,
    hosted: std::sync::Mutex<Option<std::sync::Arc<dyn Connectors>>>,
    accepted: Accepted,
}

impl Ports {
    pub fn new(builders: Vec<crate::BuildKind>) -> Self {
        Self {
            builders,
            hosted: std::sync::Mutex::default(),
            accepted: Accepted::default(),
        }
    }

    fn hosted(&self) -> Option<std::sync::Arc<dyn Connectors>> {
        self.hosted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn company_error(error: impl std::fmt::Display) -> instrukt_runner::interfaces::CompanyError {
    instrukt_runner::interfaces::CompanyError(error.to_string())
}

impl instrukt_runner::interfaces::CompanyPorts for Ports {
    fn demand(&self, admitted: &Admitted) -> bool {
        !admitted_demand(admitted).is_empty()
    }

    fn start(
        &self,
        start: instrukt_runner::interfaces::CompanyStart<'_>,
    ) -> Result<
        Box<dyn instrukt_runner::interfaces::CompanySupervision>,
        instrukt_runner::interfaces::CompanyError,
    > {
        let kinds = self
            .builders
            .iter()
            .map(|build| {
                let lookup = |name: &str| start.env.get(name).map(str::to_owned);
                build(&lookup, std::sync::Arc::clone(&start.clock))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| instrukt_runner::interfaces::CompanyError(error.cause))?;
        let directory: std::sync::Arc<dyn Connectors> =
            std::sync::Arc::new(crate::connector_host::Directory::new(kinds, start.clock));
        *self
            .hosted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(std::sync::Arc::clone(&directory));
        crate::connector_host::Connections::start(
            start.home,
            start.current,
            directory,
            start.wake,
            start.supervise,
        )
        .map(|connections| {
            Box::new(connections) as Box<dyn instrukt_runner::interfaces::CompanySupervision>
        })
        .map_err(company_error)
    }

    fn relay(
        &self,
        home: &Home,
        admitted: &Admitted,
        source: &dyn ConversationSource,
        adapter: &dyn RuntimeAdapter,
        pass: CompanyRelayPass,
    ) -> Result<(), instrukt_runner::interfaces::CompanyError> {
        let Some(connectors) = self.hosted() else {
            return Ok(());
        };
        relay(
            home,
            admitted,
            source,
            adapter,
            connectors.as_ref(),
            &self.accepted,
            pass,
        )
        .map_err(company_error)
    }

    fn outstanding(&self, home: &Home) -> Result<bool, instrukt_runner::interfaces::CompanyError> {
        outstanding(home).map_err(company_error)
    }

    /// Recorded on the clock the hosted connectors read, or the system's
    /// before any are hosted.
    fn relay_state(&self, home: &Home, failure: Option<&str>) {
        let now = self
            .hosted()
            .map_or_else(Timestamp::now, |connectors| connectors.now());
        relay_state(home, failure, now);
    }

    fn status(
        &self,
        home: &Home,
    ) -> Result<serde_json::Value, instrukt_runner::interfaces::CompanyError> {
        let status = status(home).map_err(company_error)?;
        serde_json::to_value(status).map_err(company_error)
    }
}

impl instrukt_runner::interfaces::CompanySupervision for crate::connector_host::Connections {
    fn check(&mut self) {
        crate::connector_host::Connections::check(self);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COMPANY_AGENT_ROLE, ConnectionState, ConversationError, RoleName, Stage, effective_since,
        opening_procedure, split,
    };

    #[test]
    fn a_messages_interval_is_its_own_or_the_relays_failure_from_its_admission() {
        let at = |text: &str| Some(text.parse::<jiff::Timestamp>().unwrap());
        // Nothing open: no interval.
        assert_eq!(
            effective_since(None, at("2026-11-19T10:00:00Z"), None),
            None
        );
        // Its own interval alone.
        assert_eq!(
            effective_since(at("2026-11-19T10:05:00Z"), at("2026-11-19T10:00:00Z"), None),
            at("2026-11-19T10:05:00Z")
        );
        // A relay failure that began before the message was admitted counts
        // from its admission.
        assert_eq!(
            effective_since(None, at("2026-11-19T10:00:00Z"), at("2026-11-19T09:00:00Z")),
            at("2026-11-19T10:00:00Z")
        );
        // One that began after it counts from its start.
        assert_eq!(
            effective_since(None, at("2026-11-19T10:00:00Z"), at("2026-11-19T10:02:00Z")),
            at("2026-11-19T10:02:00Z")
        );
        // Both open: the earlier one.
        assert_eq!(
            effective_since(
                at("2026-11-19T10:01:00Z"),
                at("2026-11-19T10:00:00Z"),
                at("2026-11-19T10:02:00Z")
            ),
            at("2026-11-19T10:01:00Z")
        );
        // A message admitted before its admission was recorded counts from
        // the relay failure's start.
        assert_eq!(
            effective_since(None, None, at("2026-11-19T09:00:00Z")),
            at("2026-11-19T09:00:00Z")
        );
    }

    /// An index holding the procedure declaring the conversation role,
    /// pending its delivering todo when `pending`.
    fn declared(pending: bool) -> instrukt_core::index::GlobalIndex {
        let delivered_by = if pending {
            "  delivered_by: [draft-it]\n"
        } else {
            ""
        };
        serde_yaml_ng::from_str(&format!(
                "schema_version: 2\nsnippets:\n- id: general/procedure/company-assistance\n  description: Assist.\n  type: procedure\n  scope: global\n  domain: general\n  path: general/procedure/company-assistance.md\n  source_project: demo\n  role: company-agent\n{delivered_by}"
            ),
        )
        .unwrap()
    }

    #[test]
    fn a_conversation_starts_only_from_a_delivered_procedure_declaring_its_role() {
        let delivered = declared(false);
        let found = opening_procedure(
            delivered
                .procedures_declaring(&RoleName::parse(COMPANY_AGENT_ROLE).unwrap())
                .into_iter()
                .next(),
        );
        assert_eq!(found.unwrap(), &delivered.snippets[0]);
        let pending = declared(true);
        let refused = opening_procedure(
            pending
                .procedures_declaring(&RoleName::parse(COMPANY_AGENT_ROLE).unwrap())
                .into_iter()
                .next(),
        );
        assert!(
            matches!(
                refused,
                Err(ConversationError::NoProcedure {
                    role: "company-agent"
                })
            ),
            "{refused:?}"
        );
        assert!(matches!(
            opening_procedure(None),
            Err(ConversationError::NoProcedure {
                role: "company-agent"
            })
        ));
    }

    /// The character count of each part, and whether the parts together
    /// are the text unchanged.
    fn cut(text: &str, limit: usize) -> (Vec<usize>, bool) {
        let parts = split(text, limit);
        let counts = parts.iter().map(|part| part.chars().count()).collect();
        (counts, parts.concat() == text)
    }

    #[test]
    fn a_part_ends_after_the_last_line_break_in_its_second_half() {
        // Line breaks at characters 23 and 29 both lie in the second half.
        let text = format!("{}\n{}\n{}", "a".repeat(22), "b".repeat(5), "c".repeat(30));
        assert_eq!(cut(&text, 40), (vec![29, 30], true));
    }

    #[test]
    fn a_line_break_in_the_first_half_does_not_end_a_part() {
        let text = format!("{}\n{}", "a".repeat(10), "b".repeat(50));
        assert_eq!(cut(&text, 40), (vec![40, 21], true));
    }

    #[test]
    fn the_remainder_is_the_last_part_and_no_empty_part_follows() {
        assert_eq!(cut(&"a".repeat(41), 40), (vec![40, 1], true));
        assert_eq!(cut(&"a".repeat(80), 40), (vec![40, 40], true));
        assert_eq!(
            cut(&format!("{}\n  ", "a".repeat(39)), 40),
            (vec![40, 2], true)
        );
        assert_eq!(cut(&"a".repeat(5), 40), (vec![5], true));
        assert_eq!(cut("", 40), (vec![], true));
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        assert_eq!(cut(&"é".repeat(41), 40), (vec![40, 1], true));
    }

    #[test]
    fn stored_states_read_back_as_their_variants() {
        for state in [
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Failed,
            ConnectionState::Stopped,
        ] {
            assert_eq!(ConnectionState::parse(state.as_str()), Some(state));
        }
        for stage in [
            Stage::Received,
            Stage::Refused,
            Stage::Pending,
            Stage::Sent,
            Stage::Answered,
        ] {
            assert_eq!(Stage::parse(stage.as_str()), Some(stage));
        }
    }
}
