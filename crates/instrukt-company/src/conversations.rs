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
//! turn), failed (its turn failed without a reply), and answered (its turn
//! finished with an answer or successfully produced nothing). Each answer piece stays pending
//! until the connector accepted its post.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use instrukt_connector::{Accompanying, ConversationDisplay, Incoming, RefusalReason, Sender};
use instrukt_core::config::{
    Admitted, Agents, COMPANY_AGENT_ROLE, ConfigError, Configuration, DeclaredConnector, Env,
    ReplyMode, SessionScope, connector_declaration,
};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, SnippetId};
use instrukt_core::index::{GlobalIndex, IndexEntry, RoleProcedure};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::connector_host::Connectors;
use instrukt_runner::envelopes::{self, ContextEntry, Control, Span};
use instrukt_runner::interfaces::CompanyRelayPass;
use instrukt_runner::keyed::{self, RuntimeInvocation};
use instrukt_runner::runtime::conversations::{
    AuthorKind, ConversationSource, RecordKind, Role, SourcePosition,
};
use instrukt_runner::runtime::{
    DispatchError, Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, SendEvidence,
};
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
    /// Its turn failed without a reply; it remains unserved.
    Failed,
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
            Self::Failed => "failed",
            Self::Answered => "answered",
        }
    }
    fn parse(text: &str) -> Option<Self> {
        [
            Self::Received,
            Self::Refused,
            Self::Pending,
            Self::Sent,
            Self::Failed,
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
    NoProcedure { role: String },
    #[error("conversation: {0}")]
    Refused(String),
    #[error("conversation source configuration: {0}")]
    SourceConfiguration(String),
}

/// What became of an incoming addressed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intake {
    /// Retained, waiting for its sender's admission.
    Retained,
    /// Already retained.
    Repeated,
}

/// The binding conversation column value for a `session_scope: company` connector.
pub const COMPANY_SCOPE_CONVERSATION: &str = "";

/// The semantic start key of a conversation's session (`session_scope: conversation`).
pub fn key(company: &ProjectSlug, connector: &str, conversation: &str) -> String {
    format!(
        "instrukt:conversation:{}:{connector}:{}",
        keyed::encode(company.as_str()),
        keyed::encode(conversation)
    )
}

/// The semantic start key of a company-scoped connector's shared standing session
/// (`session_scope: company`).
pub fn company_key(company: &ProjectSlug, connector: &str) -> String {
    format!(
        "instrukt:conversation:{}:{connector}",
        keyed::encode(company.as_str()),
    )
}

fn micros(at: Timestamp) -> i64 {
    at.as_microsecond()
}

/// Retain one incoming addressed message with the connector's declared `role`,
/// `session_scope`, and `reply_mode`. Nothing here waits on the network and
/// nothing here decides who is answered.
pub fn retain_with_policy(
    home: &Home,
    company: &ProjectSlug,
    incoming: &Incoming,
    role: &RoleName,
    session_scope: SessionScope,
    reply_mode: ReplyMode,
    now: Timestamp,
) -> Result<Intake, ConversationError> {
    let state = RunnerState::new(home);
    let inserted =
        state.write(|tx| retain_in(tx, company, incoming, role, session_scope, reply_mode, now))?;
    if inserted == 0 {
        return Ok(Intake::Repeated);
    }
    tracing::info!(target: LOG_TARGET, event = "conversation_message_accepted", company = %company, connector = %incoming.connector, conversation = %incoming.conversation);
    Ok(Intake::Retained)
}

pub(crate) mod read_back;

fn retain_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    company: &ProjectSlug,
    incoming: &Incoming,
    role: &RoleName,
    session_scope: SessionScope,
    reply_mode: ReplyMode,
    now: Timestamp,
) -> Result<usize, StateError> {
    let path = state::db_path(tx);
    let settled: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM conversation_missed_identities
            WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
            UNION ALL SELECT 1 FROM conversation_context_receipts
            WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND delivery=message
         )",
        params![company.as_str(), incoming.connector, incoming.conversation, incoming.message],
        |row| row.get(0),
    ).map_err(state::sql(&path))?;
    if settled {
        return Ok(0);
    }
    // Preserve disjoint binding ownership across `session_scope` transitions:
    // - If a conversation-scoped binding already owns `incoming.conversation`,
    //   do not create a conflicting company binding for it.
    // - If a company-scoped binding already owns prior messages for
    //   `incoming.conversation`, do not create a conflicting conversation
    //   binding for it.
    match session_scope {
        SessionScope::Company => {
            let owned_by_conversation: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3 AND session_scope='conversation')",
                    params![company.as_str(), incoming.connector, incoming.conversation],
                    |row| row.get(0),
                )
                .map_err(state::sql(&path))?;
            if !owned_by_conversation {
                let semantic = company_key(company, &incoming.connector);
                tx.execute(
                    "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,role,session_scope,reply_mode) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT DO NOTHING",
                    params![
                        company.as_str(),
                        incoming.connector,
                        COMPANY_SCOPE_CONVERSATION,
                        semantic,
                        role.as_str(),
                        SessionScope::Company.as_str(),
                        reply_mode.as_str()
                    ],
                )
                .map_err(state::sql(&path))?;
            }
        }
        SessionScope::Conversation => {
            let owned_by_company: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3 AND session_scope='company')
                        AND EXISTS(SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?4)
                        AND NOT EXISTS(SELECT 1 FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?4 AND session_scope='conversation')",
                    params![
                        company.as_str(),
                        incoming.connector,
                        COMPANY_SCOPE_CONVERSATION,
                        incoming.conversation
                    ],
                    |row| row.get(0),
                )
                .map_err(state::sql(&path))?;
            if !owned_by_company {
                let semantic = key(company, &incoming.connector, &incoming.conversation);
                tx.execute(
                    "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,role,session_scope,reply_mode) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT DO NOTHING",
                    params![
                        company.as_str(),
                        incoming.connector,
                        incoming.conversation,
                        semantic,
                        role.as_str(),
                        SessionScope::Conversation.as_str(),
                        reply_mode.as_str()
                    ],
                )
                .map_err(state::sql(&path))?;
            }
        }
    }
    tx.execute(
        "INSERT INTO conversation_messages(company,connector,conversation,message,sender,written_us,written_at,arrived_at,state,text,is_direct,thread)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT DO NOTHING",
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
            incoming.text,
            i32::from(incoming.is_direct),
            incoming.thread
        ],
    )
    .map_err(state::sql(&path))
}

/// A received message: its sender still to be admitted, or what
/// accompanies it still to be read.
#[derive(Debug, Clone)]
pub struct Unread {
    pub conversation: String,
    pub message: String,
    /// The sender's connector identity; `None` for a message stored before
    /// senders were kept, which was admitted when it was stored.
    pub sender: Option<String>,
    /// Whether the sender was admitted.
    pub admitted: bool,
    /// The message's text as written.
    pub text: String,
}

/// Questions still awaiting intake or a fresh context read. This includes
/// legacy captures whose required context was missing.
pub fn unread(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<Unread>, ConversationError> {
    Ok(RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare(
                    "SELECT m.conversation,m.message,m.sender,m.sender_name IS NOT NULL,m.text
                     FROM conversation_messages m
                     WHERE m.company=?1 AND m.connector=?2
                       AND (m.state=?3 OR (m.state IN ('pending','failed') AND m.attempted_at IS NULL
                            AND (json_extract(m.context_json,'$.refresh')=1 OR json_extract(m.context_json,'$.unread')=1)))
                     ORDER BY m.written_us, m.message",
                )
                .map_err(state::sql(path))?;
            q.query_map(
                params![company.as_str(), connector, Stage::Received.as_str()],
                |r| {
                    Ok(Unread {
                        conversation: r.get(0)?,
                        message: r.get(1)?,
                        sender: r.get(2)?,
                        admitted: r.get(3)?,
                        text: r.get(4)?,
                    })
                },
            )
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
            "UPDATE conversation_messages SET sender_name=?5,sender_email=?6,sender_is_admin=?9,admitted_at=?8,admission_position=(SELECT coalesce(max(admission_position),0)+1 FROM conversation_messages),failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND state=?7 AND sender_name IS NULL",
            params![company.as_str(), connector, unread.conversation, unread.message, sender.name, sender.email, Stage::Received.as_str(), now.to_string(), i32::from(sender.is_admin)],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// Record that a received message's sender is refused under the connector's
/// admission policy (`not_own` or `not_admin`): the message is refused and the
/// one reply owed to that sender in that conversation is retained, in one
/// transaction.
pub fn refused(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    sender: &str,
    reason: RefusalReason,
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
                "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING",
                params![company.as_str(), connector, unread.conversation, sender, reason.as_str(), now.to_string()],
            )
            .map_err(state::sql(&path))?;
        }
        Ok(())
    })?;
    tracing::info!(target: LOG_TARGET, event = "conversation_sender_refused", company = %company, connector, conversation = %unread.conversation, reason = reason.as_str());
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
    /// Its sender, the provider identity.
    pub sender: Option<String>,
    /// When its effective interval ([`effective_since`]) opened, while it is
    /// open.
    pub since: Option<Timestamp>,
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
    Ok(RunnerState::new(home)
        .read(|conn, path| read_unfinished(conn, path, company, connector))?
        .unwrap_or_default())
}

fn read_unfinished(
    conn: &instrukt_core::state_db::rusqlite::Connection,
    path: &std::path::Path,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<Unfinished>, StateError> {
    type Row = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let relay: Option<Option<String>> = conn
        .query_row(
            "SELECT updated_at FROM conversation_relay WHERE id=1 AND failure IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(state::sql(path))?;
    let mut q = conn
            .prepare("SELECT m.conversation,m.message,m.sender,m.unavailable_since,m.admitted_at,m.turn_update FROM conversation_messages m WHERE m.company=?1 AND m.connector=?2 AND m.send_unknown_at IS NULL AND ((m.state=?3 AND m.sender_name IS NOT NULL) OR m.state IN (?4,?5) OR (m.state=?6 AND m.superseded_at IS NULL) OR (m.state=?7 AND EXISTS(SELECT 1 FROM conversation_answers x WHERE x.company=m.company AND x.connector=m.connector AND x.conversation=m.conversation AND x.turn=m.turn AND x.posted_at IS NULL))) ORDER BY m.written_us, m.message")
            .map_err(state::sql(path))?;
    let rows: Vec<Row> = q
        .query_map(
            params![
                company.as_str(),
                connector,
                Stage::Received.as_str(),
                Stage::Pending.as_str(),
                Stage::Sent.as_str(),
                Stage::Failed.as_str(),
                Stage::Answered.as_str()
            ],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .map_err(state::sql(path))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(state::sql(path))?;
    let at = |text: Option<String>| -> Result<Option<Timestamp>, StateError> {
        text.map(|text| {
            text.parse::<Timestamp>().map_err(|e| {
                state::invalid(path, format!("unreadable interval time `{text}`: {e}"))
            })
        })
        .transpose()
    };
    let relay = at(relay.flatten())?;
    rows.into_iter()
        .map(|(conversation, message, sender, own, admitted, update)| {
            let since = effective_since(at(own)?, at(admitted)?, relay);
            let status = match update {
                _ if since.is_some() => instrukt_connector::Status::Unavailable,
                Some(update) => instrukt_connector::Status::Update(update),
                _ => instrukt_connector::Status::Working,
            };
            Ok(Unfinished {
                conversation,
                message,
                sender,
                since,
                status,
            })
        })
        .collect()
}

/// How long a person's effective interval lasts before they are owed the
/// one notice that the assistant is temporarily unavailable, on the runner's
/// clock.
pub const NOTICE_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(1);

/// The longest a connector's intake pass waits before it evaluates the
/// notice again.
pub const INTAKE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// One person's eligible unavailable notice in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub conversation: String,
    pub sender: String,
}

/// Find due notices, retiring attempt markers whose no-work interval recovered.
pub fn notices(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    now: Timestamp,
) -> Result<Vec<Notice>, ConversationError> {
    RunnerState::new(home).write::<_, ConversationError>(|tx| {
        let path = state::db_path(tx);
        let conversations = {
            let mut query = tx.prepare("SELECT DISTINCT conversation FROM conversation_refusals WHERE company=?1 AND connector=?2 AND reason='unavailable'")
                .map_err(state::sql(&path))?;
            query.query_map(params![company.as_str(), connector], |row| row.get::<_, String>(0))
                .map_err(state::sql(&path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(&path))?
        };
        for conversation in conversations {
            retire_notices(tx, company.as_str(), connector, &conversation)?;
        }
        let mut due = BTreeSet::new();
        for message in read_unfinished(tx, &path, company, connector)? {
            if message.since.is_some_and(|since| since + NOTICE_AFTER <= now)
                && let Some(sender) = message.sender
            {
                due.insert((message.conversation, sender));
            }
        }
        let mut notices = Vec::new();
        for (conversation, sender) in due {
            let attempted = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_refusals WHERE company=?1 AND connector=?2 AND conversation=?3 AND sender=?4 AND reason='unavailable' AND attempted_at IS NOT NULL)",
                params![company.as_str(), connector, conversation, sender], |row| row.get::<_, bool>(0),
            ).map_err(state::sql(&path))?;
            if !attempted { notices.push(Notice { conversation, sender }); }
        }
        Ok(notices)
    })
}

/// The durable one-attempt decision for an unavailable notice.
#[derive(Debug, PartialEq, Eq)]
pub enum NoticeStep {
    Cancelled,
    Post,
}

/// Recheck the person's interval and mark the attempt before provider I/O.
pub fn notice_step(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    notice: &Notice,
    accepted: &Accepted,
    now: Timestamp,
) -> Result<NoticeStep, ConversationError> {
    let accepted_turns = accepted.turns(company, connector, &notice.conversation);
    RunnerState::new(home).write::<_, ConversationError>(|tx| {
        let path = state::db_path(tx);
        let overdue = read_unfinished(tx, &path, company, connector)?.into_iter().any(|message| {
            message.conversation == notice.conversation
                && message.sender.as_deref() == Some(notice.sender.as_str())
                && message.since.is_some_and(|since| since + NOTICE_AFTER <= now)
        });
        if !overdue { return Ok(NoticeStep::Cancelled); }
        for turn in accepted_turns {
            let answered = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND sender=?4 AND turn=?5)",
                params![company.as_str(), connector, notice.conversation, notice.sender, turn],
                |row| row.get::<_, bool>(0),
            ).map_err(state::sql(&path))?;
            if answered { return Ok(NoticeStep::Cancelled); }
        }
        let changed = tx.execute(
            "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,attempted_at)
             VALUES (?1,?2,?3,?4,'unavailable',?5,?5)
             ON CONFLICT(company,connector,conversation,sender,reason) DO UPDATE SET attempted_at=excluded.attempted_at,failure=NULL
             WHERE conversation_refusals.attempted_at IS NULL",
            params![company.as_str(), connector, notice.conversation, notice.sender, now.to_string()],
        ).map_err(state::sql(&path))?;
        Ok(if changed == 0 { NoticeStep::Cancelled } else { NoticeStep::Post })
    })
}

