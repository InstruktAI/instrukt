//! Resolve the session binding and speaker behind a company conversation, and queue explicit replies.
use instrukt_core::config::{Configuration, ReplyMode, SessionScope, connector_declaration};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_runner::keyed;
use instrukt_runner::runtime::RuntimeAdapter;
use instrukt_runner::state::{self, RunnerState, StateError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::_relay::split;
use super::envelopes::Span;
use super::{ConversationError, envelopes};

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
    let company = instrukt_core::ids::ProjectSlug::parse(&company).map_err(|error| {
        instrukt_core::state_db::StateDbError::invalid(
            db.path(),
            format!("a conversation's company `{company}` is no company key: {error}"),
        )
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
