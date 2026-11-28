//! Run the conversation relay: ingest explicit replies, reconcile live turns,
//! post the durable answer outbox, and deliver queued addressed messages.
use crate::connector_host::Connectors;
use instrukt_core::config::{Admitted, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::interfaces::CompanyRelayPass;
use instrukt_runner::keyed;
use instrukt_runner::runtime::{Presence, RuntimeAdapter, SessionState};
use instrukt_runner::sessions::SessionsError;
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;
use std::collections::BTreeSet;
use std::sync::{Arc, PoisonError};

use super::_availability::{retire_notices, unavailable};
use super::_delivery::{
    accepted, adopt_replacement, clear_rejected_send, next_message, record_delivery_failure,
    schedule_earlier_failures, send_reconciliation_due, send_unknown, stale_acknowledgement,
};
use super::_replies_db::ingest_explicit_replies;
use super::_store::{
    COMPANY_OWNED_ANSWERS, COMPANY_OWNED_MESSAGES, Conversation, Message, Stage,
    clear_idle_answer_failures, conversations, lock, open_messages, prune_superseded_failures,
    relay_stages, relay_work_filter, reread, set_failure, set_message_failure,
};
use super::_turns::reconcile_thread;
use super::{ConversationError, LOG_TARGET};
use _workers::{CheckArgs, CheckedDelivery, DeliveryCheck, DispatchArgs, PolledState};

mod _workers;

pub use _workers::ANSWER_OPERATOR_BOUND;
pub(super) use _workers::RelayWorkers;

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

/// The answer pieces this process saw the connector accept whose acceptance
/// it has not yet recorded. Such a piece is never posted again: only its
/// record is retried, on every pass, until it is written.
#[derive(Debug, Default)]
pub struct Accepted {
    receipts: std::sync::Mutex<
        std::collections::BTreeMap<AcceptedPiece, instrukt_connector::AnswerReceipt>,
    >,
}

type AcceptedPiece = (String, String, String, String, i64);

impl Accepted {
    /// Once the provider confirmed a post, its recipient cannot be owed a
    /// notice while the acceptance still waits to be recorded.
    pub(super) fn turns(
        &self,
        company: &ProjectSlug,
        connector: &str,
        conversation: &str,
    ) -> Vec<String> {
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
    source_message: Option<String>,
}

/// Retry retained pieces in order; a success known by this process retries only its record.
fn post(
    admitted: &Admitted,
    state: &RunnerState,
    connectors: &dyn Connectors,
    accepted: &Accepted,
    c: &Conversation,
) -> Result<(), ConversationError> {
    let pending = state
        .read(|conn, path| match c.session_scope {
            SessionScope::Conversation => {
                let mut query = conn
                    .prepare("SELECT conversation,turn,piece,text,source_message FROM conversation_answers WHERE company=?1 AND connector=?2 AND conversation=?3 AND posted_at IS NULL ORDER BY rowid")
                    .map_err(state::sql(path))?;
                query
                    .query_map(
                        params![c.company.as_str(), c.connector, c.conversation],
                        |row| {
                            Ok(Pending {
                                conversation: row.get(0)?,
                                turn: row.get(1)?,
                                piece: row.get(2)?,
                                text: row.get(3)?,
                                source_message: row.get(4)?,
                            })
                        },
                    )
                    .map_err(state::sql(path))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(state::sql(path))
            }
            SessionScope::Company => {
                let mut query = conn
                    .prepare(&format!(
                        "SELECT conversation,turn,piece,text,source_message FROM conversation_answers WHERE company=?1 AND connector=?2 AND posted_at IS NULL AND {COMPANY_OWNED_ANSWERS} ORDER BY rowid"
                    ))
                    .map_err(state::sql(path))?;
                query
                    .query_map(params![c.company.as_str(), c.connector], |row| {
                        Ok(Pending {
                            conversation: row.get(0)?,
                            turn: row.get(1)?,
                            piece: row.get(2)?,
                            text: row.get(3)?,
                            source_message: row.get(4)?,
                        })
                    })
                    .map_err(state::sql(path))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(state::sql(path))
            }
        })?
        .unwrap_or_default();
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
                    tx.execute(
                        "UPDATE conversation_answers SET attempted_at=?6 WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5 AND posted_at IS NULL",
                        params![
                            c.company.as_str(),
                            c.connector,
                            piece.conversation,
                            piece.turn,
                            piece.piece,
                            connectors.now().to_string()
                        ],
                    )
                    .map_err(state::sql(&state::db_path(tx)))
                })?;
                if claimed == 0 {
                    continue;
                }
                let answer_result = instrukt_outside::runtime::block_on(
                    connector.answer(&piece.conversation, &piece.text),
                )
                .map_err(|err| instrukt_connector::AnswerError {
                    cause: err.to_string(),
                })
                .and_then(|res| res);
                let mut receipt = match answer_result {
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
    source_message: Option<&str>,
) -> Result<(), StateError> {
    tx.execute(
        &format!(
            "UPDATE conversation_messages
             SET state=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN 'answered' ELSE state END,
                 turn=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN coalesce(turn,?4) ELSE turn END,
                 failure=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE failure END,
                 attempted_at=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE attempted_at END,
                 dispatch_command_id=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE dispatch_command_id END,
                 dispatch_write_at=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE dispatch_write_at END,
                 observed_run_id=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE observed_run_id END,
                 turn_started_at=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE turn_started_at END,
                 send_unknown_at=CASE WHEN state='sent' AND ?5 IS NOT NULL THEN NULL ELSE send_unknown_at END,
                 unavailable_since=NULL
             WHERE company=?1 AND connector=?2 AND conversation=?3
               AND (message=?5 OR (?5 IS NULL AND turn=?4))
               AND state IN ('sent','answered','failed')
               AND (unavailable_since IS NOT NULL OR (state='sent' AND ?5 IS NOT NULL))
               AND NOT EXISTS({UNPOSTED})"
        ),
        params![
            company,
            connector,
            conversation,
            turn,
            source_message
        ],
    )
    .map_err(state::sql(&state::db_path(tx)))?;
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
        piece.source_message.as_deref(),
        receipt,
        now,
    )
}