/// Keep the observed response for operator status; it never schedules a retry.
pub fn notice_posted(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    notice: &Notice,
    outcome: Result<(), String>,
    now: Timestamp,
) -> Result<(), ConversationError> {
    RunnerState::new(home).write::<_, StateError>(|tx| {
        let failure = outcome.err();
        tx.execute(
            "UPDATE conversation_refusals SET failure=?5,replied_at=CASE WHEN ?5 IS NULL THEN ?6 ELSE replied_at END
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND sender=?4 AND reason='unavailable'",
            params![company.as_str(), connector, notice.conversation, notice.sender, failure, now.to_string()],
        ).map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

fn conversation_title(
    company: &str,
    display: Option<&instrukt_connector::ConversationDisplay>,
    text: &str,
) -> String {
    match display {
        Some(instrukt_connector::ConversationDisplay::Direct(name)) => {
            format!("{company} · DM with {name}")
        }
        Some(instrukt_connector::ConversationDisplay::Channel(channel)) => {
            let words = text.split_whitespace().collect::<Vec<_>>();
            let preview = words.iter().take(8).copied().collect::<Vec<_>>().join(" ");
            let shortened = words.len() > 8 || preview.chars().count() > 80;
            let preview = preview.chars().take(80).collect::<String>();
            format!(
                "{company} · #{channel} · \"{preview}{}\"",
                if shortened { "…" } else { "" }
            )
        }
        None => format!("{company} · Slack thread"),
    }
}

/// Record what accompanies an admitted received message and frame it for
/// its turn with `text`, its text as the connector supplies it; or record
/// why what accompanies it could not be read.
pub fn captured(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    unread: &Unread,
    captured: Result<instrukt_connector::Captured, String>,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let state = RunnerState::new(home);
    let captured = match captured {
        Ok(captured) => captured,
        Err(failure) => {
            state.write::<_, StateError>(|tx| {
                tx.execute(
                    "UPDATE conversation_messages SET failure=?5,unavailable_since=coalesce(unavailable_since,?6)
                     WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
                       AND (state='received' OR (state IN ('pending','failed') AND attempted_at IS NULL
                            AND (json_extract(context_json,'$.refresh')=1 OR json_extract(context_json,'$.unread')=1)))",
                    params![company.as_str(), connector, unread.conversation, unread.message, failure, now.to_string()],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })?;
            return Ok(());
        }
    };
    let text = &captured.text;
    let display = captured.display.as_ref();
    let accompanying = captured.accompanying;
    let row = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT sender_name,written_at,sender_is_admin,is_direct FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                params![company.as_str(), connector, unread.conversation, unread.message],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i32>(2)? != 0,
                        r.get::<_, i32>(3)? != 0,
                    ))
                },
            )
            .map_err(state::sql(path))
        })?
        .ok_or_else(|| ConversationError::Refused("the received message is gone".into()))?;
    let (name, written_at, is_admin, is_direct) = row;
    let name = name
        .ok_or_else(|| ConversationError::Refused("the message's sender is not admitted".into()))?;
    let written_at: Timestamp = written_at
        .parse()
        .map_err(|e: jiff::Error| ConversationError::Refused(e.to_string()))?;
    let (token, content) = framed(
        connector,
        &unread.conversation,
        &name,
        is_admin,
        is_direct,
        written_at,
        text,
        &Accompanying::default(),
    )?;
    let context = serde_json::to_string(&accompanying)?;
    let first_content=state.read(|conn,path|conn.query_row("SELECT content FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 ORDER BY admission_position LIMIT 1",params![company.as_str(),connector,unread.conversation],|row|row.get::<_,Option<String>>(0)).map_err(state::sql(path)))?.flatten();
    let first_text = first_content.as_deref().and_then(|content| {
        envelopes::segment(content)
            .into_iter()
            .rev()
            .find_map(|span| match span {
                envelopes::Span::Member { body, .. } => Some(body.to_owned()),
                _ => None,
            })
    });
    let title = conversation_title(
        company.as_str(),
        display,
        first_text.as_deref().unwrap_or(text),
    );
    let display_label = display.map(|display| match display {
        ConversationDisplay::Channel(channel) => format!("#{channel}"),
        ConversationDisplay::Direct(name) => format!("DM with {name}"),
    });
    let company_title = format!("{company} · {connector}");
    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        tx.execute("UPDATE connected_conversations SET display_title=?4 WHERE company=?1 AND connector=?2 AND conversation=?3 AND (display_title IS NULL OR display_title=?6) AND (?5 OR display_title IS NULL)",params![company.as_str(),connector,unread.conversation,title,display.is_some(),conversation_title(company.as_str(),None,"")]).map_err(state::sql(&path))?;
        tx.execute("UPDATE connected_conversations SET display_title=?4 WHERE company=?1 AND connector=?2 AND conversation=?3 AND session_scope='company' AND display_title IS NULL", params![company.as_str(), connector, COMPANY_SCOPE_CONVERSATION, company_title]).map_err(state::sql(&path))?;
        tx.execute(
            "UPDATE conversation_messages
             SET state=CASE WHEN state=?9 THEN ?5 ELSE state END,context_json=?6,boundary=?7,content=?8,
                 conversation_display=coalesce(?10,conversation_display),
                 failure=CASE WHEN state='failed' THEN failure END
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
               AND (state=?9 OR (state IN ('pending','failed') AND attempted_at IS NULL
                    AND (json_extract(context_json,'$.refresh')=1 OR json_extract(context_json,'$.unread')=1)))",
            params![company.as_str(), connector, unread.conversation, unread.message, Stage::Pending.as_str(), context, token, content, Stage::Received.as_str(), display_label],
        )
        .map_err(state::sql(&path))?;
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
            "UPDATE conversation_refusals SET failure=?3 WHERE company=?1 AND connector=?2 AND reason IN ('not_own','not_admin') AND replied_at IS NULL",
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
    pub reason: RefusalReason,
}

/// The refusals of one company's connector, oldest first, each with whether
/// its reply was posted.
pub fn refusals(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
) -> Result<Vec<(Refusal, bool)>, ConversationError> {
    let rows = RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT conversation,sender,reason,replied_at IS NOT NULL FROM conversation_refusals WHERE company=?1 AND connector=?2 AND reason IN ('not_own','not_admin') ORDER BY rowid")
                .map_err(state::sql(path))?;
            q.query_map(params![company.as_str(), connector], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    rows.into_iter()
        .map(|(conversation, sender, reason, posted)| {
            let reason = RefusalReason::parse(&reason).ok_or_else(|| {
                ConversationError::Refused(format!("unknown refusal reason `{reason}`"))
            })?;
            Ok((
                Refusal {
                    conversation,
                    sender,
                    reason,
                },
                posted,
            ))
        })
        .collect()
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
            "UPDATE conversation_refusals SET replied_at=CASE WHEN ?5 IS NULL THEN ?6 END,failure=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND sender=?4 AND reason=?7",
            params![company.as_str(), connector, refusal.conversation, refusal.sender, failure, now.to_string(), refusal.reason.as_str()],
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

#[derive(Clone)]
struct Conversation {
    company: ProjectSlug,
    connector: String,
    conversation: String,
    semantic_key: String,
    opening: Option<String>,
    opening_conversation: Option<String>,
    request: Option<String>,
    session: Option<SessionRef>,
    source_identity: Option<String>,
    position: Option<SourcePosition>,
    role: RoleName,
    session_scope: SessionScope,
    reply_mode: ReplyMode,
}

#[derive(Clone)]
struct Message {
    conversation: String,
    message: String,
    stage: Stage,
    /// Present from `pending` on.
    boundary: Option<String>,
    /// The framed input, present from `pending` on.
    content: Option<String>,
    /// A send to the runtime was begun; its acceptance may be unrecorded.
    attempted: Option<Timestamp>,
    /// The cause retained from its last failed delivery.
    failure: Option<String>,
    retry_at: Option<Timestamp>,
    failed_turns: u32,
    send_unknown: bool,
    dispatch_id: Option<String>,
    dispatch_write_at: Option<Timestamp>,
}

const CONVERSATION_COLUMNS: &str = "company,connector,conversation,semantic_key,opening,opening_conversation,request,session,source_identity,position_json,role,session_scope,reply_mode";

const RELAY_WORK: &str = "SELECT company,connector,conversation FROM conversation_messages WHERE state IN (?1,?2,?3) OR (state='failed' AND superseded_at IS NULL AND boundary IS NOT NULL)
    UNION ALL SELECT company,connector,conversation FROM conversation_answers WHERE posted_at IS NULL";

/// SQL predicate restricting a company-scoped binding to conversations that
/// do NOT have their own `session_scope = 'conversation'` binding.
const COMPANY_OWNED_MESSAGES: &str = "NOT EXISTS (SELECT 1 FROM connected_conversations cc WHERE cc.company=conversation_messages.company AND cc.connector=conversation_messages.connector AND cc.conversation=conversation_messages.conversation AND cc.session_scope='conversation')";
const COMPANY_OWNED_ANSWERS: &str = "NOT EXISTS (SELECT 1 FROM connected_conversations cc WHERE cc.company=conversation_answers.company AND cc.connector=conversation_answers.connector AND cc.conversation=conversation_answers.conversation AND cc.session_scope='conversation')";

fn relay_work_filter() -> String {
    format!(
        "((session_scope='conversation' AND (company,connector,conversation) IN ({RELAY_WORK})) OR (session_scope='company' AND (company,connector) IN (SELECT w.company,w.connector FROM ({RELAY_WORK}) w WHERE NOT EXISTS (SELECT 1 FROM connected_conversations cc WHERE cc.company=w.company AND cc.connector=w.connector AND cc.conversation=w.conversation AND cc.session_scope='conversation'))))"
    )
}

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
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
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
                opening_conversation,
                request,
                session,
                source_identity,
                position,
                role,
                session_scope,
                reply_mode,
            )| {
                Ok(Conversation {
                    company: ProjectSlug::parse(&company)
                        .map_err(|e| ConversationError::Refused(e.to_string()))?,
                    connector,
                    conversation,
                    semantic_key,
                    opening,
                    opening_conversation,
                    request,
                    session: session
                        .map(|s| SessionRef::parse(&s))
                        .transpose()
                        .map_err(|e| ConversationError::Refused(e.to_string()))?,
                    source_identity,
                    position: position.map(|p| serde_json::from_str(&p)).transpose()?,
                    role: RoleName::parse(&role)
                        .map_err(|e| ConversationError::Refused(e.to_string()))?,
                    session_scope: SessionScope::parse(&session_scope).ok_or_else(|| {
                        ConversationError::Refused(format!(
                            "unknown session_scope `{session_scope}`"
                        ))
                    })?,
                    reply_mode: ReplyMode::parse(&reply_mode).ok_or_else(|| {
                        ConversationError::Refused(format!("unknown reply_mode `{reply_mode}`"))
                    })?,
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

/// The conversation binding's messages still requiring relay work, oldest
/// written first, followed by failed turns in retry order.
fn open_messages(state: &RunnerState, c: &Conversation) -> Result<Vec<Message>, ConversationError> {
    let rows = state
        .read(|conn, path| {
            let (sql, by_company) = match c.session_scope {
                SessionScope::Conversation => (
                    "SELECT conversation,message,state,boundary,content,attempted_at,failure,retry_at,failed_turns,send_unknown_at IS NOT NULL,dispatch_command_id,dispatch_write_at
                     FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND state NOT IN (?4,?5) AND superseded_at IS NULL
                     ORDER BY state='failed', retry_at, written_us, message"
                        .to_owned(),
                    false,
                ),
                SessionScope::Company => (
                    format!(
                        "SELECT conversation,message,state,boundary,content,attempted_at,failure,retry_at,failed_turns,send_unknown_at IS NOT NULL,dispatch_command_id,dispatch_write_at
                         FROM conversation_messages WHERE company=?1 AND connector=?2 AND state NOT IN (?3,?4) AND superseded_at IS NULL AND {COMPANY_OWNED_MESSAGES}
                         ORDER BY state='failed', retry_at, written_us, conversation, message"
                    ),
                    true,
                ),
            };
            let mut q = conn.prepare(&sql).map_err(state::sql(path))?;
            let map_row = |r: &instrukt_core::state_db::rusqlite::Row<'_>| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, u32>(8)?,
                    r.get::<_, bool>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, Option<String>>(11)?,
                ))
            };
            if by_company {
                q.query_map(
                    params![
                        c.company.as_str(),
                        c.connector,
                        Stage::Answered.as_str(),
                        Stage::Refused.as_str()
                    ],
                    map_row,
                )
            } else {
                q.query_map(
                    params![
                        c.company.as_str(),
                        c.connector,
                        c.conversation,
                        Stage::Answered.as_str(),
                        Stage::Refused.as_str()
                    ],
                    map_row,
                )
            }
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default();
    rows.into_iter()
        .map(
            |(
                conversation,
                message,
                stage,
                boundary,
                content,
                attempted,
                failure,
                retry_at,
                failed_turns,
                send_unknown,
                dispatch_id,
                dispatch_write_at,
            )| {
                Ok(Message {
                    conversation,
                    message,
                    stage: Stage::parse(&stage).ok_or_else(|| {
                        ConversationError::Refused(format!("unknown message stage `{stage}`"))
                    })?,
                    boundary,
                    content,
                    attempted: attempted.map(|at| at.parse()).transpose().map_err(|_| {
                        ConversationError::Refused("invalid send attempt time".into())
                    })?,
                    failure,
                    retry_at: retry_at.map(|at| at.parse()).transpose().map_err(|_| {
                        ConversationError::Refused("invalid failed-turn retry time".into())
                    })?,
                    failed_turns,
                    send_unknown,
                    dispatch_id,
                    dispatch_write_at: dispatch_write_at.map(|at| at.parse()).transpose().map_err(
                        |_| ConversationError::Refused("invalid dispatch frame write time".into()),
                    )?,
                })
            },
        )
        .collect()
}

/// A source is established only by initial binding or confirmed replacement.
fn require_source(c: &Conversation, current: &str) -> Result<(), ConversationError> {
    match c.source_identity.as_deref() {
        Some(recorded) if recorded == current => Ok(()),
        None if c.session.is_none()
            && c.position.is_none()
            && c.request.is_none()
            && c.opening.is_none() =>
        {
            Ok(())
        }
        recorded => Err(ConversationError::SourceConfiguration(format!(
            "conversation `{}` records source `{}` but the current source is `{current}`; restore the recorded binding/source from retained evidence or use existing confirmed session replacement when its presence checks permit it",
            c.semantic_key,
            recorded.unwrap_or("<unavailable>"),
        ))),
    }
}

/// The configured source and clock under which this native delivery was prepared.
struct NativeDelivery<'a> {
    home: &'a Home,
    identity: &'a str,
    env: &'a Env,
    clock: &'a dyn Fn() -> Timestamp,
}

impl NativeDelivery<'_> {
    fn check_source(
        &self,
        adapter: &dyn RuntimeAdapter,
        state: &RunnerState,
        c: &Conversation,
    ) -> Result<(), ConversationError> {
        let source = adapter.conversation_source(self.home, self.env)?;
        if source.identity() != self.identity {
            return Err(ConversationError::SourceConfiguration(format!(
                "conversation `{}` was prepared against source `{}` but the current source is `{}`; restore the configured source before retrying native delivery",
                c.semantic_key,
                self.identity,
                source.identity(),
            )));
        }
        if let Some(current) = reread(state, c)? {
            require_source(&current, source.identity())
        } else {
            Err(ConversationError::SourceConfiguration(format!(
                "conversation `{}` no longer has its recorded binding",
                c.semantic_key
            )))
        }
    }
}

fn lock(home: &Home, c: &Conversation) -> Result<instrukt_core::fsx::LockGuard, ConversationError> {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(&Sha256::digest(c.semantic_key.as_bytes())[..8]);
    Ok(instrukt_core::fsx::LockGuard::acquire(
        &home.state_dir().join(format!("conversation-{digest}.lock")),
        std::time::Duration::from_secs(120),
    )?)
}

/// One addressed message framed for its own turn: the context envelope and
/// the member envelope naming its sender, sharing one fresh boundary token
/// that occurs nowhere in what they frame.
#[allow(clippy::too_many_arguments)]
fn framed(
    connector: &str,
    conversation: &str,
    sender: &str,
    is_admin: bool,
    is_direct: bool,
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
            is_admin,
            is_direct,
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
fn opening_procedure<'a>(
    role: &str,
    declared: Option<RoleProcedure<'a>>,
) -> Result<&'a IndexEntry, ConversationError> {
    match declared {
        Some(RoleProcedure::Delivered(entry)) => Ok(entry),
        Some(RoleProcedure::Pending(_)) | None => Err(ConversationError::NoProcedure {
            role: role.to_owned(),
        }),
    }
}

/// Resolve the opening procedure for a connector's `role`: first any delivered
/// published procedure declaring `role`; if `role` is configured in
/// `agents.yaml` without a dedicated procedure of its own, fall back to the
/// delivered `company-agent` procedure.
fn opening_procedure_for_role<'a>(
    index: &'a GlobalIndex,
    role: &RoleName,
    agents: &Agents,
) -> Result<&'a IndexEntry, ConversationError> {
    if let Ok(entry) = opening_procedure(
        role.as_str(),
        index.procedures_declaring(role).into_iter().next(),
    ) {
        return Ok(entry);
    }
    if role.as_str() != COMPANY_AGENT_ROLE && agents.roles.contains_key(role.as_str()) {
        let fallback_role = RoleName::parse(COMPANY_AGENT_ROLE)
            .map_err(|e| ConversationError::Refused(e.to_string()))?;
        return opening_procedure(
            COMPANY_AGENT_ROLE,
            index
                .procedures_declaring(&fallback_role)
                .into_iter()
                .next(),
        );
    }
    Err(ConversationError::NoProcedure {
        role: role.to_string(),
    })
}

