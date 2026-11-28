//! Resolve the session binding and active-turn speaker behind a company conversation.
use std::path::PathBuf;
use std::time::Duration;

use instrukt_core::config::SessionScope;
use instrukt_core::home::Home;
use instrukt_core::ids::{InputId, ProjectSlug, RoleName, SessionRef};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_runner::runtime::{OpeningInput, Presence, RuntimeAdapter, SessionState};

use super::_store::COMPANY_OWNED_MESSAGES;
use super::envelopes::{self, Span};

/// Default bounded timeout for live speaker observation calls.
const DEFAULT_SPEAKER_TIMEOUT: Duration = Duration::from_secs(5);

/// Fail-closed speaker resolution errors, classified into retryable vs deterministic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpeakerError {
    /// Pre-acceptance `runtime_bindings` row exists before `connected_conversations.session` is recorded.
    #[error(
        "session is bound to company `{company}` but its conversation acceptance is not yet recorded"
    )]
    Unbound { company: String },
    /// Transient live runtime observation error.
    #[error("live runtime observation failed: {0}")]
    RuntimeObservation(String),
    /// Transient durable state read error.
    #[error("conversation state could not be read: {0}")]
    Store(String),
    /// Session belongs to a retired generation in `runtime_generations` (`generation > 0`).
    #[error("session belongs to a retired generation of company `{company}`")]
    RetiredGeneration { company: String },
    /// Session is not actively running (`InputReady`, `EndedFailed`, `Gone`, or not present).
    #[error("the runtime has no turn of this session in progress")]
    NotRunning { state: String },
    /// Runtime reports an active state without an active turn or input identity.
    #[error("the runtime has no active turn with an input identity")]
    NoActiveTurn,
    /// Active turn's input identity is unrecognized and its opening input is unavailable.
    #[error("the active turn's opening input is unavailable to verify speaker provenance")]
    OpeningInputUnavailable,
    /// Active turn carries a company member envelope whose delivery is not matched.
    #[error("the turn's addressed message is not one this conversation delivered")]
    MismatchedTurn,
}

impl SpeakerError {
    /// Whether retrying speaker resolution within a bounded deadline may succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Unbound { .. } | Self::RuntimeObservation(_) | Self::Store(_)
        )
    }
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
            params![caller.to_string(), caller.runtime.as_str()],
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
/// `connected_conversations` or pending Turn 1 acceptance in `runtime_bindings`
/// (retired generations are excluded).
#[derive(Debug, Clone)]
pub struct ConversationBinding {
    pub company: ProjectSlug,
    pub connector: String,
    pub conversation: String,
    pub role: RoleName,
    pub session_scope: SessionScope,
}

