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

use instrukt_core::config::{CONVERSATION_ROLE, ConfigError, Configuration, SlackConnector};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::connectors::{SlackApi, SlackEndpoint};
use crate::envelopes::{self, ContextEntry, Control, Span};
use crate::keyed::{self, RuntimeInvocation};
use crate::runtime::conversations::{AuthorKind, ConversationSource, Scope, SourcePosition};
use crate::runtime::{CreationPermit, RuntimeAdapter, RuntimeAssignment, RuntimeStart};
use crate::sessions::{SessionState, SessionsError};
use crate::state::{self, RunnerState, StateError};

/// Slack's limit for an answer posted as Markdown (`markdown_text`), in
/// characters; longer answers are split.
pub const SLACK_MESSAGE_LIMIT: usize = 12_000;

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
    Source(#[from] crate::runtime::conversations::SourceError),
    #[error(transparent)]
    Lock(#[from] instrukt_core::fsx::FsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Slack(#[from] crate::connectors::SlackError),
    #[error("conversation: {0}")]
    Refused(String),
}

/// One earlier message of a conversation, kept with the addressed message
/// it accompanies until that message is delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Earlier {
    pub speaker: String,
    pub own: bool,
    pub at: Option<Timestamp>,
    pub text: String,
}

/// What accompanies one addressed message: at most `context_messages` of
/// the most recent earlier messages, and whether earlier ones were left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accompanying {
    pub messages: Vec<Earlier>,
    pub left_out: bool,
}

/// What a connector supplies for one message addressed to the agent, as it
/// arrives.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub company: ProjectSlug,
    pub connector: String,
    pub conversation: String,
    /// The connector's identity of the message, unique in its conversation.
    pub message: String,
    /// The sender's identity on the connector. It decides nothing:
    /// admission reads the provider's record for it.
    pub sender: String,
    pub written_at: Timestamp,
    pub text: String,
}

/// What became of an incoming addressed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intake {
    /// Retained, waiting for its sender's admission.
    Retained,
    /// Already retained.
    Repeated,
}

