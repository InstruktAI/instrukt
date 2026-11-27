//! Read finished turns into answers and observe the status of running turns.
use instrukt_core::config::ReplyMode;
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::runtime::conversations::{AuthorKind, ConversationSource, RecordKind, Role};
use instrukt_runner::runtime::{Presence, RuntimeAdapter};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;
use std::collections::{BTreeMap, BTreeSet};

use super::_availability::{available, turn_update, unavailable};
use super::_delivery::{possible_dispatch_at, require_source, retry_delay};
use super::_relay::{settle_failed_messages, split};
use super::_store::{Conversation, Message, Stage, open_messages, remember_delivery};
use super::envelopes::Span;
use super::{ConversationError, LOG_TARGET, envelopes};

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
pub(super) fn harvest(
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
                    !said.is_empty()
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

/// What the session's running turn shows of the messages handed to it. A
/// running turn shows the work going on for every message waiting behind it,
/// and for its own message, whose status then shows the newest message the
/// agent has written in it (under `turn_output`). Under `explicit`, raw turn
/// updates are never surfaced to the connector. A session whose turn cannot
/// be observed opens the no-work interval of what was handed to it; one
/// showing no running turn changes nothing.
pub(super) fn observe(
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
