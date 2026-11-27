//! Persisted conversation bindings, message stages, and relay work queries.
use instrukt_core::config::{ReplyMode, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef};
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::keyed;
use instrukt_runner::runtime::conversations::SourcePosition;
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;

use super::ConversationError;

/// Where an addressed message is on its way to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
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
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Refused => "refused",
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Answered => "answered",
        }
    }
    pub(super) fn parse(text: &str) -> Option<Self> {
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

pub(super) fn micros(at: Timestamp) -> i64 {
    at.as_microsecond()
}

#[derive(Clone)]
pub(super) struct Conversation {
    pub(super) company: ProjectSlug,
    pub(super) connector: String,
    pub(super) conversation: String,
    pub(super) semantic_key: String,
    pub(super) opening: Option<String>,
    pub(super) opening_conversation: Option<String>,
    pub(super) request: Option<String>,
    pub(super) session: Option<SessionRef>,
    pub(super) source_identity: Option<String>,
    pub(super) position: Option<SourcePosition>,
    pub(super) role: RoleName,
    pub(super) session_scope: SessionScope,
    pub(super) reply_mode: ReplyMode,
}

#[derive(Clone)]
pub(super) struct Message {
    pub(super) conversation: String,
    pub(super) message: String,
    pub(super) stage: Stage,
    /// Present from `pending` on.
    pub(super) boundary: Option<String>,
    /// The framed input, present from `pending` on.
    pub(super) content: Option<String>,
    /// A send to the runtime was begun; its acceptance may be unrecorded.
    pub(super) attempted: Option<Timestamp>,
    /// The cause retained from its last failed delivery.
    pub(super) failure: Option<String>,
    pub(super) retry_at: Option<Timestamp>,
    pub(super) failed_turns: u32,
    pub(super) send_unknown: bool,
    pub(super) dispatch_id: Option<String>,
    pub(super) dispatch_write_at: Option<Timestamp>,
}

const CONVERSATION_COLUMNS: &str = "company,connector,conversation,semantic_key,opening,opening_conversation,request,session,source_identity,position_json,role,session_scope,reply_mode";

const RELAY_WORK: &str = "SELECT company,connector,conversation FROM conversation_messages WHERE state IN (?1,?2,?3) OR (state='failed' AND superseded_at IS NULL AND boundary IS NOT NULL)
    UNION ALL SELECT company,connector,conversation FROM conversation_answers WHERE posted_at IS NULL";

/// SQL predicate restricting a company-scoped binding to conversations that
/// do NOT have their own `session_scope = 'conversation'` binding.
pub(super) const COMPANY_OWNED_MESSAGES: &str = "NOT EXISTS (SELECT 1 FROM connected_conversations cc WHERE cc.company=conversation_messages.company AND cc.connector=conversation_messages.connector AND cc.conversation=conversation_messages.conversation AND cc.session_scope='conversation')";
pub(super) const COMPANY_OWNED_ANSWERS: &str = "NOT EXISTS (SELECT 1 FROM connected_conversations cc WHERE cc.company=conversation_answers.company AND cc.connector=conversation_answers.connector AND cc.conversation=conversation_answers.conversation AND cc.session_scope='conversation')";

pub(super) fn relay_work_filter() -> String {
    format!(
        "((session_scope='conversation' AND (company,connector,conversation) IN ({RELAY_WORK})) OR (session_scope='company' AND (company,connector) IN (SELECT w.company,w.connector FROM ({RELAY_WORK}) w WHERE NOT EXISTS (SELECT 1 FROM connected_conversations cc WHERE cc.company=w.company AND cc.connector=w.connector AND cc.conversation=w.conversation AND cc.session_scope='conversation'))))"
    )
}

pub(super) fn relay_stages() -> [&'static str; 3] {
    [
        Stage::Received.as_str(),
        Stage::Pending.as_str(),
        Stage::Sent.as_str(),
    ]
}

pub(super) fn conversations(
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

pub(super) fn reread(
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
pub(super) fn open_messages(
    state: &RunnerState,
    c: &Conversation,
) -> Result<Vec<Message>, ConversationError> {
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

pub(super) fn lock(
    home: &Home,
    c: &Conversation,
) -> Result<instrukt_core::fsx::LockGuard, ConversationError> {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(&Sha256::digest(c.semantic_key.as_bytes())[..8]);
    Ok(instrukt_core::fsx::LockGuard::acquire(
        &home.state_dir().join(format!("conversation-{digest}.lock")),
        std::time::Duration::from_secs(120),
    )?)
}

pub(super) fn set_failure(
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

pub(super) fn set_message_failure(
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

pub(super) fn remember_delivery(
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

/// Prune superseded failures and retain their receipts for another window.
pub(super) fn prune_superseded_failures(
    state: &RunnerState,
    now: Timestamp,
) -> Result<(), StateError> {
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
pub(super) fn clear_idle_answer_failures(
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