fn record_receipt(
    state: &RunnerState,
    identity: &AcceptedPiece,
    source_message: Option<&str>,
    receipt: &instrukt_connector::AnswerReceipt,
    now: Timestamp,
) -> Result<(), ConversationError> {
    let (company, connector, conversation, turn, piece) = identity;
    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let changed = tx
            .execute(
                "UPDATE conversation_answers SET posted_at=?6,provider_post_id=?7,failure=NULL,attempted_at=NULL WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5 AND posted_at IS NULL",
                params![
                    company,
                    connector,
                    conversation,
                    turn,
                    piece,
                    receipt.posted_at.unwrap_or(now).to_string(),
                    receipt.message_id
                ],
            )
            .map_err(state::sql(&path))?;
        if changed == 0 {
            // An already recorded receipt must not close a newer outage.
            return Ok(());
        }
        close_completed_interval(tx, company, connector, conversation, turn, source_message)?;
        settle_failed_messages(tx, company, connector, conversation, now)?;
        Ok(())
    })?;
    Ok(())
}

/// A later settled turn (excluding terminal-error rows) supersedes earlier failures of the same sender.
pub(super) fn settle_failed_messages(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    company: &str,
    connector: &str,
    conversation: &str,
    now: Timestamp,
) -> Result<(), StateError> {
    tx.execute(
        &format!(
            "UPDATE conversation_messages AS failed
             SET superseded_at=?4,unavailable_since=NULL,retry_at=NULL
             WHERE failed.company=?1 AND failed.connector=?2 AND failed.conversation=?3
               AND failed.state='failed' AND failed.superseded_at IS NULL AND failed.send_unknown_at IS NULL
               AND EXISTS(SELECT 1 FROM conversation_messages
                 WHERE company=failed.company AND connector=failed.connector
                   AND conversation=failed.conversation AND sender=failed.sender
                   AND admission_position>failed.admission_position
                   AND state='answered' AND turn IS NOT NULL
                   AND terminal_error_queued_at IS NULL
                   AND NOT EXISTS({UNPOSTED}))"
        ),
        params![company, connector, conversation, format!("{now:.9}")],
    )
    .map_err(state::sql(&state::db_path(tx)))?;
    retire_notices(tx, company, connector, conversation)?;
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
            params![c.company.as_str(), c.connector, piece.conversation, piece.turn, piece.piece, cause],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    Ok(())
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
                "the runtime cannot establish presence for thread {session} under `{}`",
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

fn post_pending_answers(
    admitted: &Admitted,
    state: &RunnerState,
    connectors: &dyn Connectors,
    receipts: &Accepted,
    c: &Conversation,
) -> Result<(), ConversationError> {
    unconfirmed(state, c, connectors.now())?;
    if let Err(error) = post(admitted, state, connectors, receipts, c) {
        tracing::warn!(target: LOG_TARGET, event = "conversation_answer_unposted", company = %c.company, connector = %c.connector, key = %c.semantic_key, %error);
    }
    unconfirmed(state, c, connectors.now())?;
    Ok(())
}

/// One pass of the company conversation relay:
/// 1. Ingest explicit replies from `state/company-replies.db` strictly BEFORE
///    `relay_work_filter()` and turn-completion checks (on outbox read error,
///    record relay failure, skip turn-completion checks, but still post
///    already-ingested `conversation_answers`).
/// 2. Post retained answers independently of slow runtime RPCs or active dispatch.
/// 3. Poll live thread state off-loop via `RelayWorkers::poll_state`, reconcile
///    in-flight turns, trigger demand-driven rollover for pre-upgrade sessions,
///    and deliver the next queued addressed message.
pub(super) fn relay(
    home: &Home,
    admitted: &Admitted,
    adapter: Arc<dyn RuntimeAdapter>,
    workers: &RelayWorkers,
    receipts: &Accepted,
    pass: CompanyRelayPass,
) -> Result<(), ConversationError> {
    let state = RunnerState::new(home);
    let connectors = workers.connectors.as_ref();
    let clock = || connectors.now();
    prune_superseded_failures(&state, clock())?;

    let outbox_error = match ingest_explicit_replies(home, &state) {
        Ok(_) => {
            relay_state(home, None, clock());
            None
        }
        Err(error) => {
            let cause = error.to_string();
            tracing::warn!(target: LOG_TARGET, event = "company_replies_outbox_unreadable", error = %cause);
            relay_state(home, Some(&cause), clock());
            Some(error)
        }
    };
    let outbox_unreadable = outbox_error.is_some();

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
        if workers.is_dispatching(&initial) {
            // The delivery owns the conversation record until it returns.
            // Posting retained answers is independent of the native delivery.
            post_pending_answers(admitted, &state, connectors, receipts, &initial)?;
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
        // Post already-ingested answers before any runtime RPC so slow runtime
        // calls never block answer delivery.
        post_pending_answers(admitted, &state, connectors, receipts, &c)?;
        if outbox_unreadable {
            continue;
        }

        let before_read = open_messages(&state, &c)?;
        if before_read.iter().any(|message| {
            message.stage == Stage::Failed
                && !message.send_unknown
                && message.retry_at.is_none()
                && message.boundary.is_some()
                && message.terminal_error_queued_at.is_none()
        }) {
            schedule_earlier_failures(&state, &c, clock())?;
        }

        let original_binding = c.session.clone();
        let replacement = if before_read.is_empty() {
            None
        } else {
            adopt_replacement(home, &state, adapter.as_ref(), &mut c)?
        };
        if replacement.is_some() || c.session != original_binding {
            if let CheckedDelivery::Checked {
                candidate,
                result: Ok(DeliveryCheck::Accepted),
            } = workers.poll(&c)
            {
                stale_acknowledgement(&c, &candidate.message);
            }
            workers.invalidate_check(&c);
            workers.invalidate_state_read(&c);
            if replacement.is_some() {
                workers.succeed(home, admitted, Arc::clone(&adapter), c.clone());
            }
            continue;
        }

        if let Some(session) = c.session.clone() {
            if before_read.is_empty() && !workers.is_checking(&c) {
                let _ = workers.poll(&c);
                continue;
            }
            let is_checking = workers.is_checking(&c);
            let needs_turn_poll = before_read.iter().any(|m| {
                m.stage == Stage::Sent
                    || (m.send_unknown && matches!(m.stage, Stage::Pending | Stage::Failed))
                    || (is_checking
                        && m.attempted.is_some()
                        && matches!(m.stage, Stage::Pending | Stage::Failed))
            }) || c.needs_explicit_rollover;
            let polled_thread = if needs_turn_poll {
                match workers.poll_state(Arc::clone(&adapter), &c, &session) {
                    PolledState::Pending => continue,
                    PolledState::Ready(result) => match *result {
                        Ok(thread) => Some(thread),
                        Err(error) => {
                            let cause = adapter
                                .diagnostic("conversation turn observation", &error)
                                .to_string();
                            tracing::warn!(target: LOG_TARGET, event = "conversation_turn_unobserved", company = %c.company, connector = %c.connector, key = %c.semantic_key, error = %cause);
                            set_failure(&state, &c, Some(&cause))?;
                            let open = open_messages(&state, &c)?;
                            let sent = open
                                .iter()
                                .filter(|m| m.stage == Stage::Sent)
                                .map(|m| (m.conversation.clone(), m.message.clone()))
                                .collect::<Vec<_>>();
                            let now = clock();
                            unavailable(&state, &c, &sent, now)?;
                            for message in open.iter().filter(|m| send_reconciliation_due(m, now)) {
                                send_unknown(&state, &c, message, now, &cause)?;
                            }
                            workers.report_answer_failure(admitted, &state, &c, true, now)?;
                            continue;
                        }
                    },
                }
            } else {
                None
            };

            let check = match workers.poll(&c) {
                CheckedDelivery::Checked {
                    candidate,
                    result: Ok(DeliveryCheck::Accepted),
                } => {
                    if candidate.invalidated || c.session.as_ref() != Some(&candidate.session) {
                        stale_acknowledgement(&c, &candidate.message);
                    } else if accepted(
                        &state,
                        &c,
                        &candidate.conversation,
                        &candidate.message,
                        candidate.boundary.as_deref(),
                        None,
                        None,
                    )? {
                        continue;
                    }
                    CheckedDelivery::Idle
                }
                other => other,
            };

            if let Some(ref thread) = polled_thread {
                let observed = thread.presence;
                if observed == Presence::Present {
                    set_failure(&state, &c, None)?;
                } else {
                    if !open_messages(&state, &c)?.is_empty() {
                        if matches!(observed, Presence::Archived | Presence::Absent) {
                            workers.succeed(home, admitted, Arc::clone(&adapter), c.clone());
                        } else {
                            blocked(
                                home,
                                adapter.as_ref(),
                                &state,
                                &c,
                                &session,
                                Some(observed),
                                clock(),
                            )?;
                        }
                    }
                    continue;
                }

                if !outbox_unreadable {
                    let message_chars = connectors
                        .open(admitted, &c.company, &c.connector)
                        .map(|connector| connector.limits().message_chars)
                        .unwrap_or(4000);
                    let outcome = reconcile_thread(&state, &c, thread, message_chars, clock())?;
                    for (conversation, message) in &outcome.terminal_errors {
                        workers.report_terminal_anomaly(admitted, &c, conversation, message);
                    }
                    if !outcome.terminal_errors.is_empty() {
                        post_pending_answers(admitted, &state, connectors, receipts, &c)?;
                    }
                }
            }

            let open_after = open_messages(&state, &c)?;
            if open_after
                .iter()
                .any(|m| m.stage == Stage::Failed && m.failed_turns > 0)
            {
                workers.report_answer_failure(admitted, &state, &c, false, clock())?;
            }

            // Demand-driven one-time rollover for pre-upgrade turn_output bindings.
            if let Some(ref thread) = polled_thread
                && c.needs_explicit_rollover
                && !open_after.is_empty()
            {
                let is_idle = thread.active_turn.is_none()
                    && matches!(
                        thread.state,
                        SessionState::InputReady | SessionState::EndedFailed
                    );
                if is_idle {
                    workers.rollover(home, admitted, Arc::clone(&adapter), c.clone());
                }
                continue;
            }

            let check = match check {
                CheckedDelivery::Checked { candidate, result }
                    if c.session.as_ref() == Some(&candidate.session) =>
                {
                    if candidate.invalidated {
                        None
                    } else if let Ok(DeliveryCheck::Presence(presence)) = result
                        && presence != Presence::Present
                    {
                        if matches!(presence, Presence::Archived | Presence::Absent) {
                            workers.succeed(home, admitted, Arc::clone(&adapter), c.clone());
                        } else {
                            blocked(
                                home,
                                adapter.as_ref(),
                                &state,
                                &c,
                                &candidate.session,
                                Some(presence),
                                clock(),
                            )?;
                        }
                        continue;
                    } else {
                        Some((candidate, result))
                    }
                }
                CheckedDelivery::Checking(candidate) => {
                    if !candidate.invalidated && c.session.as_ref() == Some(&candidate.session) {
                        let now = clock();
                        if let Some(message) = open_after.iter().find(|message| {
                            message.conversation == candidate.conversation
                                && message.message == candidate.message
                                && message.boundary == candidate.boundary
                                && message.attempted == candidate.attempted
                                && send_reconciliation_due(message, now)
                        }) {
                            send_unknown(
                                &state,
                                &c,
                                message,
                                now,
                                "the runtime did not finish reading the input",
                            )?;
                        }
                    }
                    continue;
                }
                CheckedDelivery::DispatchFailed => continue,
                CheckedDelivery::Misplaced => {
                    if !open_after.is_empty() {
                        workers.succeed(home, admitted, Arc::clone(&adapter), c.clone());
                    }
                    continue;
                }
                _ => None,
            };

            let mut checked_head: Option<(String, String)> = None;
            if let Some((candidate, result)) = check
                && let Some(next) = open_after.iter().find(|message| {
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
                        if next_message(&open_after, &c, clock()).is_some_and(|head| {
                            head.conversation == next.conversation && head.message == next.message
                        }) {
                            workers.dispatch(DispatchArgs {
                                home,
                                admitted,
                                adapter: Arc::clone(&adapter),
                                c,
                                conversation: next.conversation.clone(),
                                message: next.message.clone(),
                            });
                        }
                        continue;
                    }
                    Ok(DeliveryCheck::UnconfirmedSend)
                        if send_reconciliation_due(next, clock()) =>
                    {
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
                    Err(error)
                        if matches!(
                            *error,
                            SessionsError::ThreadMisplaced { idle: true, .. }
                                | SessionsError::Runtime(
                                    instrukt_runner::runtime::RuntimeError::ThreadMisplaced {
                                        idle: true,
                                        ..
                                    }
                                )
                        ) =>
                    {
                        if !open_after.is_empty() {
                            workers.succeed(home, admitted, Arc::clone(&adapter), c.clone());
                        }
                        continue;
                    }
                    Err(error)
                        if matches!(
                            *error,
                            SessionsError::ThreadMisplaced { idle: false, .. }
                                | SessionsError::Runtime(
                                    instrukt_runner::runtime::RuntimeError::ThreadMisplaced {
                                        idle: false,
                                        ..
                                    }
                                )
                        ) =>
                    {
                        let cause = match &*error {
                            SessionsError::Runtime(native) => adapter
                                .diagnostic("conversation delivery", native)
                                .to_string(),
                            other => other.to_string(),
                        };
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
                            let cause = match &*error {
                                SessionsError::Runtime(native) => adapter
                                    .diagnostic("conversation delivery", native)
                                    .to_string(),
                                other => other.to_string(),
                            };
                            send_unknown(&state, &c, next, clock(), &cause)?;
                            continue;
                        }
                        tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_retry", session = %session);
                        record_delivery_failure(
                            adapter.as_ref(),
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

            let open = open_messages(&state, &c)?;
            let was_before = |candidate: &Message| {
                before_read.iter().any(|message| {
                    matches!(message.stage, Stage::Pending | Stage::Failed)
                        && message.conversation == candidate.conversation
                        && message.message == candidate.message
                })
            };
            let next = next_message(&open, &c, clock()).filter(|candidate| was_before(candidate));

            // Accept any lost acknowledgment before trying to serve a
            // different queue head that arrived meanwhile.
            let unconfirmed_send = open.iter().find(|m| {
                matches!(m.stage, Stage::Pending | Stage::Failed)
                    && m.attempted.is_some()
                    && !m.send_unknown
                    && was_before(m)
            });
            if let Some(candidate) = unconfirmed_send {
                workers.check(CheckArgs {
                    adapter: Arc::clone(&adapter),
                    c: &c,
                    session: &session,
                    next: candidate,
                    head: next,
                    checkout: home.company_dir(&c.company),
                });
                continue;
            }
            let next_id = next.map(|m| (m.conversation.as_str(), m.message.as_str()));
            let checked_id = checked_head
                .as_ref()
                .map(|(conv, msg)| (conv.as_str(), msg.as_str()));
            if next_id != checked_id || checked_head.is_none() {
                if let Some(candidate) = next {
                    workers.check(CheckArgs {
                        adapter: Arc::clone(&adapter),
                        c: &c,
                        session: &session,
                        next: candidate,
                        head: next,
                        checkout: home.company_dir(&c.company),
                    });
                }
                continue;
            }
            if let Some(next) = next {
                workers.dispatch(DispatchArgs {
                    home,
                    admitted,
                    adapter: Arc::clone(&adapter),
                    c,
                    conversation: next.conversation.clone(),
                    message: next.message.clone(),
                });
            }
        } else {
            if matches!(workers.poll(&c), CheckedDelivery::DispatchFailed) {
                continue;
            }
            let open = open_messages(&state, &c)?;
            let next = next_message(&open, &c, clock()).filter(|candidate| {
                before_read.iter().any(|message| {
                    matches!(message.stage, Stage::Pending | Stage::Failed)
                        && message.conversation == candidate.conversation
                        && message.message == candidate.message
                })
            });
            if let Some(next) = next {
                workers.dispatch(DispatchArgs {
                    home,
                    admitted,
                    adapter: Arc::clone(&adapter),
                    c,
                    conversation: next.conversation.clone(),
                    message: next.message.clone(),
                });
            }
        }
    }
    if let Some(error) = outbox_error {
        return Err(error);
    }
    clear_idle_answer_failures(&state)?;
    Ok(())
}

const UNPOSTED: &str = "SELECT 1 FROM conversation_answers x WHERE x.company=conversation_messages.company AND x.connector=conversation_messages.connector AND x.conversation=conversation_messages.conversation AND (x.source_message=conversation_messages.message OR (x.source_message IS NULL AND x.turn=conversation_messages.turn)) AND x.posted_at IS NULL";

/// A message whose answer is not posted although a post of it was tried —
/// it failed, or its outcome was lost, a restart included — has its interval
/// opened from its turn's end (`now` for a turn ended before that end was
/// kept). A post that was not tried yet opens nothing: the healthy path never
/// shows the unavailable text.
fn unconfirmed(state: &RunnerState, c: &Conversation, now: Timestamp) -> Result<(), StateError> {
    state.write(|tx| {
        match c.session_scope {
            SessionScope::Conversation => tx.execute(
                &format!(
                    "UPDATE conversation_messages SET unavailable_since=coalesce(turn_ended_at,?4) WHERE company=?1 AND connector=?2 AND conversation=?3 AND state IN ('sent','answered','failed') AND unavailable_since IS NULL AND EXISTS({UNPOSTED} AND (x.attempted_at IS NOT NULL OR x.failure IS NOT NULL))"
                ),
                params![
                    c.company.as_str(),
                    c.connector,
                    c.conversation,
                    now.to_string()
                ],
            ),
            SessionScope::Company => tx.execute(
                &format!(
                    "UPDATE conversation_messages SET unavailable_since=coalesce(turn_ended_at,?3) WHERE company=?1 AND connector=?2 AND state IN ('sent','answered','failed') AND unavailable_since IS NULL AND {COMPANY_OWNED_MESSAGES} AND EXISTS({UNPOSTED} AND (x.attempted_at IS NOT NULL OR x.failure IS NOT NULL))"
                ),
                params![c.company.as_str(), c.connector, now.to_string()],
            ),
        }
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

/// Record at `now` whether the conversation relay runs: the cause of its
/// failure to start or of its end, kept until it runs again, with the start of
/// that failure kept across its retries.
pub fn relay_state(home: &Home, failure: Option<&str>, now: Timestamp) {
    let result = RunnerState::new(home).write::<_, StateError>(|tx| {
        tx.execute(
            "INSERT INTO conversation_relay(id,failure,updated_at) VALUES (1,?1,?2) ON CONFLICT(id) DO UPDATE SET failure=excluded.failure,updated_at=CASE WHEN excluded.failure IS NOT NULL AND conversation_relay.failure IS NOT NULL THEN conversation_relay.updated_at ELSE excluded.updated_at END",
            params![failure, now.to_string()],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        if failure.is_none() {
            let addresses = {
                let mut query = tx
                    .prepare("SELECT DISTINCT company,connector,conversation FROM conversation_refusals WHERE reason='unavailable'")
                    .map_err(state::sql(&state::db_path(tx)))?;
                query
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(state::sql(&state::db_path(tx)))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(state::sql(&state::db_path(tx)))?
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