/// The runner-authored opening assignment of a conversation session: the
/// loader of its procedure, with the facts it is assigned.
fn assignment(
    admitted: &Admitted,
    c: &Conversation,
    procedure: &SnippetId,
) -> Result<String, ConversationError> {
    let role = c.role.as_str();
    let capabilities = admitted.agents().capabilities(role)?.describe();
    let scope_line = match c.session_scope {
        SessionScope::Conversation => format!("Conversation: {}", c.conversation),
        SessionScope::Company => {
            "Session scope: company (shared across admitted conversations on this connector)"
                .to_owned()
        }
    };
    let reply_line = match (c.reply_mode, c.session_scope) {
        (ReplyMode::TurnOutput, _) => String::new(),
        (ReplyMode::Explicit, SessionScope::Conversation) => "\nReply mode: explicit (raw turn output is not posted; send replies with `instrukt company reply --text <text>`)".to_owned(),
        (ReplyMode::Explicit, SessionScope::Company) => "\nReply mode: explicit (raw turn output is not posted; send replies with `instrukt company reply --conversation <id> --text <text>`)".to_owned(),
    };
    Ok(Control::Assignment.frame(&format!(
        "instrukt conversation assignment\nProcedure: {procedure}\nRole: {role}\nResolved role capabilities: {capabilities}\nCompany: {}\nConnector: {}\n{scope_line}{reply_line}\nLoad `instrukt docs get {procedure} --closure` before handling the member's message.\n",
        c.company, c.connector
    )))
}

/// The position a new session's answers are read from: one over no
/// conversation yet, so everything its own thread comes to hold is read after
/// it. Capturing it reads no native history.
fn opening_position(source: &dyn ConversationSource) -> Result<SourcePosition, ConversationError> {
    Ok(source.changed_since(None, &[])?.position)
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
    conversation: &str,
    message: &str,
    failure: Option<&str>,
) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET failure=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND superseded_at IS NULL",
            params![c.company.as_str(), c.connector, conversation, message, failure],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// Open the no-work interval of each `(conversation, message)` at `at`, where
