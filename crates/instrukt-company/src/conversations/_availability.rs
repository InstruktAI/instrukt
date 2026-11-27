//! Unfinished work, working status, and unavailable notice intervals.
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;
use std::collections::BTreeSet;

use super::_relay::Accepted;
use super::_store::{Conversation, Stage};
use super::ConversationError;

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

/// Open the no-work interval of each `(conversation, message)` at `at`, where
/// none is open: a confirmed failure keeps the agent from the work they wait on.
pub(super) fn unavailable(
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
pub(super) fn available(
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
pub(super) fn turn_update(
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

/// A recovered no-work interval rearms the person's next outage notice.
pub(super) fn retire_notices(
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