/// Look up the accepted conversation binding recorded on `connected_conversations`
/// for `caller`. Pre-acceptance `runtime_bindings` rows are recognized by
/// [`conversation_assignment`], not by `conversation_binding`; retired
/// generations are excluded.
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
            "SELECT company,connector,conversation,role,session_scope
             FROM connected_conversations
             WHERE session=?1",
            [caller.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(instrukt_core::state_db::StateDbError::sqlite(db.path()))?;
    let Some((company, connector, conversation, role, session_scope)) = row else {
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
    Ok(Some(ConversationBinding {
        company,
        connector,
        conversation,
        role,
        session_scope,
    }))
}

/// Resolved Outcome 1 active addressed turn with its speaker and turn identity.
#[derive(Debug, Clone)]
pub(super) struct ActiveTurnMatch {
    pub speaker: instrukt_runner::mcp::Speaker,
    pub conversation: String,
    pub message: String,
    pub input_id: InputId,
}

/// Fail-closed 3-outcome active-turn speaker resolution:
/// - Outcome 1 (`Ok(Some)`): active `connected_conversations` session in
///   `Running`/`NeedsInput` whose `active_turn.input_id` matches an in-flight
///   (`state IN ('pending','failed','sent') AND superseded_at IS NULL`) row's
///   `dispatch_command_id`.
/// - Outcome 2 (`Err(SpeakerError)`): pre-acceptance `runtime_bindings`
///   (`Unbound`), retired `runtime_generations` (`RetiredGeneration`), store or
///   live observation failure, non-active state, missing `active_turn`/`input_id`,
///   `OpeningInput::Unavailable`, or unrecognized `input_id` whose opening text
///   carries a company member envelope (`MismatchedTurn`).
/// - Outcome 3 (`Ok(None)`): non-company session, or confirmed `Running`/`NeedsInput`
///   non-company (direct-operator) turn on an active company session.
pub(super) fn resolve_active_turn_within(
    home: &Home,
    session: &SessionRef,
    adapter: &dyn RuntimeAdapter,
    timeout: Duration,
) -> Result<Option<ActiveTurnMatch>, SpeakerError> {
    let Some(db) = instrukt_core::state_db::StateRead::open(home)
        .map_err(|err| SpeakerError::Store(err.to_string()))?
    else {
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
        .map_err(|err| SpeakerError::Store(err.to_string()))?;
    let Some((company, connector, binding_conversation, session_scope)) = conversation else {
        let bound = db
            .conn()
            .query_row(
                "SELECT c.company FROM runtime_bindings b
                 JOIN connected_conversations c ON c.semantic_key=b.semantic_key
                 WHERE b.runtime=?1 AND b.session=?2 LIMIT 1",
                params![session.runtime.as_str(), session.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| SpeakerError::Store(err.to_string()))?;
        if let Some(company) = bound {
            return Err(SpeakerError::Unbound { company });
        }
        let retired = db
            .conn()
            .query_row(
                "SELECT c.company FROM runtime_generations g
                 JOIN connected_conversations c ON c.semantic_key=g.semantic_key
                 WHERE g.session=?1 AND g.generation>0 LIMIT 1",
                [session.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| SpeakerError::Store(err.to_string()))?;
        if let Some(company) = retired {
            return Err(SpeakerError::RetiredGeneration { company });
        }
        return Ok(None);
    };
    let thread = adapter
        .read_state_within(session, timeout)
        .map_err(|err| SpeakerError::RuntimeObservation(err.to_string()))?;
    if thread.presence != Presence::Present {
        return Err(SpeakerError::NotRunning {
            state: format!("{:?}", thread.presence).to_lowercase(),
        });
    }
    if !matches!(
        thread.state,
        SessionState::Running | SessionState::NeedsInput
    ) {
        return Err(SpeakerError::NotRunning {
            state: thread.state.as_str().to_owned(),
        });
    }
    let Some(active_turn) = thread.active_turn else {
        return Err(SpeakerError::NoActiveTurn);
    };
    let Some(input_id) = active_turn.input_id else {
        return Err(SpeakerError::NoActiveTurn);
    };
    let is_company_scope = session_scope == SessionScope::Company.as_str();
    let row = if is_company_scope {
        db.conn()
            .query_row(
                &format!(
                    "SELECT conversation, message, sender, sender_name, sender_email, sender_is_admin, is_direct
                     FROM conversation_messages
                     WHERE company=?1 AND connector=?2 AND dispatch_command_id=?3
                       AND state IN ('pending','failed','sent','answered') AND superseded_at IS NULL
                       AND {COMPANY_OWNED_MESSAGES}
                     LIMIT 1"
                ),
                params![company, connector, input_id.as_str()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i32>(5)? != 0,
                        r.get::<_, i32>(6)? != 0,
                    ))
                },
            )
            .optional()
            .map_err(|err| SpeakerError::Store(err.to_string()))?
    } else {
        db.conn()
            .query_row(
                "SELECT conversation, message, sender, sender_name, sender_email, sender_is_admin, is_direct
                 FROM conversation_messages
                 WHERE company=?1 AND connector=?2 AND conversation=?3 AND dispatch_command_id=?4
                   AND state IN ('pending','failed','sent','answered') AND superseded_at IS NULL
                 LIMIT 1",
                params![company, connector, binding_conversation, input_id.as_str()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i32>(5)? != 0,
                        r.get::<_, i32>(6)? != 0,
                    ))
                },
            )
            .optional()
            .map_err(|err| SpeakerError::Store(err.to_string()))?
    };
    let Some((target_conversation, message, identity, name, email, is_admin, is_direct)) = row
    else {
        return match active_turn.opening_input {
            OpeningInput::Unavailable => Err(SpeakerError::OpeningInputUnavailable),
            OpeningInput::Text(text) => {
                // Late-surfaced `send_unknown` delivery whose exact delivered
                // envelope now appears on the runtime under a different input id.
                let late_row = if is_company_scope {
                    db.conn()
                        .query_row(
                            &format!(
                                "SELECT conversation, message, sender, sender_name, sender_email, sender_is_admin, is_direct
                                 FROM conversation_messages
                                 WHERE company=?1 AND connector=?2 AND content=?3
                                   AND send_unknown_at IS NOT NULL AND superseded_at IS NULL
                                   AND state IN ('pending','failed','sent')
                                   AND {COMPANY_OWNED_MESSAGES}
                                 LIMIT 1"
                            ),
                            params![company, connector, text],
                            |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, String>(1)?,
                                    r.get::<_, Option<String>>(2)?,
                                    r.get::<_, Option<String>>(3)?,
                                    r.get::<_, Option<String>>(4)?,
                                    r.get::<_, i32>(5)? != 0,
                                    r.get::<_, i32>(6)? != 0,
                                ))
                            },
                        )
                        .optional()
                        .map_err(|err| SpeakerError::Store(err.to_string()))?
                } else {
                    db.conn()
                        .query_row(
                            "SELECT conversation, message, sender, sender_name, sender_email, sender_is_admin, is_direct
                             FROM conversation_messages
                             WHERE company=?1 AND connector=?2 AND conversation=?3 AND content=?4
                               AND send_unknown_at IS NOT NULL AND superseded_at IS NULL
                               AND state IN ('pending','failed','sent')
                             LIMIT 1",
                            params![company, connector, binding_conversation, text],
                            |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, String>(1)?,
                                    r.get::<_, Option<String>>(2)?,
                                    r.get::<_, Option<String>>(3)?,
                                    r.get::<_, Option<String>>(4)?,
                                    r.get::<_, i32>(5)? != 0,
                                    r.get::<_, i32>(6)? != 0,
                                ))
                            },
                        )
                        .optional()
                        .map_err(|err| SpeakerError::Store(err.to_string()))?
                };
                if let Some((
                    target_conversation,
                    message,
                    identity,
                    name,
                    email,
                    is_admin,
                    is_direct,
                )) = late_row
                {
                    let company_slug = ProjectSlug::parse(&company).map_err(|error| {
                        SpeakerError::Store(format!(
                            "a conversation's company `{company}` is no company key: {error}"
                        ))
                    })?;
                    let speaker = instrukt_runner::mcp::Speaker {
                        company: company_slug,
                        name: name
                            .or_else(|| identity.clone())
                            .unwrap_or_else(|| "an unnamed sender".to_owned()),
                        identity,
                        email: email.filter(|email| !email.trim().is_empty()),
                        is_admin,
                        is_direct,
                        connector: Some(connector),
                        conversation: Some(target_conversation.clone()),
                        message: Some(message.clone()),
                    };
                    return Ok(Some(ActiveTurnMatch {
                        speaker,
                        conversation: target_conversation,
                        message,
                        input_id,
                    }));
                }
                let has_member_frame = envelopes::segment(&text)
                    .into_iter()
                    .any(|span| matches!(span, Span::Member { .. }));
                if has_member_frame {
                    Err(SpeakerError::MismatchedTurn)
                } else {
                    Ok(None)
                }
            }
        };
    };
    let company_slug = ProjectSlug::parse(&company).map_err(|error| {
        SpeakerError::Store(format!(
            "a conversation's company `{company}` is no company key: {error}"
        ))
    })?;
    let speaker = instrukt_runner::mcp::Speaker {
        company: company_slug,
        name: name
            .or_else(|| identity.clone())
            .unwrap_or_else(|| "an unnamed sender".to_owned()),
        identity,
        email: email.filter(|email| !email.trim().is_empty()),
        is_admin,
        is_direct,
        connector: Some(connector),
        conversation: Some(target_conversation.clone()),
        message: Some(message.clone()),
    };
    Ok(Some(ActiveTurnMatch {
        speaker,
        conversation: target_conversation,
        message,
        input_id,
    }))
}

/// Resolve the active-turn speaker of `session` within the default timeout.
pub fn speaker(
    home: &Home,
    session: &SessionRef,
    adapter: &dyn RuntimeAdapter,
) -> Result<Option<instrukt_runner::mcp::Speaker>, SpeakerError> {
    speaker_within(home, session, adapter, DEFAULT_SPEAKER_TIMEOUT)
}

/// Resolve the active-turn speaker of `session`, clamping live runtime
/// observation to `timeout`.
pub fn speaker_within(
    home: &Home,
    session: &SessionRef,
    adapter: &dyn RuntimeAdapter,
    timeout: Duration,
) -> Result<Option<instrukt_runner::mcp::Speaker>, SpeakerError> {
    Ok(resolve_active_turn_within(home, session, adapter, timeout)?.map(|matched| matched.speaker))
}