/// none is open: a confirmed failure keeps the agent from the work they wait on.
fn unavailable(
    state: &RunnerState,
    c: &Conversation,
    messages: &[(String, String)],
    at: Timestamp,
) -> Result<(), StateError> {
    if messages.is_empty() {
        return Ok(());
    }
    state.write(|tx| {
        for (conversation, message) in messages {
            tx.execute(
                "UPDATE conversation_messages SET unavailable_since=coalesce(unavailable_since,?5) WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND superseded_at IS NULL AND send_unknown_at IS NULL",
                params![c.company.as_str(), c.connector, conversation, message, at.to_string()],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
        }
        Ok(())
    })
}

/// Close the no-work interval of each `(conversation, message)`: the work on
/// them is shown to go on.
fn available(
    state: &RunnerState,
    c: &Conversation,
    messages: &[(String, String)],
) -> Result<(), StateError> {
    if messages.is_empty() {
        return Ok(());
    }
    state.write(|tx| {
        let mut conversations = BTreeSet::new();
        for (conversation, message) in messages {
            tx.execute(
                "UPDATE conversation_messages SET unavailable_since=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND unavailable_since IS NOT NULL",
                params![c.company.as_str(), c.connector, conversation, message],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
            conversations.insert(conversation.as_str());
        }
        for conversation in conversations {
            retire_notices(tx, c.company.as_str(), &c.connector, conversation)?;
        }
        Ok(())
    })
}

/// Keep `update`, the newest message the agent has written in the running
/// turn of `message`, as what its status shows.
fn turn_update(
    state: &RunnerState,
    c: &Conversation,
    conversation: &str,
    message: &str,
    update: Option<&str>,
) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET turn_update=?5 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND turn_update IS NOT ?5",
            params![c.company.as_str(), c.connector, conversation, message, update],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

pub(crate) fn context_exclusions(
    state: &RunnerState,
    company: &ProjectSlug,
    connector: &str,
    conversation: &str,
) -> Result<std::collections::HashSet<String>, StateError> {
    Ok(state.read(|conn, path| {
        let mut query = conn.prepare(
            "SELECT message FROM conversation_context_receipts
             WHERE company=?1 AND connector=?2 AND conversation=?3
             UNION SELECT message FROM conversation_messages
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND (state IN ('received','pending') OR send_unknown_at IS NOT NULL)",
        ).map_err(state::sql(path))?;
        query.query_map(params![company.as_str(), connector, conversation], |row| row.get::<_, String>(0))
            .map_err(state::sql(path))?.collect::<Result<std::collections::HashSet<_>, _>>().map_err(state::sql(path))
    })?.unwrap_or_default())
}

/// Save the precise input before native I/O. A pending native attempt
/// keeps its original input so lost acknowledgments can be reconciled.
fn prepare_delivery(
    state: &RunnerState,
    c: &Conversation,
    next: &Message,
) -> Result<Option<Message>, ConversationError> {
    if next.attempted.is_some() || (c.session.is_none() && c.request.is_some()) {
        return Ok(Some(next.clone()));
    }
    let Some(content) = next.content.as_deref() else {
        return Ok(Some(next.clone()));
    };
    let row = state.read(|conn, path| {
        conn.query_row(
            "SELECT context_json,sender_name,written_at,coalesce(json_extract(context_json,'$.unread'),0)=1,sender_is_admin,is_direct,sender,sender_email,conversation_display,thread FROM conversation_messages
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
            params![
                c.company.as_str(),
                c.connector,
                next.conversation,
                next.message
            ],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, i32>(4)? != 0,
                    row.get::<_, i32>(5)? != 0,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .map_err(state::sql(path))
    })?;
    let Some((
        Some(captured),
        Some(sender),
        written_at,
        incomplete,
        is_admin,
        is_direct,
        sender_id,
        sender_email,
        conversation_display,
        thread,
    )) = row
    else {
        return Ok(Some(next.clone()));
    };
    let mut accompanying: Accompanying = serde_json::from_str(&captured)?;
    if incomplete || accompanying.refresh {
        return Ok(None);
    }
    let excluded = context_exclusions(state, &c.company, &c.connector, &next.conversation)?;
    accompanying
        .messages
        .retain(|earlier| earlier.id.as_ref().is_none_or(|id| !excluded.contains(id)));
    if let Some(limit) = accompanying.limit {
        let trimmed = accompanying.messages.len().saturating_sub(limit);
        let has_covered_selection = accompanying
            .messages
            .iter()
            .rev()
            .take_while(|entry| {
                accompanying
                    .complete_from
                    .is_none_or(|first| entry.at.is_some_and(|at| at >= first))
            })
            .take(limit.saturating_add(1))
            .count()
            > limit;
        if accompanying.stopped_early && !has_covered_selection {
            // Eligibility changed after capture. Read further before delivery.
            state.write::<_, StateError>(|tx| {
                tx.execute(
                    "UPDATE conversation_messages SET context_json=json_set(context_json,'$.refresh',json('true'))
                     WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                    params![c.company.as_str(), c.connector, next.conversation, next.message],
                ).map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })?;
            return Ok(None);
        }
        accompanying.left_out = trimmed > 0;
        accompanying.messages.drain(..trimmed);
    }
    let ids = accompanying
        .messages
        .iter()
        .filter_map(|earlier| earlier.id.as_deref())
        .collect::<Vec<_>>();
    let text = envelopes::segment(content)
        .into_iter()
        .find_map(|span| match span {
            Span::Member { body, .. } => Some(body),
            _ => None,
        })
        .ok_or_else(|| {
            ConversationError::Refused("the retained question has no member envelope".into())
        })?;
    let written_at: Timestamp = written_at
        .parse()
        .map_err(|error: jiff::Error| ConversationError::Refused(error.to_string()))?;
    let (boundary, base_content) = framed(
        &c.connector,
        &next.conversation,
        &sender,
        is_admin,
        is_direct,
        written_at,
        text,
        &accompanying,
    )?;
    let content = match c.session_scope {
        SessionScope::Conversation => base_content,
        SessionScope::Company => {
            let safe_conversation = envelopes::control_field(&next.conversation);
            let mut body = format!(
                "Company conversation turn\nConnector: {}\nConversation: {safe_conversation}\nDirect: {is_direct}\n",
                envelopes::control_field(&c.connector)
            );
            if let Some(display) = conversation_display.as_deref() {
                let place = envelopes::control_field(display);
                if !place.is_empty() {
                    body.push_str(&format!("Context: {place}\n"));
                }
            }
            if let Some(thread) = thread.as_deref() {
                let thread = envelopes::control_field(thread);
                if !thread.is_empty() {
                    body.push_str(&format!("Thread: {thread}\n"));
                }
            } else if is_direct {
                body.push_str("Thread: direct-message\n");
            }
            if let Some(id) = sender_id.as_deref() {
                body.push_str(&format!("Speaker ID: {}\n", envelopes::control_field(id)));
            }
            if let Some(email) = sender_email
                .as_deref()
                .filter(|email| !email.trim().is_empty())
            {
                body.push_str(&format!(
                    "Speaker Email: {}\n",
                    envelopes::control_field(email)
                ));
            }
            body.push_str(&format!("Speaker Admin: {is_admin}\n"));
            body.push_str(&format!(
                "Speaker Name: {}\n",
                envelopes::control_field(&sender)
            ));
            if c.reply_mode == ReplyMode::Explicit {
                body.push_str(&format!(
                    "Reply command: instrukt company reply --conversation {safe_conversation} --text <text>\n"
                ));
            }
            let header = Control::Notification.frame(&body);
            format!("{header}\n{base_content}")
        }
    };
    let ids = serde_json::to_string(&ids)?;
    state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET content=?5,boundary=?6,context_delivery_ids=?7
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
            params![
                c.company.as_str(),
                c.connector,
                next.conversation,
                next.message,
                content,
                boundary,
                ids
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(Some(Message {
        boundary: Some(boundary),
        content: Some(content),
        ..next.clone()
    }))
}

fn remember_delivery(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    c: &Conversation,
    conversation: &str,
    message: &str,
) -> Result<(), StateError> {
    tx.execute(
        "INSERT INTO conversation_context_receipts(company,connector,conversation,message,delivery)
         SELECT ?1,?2,?3,?4,?4
         UNION ALL SELECT ?1,?2,?3,ids.value,?4 FROM conversation_messages m,
              json_each(m.context_delivery_ids) ids
         WHERE m.company=?1 AND m.connector=?2 AND m.conversation=?3 AND m.message=?4
         ON CONFLICT(company,connector,conversation,message) DO UPDATE SET
             delivery=excluded.delivery,retired_at=NULL",
        params![c.company.as_str(), c.connector, conversation, message],
    )
    .map_err(state::sql(&state::db_path(tx)))?;
    Ok(())
}

/// The runtime accepted `message` as its own turn, which ends its no-work
/// interval; a first message's acceptance also binds the conversation's
/// session.
fn accepted(
    state: &RunnerState,
    c: &Conversation,
    conversation: &str,
    message: &str,
    boundary: Option<&str>,
    session: Option<&SessionRef>,
    source_identity: &str,
) -> Result<bool, StateError> {
    let applied = state.write(|tx| {
        let path = state::db_path(tx);
        let changed = tx.execute(
            "UPDATE conversation_messages SET state='sent',failure=NULL,unavailable_since=NULL,turn_update=NULL,retry_at=NULL,send_unknown_at=NULL
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
               AND boundary=?5 AND attempted_at IS NOT NULL
               AND state IN ('pending','failed') AND superseded_at IS NULL
               AND EXISTS(SELECT 1 FROM connected_conversations c WHERE c.company=?1 AND c.connector=?2 AND c.conversation=?7 AND c.source_identity=?6)",
            params![c.company.as_str(), c.connector, conversation, message, boundary, source_identity, c.conversation],
        ).map_err(state::sql(&path))?;
        if changed != 0 {
            remember_delivery(tx, c, conversation, message)?;
            retire_notices(tx, c.company.as_str(), &c.connector, conversation)?;
        }
        if changed != 0 && let Some(session) = session {
            tx.execute(
                "UPDATE connected_conversations SET session=?4 WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![c.company.as_str(), c.connector, c.conversation, session.to_string()],
            ).map_err(state::sql(&path))?;
        }
        Ok::<_, StateError>(changed != 0)
    })?;
    if !applied {
        stale_acknowledgement(c, message);
    }
    Ok(applied)
}

fn stale_acknowledgement(c: &Conversation, message: &str) {
    tracing::warn!(target: LOG_TARGET, event = "conversation_stale_acknowledgement", key = %c.semantic_key, message);
}

const FAILED_RETRY_DELAY: SignedDuration = SignedDuration::from_mins(2);
const FAILED_RETRY_LIMIT: SignedDuration = SignedDuration::from_mins(15);
const ANSWER_OPERATOR_BOUND: SignedDuration = SignedDuration::from_mins(5);
const SEND_RECONCILE_BOUND: SignedDuration = SignedDuration::from_mins(5);

fn retry_delay(previous_failures: u32) -> SignedDuration {
    (FAILED_RETRY_DELAY * (1_i32 << previous_failures.min(3))).min(FAILED_RETRY_LIMIT)
}

/// A failed turn with a ready retry still yields to new messages.
fn next_message<'a>(open: &'a [Message], c: &Conversation, now: Timestamp) -> Option<&'a Message> {
    let available = || open.iter().filter(|message| !message.send_unknown);
    if available().any(|m| m.stage == Stage::Sent) {
        return None;
    }
    let next = match (&c.session, &c.opening) {
        (None, Some(opening)) => open.iter().find(|m| {
            &m.message == opening
                && c.opening_conversation
                    .as_deref()
                    .is_none_or(|conv| conv == m.conversation)
        }),
        _ => available()
            .find(|m| {
                m.attempted.is_some()
                    && m.dispatch_id.is_some()
                    && matches!(m.stage, Stage::Pending | Stage::Failed)
            })
            .or_else(|| available().find(|m| m.stage != Stage::Failed))
            .or_else(|| available().find(|m| m.retry_at.is_some_and(|at| now >= at))),
    }?;
    matches!(next.stage, Stage::Pending | Stage::Failed).then_some(next)
}

fn schedule_earlier_failures(
    state: &RunnerState,
    c: &Conversation,
    now: Timestamp,
) -> Result<(), StateError> {
    state.write(|tx| {
        let retry = format!("{:.9}", now + FAILED_RETRY_DELAY);
        match c.session_scope {
            SessionScope::Conversation => tx.execute(
                "UPDATE conversation_messages SET retry_at=?4,failed_turns=1
                 WHERE company=?1 AND connector=?2 AND conversation=?3 AND state='failed'
                   AND superseded_at IS NULL AND send_unknown_at IS NULL AND retry_at IS NULL AND boundary IS NOT NULL",
                params![c.company.as_str(), c.connector, c.conversation, retry],
            ),
            SessionScope::Company => tx.execute(
                &format!(
                    "UPDATE conversation_messages SET retry_at=?3,failed_turns=1
                     WHERE company=?1 AND connector=?2 AND state='failed'
                       AND superseded_at IS NULL AND send_unknown_at IS NULL AND retry_at IS NULL AND boundary IS NOT NULL
                       AND {COMPANY_OWNED_MESSAGES}"
                ),
                params![c.company.as_str(), c.connector, retry],
            ),
        }
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// Deliver the conversation's next addressed message. The first starts its
/// session by key in the company's home, with an opening request owned by
/// that one message; a later one waits until the session's current turn has
/// finished. Failures stay with the message until it is delivered.
fn deliver_one(
    invocation: &RuntimeInvocation,
    admitted: &Admitted,
    adapter: &dyn RuntimeAdapter,
    opening_source: Result<Option<OpeningSource>, ConversationError>,
    c: &Conversation,
    next: &Message,
    delivery: &NativeDelivery<'_>,
) -> Result<(), ConversationError> {
    let state = &RunnerState::new(invocation.home());
    if let Err(error) = delivery.check_source(adapter, state, c) {
        set_failure(state, c, Some(&error.to_string()))?;
        return Err(error);
    }
    let result = match prepare_delivery(state, c, next) {
        Ok(None) => return Ok(()),
        Err(error) => Err(error),
        Ok(Some(prepared)) => match &c.session {
            None => opening_source.and_then(|source| {
                start(
                    invocation, admitted, adapter, source, c, &prepared, delivery,
                )
            }),
            Some(session) => send(adapter, state, c, session, &prepared, delivery),
        },
    };
    record_delivery_failure(adapter, state, c, next, delivery.clock, &result)?;
    result
}

fn record_delivery_failure(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    next: &Message,
    clock: &dyn Fn() -> Timestamp,
    result: &Result<(), ConversationError>,
) -> Result<(), ConversationError> {
    if let Err(error @ ConversationError::SourceConfiguration(_)) = result {
        set_failure(state, c, Some(&error.to_string()))?;
        return Ok(());
    }
    // A runtime failure is retained through the adapter's safe diagnostic,
    // never its raw native output.
    let failure = result.as_ref().err().map(|error| match error {
        ConversationError::Sessions(native) => adapter
            .diagnostic("conversation delivery", native)
            .to_string(),
        other => other.to_string(),
    });
    if let (Err(error), Some(cause)) = (result, &failure)
        && reports(error, next.failure.as_deref(), cause)
    {
        tracing::warn!(target: LOG_TARGET, event = "conversation_delivery_failed", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %cause);
    }
    if let Some(cause) = &failure
        && send_reconciliation_due(next, clock())
    {
        send_unknown(state, c, next, clock(), cause)?;
        return Ok(());
    }
    set_message_failure(
        state,
        c,
        &next.conversation,
        &next.message,
        failure.as_deref(),
    )?;
    // A start, send or listing that is refused or fails keeps the agent from
    // the message: its no-work interval opens when the failure is
    // established, read on the clock then.
    if result.is_err() {
        unavailable(
            state,
            c,
            &[(next.conversation.clone(), next.message.clone())],
            clock(),
        )?;
    }
    Ok(())
}

struct OpeningSource {
    identity: String,
    position: SourcePosition,
}

#[allow(clippy::too_many_arguments)]
fn start(
    invocation: &RuntimeInvocation,
    admitted: &Admitted,
    adapter: &dyn RuntimeAdapter,
    opening_source: Option<OpeningSource>,
    c: &Conversation,
    next: &Message,
    delivery: &NativeDelivery<'_>,
) -> Result<(), ConversationError> {
    let home = invocation.home();
    let state = &RunnerState::new(home);
    let now = (delivery.clock)();
    let request = match &c.request {
        Some(request) => serde_json::from_str::<RuntimeStart>(request)?,
        None => {
            let index = GlobalIndex::load_readable(home)?.unwrap_or_default();
            let procedure = opening_procedure_for_role(&index, &c.role, admitted.agents())?
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
                Some(c.role.as_str()),
                company.profiles()?.as_deref(),
            )?;
            let selection = instrukt_runner::agents::selections(config, home, &preferences, now)?
                .into_iter()
                .next();
            let input = next.content.as_deref().ok_or_else(|| {
                ConversationError::Refused("the message is not framed yet".into())
            })?;
            let default_title = match c.session_scope {
                SessionScope::Company => format!("{} · {}", c.company, c.connector),
                SessionScope::Conversation => conversation_title(c.company.as_str(), None, ""),
            };
            let request = RuntimeStart {
                preferences,
                selection,
                selection_evidence: Some(adapter.selection_realization(
                    instrukt_runner::runtime::SelectionOperation::ThreadStart,
                )),
                checkout,
                title: state.read(|conn,path|conn.query_row("SELECT display_title FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3",params![c.company.as_str(),c.connector,c.conversation],|row|row.get::<_,Option<String>>(0)).map_err(state::sql(path)))?.flatten().unwrap_or(default_title),
                key: c.semantic_key.clone(),
                brief: format!("{}\n{input}", assignment(admitted, c, &procedure)?),
                worktree: None,
                launch_modes: None, native_launch: None, profiles: instrukt_runner::runtime::Profiles(Some(admitted.shared_agents())),
            };
            let source = opening_source.ok_or_else(|| {
                ConversationError::Refused("the opening source is unavailable".into())
            })?;
            state.write::<_, StateError>(|tx| {
                let path = state::db_path(tx);
                tx.execute(
                    "UPDATE connected_conversations SET opening=?4,opening_conversation=?8,request=?5,source_identity=?6,position_json=?7 WHERE company=?1 AND connector=?2 AND conversation=?3",
                    params![
                        c.company.as_str(),
                        c.connector,
                        c.conversation,
                        next.message,
                        serde_json::to_string(&request).map_err(|e| state::invalid(&path, e))?,
                        source.identity,
                        serde_json::to_string(&source.position).map_err(|e| state::invalid(&path, e))?,
                        next.conversation
                    ],
                )
                .map_err(state::sql(&path))?;
                Ok(())
            })?;
            request
        }
    };
    state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET attempted_at=coalesce(attempted_at,?5)
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
            params![
                c.company.as_str(),
                c.connector,
                next.conversation,
                next.message,
                now.to_string()
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    delivery.check_source(adapter, state, c)?;
    let started = adapter.start_or_wake(&RuntimeAssignment {
        bindings: None,
        invocation,
        project: None,
        request: &request,
        now,
    });
    delivery.check_source(adapter, state, c)?;
    let thread = started?;
    accepted(
        state,
        c,
        &next.conversation,
        &next.message,
        next.boundary.as_deref(),
        Some(&thread.session),
        delivery.identity,
    )?;
    tracing::info!(target: LOG_TARGET, event = "conversation_session_started", company = %c.company, connector = %c.connector, key = %c.semantic_key, session = %thread.session);
    Ok(())
}

fn send(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    session: &SessionRef,
    next: &Message,
    delivery: &NativeDelivery<'_>,
) -> Result<(), ConversationError> {
    let clock = delivery.clock;
    let now = clock();
    let content = next
        .content
        .as_deref()
        .ok_or_else(|| ConversationError::Refused("the message is not framed yet".into()))?;
    // The bound thread must run in the company home's own runtime project
    // before anything is attempted: a misplaced one is sent nothing.
    let thread = adapter.read_state(session)?;
    if thread.presence != Presence::Present {
        return Err(SessionsError::Gone(session.clone()).into());
    }
    adapter.verify_thread_project(
        &delivery.home.company_dir(&c.company),
        &thread,
        Some(&c.semantic_key),
    )?;
    if let Some(id) = &next.dispatch_id {
        dispatch_frame(adapter, state, c, next, session, (id, content), delivery)?;
        accepted(
            state,
            c,
            &next.conversation,
            &next.message,
            next.boundary.as_deref(),
            None,
            delivery.identity,
        )?;
        return Ok(());
    }
    let old_boundary = next
        .boundary
        .as_deref()
        .ok_or_else(|| ConversationError::Refused("the message has no delivery boundary".into()))?;
    let boundary = envelopes::delivery_boundary(&[content])?;
    let content = content.replace(old_boundary, &boundary);
    use sha2::{Digest, Sha256};
    let id = format!(
        "instrukt-company-{}",
        hex::encode(Sha256::digest(serde_json::to_vec(&(
            &c.semantic_key,
            &next.conversation,
            &next.message,
            &boundary
        ))?))
    );
    let recorded = state.write::<_, StateError>(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET attempted_at=?5,content=?6,boundary=?7,dispatch_command_id=?9
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
               AND boundary=?8 AND attempted_at IS NULL AND superseded_at IS NULL",
            params![c.company.as_str(), c.connector, next.conversation, next.message, now.to_string(),
                content, boundary, old_boundary, id],
        ).map_err(state::sql(&state::db_path(tx)))
    })?;
    if recorded != 0 {
        if let Err(error) =
            dispatch_frame(adapter, state, c, next, session, (&id, &content), delivery)
        {
            if let ConversationError::Sessions(error) = &error
                && adapter.send_certainly_rejected(session, error)
            {
                clear_rejected_send(state, c, &next.conversation, &next.message, &boundary)?;
            }
            return Err(error);
        }
        accepted(
            state,
            c,
            &next.conversation,
            &next.message,
            Some(&boundary),
            None,
            delivery.identity,
        )?;
    }
    Ok(())
}

fn dispatch_frame(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    next: &Message,
    session: &SessionRef,
    (id, content): (&str, &str),
    delivery: &NativeDelivery<'_>,
) -> Result<(), ConversationError> {
    let clock = delivery.clock;
    delivery.check_source(adapter, state, c)?;
    let mut marked = false;
    let result = adapter.dispatch_send(session, id, content, &mut || {
        if next.dispatch_write_at.is_none() {
            record_dispatch_write(
                state,
                c,
                &next.conversation,
                &next.message,
                id,
                Some(clock()),
            )?;
            marked = true;
        }
        Ok(())
    });
    delivery.check_source(adapter, state, c)?;
    match result {
        Ok(()) => Ok(()),
        Err(DispatchError::BeforeFrame(error)) => {
            if marked {
                // This write sent zero bytes. Keep any marker from an earlier
                // write that already made the command uncertain.
                record_dispatch_write(state, c, &next.conversation, &next.message, id, None)?;
            }
            Err(error.into())
        }
        Err(DispatchError::AfterFrame(error)) => Err(error.into()),
    }
}

fn record_dispatch_write(
    state: &RunnerState,
    c: &Conversation,
    conversation: &str,
    message: &str,
    id: &str,
    at: Option<Timestamp>,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let changed = tx.execute(
            "UPDATE conversation_messages SET dispatch_write_at=?6
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND dispatch_command_id=?5
               AND attempted_at IS NOT NULL AND send_unknown_at IS NULL
               AND state IN ('pending','failed')",
            params![c.company.as_str(), c.connector, conversation, message, id, at.map(|time| time.to_string())],
        ).map_err(state::sql(&path))?;
        if changed != 1 {
            return Err(state::invalid(&path, "the dispatch attempt changed before writing its frame"));
        }
        Ok(())
    })
}

fn clear_rejected_send(
    state: &RunnerState,
    c: &Conversation,
    conversation: &str,
    message: &str,
    boundary: &str,
) -> Result<(), StateError> {
    state.write(|tx| {
        tx.execute(
            "UPDATE conversation_messages SET attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND boundary=?5
               AND state IN ('pending','failed') AND send_unknown_at IS NULL
               AND EXISTS(SELECT 1 FROM connected_conversations c WHERE c.company=?1 AND c.connector=?2 AND c.conversation=?7 AND c.source_identity IS ?6)",
            params![
                c.company.as_str(),
                c.connector,
                conversation,
                message,
                boundary,
                c.source_identity,
                c.conversation
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

fn send_unknown(
    state: &RunnerState,
    c: &Conversation,
    message: &Message,
    now: Timestamp,
    cause: &str,
) -> Result<(), StateError> {
    state.write(|tx| {
        let cause = format!("delivery unknown for message {}: {cause}", message.message);
        let changed = tx.execute(
            "UPDATE conversation_messages SET send_unknown_at=?6,failure=?7,unavailable_since=NULL,retry_at=NULL
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND boundary=?5
               AND attempted_at IS NOT NULL AND state IN ('pending','failed')
               AND send_unknown_at IS NULL AND superseded_at IS NULL",
            params![c.company.as_str(), c.connector, message.conversation, message.message, message.boundary, now.to_string(), cause],
        ).map_err(state::sql(&state::db_path(tx)))?;
        if changed != 0 {
            retire_notices(tx, c.company.as_str(), &c.connector, &message.conversation)?;
            tracing::warn!(target: LOG_TARGET, event = "conversation_send_delivery_unknown", key = %c.semantic_key, conversation = %message.conversation, message = %message.message, %cause);
        }
        Ok(())
    })
}

fn possible_dispatch_at(message: &Message) -> Option<Timestamp> {
    if message.dispatch_id.is_some() {
        message.dispatch_write_at
    } else {
        message.attempted
    }
}

fn send_reconciliation_due(message: &Message, now: Timestamp) -> bool {
    !message.send_unknown
        && matches!(message.stage, Stage::Pending | Stage::Failed)
        && possible_dispatch_at(message).is_some_and(|at| now >= at + SEND_RECONCILE_BOUND)
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

const FAILED_TURN: &str = "the turn ended without a reply";

fn message_for_input<'a>(sent: &'a [Message], text: &str) -> Option<&'a Message> {
    envelopes::segment(text)
        .into_iter()
        .find_map(|span| match span {
            Span::Member { boundary, .. } => sent
                .iter()
                .find(|message| message.boundary.as_deref() == Some(boundary)),
            _ => None,
        })
}

/// Read finished turns of a conversation session from its own retained
/// source position: the conversational output of a turn its addressed
/// message started becomes that message's answer (under `turn_output`), or
/// closes the turn without posting raw turn output (under `explicit`). A turn
/// started any other way is not posted. Missing native content leaves the
/// delivery unresolved with its cause; the position does not move past it.
fn harvest(
    state: &RunnerState,
    source: &dyn ConversationSource,
    c: &Conversation,
    message_chars: usize,
    reply_mode: ReplyMode,
    clock: &dyn Fn() -> Timestamp,
) -> Result<Presence, ConversationError> {
    require_source(c, source.identity())?;
    let Some(session) = &c.session else {
        return Ok(Presence::Present);
    };
    let mut read_from = c.position.clone();
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    // Only the session's own thread is read: other threads, changing or
    // not, neither delay nor fail its answers.
    let own = std::slice::from_ref(&session.id);
    loop {
        let batch = source.changed_since(read_from.as_ref(), own)?;
        let presence = batch.scoped_presence.unwrap_or(Presence::Present);
        if batch.missing.iter().any(|id| id == &session.id) {
            return Ok(Presence::Absent);
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
        if !batch.has_more
            && batch.records.is_empty()
            && batch.completed_turns.is_empty()
            && read_from.as_ref() == Some(&batch.position)
        {
            return Ok(presence);
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
            .filter(|m| m.stage == Stage::Sent || possible_dispatch_at(m).is_some())
            .collect::<Vec<_>>();
        // A retained input may belong to a turn that is still running.
        // A completed run that has no whole turn yet is retried from this same
        // position, even when the source could read its opening input.
        let complete = turns
            .iter()
            .map(|turn| turn.native_turn.as_str())
            .collect::<BTreeSet<_>>();
        let runs = batch
            .records
            .iter()
            .filter(|record| matches!(record.kind, RecordKind::Run))
            .map(|record| (record.id.as_str(), record.completed))
            .collect::<BTreeMap<_, _>>();
        for input in batch.records.iter().filter(|record| {
            matches!(record.kind, RecordKind::Message) && matches!(record.role, Some(Role::User))
        }) {
            let Some(message) = input
                .text
                .as_deref()
                .and_then(|text| message_for_input(&sent, text))
            else {
                continue;
            };
            let Some((run_id, completed)) = input
                .run_id
                .as_deref()
                .and_then(|run_id| runs.get_key_value(run_id))
            else {
                continue;
            };
            if *completed && !complete.contains(run_id) {
                return Err(ConversationError::Refused(
                    "completed-turn evidence unavailable: the turn's content is unstable".into(),
                ));
            }
            found.insert((message.conversation.clone(), message.message.clone()));
        }
        let mut answers = Vec::new();
        for turn in turns {
            let started = turn
                .contributions
                .iter()
                .filter(|contribution| contribution.author.kind == AuthorKind::Human)
                .find_map(|contribution| message_for_input(&sent, &contribution.text));
            let Some(message) = started else {
                continue;
            };
            found.insert((message.conversation.clone(), message.message.clone()));
            if answers.iter().any(|(prior, _, _): &(&Message, _, _)| {
                prior.conversation == message.conversation && prior.message == message.message
            }) {
                continue;
            }
            let output = turn
                .contributions
                .iter()
                .filter(|contribution| contribution.author.kind == AuthorKind::Agent)
                .map(|contribution| contribution.text.as_str())
                .filter(|text| {
                    let said = text.trim();
                    !said.is_empty() && said != "NOOP"
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            answers.push((message, turn, output));
        }
        let ended = clock();
        state.write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            let mut settle_conversations: BTreeSet<&str> = BTreeSet::new();
            for (message, turn, output) in &answers {
                remember_delivery(tx, c, &message.conversation, &message.message)?;
                if !turn.succeeded {
                    tx.execute(
                        "UPDATE conversation_messages SET state='failed',failure=?5,turn_update=NULL,
                         unavailable_since=coalesce(unavailable_since,?6),attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,send_unknown_at=NULL,retry_at=?7,failed_turns=failed_turns+1
                         WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND superseded_at IS NULL",
                        params![c.company.as_str(), c.connector, message.conversation, message.message, FAILED_TURN, ended.to_string(), format!("{:.9}", ended + retry_delay(message.failed_turns))],
                    )
                    .map_err(state::sql(&path))?;
                    settle_conversations.insert(message.conversation.as_str());
                    continue;
                }
                let turn_id = turn.native_turn.to_string();
                let has_answer = match reply_mode {
                    ReplyMode::TurnOutput => {
                        for (piece, text) in split(output, message_chars).iter().enumerate() {
                            tx.execute(
                                "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,admission_position)
                                 SELECT ?1,?2,?3,?4,?5,?6,admission_position FROM conversation_messages
                                 WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?7 AND superseded_at IS NULL
                                 ON CONFLICT DO NOTHING",
                                params![c.company.as_str(), c.connector, message.conversation, turn_id, i64::try_from(piece).unwrap_or(i64::MAX), text, message.message],
                            )
                            .map_err(state::sql(&path))?;
                        }
                        !output.is_empty()
                    }
                    ReplyMode::Explicit => false,
                };
                if !has_answer {
                    settle_conversations.insert(message.conversation.as_str());
                }
                tx.execute(
                    // The finished turn proves the message was accepted: its
                    // resolved delivery cause is retired with it. Its turn's
                    // end is kept before any post is tried: a post that then
                    // fails opens its interval from that end. A turn with an
                    // answer to post opens nothing now, and its status keeps
                    // its newest message while the answer is posted; one with
                    // nothing to post completes the message.
                    "UPDATE conversation_messages SET state=?5,turn=?6,failure=NULL,retry_at=NULL,attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,send_unknown_at=NULL,turn_ended_at=?8,turn_update=CASE WHEN ?7 THEN turn_update END,unavailable_since=CASE WHEN ?7 THEN unavailable_since END WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND superseded_at IS NULL",
                    params![c.company.as_str(), c.connector, message.conversation, message.message, Stage::Answered.as_str(), turn_id, has_answer, ended.to_string()],
                )
                .map_err(state::sql(&path))?;
            }
            for conversation in settle_conversations {
                settle_failed_messages(tx, c.company.as_str(), &c.connector, conversation, ended)?;
            }
            if !batch.has_more {
                for message in sent.iter().filter(|message| message.send_unknown) {
                    if found.contains(&(message.conversation.clone(), message.message.clone())) {
                        let changed = tx.execute(
                            "UPDATE conversation_messages SET state='sent',failure=NULL,retry_at=NULL,send_unknown_at=NULL
                             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
                               AND state IN ('pending','sent','failed')
                               AND (state='sent' OR attempted_at IS NOT NULL) AND superseded_at IS NULL",
                            params![c.company.as_str(), c.connector, message.conversation, message.message],
                        ).map_err(state::sql(&path))?;
                        if changed != 0 {
                            remember_delivery(tx, c, &message.conversation, &message.message)?;
                        }
                    }
                }
            }
            tx.execute(
                "UPDATE connected_conversations SET position_json=?4 WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![c.company.as_str(), c.connector, c.conversation, serde_json::to_string(&batch.position).map_err(|e| state::invalid(&path, e))?],
            )
            .map_err(state::sql(&path))?;
            Ok(())
        })?;
        if !batch.has_more {
            return Ok(presence);
        }
        read_from = Some(batch.position);
    }
}

/// The answer pieces this process saw the connector accept whose acceptance
/// it has not yet recorded. Such a piece is never posted again: only its
/// record is retried, on every pass, until it is written.
#[derive(Debug, Default)]
pub struct Accepted {
    receipts: std::sync::Mutex<
        std::collections::BTreeMap<AcceptedPiece, instrukt_connector::AnswerReceipt>,
    >,
}

/// Per-conversation native work. A check takes no dispatch or conversation
/// lock. Once cleared to dispatch, only that conversation's worker can send
/// or start a turn until it returns.
struct RelayWorkers {
    connectors: Arc<dyn Connectors>,
    wake: SyncSender<()>,
    probes: Mutex<BTreeMap<String, MissingProbe>>,
    absence_reads: Mutex<BTreeMap<String, AbsenceRead>>,
    work: Mutex<BTreeMap<String, DeliveryWork>>,
    answer_anomalies: Mutex<BTreeSet<String>>,
    env: Env,
}

#[derive(Clone)]
struct DeliveryCandidate {
    session: SessionRef,
    conversation: String,
    message: String,
    head_conversation: Option<String>,
    head: Option<String>,
    boundary: Option<String>,
    attempted: Option<Timestamp>,
    source_presence: Presence,
    source_identity: Option<String>,
    invalidated: bool,
}

enum DeliveryWork {
    Checking {
        candidate: Box<DeliveryCandidate>,
        reply: Receiver<Result<DeliveryCheck, SessionsError>>,
    },
    Dispatching(Receiver<Result<(), ConversationError>>),
}

enum DeliveryCheck {
    Accepted,
    Presence(Presence),
    Ready,
    Busy,
    RetrySend,
    RejectedSend,
    UnconfirmedSend,
}

enum CheckedDelivery {
    Idle,
    Checking(Box<DeliveryCandidate>),
    Checked {
        candidate: Box<DeliveryCandidate>,
        result: Result<DeliveryCheck, Box<SessionsError>>,
    },
    Dispatching,
    DispatchFailed,
    /// The dispatch found the bound thread idle in another runtime project.
    Misplaced,
}

impl RelayWorkers {
    fn report_answer_failure(
        &self,
        admitted: &Admitted,
        state: &RunnerState,
        c: &Conversation,
        unread: bool,
        now: Timestamp,
    ) -> Result<(), StateError> {
        let cutoff = (now - ANSWER_OPERATOR_BOUND).to_string();
        let due = state.read(|conn, path| match c.session_scope {
            SessionScope::Conversation => conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3
                    AND superseded_at IS NULL AND (
                        (state='failed' AND failed_turns>=2)
                        OR ((state='sent' AND ?4)
                            AND julianday(unavailable_since)<=julianday(?5))
                    )
                )",
                params![c.company.as_str(), c.connector, c.conversation, unread, cutoff],
                |row| row.get::<_, bool>(0),
            ),
            SessionScope::Company => conn.query_row(
                &format!(
                    "SELECT EXISTS(
                        SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2
                        AND superseded_at IS NULL AND {COMPANY_OWNED_MESSAGES} AND (
                            (state='failed' AND failed_turns>=2)
                            OR ((state='sent' AND ?3)
                                AND julianday(unavailable_since)<=julianday(?4))
                        )
                    )"
                ),
                params![c.company.as_str(), c.connector, unread, cutoff],
                |row| row.get::<_, bool>(0),
            ),
        }.map_err(state::sql(path)))?.unwrap_or(false);
        if due
            && self
                .answer_anomalies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(c.semantic_key.clone())
        {
            let reference = c
                .session
                .as_ref()
                .map_or_else(|| c.semantic_key.clone(), ToString::to_string);
            let config = admitted.global().clone();
            let env = self.env.clone();
            drop(
                self.spawn(move || {
                    instrukt_runner::anomaly::notify(&config, &env, &reference, None)
                }),
            );
        }
        Ok(())
    }

    fn poll(&self, c: &Conversation) -> CheckedDelivery {
        let mut work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        let key = &c.semantic_key;
        match work.get(key) {
            Some(DeliveryWork::Checking { candidate, reply }) => {
                let result = match reply.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => {
                        return CheckedDelivery::Checking(candidate.clone());
                    }
                    Err(TryRecvError::Disconnected) => {
                        work.remove(key);
                        return CheckedDelivery::Idle;
                    }
                };
                let check = CheckedDelivery::Checked {
                    candidate: candidate.clone(),
                    result: result.map_err(Box::new),
                };
                work.remove(key);
                check
            }
            Some(DeliveryWork::Dispatching(reply)) => {
                let result = match reply.try_recv() {
                    Err(TryRecvError::Empty) => return CheckedDelivery::Dispatching,
                    Ok(Ok(())) => CheckedDelivery::Idle,
                    Ok(Err(ConversationError::Sessions(SessionsError::ThreadMisplaced {
                        idle: true,
                        ..
                    }))) => CheckedDelivery::Misplaced,
                    _ => CheckedDelivery::DispatchFailed,
                };
                work.remove(key);
                result
            }
            None => CheckedDelivery::Idle,
        }
    }

    fn update_check_source(&self, c: &Conversation, observed: Option<Presence>) {
        if let Some(DeliveryWork::Checking { candidate, .. }) = self
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&c.semantic_key)
        {
            candidate.invalidated |= c.source_identity != candidate.source_identity
                || c.session.as_ref() != Some(&candidate.session)
                || observed != Some(candidate.source_presence);
        }
    }

    // The check carries every fact its worker thread needs, read by the
    // pass that starts it; grouping them would only rename the list.
    #[allow(clippy::too_many_arguments)]
    fn check(
        &self,
        adapter: &dyn RuntimeAdapter,
        c: &Conversation,
        session: &SessionRef,
        next: &Message,
        head: Option<&Message>,
        source_presence: Presence,
        checkout: std::path::PathBuf,
    ) {
        let runtime = adapter.runtime();
        let key = c.semantic_key.clone();
        let target = session.clone();
        let attempted = next.attempted;
        let command_id = next.dispatch_id.clone();
        let input = next.content.clone();
        let reply = self.spawn(move || {
            let adapter = instrukt_runner::runtime::adapter(runtime);
            if attempted.is_some() {
                let Some(input) = input else {
                    return Ok(DeliveryCheck::UnconfirmedSend);
                };
                return Ok(
                    match adapter.observe_send(&target, &input, command_id.as_deref())? {
                        SendEvidence::Accepted => DeliveryCheck::Accepted,
                        SendEvidence::Rejected => DeliveryCheck::RejectedSend,
                        SendEvidence::Presence(presence) => DeliveryCheck::Presence(presence),
                        SendEvidence::IdleWithoutInput if command_id.is_some() => {
                            DeliveryCheck::RetrySend
                        }
                        SendEvidence::IdleWithoutInput => DeliveryCheck::UnconfirmedSend,
                        SendEvidence::Unresolved => DeliveryCheck::UnconfirmedSend,
                    },
                );
            }
            let thread = adapter.read_state(&target)?;
            if thread.presence != Presence::Present {
                return Ok(DeliveryCheck::Presence(thread.presence));
            }
            // A thread in another runtime project is never served, busy or
            // not: its outcome says whether it can roll over now.
            adapter.verify_thread_project(&checkout, &thread, Some(&key))?;
            Ok(
                if matches!(
                    thread.state,
                    SessionState::InputReady | SessionState::EndedFailed
                ) {
                    DeliveryCheck::Ready
                } else {
                    DeliveryCheck::Busy
                },
            )
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                c.semantic_key.clone(),
                DeliveryWork::Checking {
                    candidate: Box::new(DeliveryCandidate {
                        session: session.clone(),
                        conversation: next.conversation.clone(),
                        message: next.message.clone(),
                        head_conversation: head.map(|head| head.conversation.clone()),
                        head: head.map(|head| head.message.clone()),
                        boundary: next.boundary.clone(),
                        attempted: next.attempted,
                        source_presence,
                        source_identity: c.source_identity.clone(),
                        invalidated: false,
                    }),
                    reply,
                },
            );
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        home: &Home,
        admitted: &Admitted,
        adapter: &dyn RuntimeAdapter,
        source: &dyn ConversationSource,
        c: Conversation,
        conversation: String,
        message: String,
    ) {
        let opening = if c.session.is_none() && c.request.is_none() {
            opening_position(source).map(|position| {
                Some(OpeningSource {
                    identity: source.identity().to_owned(),
                    position,
                })
            })
        } else {
            Ok(None)
        };
        let identity = source.identity().to_owned();
        let env = self.env.clone();
        let key = c.semantic_key.clone();
        let home = home.clone();
        let admitted = admitted.clone();
        let runtime = adapter.runtime();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || {
            let state = RunnerState::new(&home);
            let ready = if c.session.is_some() {
                RuntimeInvocation::try_acquire(&home)
            } else {
                Ok(None)
            };
            let invocation = match ready.and_then(|ready| match ready {
                Some(invocation) => Ok(Some(invocation)),
                None => RuntimeInvocation::acquire(&home, LOCK_WAIT).map(|invocation| {
                    // A bound thread needs a fresh readiness check after
                    // waiting for dispatch. Do not hold this lock across it.
                    c.session.is_none().then_some(invocation)
                }),
            }) {
                Ok(Some(invocation)) => invocation,
                Ok(None) => return Ok(()),
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "conversation_dispatch_unavailable", error = %error);
                    unavailable(&state, &c, &[(conversation, message)], connectors.now())?;
                    return Err(error.into());
                }
            };
            let _lock = match lock(&home, &c) {
                Ok(lock) => lock,
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "conversation_lock_unavailable", company = %c.company, connector = %c.connector, key = %c.semantic_key, %error);
                    unavailable(&state, &c, &[(conversation, message)], connectors.now())?;
                    return Err(error);
                }
            };
            let Some(current) = reread(&state, &c)?.filter(|current| current.session == c.session)
            else {
                return Ok(());
            };
            let adapter = instrukt_runner::runtime::adapter(runtime);
            let clock = || connectors.now();
            let delivery = NativeDelivery { home: &home, identity: &identity, env: &env, clock: &clock };
            if let Err(error) = delivery.check_source(adapter.as_ref(), &state, &current) {
                set_failure(&state, &current, Some(&error.to_string()))?;
                return Err(error);
            }
            let _key = current.session.as_ref().map(|_| {
                keyed::KeyGuard::claim(
                    &invocation,
                    adapter.as_ref(),
                    &current.semantic_key,
                    None,
                    connectors.now(),
                )
            }).transpose()?;
            if let Some(session) = &current.session
                && keyed::replacement_binding(&home, adapter.as_ref(), &current.semantic_key, session)?
                    .is_some()
            {
                return Ok(());
            }
            // Intake can change the head while delivery waits for the lock.
            let open = open_messages(&state, &current)?;
            let Some(next) = next_message(&open, &current, connectors.now())
                .filter(|next| next.conversation == conversation && next.message == message)
            else {
                return Ok(());
            };
            deliver_one(
                &invocation,
                &admitted,
                adapter.as_ref(),
                opening,
                &current,
                next,
                &delivery,
            )
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    fn succeed(&self, home: &Home, adapter: &dyn RuntimeAdapter, c: Conversation) {
        let home = home.clone();
        let runtime = adapter.runtime();
        let key = c.semantic_key.clone();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || -> Result<(), ConversationError> {
            let _lock = lock(&home, &c)?;
            let state = RunnerState::new(&home);
            let Some(current) = reread(&state, &c)?.filter(|current| current.session == c.session)
            else {
                return Ok(());
            };
            let invocation = RuntimeInvocation::acquire(&home, LOCK_WAIT)?;
            let adapter = instrukt_runner::runtime::adapter(runtime);
            let title=state.read(|conn,path|conn.query_row("SELECT display_title FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3",params![c.company.as_str(),c.connector,c.conversation],|row|row.get::<_,Option<String>>(0)).map_err(state::sql(path)))?.flatten().unwrap_or_else(||conversation_title(c.company.as_str(),None,""));
            let mut request=match current.request.as_deref() {Some(raw)=>serde_json::from_str::<RuntimeStart>(raw)?,None=>keyed::request(&home,adapter.as_ref(),&current.semantic_key)?};
            let original=request.clone();
            request.title=title;
            let session=current.session.as_ref().ok_or_else(||ConversationError::Refused("successor has no owner-retained native ID".into()))?;
            if let Err(error)=keyed::succeed(adapter.as_ref(),&instrukt_runner::runtime::FollowupAssignment{bindings:None,original_request:Some(&original),invocation:&invocation,request:&request,session,worktree_path:None,project:None,now:connectors.now()}) {
                set_failure(&state, &current, Some(&error.to_string()))?;
                let waiting = open_messages(&state, &current)?
                    .into_iter()
                    .map(|message| (message.conversation, message.message))
                    .collect::<Vec<_>>();
                unavailable(&state, &current, &waiting, connectors.now())?;
                return Err(error.into());
            }
            Ok(())
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    fn spawn<T: Send + 'static>(&self, run: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
        let (send, reply) = mpsc::channel();
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new().spawn(move || {
            let _ = send.send(run());
            let _ = wake.try_send(());
        });
        reply
    }
}

struct AbsenceRead {
    session: SessionRef,
    reply: Receiver<Option<Presence>>,
    delay_secs: i64,
    invalidated: bool,
}

enum MissingProbe {
    Backoff {
        retry_at: Timestamp,
        delay_secs: i64,
    },
    Confirmed(Presence),
    LiveUntil(Timestamp),
}

impl RelayWorkers {
    // A projection change can invalidate the result, but only the worker's
    // completion releases the conversation's slot for another native read.
    fn absence_in_flight(
        &self,
        c: &Conversation,
        missing: Option<&SessionRef>,
        now: Timestamp,
    ) -> bool {
        let mut reads = self
            .absence_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(read) = reads.get_mut(&c.semantic_key) else {
            return false;
        };
        read.invalidated |= Some(&read.session) != missing;
        let observation = match read.reply.try_recv() {
            Ok(observation) => observation,
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) => return true,
        };
        if !read.invalidated {
            let state = if let Some(presence) = observation {
                if presence != Presence::Present {
                    tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_confirmed", session = %read.session, ?presence);
                }
                Self::observation(presence, now)
            } else {
                let session = &read.session;
                let retry_at = now + jiff::SignedDuration::from_secs(read.delay_secs);
                tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_retry", %session, %retry_at);
                MissingProbe::Backoff {
                    retry_at,
                    delay_secs: (read.delay_secs * 2).min(30),
                }
            };
            self.probes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(read.session.to_string(), state);
        }
        reads.remove(&c.semantic_key);
        false
    }

    fn confirm_missing(
        &self,
        adapter: &dyn RuntimeAdapter,
        c: &Conversation,
        session: &SessionRef,
        now: Timestamp,
    ) -> Option<Presence> {
        if self.absence_in_flight(c, Some(session), now) {
            return None;
        }
        let id = session.to_string();
        let delay_secs = match self
            .probes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            Some(MissingProbe::Confirmed(presence)) => return Some(*presence),
            Some(MissingProbe::LiveUntil(retry_at)) if now < *retry_at => {
                return Some(Presence::Present);
            }
            Some(MissingProbe::Backoff {
                retry_at,
                delay_secs,
            }) if now >= *retry_at => *delay_secs,
            Some(MissingProbe::Backoff { .. }) => return None,
            _ => 2,
        };
        let runtime = adapter.runtime();
        let session = session.clone();
        let target = session.clone();
        tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_started", %session);
        let reply = self.spawn(move || {
            instrukt_runner::runtime::adapter(runtime)
                .read_state(&target)
                .ok()
                .map(|thread| thread.presence)
        });
        self.absence_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                c.semantic_key.clone(),
                AbsenceRead {
                    session,
                    reply,
                    delay_secs,
                    invalidated: false,
                },
            );
        None
    }

    fn observation(presence: Presence, now: Timestamp) -> MissingProbe {
        match presence {
            // A live CLI thread can disappear while the projection is
            // still missing. Refresh this result if no delivery checks it.
            Presence::Present => MissingProbe::LiveUntil(now + jiff::SignedDuration::from_secs(2)),
            _ => MissingProbe::Confirmed(presence),
        }
    }

    fn confirmed_presence(&self, session: &SessionRef, presence: Presence, now: Timestamp) {
        self.probes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session.to_string(), Self::observation(presence, now));
    }

    fn source_present(&self, session: &SessionRef) {
        self.probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&session.to_string());
    }
}

