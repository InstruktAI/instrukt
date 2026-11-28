//! Reconcile in-flight addressed turns against live runtime projection state
//! (`active_turn` + bounded `recent_turns`) and durable explicit-reply records.
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_runner::runtime::{ObservedTurn, OpeningInput, SessionState, Thread, TurnState};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use std::collections::BTreeSet;

use super::_availability::{available, retire_notices};
use super::_delivery::{FAILED_RETRY_MAX_ATTEMPTS, retry_delay};
use super::_relay::{settle_failed_messages, split};
use super::_store::{Conversation, Message, Stage, open_messages, remember_delivery};
use super::ConversationError;
use super::envelopes::{self, Span};

/// Grace period after send acceptance while a thread stays idle without
/// matching any active or recent turn before escalating (`UC-CV26`).
pub const UNOBSERVED_SENT_GRACE: SignedDuration = SignedDuration::from_mins(2);

/// Sanitized member-facing reply queued when bounded retries exhaust or a
/// confirmed addressed turn finishes idle without replying (`UC-CV26`).
pub const STANDARD_ERROR_REPLY: &str =
    "Sorry, I ran into a problem while handling your request. Please try again in a moment.";

const FAILED_TURN_CAUSE: &str = "the turn ended without a reply";
const IDLE_WITHOUT_REPLY_CAUSE: &str = "the turn finished idle without replying";
const RETRY_EXHAUSTED_CAUSE: &str = "failed-turn retries exhausted without a reply";
const EVICTED_WITHOUT_REPLY_CAUSE: &str =
    "the delivered turn completed outside the recent turn window without replying";
const UNOBSERVED_GRACE_EXPIRED_CAUSE: &str =
    "the delivered turn was never observed running before the idle grace deadline";

/// Projection window size beyond which a full `recent_turns` window whose
/// oldest timestamp postdates `attempted_at` proves eviction.
const RECENT_TURNS_WINDOW_CAP: usize = 10;

/// Messages that atomically transitioned to terminal error (`UC-CV26`) during
/// one live-thread reconciliation pass.
#[derive(Debug, Default)]
pub(super) struct ReconcileOutcome {
    pub(super) terminal_errors: Vec<(String, String)>,
}

fn input_matches_boundary(opening: &OpeningInput, boundary: Option<&str>) -> bool {
    let (OpeningInput::Text(text), Some(boundary)) = (opening, boundary) else {
        return false;
    };
    envelopes::segment(text)
        .into_iter()
        .any(|span| matches!(span, Span::Member { boundary: found, .. } if found == boundary))
}

fn matches_active(message: &Message, thread: &Thread) -> bool {
    let Some(active) = &thread.active_turn else {
        return false;
    };
    if let (Some(expected), Some(actual)) = (
        message.observed_run_id.as_deref(),
        Some(active.run_id.as_str()),
    ) && expected == actual
    {
        return true;
    }
    if let (Some(expected), Some(actual)) = (
        message.dispatch_id.as_deref(),
        active.input_id.as_ref().map(|id| id.as_str()),
    ) && expected == actual
    {
        return true;
    }
    input_matches_boundary(&active.opening_input, message.boundary.as_deref())
}

fn find_recent_turn<'a>(message: &Message, recent: &'a [ObservedTurn]) -> Option<&'a ObservedTurn> {
    if let Some(run_id) = message.observed_run_id.as_deref()
        && let Some(found) = recent
            .iter()
            .rev()
            .find(|turn| turn.run_id.as_str() == run_id)
    {
        return Some(found);
    }
    if let Some(dispatch_id) = message.dispatch_id.as_deref()
        && let Some(found) = recent.iter().rev().find(|turn| {
            turn.input_id
                .as_ref()
                .is_some_and(|input| input.as_str() == dispatch_id)
        })
    {
        return Some(found);
    }
    None
}

fn oldest_recent_turn_time(recent: &[ObservedTurn]) -> Option<Timestamp> {
    recent
        .iter()
        .filter_map(|turn| match turn.state {
            TurnState::Active { started_at } => Some(started_at),
            TurnState::Completed { started_at, .. } => Some(started_at),
            TurnState::Failed {
                started_at,
                completed_at,
            } => Some(started_at.unwrap_or(completed_at)),
            TurnState::Preparing | TurnState::Queued => None,
        })
        .min()
}

