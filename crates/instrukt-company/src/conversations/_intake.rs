//! Retain addressed messages, record sender decisions, and capture accompanying context.
use instrukt_connector::{Accompanying, ConversationDisplay, Incoming, RefusalReason, Sender};
use instrukt_core::config::{Admitted, ConfigError, DeclaredConnector, ReplyMode, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName};
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;

use super::_envelopes::framed;
use super::_store::{COMPANY_SCOPE_CONVERSATION, Stage, company_key, key, micros};
use super::{ConversationError, LOG_TARGET, envelopes};

/// What became of an incoming addressed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intake {
    /// Retained, waiting for its sender's admission.
    Retained,
    /// Already retained.
    Repeated,
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

pub(super) fn retain_in(
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

pub(super) fn conversation_title(
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