type AcceptedPiece = (String, String, String, String, i64);

impl Accepted {
    /// Once the provider confirmed a post, its recipient cannot be owed a
    /// notice while the acceptance still waits to be recorded.
    fn turns(&self, company: &ProjectSlug, connector: &str, conversation: &str) -> Vec<String> {
        self.receipts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .filter(|(c, k, place, _, _)| {
                c == company.as_str() && k == connector && place == conversation
            })
            .map(|(_, _, _, turn, _)| turn.clone())
            .collect()
    }

    fn key(c: &Conversation, conversation: &str, turn: &str, piece: i64) -> AcceptedPiece {
        (
            c.company.to_string(),
            c.connector.clone(),
            conversation.to_owned(),
            turn.to_owned(),
            piece,
        )
    }
    fn at(
        &self,
        c: &Conversation,
        conversation: &str,
        turn: &str,
        piece: i64,
    ) -> Option<instrukt_connector::AnswerReceipt> {
        self.receipts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&Self::key(c, conversation, turn, piece))
            .cloned()
    }
    fn keep(
        &self,
        c: &Conversation,
        conversation: &str,
        turn: &str,
        piece: i64,
        receipt: instrukt_connector::AnswerReceipt,
    ) {
        self.receipts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(Self::key(c, conversation, turn, piece), receipt);
    }
    fn release(&self, c: &Conversation, conversation: &str, turn: &str, piece: i64) {
        self.receipts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&Self::key(c, conversation, turn, piece));
    }
}