fn explicit_reply_turn(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    c: &Conversation,
    message: &Message,
) -> Result<Option<String>, StateError> {
    tx.query_row(
        "SELECT turn FROM conversation_answers
         WHERE company=?1 AND connector=?2 AND conversation=?3 AND source_message=?4
         ORDER BY rowid DESC LIMIT 1",
        params![
            c.company.as_str(),
            c.connector,
            message.conversation,
            message.message
        ],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(state::sql(&state::db_path(tx)))
}

fn settle_answered_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    c: &Conversation,
    message: &Message,
    reply_turn: &str,
    now: Timestamp,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    let has_unposted: bool = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_answers
                WHERE company=?1 AND connector=?2 AND conversation=?3
                  AND (source_message=?4 OR (source_message IS NULL AND turn=?5))
                  AND posted_at IS NULL
            )",
            params![
                c.company.as_str(),
                c.connector,
                message.conversation,
                message.message,
                reply_turn
            ],
            |r| r.get(0),
        )
        .map_err(state::sql(&path))?;
    remember_delivery(tx, c, &message.conversation, &message.message)?;
    tx.execute(
        "UPDATE conversation_messages
         SET state=?5,
             turn=?6,
             failure=NULL,
             retry_at=NULL,
             attempted_at=NULL,
             dispatch_command_id=NULL,
             dispatch_write_at=NULL,
             observed_run_id=NULL,
             turn_started_at=NULL,
             send_unknown_at=NULL,
             turn_ended_at=coalesce(turn_ended_at,?8),
             unavailable_since=CASE WHEN ?7 THEN unavailable_since END
         WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND superseded_at IS NULL",
        params![
            c.company.as_str(),
            c.connector,
            message.conversation,
            message.message,
            Stage::Answered.as_str(),
            reply_turn,
            has_unposted,
            now.to_string(),
        ],
    )
    .map_err(state::sql(&path))?;
    if !has_unposted {
        settle_failed_messages(
            tx,
            c.company.as_str(),
            &c.connector,
            &message.conversation,
            now,
        )?;
    }
    Ok(())
}

fn schedule_failed_retry_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    c: &Conversation,
    message: &Message,
    now: Timestamp,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    remember_delivery(tx, c, &message.conversation, &message.message)?;
    tx.execute(
        "UPDATE conversation_messages
         SET state='failed',
             failure=?5,
             unavailable_since=coalesce(unavailable_since,?6),
             attempted_at=NULL,
             dispatch_command_id=NULL,
             dispatch_write_at=NULL,
             observed_run_id=NULL,
             turn_started_at=NULL,
             send_unknown_at=NULL,
             retry_at=?7,
             failed_turns=failed_turns+1
         WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND superseded_at IS NULL",
        params![
            c.company.as_str(),
            c.connector,
            message.conversation,
            message.message,
            FAILED_TURN_CAUSE,
            now.to_string(),
            format!("{:.9}", now + retry_delay(message.failed_turns)),
        ],
    )
    .map_err(state::sql(&path))?;
    Ok(())
}

fn settle_terminal_error_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    c: &Conversation,
    message: &Message,
    message_chars: usize,
    cause: &str,
    now: Timestamp,
) -> Result<bool, StateError> {
    let path = state::db_path(tx);
    let error_turn = format!("error:{}", message.message);
    let stamp = format!("{now:.9}");
    let changed = tx
        .execute(
            "UPDATE conversation_messages
             SET state=?5,
                 turn=?6,
                 failure=?7,
                 terminal_error_queued_at=?8,
                 retry_at=NULL,
                 attempted_at=NULL,
                 dispatch_command_id=NULL,
                 dispatch_write_at=NULL,
                 observed_run_id=NULL,
                 turn_started_at=NULL,
                 send_unknown_at=NULL,
                 turn_ended_at=?8
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
               AND superseded_at IS NULL AND terminal_error_queued_at IS NULL",
            params![
                c.company.as_str(),
                c.connector,
                message.conversation,
                message.message,
                Stage::Failed.as_str(),
                error_turn,
                cause,
                stamp,
            ],
        )
        .map_err(state::sql(&path))?;
    if changed == 0 {
        return Ok(false);
    }
    remember_delivery(tx, c, &message.conversation, &message.message)?;
    for (piece, text) in split(STANDARD_ERROR_REPLY, message_chars)
        .iter()
        .enumerate()
    {
        tx.execute(
            "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,admission_position,source_message)
             SELECT ?1,?2,?3,?4,?5,?6,admission_position,?7 FROM conversation_messages
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?7 AND superseded_at IS NULL
             ON CONFLICT DO NOTHING",
            params![
                c.company.as_str(),
                c.connector,
                message.conversation,
                error_turn,
                i64::try_from(piece).unwrap_or(i64::MAX),
                text,
                message.message,
            ],
        )
        .map_err(state::sql(&path))?;
    }
    retire_notices(tx, c.company.as_str(), &c.connector, &message.conversation)?;
    Ok(true)
}