/// The sender of an admitted message, as the provider gave them when it was
/// admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    pub name: String,
    pub email: Option<String>,
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
    incoming: &Incoming,
    now: Timestamp,
) -> Result<Intake, ConversationError> {
    let state = RunnerState::new(home);
    let company = incoming.company.as_str();
    let semantic = key(
        &incoming.company,
        &incoming.connector,
        &incoming.conversation,
    );
    let inserted = state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key) VALUES (?1,?2,?3,?4) ON CONFLICT DO NOTHING",
            params![company, incoming.connector, incoming.conversation, semantic],
        )
        .map_err(state::sql(&path))?;
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,written_us,written_at,arrived_at,state,text) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT DO NOTHING",
            params![
                company,
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
    tracing::info!(event = "conversation_message_accepted", company = %incoming.company, connector = %incoming.connector, conversation = %incoming.conversation);
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

/// Record the sender of a received message as admitted, with the name and
/// email address the provider gave; once written they do not change.
pub fn admitted(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    sender: &Sender,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET sender_name=?5,sender_email=?6,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?7 AND sender_name IS NULL",
            params![company.as_str(), connector, unread.conversation, unread.message, sender.name, sender.email, Stage::Received.as_str()],
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
    tracing::info!(event = "conversation_sender_refused", company = %company, connector, conversation = %unread.conversation);
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

/// The admitted messages of one company's connector whose turn has not yet
/// ended, oldest written first: those the connector shows the agent working
/// on.
pub fn working(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<(String, String)>, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT conversation,message FROM conversation_messages WHERE company=?1 AND connector=?2 AND ((state=?3 AND sender_name IS NOT NULL) OR state IN (?4,?5)) ORDER BY written_us, message")
                .map_err(state::sql(path))?;
            q.query_map(
                params![company.as_str(), connector, Stage::Received.as_str(), Stage::Pending.as_str(), Stage::Sent.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default())
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

pub fn unsent_refusals(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<Refusal>, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT conversation,sender FROM conversation_refusals WHERE company=?1 AND connector=?2 AND replied_at IS NULL ORDER BY rowid")
                .map_err(state::sql(path))?;
            q.query_map(params![company.as_str(), connector], |r| {
                Ok(Refusal {
                    conversation: r.get(0)?,
                    sender: r.get(1)?,
                })
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
pub fn demand(home: &Home) -> Result<Vec<(ProjectSlug, SlackConnector)>, ConversationError> {
    let mut hosted = Vec::new();
    for (name, config) in instrukt_core::config::company_homes(home)? {
        match config.and_then(|config| config.slack_connector()) {
            Ok(Some(slack)) => hosted.push((name, slack)),
            Ok(None) => {}
            Err(ConfigError::NoLayer { .. }) => {}
            Err(error) => {
                tracing::warn!(event = "connector_declaration_unreadable", company = %name, error = %error);
            }
        }
    }
    Ok(hosted)
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
}

const CONVERSATION_COLUMNS: &str = "company,connector,conversation,semantic_key,opening,request,session,source_identity,position_json";

fn conversations(state: &RunnerState) -> Result<Vec<Conversation>, ConversationError> {
    let sql = format!("SELECT {CONVERSATION_COLUMNS} FROM connected_conversations ORDER BY rowid");
    let rows = state
        .read(|conn, path| {
            let mut q = conn.prepare(&sql).map_err(state::sql(path))?;
            q.query_map([], |r| {
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
    Ok(conversations(state)?
        .into_iter()
        .find(|o| o.semantic_key == c.semantic_key))
}

/// The conversation's messages neither answered nor refused, oldest written
/// first.
fn open_messages(state: &RunnerState, c: &Conversation) -> Result<Vec<Message>, ConversationError> {
    let rows = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT message,state,boundary,content,attempted_at IS NOT NULL FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND state NOT IN (?4,?5) ORDER BY written_us, message")
                .map_err(state::sql(path))?;
            q.query_map(params![c.company.as_str(), c.connector, c.conversation, Stage::Answered.as_str(), Stage::Refused.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    rows.into_iter()
        .map(|(message, stage, boundary, content, attempted)| {
            Ok(Message {
                message,
                stage: Stage::parse(&stage).ok_or_else(|| {
                    ConversationError::Refused(format!("unknown message stage `{stage}`"))
                })?,
                boundary,
                content,
                attempted,
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

/// The runner-authored opening assignment of a conversation session.
fn assignment(home: &Home, c: &Conversation) -> Result<String, ConversationError> {
    let capabilities = instrukt_core::config::Agents::load(home)?
        .capabilities(CONVERSATION_ROLE)?
        .describe();
    Ok(Control::Assignment.frame(&format!(
        "instrukt conversation assignment\nCompany: {}\nConnector: {}\nConversation: {}\nRole: {CONVERSATION_ROLE}\nResolved role capabilities: {capabilities}\nYou answer people of this company who address you in this conversation. Each of their messages arrives as its own turn in a member envelope naming who wrote it, after a conversation context envelope holding what was said before as data.\nYour conversational output of a turn started by such a message is posted back to this conversation; nothing else you write is posted. Answer with `NOOP` to post nothing.",
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

/// The runtime accepted `message` as its own turn; a first message's
/// acceptance also binds the conversation's session.
fn accepted(
    state: &RunnerState,
    c: &Conversation,
    message: &str,
    session: Option<&SessionRef>,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            "UPDATE conversation_messages SET state=?5,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
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

/// Deliver the conversation's next addressed message. The first starts its
/// session by key in the company's home, with an opening request owned by
/// that one message; a later one waits until the session's current turn has
/// finished. Failures stay with the message until it is delivered.
fn deliver_one(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    source: &dyn ConversationSource,
    state: &RunnerState,
    c: &Conversation,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let open = open_messages(state, c)?;
    if open.iter().any(|m| m.stage == Stage::Sent) {
        return Ok(());
    }
    // A captured opening request belongs to its own message alone.
    // Until its session is bound, a captured opening request belongs to its
    // own message alone.
    let next = match (&c.session, &c.opening) {
        (None, Some(opening)) => open.iter().find(|m| &m.message == opening),
        _ => open.first(),
    };
    let Some(next) = next else {
        return Ok(());
    };
    if next.stage != Stage::Pending {
        return Ok(());
    }
    let result = match &c.session {
        None => start(invocation, adapter, source, state, c, next, now),
        Some(session) => send(adapter, state, c, session, next, now),
    };
    // A runtime failure is retained through the adapter's safe diagnostic,
    // never its raw native output. An unresolved start says so, with its key
    // and the command that resolves it.
    let failure = result.as_ref().err().map(|error| match error {
        ConversationError::Sessions(
            crate::sessions::SessionsError::Unresolved { key }
            | crate::sessions::SessionsError::Uncertain { key, .. },
        ) => crate::sessions::SessionsError::Unresolved { key: key.clone() }.to_string(),
        ConversationError::Sessions(native) => adapter
            .diagnostic("conversation delivery", native)
            .to_string(),
        other => other.to_string(),
    });
    if let Some(error) = &failure {
        tracing::warn!(event = "conversation_delivery_failed", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
    }
    set_message_failure(state, c, &next.message, failure.as_deref())?;
    result
}

fn start(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    source: &dyn ConversationSource,
    state: &RunnerState,
    c: &Conversation,
    next: &Message,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let home = invocation.home();
    let request = match &c.request {
        Some(request) => serde_json::from_str::<RuntimeStart>(request)?,
        None => {
            let checkout: PathBuf = home.company_dir(&c.company);
            let company = Configuration::load_company(home, &c.company)?;
            let config = Configuration::load_global(home)?;
            let preferences = crate::agents::role_order(
                &config,
                None,
                Some(CONVERSATION_ROLE),
                company.profiles()?.as_deref(),
            )?;
            let selection = crate::agents::selections(&config, home, &preferences, now)?
                .into_iter()
                .next();
            let input = next.content.as_deref().ok_or_else(|| {
                ConversationError::Refused("the message is not framed yet".into())
            })?;
            let request = RuntimeStart {
                preferences,
                selection,
                selection_evidence: Some(
                    adapter.selection_realization(crate::runtime::SelectionOperation::ThreadStart),
                ),
                checkout,
                title: crate::runtime::thread_title(
                    &format!("conversation · {}", c.connector),
                    &c.semantic_key,
                ),
                key: c.semantic_key.clone(),
                brief: format!("{}\n{input}", assignment(home, c)?),
                worktree: None,
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
    tracing::info!(event = "conversation_session_started", company = %c.company, connector = %c.connector, key = %c.semantic_key, session = %thread.session);
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
    if adapter.observe_input(session, content, now)?.state == crate::runtime::InputState::Unknown {
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
                .map(|contribution| contribution.text.trim())
                .filter(|text| !text.is_empty() && *text != "NOOP")
                .collect::<Vec<_>>()
                .join("\n\n");
            answers.push((
                message.message.clone(),
                turn.native_turn.to_string(),
                output,
            ));
        }
        position = batch.position;
        state.write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            for (message, turn, output) in &answers {
                for (piece, text) in split(output, SLACK_MESSAGE_LIMIT).iter().enumerate() {
                    tx.execute(
                        "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING",
                        params![c.company.as_str(), c.connector, c.conversation, turn, i64::try_from(piece).unwrap_or(i64::MAX), text],
                    )
                    .map_err(state::sql(&path))?;
                }
                tx.execute(
                    // The finished turn proves the message was accepted: its
                    // resolved delivery cause is retired with it.
                    "UPDATE conversation_messages SET state=?5,turn=?6,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                    params![c.company.as_str(), c.connector, c.conversation, message, Stage::Answered.as_str(), turn],
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

/// Post the conversation's retained answer pieces in order. A failed piece
/// keeps its cause, and the pieces after it wait.
fn post(
    home: &Home,
    state: &RunnerState,
    endpoint: &SlackEndpoint,
    c: &Conversation,
) -> Result<(), ConversationError> {
    let pending = state
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT turn,piece,text FROM conversation_answers WHERE company=?1 AND connector=?2 AND conversation=?3 AND posted_at IS NULL ORDER BY rowid")
                .map_err(state::sql(path))?;
            q.query_map(params![c.company.as_str(), c.connector, c.conversation], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
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
    let api = match SlackApi::for_company(home, &c.company, endpoint) {
        Ok(api) => api,
        Err(error) => {
            let (turn, piece, _) = &pending[0];
            let cause = error.to_string();
            state.write::<_, StateError>(|tx| {
                tx.execute(
                    "UPDATE conversation_answers SET failure=?6 WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5",
                    params![c.company.as_str(), c.connector, c.conversation, turn, piece, cause],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })?;
            return Err(error.into());
        }
    };
    for (turn, piece, text) in pending {
        let result = api
            .answer(&c.conversation, &text)
            .map_err(|e| e.to_string());
        state.write::<_, StateError>(|tx| {
            tx.execute(
                "UPDATE conversation_answers SET posted_at=CASE WHEN ?6 IS NULL THEN ?7 END,failure=?6 WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5",
                params![c.company.as_str(), c.connector, c.conversation, turn, piece, result.as_ref().err(), Timestamp::now().to_string()],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
            Ok(())
        })?;
        result.map_err(|error| {
            ConversationError::Refused(format!("post failed and stays pending: {error}"))
        })?;
    }
    Ok(())
}

/// The connector destination of the finished-turn relay `instrukt run`
/// already hosts: for every conversation, read its finished turns into
/// answers, post what is retained, then deliver its next addressed message.
pub fn relay(
    home: &Home,
    source: &dyn ConversationSource,
    adapter: &dyn RuntimeAdapter,
    endpoint: &SlackEndpoint,
) -> Result<(), ConversationError> {
    let state = RunnerState::new(home);
    let mut invocation = None;
    for initial in conversations(&state)? {
        let _lock = lock(home, &initial)?;
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
                    reconcile(adapter, &state, &c, session, &message, Timestamp::now())
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
        if !answers(home, &state, source, endpoint, &c)? {
            continue;
        }
        let Some(c) = reread(&state, &c)? else {
            continue;
        };
        if invocation.is_none() {
            invocation = Some(RuntimeInvocation::acquire(
                home,
                std::time::Duration::from_secs(120),
            )?);
        }
        if let Some(invocation) = &invocation {
            // Its failure stays with the message it concerns.
            let _ = deliver_one(invocation, adapter, source, &state, &c, Timestamp::now());
        }
        // A session bound just now may be one a recovery found with its
        // opening turn already finished while the runner was down: its answer
        // is read now, not left for an unrelated native change.
        if c.session.is_none()
            && let Some(bound) = reread(&state, &c)?
            && bound.session.is_some()
        {
            answers(home, &state, source, endpoint, &bound)?;
        }
    }
    Ok(())
}

/// Read the conversation's finished turns into answers and post what is
/// retained. Returns whether the read succeeded; its cause is kept on the
/// conversation until a later read recovers.
fn answers(
    home: &Home,
    state: &RunnerState,
    source: &dyn ConversationSource,
    endpoint: &SlackEndpoint,
    c: &Conversation,
) -> Result<bool, ConversationError> {
    let read = harvest(state, source, c);
    set_failure(
        state,
        c,
        read.as_ref().err().map(ToString::to_string).as_deref(),
    )?;
    if let Err(error) = read {
        tracing::warn!(event = "conversation_answer_unread", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
        return Ok(false);
    }
    if let Err(error) = post(home, state, endpoint, c) {
        tracing::warn!(event = "conversation_answer_unposted", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
    }
    Ok(true)
}

/// Whether any conversation work is outstanding: a message not yet
/// accepted, an answer not yet posted or a refusal not yet sent.
pub fn outstanding(home: &Home) -> Result<bool, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE state IN (?1,?2)) OR EXISTS(SELECT 1 FROM conversation_answers WHERE posted_at IS NULL) OR EXISTS(SELECT 1 FROM conversation_refusals WHERE replied_at IS NULL)",
                [Stage::Received.as_str(), Stage::Pending.as_str()],
                |r| r.get::<_, bool>(0),
            )
            .map_err(state::sql(path))
        })?
        .unwrap_or(false))
}

/// Record whether the finished-turn relay serving conversations runs: the
/// cause of its failure to start or of its end, kept until it runs again.
pub fn relay_state(home: &Home, failure: Option<&str>) {
    let result = RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "INSERT INTO conversation_relay(id,failure,updated_at) VALUES (1,?1,?2) ON CONFLICT(id) DO UPDATE SET failure=excluded.failure,updated_at=excluded.updated_at",
            params![failure, Timestamp::now().to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    });
    if let Err(error) = result {
        tracing::error!(event = "conversation_relay_state_unrecorded", error = %error);
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
    let hosted = crate::runner::hosted(home);
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
    for (company, _) in demand(home)? {
        let listed = connectors.iter().any(|known| {
            known.company == company.as_str() && known.connector == crate::connectors::SLACK
        });
        if !listed {
            connectors.push(ConnectorStatus {
                company: company.to_string(),
                connector: crate::connectors::SLACK.to_owned(),
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

#[cfg(test)]
mod tests {
    use super::{ConnectionState, Stage, split};

    #[test]
    fn a_part_ends_after_the_last_line_break_in_its_second_half() {
        let first = format!("{}\n{}\n", "a".repeat(10), "b".repeat(15));
        let rest = "c".repeat(30);
        assert_eq!(split(&format!("{first}{rest}"), 40), [first, rest]);
    }

    #[test]
    fn a_line_break_in_the_first_half_does_not_end_a_part() {
        let text = format!("{}\n{}", "a".repeat(10), "b".repeat(50));
        assert_eq!(
            split(&text, 40),
            [text[..40].to_owned(), text[40..].to_owned()]
        );
    }

    #[test]
    fn the_remainder_is_the_last_part_and_no_empty_part_follows() {
        assert_eq!(split(&"a".repeat(41), 40), ["a".repeat(40), "a".to_owned()]);
        assert_eq!(split(&"a".repeat(80), 40), ["a".repeat(40), "a".repeat(40)]);
        let spaced = format!("{}\n  ", "a".repeat(39));
        assert_eq!(
            split(&spaced, 40),
            [format!("{}\n", "a".repeat(39)), "  ".to_owned()]
        );
        assert_eq!(split("short", 40), ["short"]);
        assert!(split("", 40).is_empty());
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let text = "é".repeat(41);
        assert_eq!(split(&text, 40), ["é".repeat(40), "é".to_owned()]);
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