/// One pending answer piece, kept in delivery order.
struct Pending {
    conversation: String,
    turn: String,
    piece: i64,
    text: String,
}

/// Retry retained pieces in order; a success known by this process retries only its record.
fn post(
    admitted: &Admitted,
    state: &RunnerState,
    connectors: &dyn Connectors,
    accepted: &Accepted,
    c: &Conversation,
) -> Result<(), ConversationError> {
    let pending = state.read(|conn, path| match c.session_scope {
        SessionScope::Conversation => {
            let mut query = conn.prepare("SELECT conversation,turn,piece,text FROM conversation_answers WHERE company=?1 AND connector=?2 AND conversation=?3 AND posted_at IS NULL ORDER BY rowid")
                .map_err(state::sql(path))?;
            query.query_map(params![c.company.as_str(), c.connector, c.conversation], |row| Ok(Pending {
                conversation: row.get(0)?, turn: row.get(1)?, piece: row.get(2)?, text: row.get(3)?,
            })).map_err(state::sql(path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
        }
        SessionScope::Company => {
            let mut query = conn
                .prepare(&format!(
                    "SELECT conversation,turn,piece,text FROM conversation_answers WHERE company=?1 AND connector=?2 AND posted_at IS NULL AND {COMPANY_OWNED_ANSWERS} ORDER BY rowid"
                ))
                .map_err(state::sql(path))?;
            query.query_map(params![c.company.as_str(), c.connector], |row| Ok(Pending {
                conversation: row.get(0)?, turn: row.get(1)?, piece: row.get(2)?, text: row.get(3)?,
            })).map_err(state::sql(path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
        }
    })?.unwrap_or_default();
    if pending.is_empty() {
        return Ok(());
    }
    let connector = connectors.open(admitted, &c.company, &c.connector);
    let mut failed_conversations: BTreeSet<String> = BTreeSet::new();
    let mut first_error: Option<ConversationError> = None;
    for piece in &pending {
        if failed_conversations.contains(&piece.conversation) {
            continue;
        }
        let receipt = match accepted.at(c, &piece.conversation, &piece.turn, piece.piece) {
            Some(receipt) => receipt,
            None => {
                let connector = match &connector {
                    Ok(connector) => connector.as_ref(),
                    Err(error) => {
                        piece_failed(state, c, piece, &error.cause)?;
                        return Err(error.clone().into());
                    }
                };
                let claimed = state.write::<_, StateError>(|tx| {
                    tx.execute("UPDATE conversation_answers SET attempted_at=?6 WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5 AND posted_at IS NULL",
                        params![c.company.as_str(), c.connector, piece.conversation, piece.turn, piece.piece, connectors.now().to_string()])
                        .map_err(state::sql(&state::db_path(tx)))
                })?;
                if claimed == 0 {
                    continue;
                }
                let mut receipt = match connector.answer(&piece.conversation, &piece.text) {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        piece_failed(state, c, piece, &error.cause)?;
                        failed_conversations.insert(piece.conversation.clone());
                        first_error.get_or_insert_with(|| {
                            ConversationError::Refused(format!(
                                "post failed and stays pending: {}",
                                error.cause
                            ))
                        });
                        continue;
                    }
                };
                receipt.posted_at.get_or_insert_with(|| connectors.now());
                accepted.keep(
                    c,
                    &piece.conversation,
                    &piece.turn,
                    piece.piece,
                    receipt.clone(),
                );
                receipt
            }
        };
        posted(state, c, piece, &receipt, connectors.now())?;
        accepted.release(c, &piece.conversation, &piece.turn, piece.piece);
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

fn close_completed_interval(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    company: &str,
    connector: &str,
    conversation: &str,
    turn: &str,
) -> Result<(), StateError> {
    tx.execute(
        &format!("UPDATE conversation_messages SET unavailable_since=NULL,turn_update=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND state=?5 AND (unavailable_since IS NOT NULL OR turn_update IS NOT NULL) AND NOT EXISTS({UNPOSTED})"),
        params![company, connector, conversation, turn, Stage::Answered.as_str()],
    ).map_err(state::sql(&state::db_path(tx)))?;
    Ok(())
}

/// Record provider acceptance; the final piece and its no-work interval settle atomically.
fn posted(
    state: &RunnerState,
    c: &Conversation,
    piece: &Pending,
    receipt: &instrukt_connector::AnswerReceipt,
    now: Timestamp,
) -> Result<(), ConversationError> {
    record_receipt(
        state,
        &Accepted::key(c, &piece.conversation, &piece.turn, piece.piece),
        receipt,
        now,
    )
}

fn record_receipt(
    state: &RunnerState,
    identity: &AcceptedPiece,
    receipt: &instrukt_connector::AnswerReceipt,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let (company, connector, conversation, turn, piece) = identity;
    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let changed = tx.execute(
            "UPDATE conversation_answers SET posted_at=?6,provider_post_id=?7,failure=NULL,attempted_at=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5 AND posted_at IS NULL",
            params![company, connector, conversation, turn, piece, receipt.posted_at.unwrap_or(now).to_string(), receipt.message_id],
        )
        .map_err(state::sql(&path))?;
        if changed == 0 {
            // An already recorded receipt must not close a newer outage.
            return Ok(());
        }
        close_completed_interval(tx, company, connector, conversation, turn)?;
        settle_failed_messages(tx, company, connector, conversation, now)?;
        Ok(())
    })?;
    Ok(())
}

/// A later settled turn supersedes earlier failures of the same sender.
fn settle_failed_messages(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    company: &str,
    connector: &str,
    conversation: &str,
    now: Timestamp,
) -> Result<(), StateError> {
    tx.execute(
        &format!(
            "UPDATE conversation_messages AS failed
             SET superseded_at=?4,unavailable_since=NULL,turn_update=NULL,retry_at=NULL
             WHERE failed.company=?1 AND failed.connector=?2 AND failed.conversation=?3
               AND failed.state='failed' AND failed.superseded_at IS NULL AND failed.send_unknown_at IS NULL
               AND EXISTS(SELECT 1 FROM conversation_messages
                 WHERE company=failed.company AND connector=failed.connector
                   AND conversation=failed.conversation AND sender=failed.sender
                   AND admission_position>failed.admission_position
                   AND state='answered' AND turn IS NOT NULL AND NOT EXISTS({UNPOSTED}))"
        ),
        params![company, connector, conversation, format!("{now:.9}")],
    )
    .map_err(state::sql(&state::db_path(tx)))?;
    retire_notices(tx, company, connector, conversation)?;
    Ok(())
}

/// A recovered no-work interval rearms the person's next outage notice.
fn retire_notices(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    company: &str,
    connector: &str,
    conversation: &str,
) -> Result<usize, StateError> {
    let path = state::db_path(tx);
    let company_id = ProjectSlug::parse(company).map_err(|error| state::invalid(&path, error))?;
    let active = read_unfinished(tx, &path, &company_id, connector)?
        .into_iter()
        .filter(|message| message.conversation == conversation && message.since.is_some())
        .filter_map(|message| message.sender)
        .collect::<BTreeSet<_>>();
    let senders = {
        let mut query = tx.prepare("SELECT sender FROM conversation_refusals WHERE company=?1 AND connector=?2 AND conversation=?3 AND reason='unavailable'")
            .map_err(state::sql(&path))?;
        query
            .query_map(params![company, connector, conversation], |row| {
                row.get::<_, String>(0)
            })
            .map_err(state::sql(&path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(&path))?
    };
    let mut retired = 0;
    for sender in senders
        .into_iter()
        .filter(|sender| !active.contains(sender))
    {
        retired += tx.execute("DELETE FROM conversation_refusals WHERE company=?1 AND connector=?2 AND conversation=?3 AND sender=?4 AND reason='unavailable'",
            params![company, connector, conversation, sender]).map_err(state::sql(&path))?;
    }
    Ok(retired)
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
            params![c.company.as_str(), c.connector, piece.conversation, piece.turn, piece.piece, cause],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
}

/// How long the relay waits for a conversation's lock or the runtime's
/// dispatch lock before it leaves that work for its next pass.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

fn adopt_replacement(
    home: &Home,
    state: &RunnerState,
    adapter: &dyn RuntimeAdapter,
    source: &dyn ConversationSource,
    c: &mut Conversation,
) -> Result<Option<SessionRef>, ConversationError> {
    let Some(current) = &c.session else {
        return Ok(None);
    };
    if let Some((successor, ready)) =
        keyed::replacement_binding(home, adapter, &c.semantic_key, current)?
    {
        if !ready {
            return Ok(Some(successor));
        }
        let position = opening_position(source)?;
        let serialized = serde_json::to_string(&position)?;
        state.write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            tx.execute(
                "UPDATE connected_conversations SET session=?4,source_identity=?5,position_json=?6 WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![c.company.as_str(), c.connector, c.conversation, successor.to_string(), source.identity(), serialized],
            ).map_err(state::sql(&path))?;
            match c.session_scope {
                SessionScope::Conversation => {
                    tx.execute(
                        "UPDATE conversation_messages SET state=?4,attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,turn_update=NULL,failure=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND state IN (?4,?5) AND send_unknown_at IS NULL",
                        params![c.company.as_str(), c.connector, c.conversation, Stage::Pending.as_str(), Stage::Sent.as_str()],
                    ).map_err(state::sql(&path))?;
                    tx.execute(
                        "UPDATE conversation_messages SET attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND state='failed' AND superseded_at IS NULL AND send_unknown_at IS NULL",
                        params![c.company.as_str(), c.connector, c.conversation],
                    ).map_err(state::sql(&path))?;
                }
                SessionScope::Company => {
                    tx.execute(
                        &format!("UPDATE conversation_messages SET state=?3,attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,turn_update=NULL,failure=NULL WHERE company=?1 AND connector=?2 AND state IN (?3,?4) AND send_unknown_at IS NULL AND {COMPANY_OWNED_MESSAGES}"),
                        params![c.company.as_str(), c.connector, Stage::Pending.as_str(), Stage::Sent.as_str()],
                    ).map_err(state::sql(&path))?;
                    tx.execute(
                        &format!("UPDATE conversation_messages SET attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL WHERE company=?1 AND connector=?2 AND state='failed' AND superseded_at IS NULL AND send_unknown_at IS NULL AND {COMPANY_OWNED_MESSAGES}"),
                        params![c.company.as_str(), c.connector],
                    ).map_err(state::sql(&path))?;
                }
            }
            Ok(())
        })?;
        c.session = Some(successor);
        c.source_identity = Some(source.identity().to_owned());
        c.position = Some(position);
    }
    Ok(None)
}

fn blocked(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    session: &SessionRef,
    presence: Option<Presence>,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let finding = match presence {
        Some(presence) => {
            keyed::presence_finding(home, adapter, &c.semantic_key, session, presence)?
        }
        None => None,
    };
    let failure = finding.map_or_else(
        || {
            format!(
                "the source cannot establish presence for thread {session} under `{}`",
                c.semantic_key
            )
        },
        |finding| finding.to_string(),
    );
    set_failure(state, c, Some(&failure))?;
    let waiting = open_messages(state, c)?
        .into_iter()
        .map(|message| (message.conversation, message.message))
        .collect::<Vec<_>>();
    unavailable(state, c, &waiting, now)?;
    Ok(())
}

/// The connector destination of the finished-turn relay `instrukt run`
/// already hosts: for every conversation, read its finished turns into
/// answers, post what is retained, observe the turn its session runs, then
/// deliver its next addressed message. A conversation whose lock is not had
/// within its wait is left for the next pass, as is delivery while the
/// runtime's dispatch lock is not had; the messages that waited on either
/// have their no-work interval opened.
fn relay(
    home: &Home,
    admitted: &Admitted,
    source: &dyn ConversationSource,
    adapter: &dyn RuntimeAdapter,
    workers: &RelayWorkers,
    receipts: &Accepted,
    pass: CompanyRelayPass,
) -> Result<(), ConversationError> {
    let state = RunnerState::new(home);
    let connectors = workers.connectors.as_ref();
    let clock = || connectors.now();
    prune_superseded_failures(&state, clock())?;
    let work = match pass {
        CompanyRelayPass::CatchUp => conversations(&state, "", [])?,
        CompanyRelayPass::Retry => conversations(
            &state,
            &format!("WHERE {}", relay_work_filter()),
            relay_stages(),
        )?,
    };
    for initial in work {
        if !connectors.ready(admitted, &initial.company, &initial.connector) {
            continue;
        }
        let check = workers.poll(&initial);
        if matches!(check, CheckedDelivery::Dispatching) {
            // The delivery owns the conversation record until it returns.
            // Posting retained answers is independent of the native delivery.
            if let Err(error) = require_source(&initial, source.identity()) {
                set_failure(&state, &initial, Some(&error.to_string()))?;
            }
            unconfirmed(&state, &initial, clock())?;
            if let Err(error) = post(admitted, &state, connectors, receipts, &initial) {
                tracing::warn!(target: LOG_TARGET, event = "conversation_answer_unposted", company = %initial.company, connector = %initial.connector, key = %initial.semantic_key, %error);
            }
            unconfirmed(&state, &initial, clock())?;
            continue;
        }
        let _lock = match lock(home, &initial) {
            Ok(lock) => lock,
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "conversation_lock_unavailable", company = %initial.company, connector = %initial.connector, key = %initial.semantic_key, error = %error);
                let waited = open_messages(&state, &initial)?
                    .into_iter()
                    .filter(|m| matches!(m.stage, Stage::Pending | Stage::Sent))
                    .map(|m| (m.conversation, m.message))
                    .collect::<Vec<_>>();
                unavailable(&state, &initial, &waited, connectors.now())?;
                continue;
            }
        };
        let Some(mut c) = reread(&state, &initial)? else {
            continue;
        };
        // Intake runs alongside source reads. Only work already captured
        // when the read began may use the presence it observed.
        let before_read = open_messages(&state, &c)?;
        let observed = answers(admitted, &state, source, connectors, receipts, &c)?;
        if let Err(error) = require_source(&c, source.identity()) {
            if !open_messages(&state, &c)?.is_empty() {
                adopt_replacement(home, &state, adapter, source, &mut c)?;
            }
            if require_source(&c, source.identity()).is_err() {
                set_failure(&state, &c, Some(&error.to_string()))?;
                let waiting = open_messages(&state, &c)?
                    .into_iter()
                    .map(|message| (message.conversation, message.message))
                    .collect::<Vec<_>>();
                unavailable(&state, &c, &waiting, clock())?;
            }
            workers.update_check_source(&c, None);
            continue;
        }
        if before_read.iter().any(|message| {
            message.stage == Stage::Failed
                && !message.send_unknown
                && message.retry_at.is_none()
                && message.boundary.is_some()
        }) {
            schedule_earlier_failures(&state, &c, clock())?;
        }
        let answer_may_need_attention = before_read.iter().any(|message| match message.stage {
            Stage::Failed => message.failed_turns > 0,
            Stage::Sent => message.failed_turns > 0 || observed.is_none(),
            _ => false,
        });
        if answer_may_need_attention {
            workers.report_answer_failure(admitted, &state, &c, observed.is_none(), clock())?;
        }
        if let Some(session) = &c.session
            && matches!(observed, Some(Presence::Present | Presence::Archived))
        {
            workers.source_present(session);
        }
        let original_binding = c.session.clone();
        let replacement = if open_messages(&state, &c)?.is_empty() {
            None
        } else {
            adopt_replacement(home, &state, adapter, source, &mut c)?
        };
        if (replacement.is_some() || c.session != original_binding)
            && let CheckedDelivery::Checked {
                candidate,
                result: Ok(DeliveryCheck::Accepted),
            } = &check
        {
            stale_acknowledgement(&c, &candidate.message);
        }
        if replacement.is_some() {
            workers.update_check_source(&c, None);
            workers.succeed(home, adapter, c.clone());
            continue;
        }
        // The batch belonged to the old binding. Read the successor on
        // the next pass before using its presence for dispatch.
        if c.session != original_binding {
            if let Some(original) = &original_binding {
                workers.source_present(original);
            }
            workers.update_check_source(&c, None);
            workers.absence_in_flight(&c, None, clock());
            continue;
        }
        let check = match check {
            CheckedDelivery::Checked {
                candidate,
                result: Ok(DeliveryCheck::Accepted),
            } => {
                if candidate.invalidated
                    || candidate.source_identity != c.source_identity
                    || c.session.as_ref() != Some(&candidate.session)
                {
                    stale_acknowledgement(&c, &candidate.message);
                } else if accepted(
                    &state,
                    &c,
                    &candidate.conversation,
                    &candidate.message,
                    candidate.boundary.as_deref(),
                    None,
                    source.identity(),
                )? {
                    continue;
                }
                CheckedDelivery::Idle
            }
            other => other,
        };
        let Some(observed) = observed else {
            // A source outage must not prolong an ambiguous send forever.
            // A known archived/deleted target retains its existing recovery path.
            if c.source_identity.as_deref() == Some(source.identity())
                && !matches!(
                    &check,
                    CheckedDelivery::Checked {
                        result: Ok(DeliveryCheck::Presence(
                            Presence::Absent | Presence::Archived
                        )),
                        ..
                    }
                )
            {
                let now = clock();
                for message in open_messages(&state, &c)?
                    .iter()
                    .filter(|m| send_reconciliation_due(m, now))
                {
                    send_unknown(
                        &state,
                        &c,
                        message,
                        now,
                        "the runtime source could not be read",
                    )?;
                }
            }
            continue;
        };
        let check = match check {
            CheckedDelivery::Checked { candidate, result }
                if c.session.as_ref() == Some(&candidate.session) =>
            {
                // A changed projection invalidates a conflicting result
                // from a check begun against the older projection.
                let checked_presence = match result {
                    Ok(DeliveryCheck::Presence(presence)) => Some(presence),
                    Ok(
                        DeliveryCheck::Ready
                        | DeliveryCheck::Busy
                        | DeliveryCheck::RetrySend
                        | DeliveryCheck::RejectedSend
                        | DeliveryCheck::UnconfirmedSend,
                    ) => Some(Presence::Present),
                    _ => None,
                };
                if candidate.invalidated
                    || candidate.source_identity != c.source_identity
                    || (observed != candidate.source_presence && checked_presence != Some(observed))
                {
                    None
                } else {
                    if let Some(presence) = checked_presence
                        && presence != Presence::Present
                    {
                        workers.confirmed_presence(&candidate.session, presence, clock());
                        blocked(
                            home,
                            adapter,
                            &state,
                            &c,
                            &candidate.session,
                            Some(presence),
                            clock(),
                        )?;
                        continue;
                    }
                    if observed == Presence::Absent && checked_presence == Some(Presence::Present) {
                        workers.confirmed_presence(&candidate.session, Presence::Present, clock());
                    }
                    Some((candidate, result))
                }
            }
            CheckedDelivery::Checking(candidate) => {
                workers.update_check_source(&c, Some(observed));
                if observed == Presence::Present
                    && candidate.source_presence == observed
                    && !candidate.invalidated
                    && candidate.source_identity == c.source_identity
                    && c.session.as_ref() == Some(&candidate.session)
                {
                    let now = clock();
                    if let Some(message) = open_messages(&state, &c)?.into_iter().find(|message| {
                        message.conversation == candidate.conversation
                            && message.message == candidate.message
                            && message.boundary == candidate.boundary
                            && message.attempted == candidate.attempted
                            && send_reconciliation_due(message, now)
                    }) {
                        send_unknown(
                            &state,
                            &c,
                            &message,
                            now,
                            "the runtime did not finish reading the input",
                        )?;
                    }
                }
                if let Some(session) = &c.session
                    && observed == Presence::Archived
                {
                    blocked(home, adapter, &state, &c, session, Some(observed), clock())?;
                }
                continue;
            }
            CheckedDelivery::DispatchFailed => continue,
            CheckedDelivery::Misplaced => {
                // Between check and send the thread went idle in another
                // project: its owner rolls it over.
                if !open_messages(&state, &c)?.is_empty() {
                    workers.succeed(home, adapter, c.clone());
                }
                continue;
            }
            _ => None,
        };
        if let Some(session) = &c.session {
            let presence = if observed == Presence::Absent {
                if open_messages(&state, &c)?.is_empty() {
                    workers.absence_in_flight(&c, None, clock());
                    continue;
                }
                let confirmed = workers.confirm_missing(adapter, &c, session, clock());
                if confirmed == Some(Presence::Present) {
                    set_failure(&state, &c, None)?;
                }
                confirmed
            } else {
                if workers.absence_in_flight(&c, None, clock()) && observed == Presence::Present {
                    continue;
                }
                Some(observed)
            };
            if presence != Some(Presence::Present) {
                if !open_messages(&state, &c)?.is_empty() {
                    if matches!(presence, Some(Presence::Archived | Presence::Absent)) {
                        workers.succeed(home, adapter, c.clone());
                    } else {
                        blocked(home, adapter, &state, &c, session, presence, clock())?;
                    }
                }
                continue;
            }
        }
        let open = open_messages(&state, &c)?;
        let mut checked_head: Option<(String, String)> = None;
        if let Some((candidate, result)) = check
            && let Some(next) = open.iter().find(|message| {
                matches!(message.stage, Stage::Pending | Stage::Failed)
                    && !message.send_unknown
                    && message.conversation == candidate.conversation
                    && message.message == candidate.message
                    && message.boundary == candidate.boundary
                    && message.attempted == candidate.attempted
            })
        {
            match result {
                Ok(DeliveryCheck::Ready) => {
                    checked_head = candidate.head_conversation.zip(candidate.head);
                }
                Ok(DeliveryCheck::RejectedSend) => {
                    if let Some(boundary) = next.boundary.as_deref() {
                        clear_rejected_send(
                            &state,
                            &c,
                            &next.conversation,
                            &next.message,
                            boundary,
                        )?;
                    }
                    continue;
                }
                Ok(DeliveryCheck::RetrySend) => {
                    if next_message(&open, &c, clock()).is_some_and(|head| {
                        head.conversation == next.conversation && head.message == next.message
                    }) {
                        workers.dispatch(
                            home,
                            admitted,
                            adapter,
                            source,
                            c,
                            next.conversation.clone(),
                            next.message.clone(),
                        );
                    }
                    continue;
                }
                Ok(DeliveryCheck::UnconfirmedSend) if send_reconciliation_due(next, clock()) => {
                    send_unknown(
                        &state,
                        &c,
                        next,
                        clock(),
                        "the runtime has other work or cannot confirm this input",
                    )?;
                    continue;
                }
                Ok(_) => continue,
                // An idle thread in another project is rolled over by the
                // conversation's succession.
                Err(error)
                    if matches!(*error, SessionsError::ThreadMisplaced { idle: true, .. }) =>
                {
                    if !open.is_empty() {
                        workers.succeed(home, adapter, c.clone());
                    }
                    continue;
                }
                // A busy one keeps the message pending with its operator
                // cause; it rolls over once its turn ends. The member hears
                // nothing beyond what any busy turn produces.
                Err(error)
                    if matches!(*error, SessionsError::ThreadMisplaced { idle: false, .. }) =>
                {
                    let cause = adapter
                        .diagnostic("conversation delivery", &error)
                        .to_string();
                    tracing::warn!(target: LOG_TARGET, event = "conversation_delivery_failed", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %cause);
                    set_message_failure(
                        &state,
                        &c,
                        &next.conversation,
                        &next.message,
                        Some(&cause),
                    )?;
                    continue;
                }
                Err(error) => {
                    if send_reconciliation_due(next, clock()) {
                        let cause = adapter
                            .diagnostic("conversation delivery", &error)
                            .to_string();
                        send_unknown(&state, &c, next, clock(), &cause)?;
                        continue;
                    }
                    record_delivery_failure(
                        adapter,
                        &state,
                        &c,
                        next,
                        &clock,
                        &Err((*error).into()),
                    )?;
                    continue;
                }
            }
        }
        observe(adapter, source, &state, &c, &clock)?;
        let open = open_messages(&state, &c)?;
        let before_read = |candidate: &Message| {
            before_read.iter().any(|message| {
                matches!(message.stage, Stage::Pending | Stage::Failed)
                    && message.conversation == candidate.conversation
                    && message.message == candidate.message
            })
        };
        let next = next_message(&open, &c, clock()).filter(|candidate| before_read(candidate));
        if let Some(session) = &c.session {
            // Accept any lost acknowledgment before trying to serve a
            // different queue head that arrived meanwhile.
            let unconfirmed = open.iter().find(|m| {
                matches!(m.stage, Stage::Pending | Stage::Failed)
                    && m.attempted.is_some()
                    && !m.send_unknown
                    && before_read(m)
            });
            if let Some(candidate) = unconfirmed {
                workers.check(
                    adapter,
                    &c,
                    session,
                    candidate,
                    next,
                    observed,
                    home.company_dir(&c.company),
                );
                continue;
            }
            let next_id = next.map(|m| (m.conversation.as_str(), m.message.as_str()));
            let checked_id = checked_head
                .as_ref()
                .map(|(conv, msg)| (conv.as_str(), msg.as_str()));
            if next_id != checked_id || checked_head.is_none() {
                if let Some(candidate) = next {
                    workers.check(
                        adapter,
                        &c,
                        session,
                        candidate,
                        next,
                        observed,
                        home.company_dir(&c.company),
                    );
                }
                continue;
            }
        }
        if let Some(next) = next {
            workers.dispatch(
                home,
                admitted,
                adapter,
                source,
                c,
                next.conversation.clone(),
                next.message.clone(),
            );
        }
    }
    clear_idle_answer_failures(&state, source.identity())?;
    Ok(())
}