/// Reconcile in-flight `Stage::Sent` and late-matched `send_unknown` messages
/// against the thread's live `active_turn` and bounded `recent_turns` window.
/// Must run strictly AFTER `ingest_explicit_replies` has succeeded so any
/// queued explicit reply is visible in `conversation_answers`.
pub(super) fn reconcile_thread(
    state: &RunnerState,
    c: &Conversation,
    thread: &Thread,
    message_chars: usize,
    now: Timestamp,
) -> Result<ReconcileOutcome, ConversationError> {
    let open = open_messages(state, c)?;
    let pending_keys = open
        .iter()
        .filter(|m| m.stage == Stage::Pending && !m.send_unknown)
        .map(|m| (m.conversation.clone(), m.message.clone()))
        .collect::<Vec<_>>();

    // Late-match any send_unknown or unconfirmed-send rows that now appear in
    // live active_turn / recent_turns, or already have an explicit reply linked
    // via `source_message`.
    let mut late_matched = Vec::new();
    for m in open.iter().filter(|m| {
        (m.send_unknown || m.attempted.is_some())
            && matches!(m.stage, Stage::Pending | Stage::Failed)
    }) {
        if matches_active(m, thread) || find_recent_turn(m, &thread.recent_turns).is_some() {
            late_matched.push(m.clone());
        }
    }
    let unknown_with_reply: Vec<Message> = state
        .read(|conn, path| {
            let mut found = Vec::new();
            for m in open.iter().filter(|m| {
                m.send_unknown
                    && matches!(m.stage, Stage::Pending | Stage::Failed)
                    && !late_matched
                        .iter()
                        .any(|lm| lm.conversation == m.conversation && lm.message == m.message)
            }) {
                let has_reply: bool = conn
                    .query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM conversation_answers
                            WHERE company=?1 AND connector=?2 AND conversation=?3 AND source_message=?4
                        )",
                        params![c.company.as_str(), c.connector, m.conversation, m.message],
                        |r| r.get(0),
                    )
                    .map_err(state::sql(path))?;
                if has_reply {
                    found.push(m.clone());
                }
            }
            Ok(found)
        })?
        .unwrap_or_default();
    late_matched.extend(unknown_with_reply);
    if !late_matched.is_empty() {
        state.write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            for m in &late_matched {
                let changed = tx
                    .execute(
                        "UPDATE conversation_messages
                         SET state='sent',failure=NULL,retry_at=NULL,send_unknown_at=NULL
                         WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
                           AND state IN ('pending','failed')
                           AND (attempted_at IS NOT NULL OR send_unknown_at IS NOT NULL)
                           AND superseded_at IS NULL",
                        params![c.company.as_str(), c.connector, m.conversation, m.message],
                    )
                    .map_err(state::sql(&path))?;
                if changed != 0 {
                    remember_delivery(tx, c, &m.conversation, &m.message)?;
                }
            }
            Ok(())
        })?;
    }

    let open = if late_matched.is_empty() {
        open
    } else {
        open_messages(state, c)?
    };
    let sent = open
        .iter()
        .filter(|m| m.stage == Stage::Sent)
        .collect::<Vec<_>>();

    // If a turn is actively running, messages waiting behind it are making progress.
    if thread.active_turn.is_some() && !pending_keys.is_empty() {
        available(state, c, &pending_keys)?;
    }

    if sent.is_empty() {
        return Ok(ReconcileOutcome::default());
    }

    let mut active_matched = BTreeSet::new();
    let mut newly_available = Vec::new();
    let mut outcome = ReconcileOutcome::default();

    // 1. Active turn observation: record observed_run_id / turn_started_at and keep Stage::Sent.
    if let Some(active) = &thread.active_turn {
        for m in &sent {
            if matches_active(m, thread) {
                active_matched.insert((m.conversation.clone(), m.message.clone()));
                newly_available.push((m.conversation.clone(), m.message.clone()));
                state.write::<_, StateError>(|tx| {
                    tx.execute(
                        "UPDATE conversation_messages
                         SET observed_run_id=?5,
                             turn_started_at=coalesce(turn_started_at,?6)
                         WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
                           AND state='sent' AND superseded_at IS NULL",
                        params![
                            c.company.as_str(),
                            c.connector,
                            m.conversation,
                            m.message,
                            active.run_id.as_str(),
                            format!("{:.9}", active.started_at),
                        ],
                    )
                    .map_err(state::sql(&state::db_path(tx)))?;
                    Ok(())
                })?;
            }
        }
    }

    // 2. Reconcile all Stage::Sent messages not currently matched as the active turn.
    let is_idle = thread.active_turn.is_none()
        && matches!(
            thread.state,
            SessionState::InputReady | SessionState::EndedFailed
        );
    let window_full = thread.recent_turns.len() >= RECENT_TURNS_WINDOW_CAP;
    let oldest_recent = oldest_recent_turn_time(&thread.recent_turns);

    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        for m in &sent {
            if active_matched.contains(&(m.conversation.clone(), m.message.clone())) {
                continue;
            }
            let reply_turn = explicit_reply_turn(tx, c, m)?;
            if let Some(turn) = find_recent_turn(m, &thread.recent_turns) {
                match turn.state {
                    TurnState::Preparing | TurnState::Queued => {
                        // Pre-start state: never settle as idle-without-reply.
                    }
                    TurnState::Active { started_at } => {
                        tx.execute(
                            "UPDATE conversation_messages
                             SET observed_run_id=?5,
                                 turn_started_at=coalesce(turn_started_at,?6)
                             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
                               AND state='sent' AND superseded_at IS NULL",
                            params![
                                c.company.as_str(),
                                c.connector,
                                m.conversation,
                                m.message,
                                turn.run_id.as_str(),
                                format!("{:.9}", started_at),
                            ],
                        )
                        .map_err(state::sql(&path))?;
                        newly_available.push((m.conversation.clone(), m.message.clone()));
                    }
                    TurnState::Completed { .. } => {
                        if let Some(reply_turn) = reply_turn {
                            settle_answered_in(tx, c, m, &reply_turn, now)?;
                        } else if settle_terminal_error_in(
                            tx,
                            c,
                            m,
                            message_chars,
                            IDLE_WITHOUT_REPLY_CAUSE,
                            now,
                        )? {
                            outcome
                                .terminal_errors
                                .push((m.conversation.clone(), m.message.clone()));
                        }
                    }
                    TurnState::Failed { .. } => {
                        if let Some(reply_turn) = reply_turn {
                            settle_answered_in(tx, c, m, &reply_turn, now)?;
                        } else if m.failed_turns < FAILED_RETRY_MAX_ATTEMPTS {
                            schedule_failed_retry_in(tx, c, m, now)?;
                        } else if settle_terminal_error_in(
                            tx,
                            c,
                            m,
                            message_chars,
                            RETRY_EXHAUSTED_CAUSE,
                            now,
                        )? {
                            outcome
                                .terminal_errors
                                .push((m.conversation.clone(), m.message.clone()));
                        }
                    }
                }
                continue;
            }

            // Not present in active_turn or recent_turns (Lead 1 window eviction
            // & unobserved-Sent grace deadline).
            if let Some(reply_turn) = reply_turn
                && (is_idle || m.observed_run_id.is_some())
            {
                settle_answered_in(tx, c, m, &reply_turn, now)?;
                continue;
            }

            let was_observed = m.observed_run_id.is_some();
            let window_evicted = window_full
                && m.attempted
                    .is_some_and(|at| oldest_recent.is_some_and(|oldest| oldest > at));
            let grace_expired = is_idle
                && m.attempted
                    .is_some_and(|at| now >= at + UNOBSERVED_SENT_GRACE);

            if was_observed || window_evicted || grace_expired {
                if thread.state == SessionState::EndedFailed
                    && m.failed_turns < FAILED_RETRY_MAX_ATTEMPTS
                {
                    schedule_failed_retry_in(tx, c, m, now)?;
                } else {
                    let cause = if m.failed_turns >= FAILED_RETRY_MAX_ATTEMPTS {
                        RETRY_EXHAUSTED_CAUSE
                    } else if was_observed || window_evicted {
                        EVICTED_WITHOUT_REPLY_CAUSE
                    } else {
                        UNOBSERVED_GRACE_EXPIRED_CAUSE
                    };
                    if settle_terminal_error_in(tx, c, m, message_chars, cause, now)? {
                        outcome
                            .terminal_errors
                            .push((m.conversation.clone(), m.message.clone()));
                    }
                }
            }
        }
        Ok(())
    })?;

    if !newly_available.is_empty() {
        available(state, c, &newly_available)?;
    }
    Ok(outcome)
}