/// Prune superseded failures and retain their receipts for another window.
fn prune_superseded_failures(state: &RunnerState, now: Timestamp) -> Result<(), StateError> {
    const RETENTION: jiff::SignedDuration = jiff::SignedDuration::from_hours(7 * 24);
    const BATCH: i64 = 100;
    let cutoff = format!("{:.9}", now - RETENTION);
    let expired = "SELECT rowid FROM conversation_messages
        WHERE state='failed' AND superseded_at IS NOT NULL AND superseded_at<?1
        ORDER BY superseded_at,rowid LIMIT ?2";
    let retired = "SELECT rowid FROM conversation_context_receipts
        WHERE retired_at<?1 ORDER BY retired_at,rowid LIMIT ?2";
    let due = state.read(|conn, path| {
        conn.query_row(
            &format!("SELECT EXISTS({expired}) OR EXISTS({retired})"),
            params![cutoff, BATCH],
            |row| row.get::<_, bool>(0),
        )
        .map_err(state::sql(path))
    })?;
    if !due.unwrap_or(false) {
        return Ok(());
    }
    state.write(|tx| {
        let path = state::db_path(tx);
        tx.execute(
            &format!("UPDATE conversation_context_receipts SET retired_at=?3
             WHERE (company,connector,conversation,delivery) IN
             (SELECT company,connector,conversation,message FROM conversation_messages WHERE rowid IN ({expired}))"),
            params![cutoff, BATCH, format!("{now:.9}")],
        ).map_err(state::sql(&path))?;
        tx.execute(&format!("DELETE FROM conversation_messages WHERE rowid IN ({expired})"), params![cutoff, BATCH])
            .map_err(state::sql(&path))?;
        tx.execute(&format!("DELETE FROM conversation_context_receipts WHERE rowid IN ({retired})"), params![cutoff, BATCH])
            .map_err(state::sql(&path))?;
        Ok(())
    })
}

/// A conversation without remaining answer work has no answer-read failure
/// to report. Clear it without locking the conversation or reading T3, even
/// when all other work is also finished.
fn clear_idle_answer_failures(
    state: &RunnerState,
    source_identity: &str,
) -> Result<(), StateError> {
    let idle_failure = format!(
        "failure IS NOT NULL AND (source_identity=?4 OR (session IS NULL AND position_json IS NULL AND request IS NULL AND opening IS NULL)) AND NOT {}",
        relay_work_filter()
    );
    let due = state.read(|conn, path| {
        conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM connected_conversations WHERE {idle_failure})"),
            params![
                relay_stages()[0],
                relay_stages()[1],
                relay_stages()[2],
                source_identity
            ],
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
            params![
                relay_stages()[0],
                relay_stages()[1],
                relay_stages()[2],
                source_identity
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// What the session's running turn shows of the messages handed to it. A
/// running turn shows the work going on for every message waiting behind it,
/// and for its own message, whose status then shows the newest message the
/// agent has written in it (under `turn_output`). Under `explicit`, raw turn
/// updates are never surfaced to the connector. A session whose turn cannot
/// be observed opens the no-work interval of what was handed to it; one
/// showing no running turn changes nothing.
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
            .map(|m| (m.conversation.clone(), m.message.clone()))
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
    let own_key = [(own.conversation.clone(), own.message.clone())];
    available(state, c, &own_key)?;
    if c.reply_mode == ReplyMode::Explicit {
        return Ok(());
    }
    match source.latest_update(&session.id) {
        Ok(update) => turn_update(state, c, &own.conversation, &own.message, update.as_deref())?,
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, event = "conversation_turn_unobserved", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
            unavailable(state, c, &own_key, clock())?;
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
) -> Result<Option<Presence>, ConversationError> {
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
            harvest(
                state,
                source,
                c,
                connector.limits().message_chars,
                c.reply_mode,
                &|| connectors.now(),
            )
        });
    if !matches!(read, Ok(Presence::Archived | Presence::Absent)) {
        set_failure(
            state,
            c,
            read.as_ref().err().map(ToString::to_string).as_deref(),
        )?;
    }
    let presence = match read {
        Ok(presence) => Some(presence),
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "conversation_answer_unread", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
            let sent = open_messages(state, c)?
                .into_iter()
                .filter(|m| m.stage == Stage::Sent)
                .map(|m| (m.conversation, m.message))
                .collect::<Vec<_>>();
            unavailable(state, c, &sent, connectors.now())?;
            None
        }
    };
    // Existing answers can post even when reading the next turn failed
    // or dispatch is waiting on session recovery.
    if let Err(error) = post(admitted, state, connectors, accepted, c) {
        tracing::warn!(target: LOG_TARGET, event = "conversation_answer_unposted", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %error);
    }
    unconfirmed(state, c, connectors.now())?;
    Ok(presence)
}

const UNPOSTED: &str = "SELECT 1 FROM conversation_answers x WHERE x.company=conversation_messages.company AND x.connector=conversation_messages.connector AND x.conversation=conversation_messages.conversation AND x.turn=conversation_messages.turn AND x.posted_at IS NULL";

/// A message whose answer is not posted although a post of it was tried —
/// it failed, or its outcome was lost, a restart included — has its interval
/// opened from its turn's end (`now` for a turn ended before that end was
/// kept). A post that was not tried yet opens nothing: the healthy path never
/// shows the unavailable text.
fn unconfirmed(state: &RunnerState, c: &Conversation, now: Timestamp) -> Result<(), StateError> {
    state.write(|tx| {
        match c.session_scope {
            SessionScope::Conversation => {
                tx.execute(
                    &format!("UPDATE conversation_messages SET unavailable_since=coalesce(turn_ended_at,?5) WHERE company=?1 AND connector=?2 AND conversation=?3 AND state=?4 AND unavailable_since IS NULL AND EXISTS({UNPOSTED} AND (x.attempted_at IS NOT NULL OR x.failure IS NOT NULL))"),
                    params![c.company.as_str(), c.connector, c.conversation, Stage::Answered.as_str(), now.to_string()],
                )
            }
            SessionScope::Company => {
                tx.execute(
                    &format!("UPDATE conversation_messages SET unavailable_since=coalesce(turn_ended_at,?4) WHERE company=?1 AND connector=?2 AND state=?3 AND unavailable_since IS NULL AND {COMPANY_OWNED_MESSAGES} AND EXISTS({UNPOSTED} AND (x.attempted_at IS NOT NULL OR x.failure IS NOT NULL))"),
                    params![c.company.as_str(), c.connector, Stage::Answered.as_str(), now.to_string()],
                )
            }
        }
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
                &format!("SELECT EXISTS({RELAY_WORK}) OR EXISTS(SELECT 1 FROM conversation_refusals WHERE reason IN ('not_own','not_admin') AND replied_at IS NULL)"),
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
        if failure.is_none() {
            let addresses = {
                let mut query = tx.prepare("SELECT DISTINCT company,connector,conversation FROM conversation_refusals WHERE reason='unavailable'")
                    .map_err(state::sql(&state::db_path(tx)))?;
                query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
                    .map_err(state::sql(&state::db_path(tx)))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(&state::db_path(tx)))?
            };
            for (company, connector, conversation) in addresses {
                retire_notices(tx, &company, &connector, &conversation)?;
            }
        }
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

fn status_at(home: &Home, now: Timestamp) -> Result<Status, ConversationError> {
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

/// A conversation session's assignment: the connector's configured role
/// (default `company-agent`), in its company's home; the record names no procedure.
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
            // In this order: the conversation's own session, a generation the
            // runner recorded, and the binding it writes with the fixed native
            // thread before the start, so a conversation session is
            // recognised from its first command. A session that is already
            // recorded is answered by its record, never by a binding alone.
            "SELECT company,semantic_key,checkout,role FROM (SELECT 0 AS rank,company,semantic_key,NULL AS checkout,role FROM connected_conversations WHERE session=?1 UNION ALL SELECT 1,c.company,c.semantic_key,g.checkout,c.role FROM runtime_generations g JOIN connected_conversations c ON c.semantic_key=g.semantic_key WHERE g.session=?1 AND g.generation>0 UNION ALL SELECT 2,c.company,c.semantic_key,b.checkout,c.role FROM runtime_bindings b JOIN connected_conversations c ON c.semantic_key=b.semantic_key WHERE b.runtime=?2 AND b.session=?1) ORDER BY rank LIMIT 1",
            instrukt_core::state_db::rusqlite::params![caller.to_string(), caller.runtime.as_str()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let Some((company, key, workspace, role)) = row else {
        return Ok(None);
    };
    let company = ProjectSlug::parse(&company)
        .map_err(|e| instrukt_core::state_db::StateDbError::invalid(db.path(), e.to_string()))?;
    let role = RoleName::parse(&role)
        .map_err(|e| instrukt_core::state_db::StateDbError::invalid(db.path(), e.to_string()))?;
    Ok(Some(instrukt_core::capabilities::Assignment {
        worker: false,
        role,
        project: None,
        procedure: None,
        todo: None,
        workspace: Some(
            workspace
                .map(PathBuf::from)
                .unwrap_or_else(|| home.company_dir(&company)),
        ),
        key,
        platform: false,
        company: Some(company),
        dispatcher: None,
    }))
}

/// Active conversation binding for a session currently recorded on
/// `connected_conversations` (retired generations are excluded).
#[derive(Debug, Clone)]
pub struct ConversationBinding {
    pub company: ProjectSlug,
    pub connector: String,
    pub conversation: String,
    pub role: RoleName,
    pub session_scope: SessionScope,
    pub reply_mode: ReplyMode,
}

/// Look up the active conversation binding for `caller`.
pub fn conversation_binding(
    home: &Home,
    caller: &SessionRef,
) -> Result<Option<ConversationBinding>, instrukt_core::state_db::StateDbError> {
    let Some(db) = instrukt_core::state_db::StateRead::open(home)? else {
        return Ok(None);
    };
    let row = db
        .conn()
        .query_row(
            "SELECT company,connector,conversation,role,session_scope,reply_mode FROM connected_conversations WHERE session=?1",
            [caller.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let Some((company, connector, conversation, role, session_scope, reply_mode)) = row else {
        return Ok(None);
    };
    let company = ProjectSlug::parse(&company)
        .map_err(|e| instrukt_core::state_db::StateDbError::invalid(db.path(), e.to_string()))?;
    let role = RoleName::parse(&role)
        .map_err(|e| instrukt_core::state_db::StateDbError::invalid(db.path(), e.to_string()))?;
    let session_scope = SessionScope::parse(&session_scope).ok_or_else(|| {
        instrukt_core::state_db::StateDbError::invalid(
            db.path(),
            format!("unknown session_scope `{session_scope}`"),
        )
    })?;
    let reply_mode = ReplyMode::parse(&reply_mode).ok_or_else(|| {
        instrukt_core::state_db::StateDbError::invalid(
            db.path(),
            format!("unknown reply_mode `{reply_mode}`"),
        )
    })?;
    Ok(Some(ConversationBinding {
        company,
        connector,
        conversation,
        role,
        session_scope,
        reply_mode,
    }))
}

/// Summary of an explicit reply queued into `conversation_answers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedReply {
    pub company: String,
    pub connector: String,
    pub conversation: String,
    pub turn: String,
    pub pieces: usize,
}

/// Queue an explicit reply from an active company conversation session into
/// the durable `conversation_answers` outbox, exactly as the agent wrote it,
/// split to the connector's declared message character limit.
pub fn queue_reply(
    home: &Home,
    caller: &SessionRef,
    requested_conversation: Option<&str>,
    text: &str,
) -> Result<QueuedReply, ConversationError> {
    let Some(binding) = conversation_binding(home, caller).map_err(StateError::from)? else {
        return Err(ConversationError::Refused(
            "this session is not bound to an active company conversation".into(),
        ));
    };
    if text.trim().is_empty() {
        return Err(ConversationError::Refused(
            "reply text must not be empty".into(),
        ));
    }
    let state = RunnerState::new(home);
    let target = match binding.session_scope {
        SessionScope::Conversation => {
            if let Some(requested) = requested_conversation
                && requested != binding.conversation
            {
                return Err(ConversationError::Refused(format!(
                    "conversation `{requested}` is outside this session's bound conversation `{}`",
                    binding.conversation
                )));
            }
            binding.conversation.clone()
        }
        SessionScope::Company => {
            let requested = requested_conversation
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ConversationError::Refused(
                        "--conversation is required when the connector uses session_scope: company"
                            .into(),
                    )
                })?;
            let admitted = state
                .read(|conn, path| {
                    conn.query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM conversation_messages m
                            WHERE m.company=?1 AND m.connector=?2 AND m.conversation=?3
                              AND m.admitted_at IS NOT NULL AND m.state <> 'refused'
                              AND NOT EXISTS (
                                  SELECT 1 FROM connected_conversations cc
                                  WHERE cc.company=m.company AND cc.connector=m.connector
                                    AND cc.conversation=m.conversation AND cc.session_scope='conversation'
                              )
                        )",
                        params![binding.company.as_str(), binding.connector, requested],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(state::sql(path))
                })?
                .unwrap_or(false);
            if !admitted {
                return Err(ConversationError::Refused(format!(
                    "conversation `{requested}` has no admitted message for connector `{}` in company `{}`",
                    binding.connector, binding.company
                )));
            }
            requested.to_owned()
        }
    };
    let declared = Configuration::load_company(&home.for_inspection(), &binding.company)?
        .connectors()?
        .into_iter()
        .find(|declared| declared.key == binding.connector)
        .ok_or_else(|| {
            ConversationError::Refused(format!(
                "connector `{}` is no longer declared for company `{}`",
                binding.connector, binding.company
            ))
        })?;
    let declaration = connector_declaration(&declared.kind).ok_or_else(|| {
        ConversationError::Refused(format!(
            "connector kind `{}` is not registered in this build",
            declared.kind
        ))
    })?;
    let message_chars = declaration.message_chars;
    let parts = split(text, message_chars);
    if parts.is_empty() {
        return Err(ConversationError::Refused(
            "reply text must not be empty".into(),
        ));
    }
    let turn = format!("reply:{}", keyed::request_id()?);
    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let admission_position: i64 = tx
            .query_row(
                "SELECT coalesce(MAX(admission_position), 0) FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![binding.company.as_str(), binding.connector, target],
                |row| row.get(0),
            )
            .map_err(state::sql(&path))?;
        for (piece, part) in parts.iter().enumerate() {
            tx.execute(
                "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,admission_position)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    binding.company.as_str(),
                    binding.connector,
                    target,
                    turn,
                    i64::try_from(piece).unwrap_or(i64::MAX),
                    part,
                    admission_position
                ],
            )
            .map_err(state::sql(&path))?;
        }
        Ok(())
    })?;
    Ok(QueuedReply {
        company: binding.company.to_string(),
        connector: binding.connector,
        conversation: target,
        turn,
        pieces: parts.len(),
    })
}

/// The speaker of `session`'s current turn: the sender recorded with the
/// addressed message that opened the turn the runtime has in progress. The
/// turn is correlated with its stored delivery by the delivery's boundary
/// token, which the member envelope of the turn's opening input carries. A
/// session bound to no conversation, and a turn whose opening input carries
/// no member envelope, have no speaker. A conversation's session the runner
/// has bound but not yet recorded as the conversation's is refused as
/// unbound: it is never a session nobody is asking in. Any other
/// uncertainty refuses.
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
            "SELECT company,connector,conversation,session_scope FROM connected_conversations WHERE session=?1",
            [session.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let Some((company, connector, conversation, session_scope)) = conversation else {
        let bound = db
            .conn()
            .query_row(
                "SELECT c.company FROM runtime_bindings b JOIN connected_conversations c ON c.semantic_key=b.semantic_key WHERE b.runtime=?1 AND b.session=?2 LIMIT 1",
                instrukt_core::state_db::rusqlite::params![
                    session.runtime.as_str(),
                    session.to_string()
                ],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
        return match bound {
            Some(company) => Err(instrukt_runner::mcp::McpError::Unbound { company }),
            None => Ok(None),
        };
    };
    let opening = adapter
        .active_turn_input(session)
        .map_err(|error| instrukt_runner::mcp::McpError::Turn {
            cause: error.to_string(),
        })?
        .ok_or_else(|| instrukt_runner::mcp::McpError::Turn {
            cause: "the runtime has no turn of this session in progress".into(),
        })?;
    let member = envelopes::segment(&opening)
        .into_iter()
        .find_map(|span| match span {
            Span::Member {
                conversation,
                boundary,
                ..
            } => Some((conversation.to_owned(), boundary.to_owned())),
            _ => None,
        });
    let Some((span_conversation, boundary)) = member else {
        return Ok(None);
    };
    let target_conversation = if session_scope == SessionScope::Company.as_str() {
        span_conversation
    } else {
        conversation
    };
    let row = db
        .conn()
        .query_row(
            "SELECT message, sender, sender_name, sender_email, sender_is_admin, is_direct FROM conversation_messages
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND boundary=?4",
            [&company, &connector, &target_conversation, &boundary],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i32>(4)? != 0,
                    r.get::<_, i32>(5)? != 0,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let (message, identity, name, email, is_admin, is_direct) =
        row.ok_or_else(|| instrukt_runner::mcp::McpError::Turn {
            cause: "the turn's addressed message is not one this conversation delivered".into(),
        })?;
    Ok(Some(instrukt_runner::mcp::Speaker {
        company,
        name: name
            .or_else(|| identity.clone())
            .unwrap_or_else(|| "an unnamed sender".to_owned()),
        identity,
        email: email.filter(|email| !email.trim().is_empty()),
        is_admin,
        is_direct,
        connector: Some(connector),
        conversation: Some(target_conversation),
        message: Some(message),
    }))
}

/// The company's ports as the runner hosts it: built by the CLI with its
/// registry's connector builders, it builds the kinds when the runner starts
/// hosting and keeps the directory they make, and the answer pieces accepted
/// but not yet recorded, for its own relay.
pub struct Ports {
    builders: Vec<crate::BuildKind>,
    hosted: Mutex<Option<Arc<RelayWorkers>>>,
    accepted: std::sync::Arc<Accepted>,
}

impl Ports {
    pub fn new(builders: Vec<crate::BuildKind>) -> Self {
        Self {
            builders,
            hosted: std::sync::Mutex::default(),
            accepted: std::sync::Arc::default(),
        }
    }

    fn hosted(&self) -> Option<Arc<RelayWorkers>> {
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
    fn admitted_proposal(
        &self,
        home: &Home,
        root: &std::path::Path,
        head: &instrukt_core::ids::CommitSha,
    ) -> Result<bool, instrukt_runner::interfaces::CompanyError> {
        crate::proposals::admitted(home, root, head)
    }

    fn unestablished_proposals(
        &self,
        home: &Home,
        root: &std::path::Path,
    ) -> Result<
        Vec<instrukt_runner::interfaces::UnestablishedProposal>,
        instrukt_runner::interfaces::CompanyError,
    > {
        crate::proposals::unestablished(home, root)
    }

    fn proposal_store(
        &self,
        home: &Home,
        root: &std::path::Path,
    ) -> Result<
        instrukt_runner::interfaces::ProposalStoreCondition,
        instrukt_runner::interfaces::CompanyError,
    > {
        crate::proposals::condition(home, root)
    }

    fn maintain_read_back(
        &self,
        home: &Home,
        now: Timestamp,
    ) -> Result<(), instrukt_runner::interfaces::CompanyError> {
        read_back::maintain_at_start(home, now)
            .map_err(|error| instrukt_runner::interfaces::CompanyError(error.cause))
    }

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
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(RelayWorkers {
            connectors: Arc::clone(&directory),
            wake: start.wake.clone(),
            probes: Mutex::default(),
            absence_reads: Mutex::default(),
            work: Mutex::default(),
            answer_anomalies: Mutex::default(),
            env: start.env.clone(),
        }));
        crate::connector_host::Connections::start(
            start.home,
            start.current,
            directory,
            std::sync::Arc::clone(&self.accepted),
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
        let Some(workers) = self.hosted() else {
            return Ok(());
        };
        relay(
            home,
            admitted,
            source,
            adapter,
            &workers,
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
            .map_or_else(Timestamp::now, |workers| workers.connectors.now());
        relay_state(home, failure, now);
    }

    fn status(
        &self,
        home: &Home,
        now: Timestamp,
    ) -> Result<serde_json::Value, instrukt_runner::interfaces::CompanyError> {
        let status = status_at(home, now).map_err(company_error)?;
        serde_json::to_value(status).map_err(company_error)
    }
}

impl instrukt_runner::interfaces::CompanySupervision for crate::connector_host::Connections {
    fn check(&mut self) {
        crate::connector_host::Connections::check(self);
    }
}

#[cfg(test)]
mod tests;
