//! Run the finished-turn relay and post its durable answer outbox.
use crate::connector_host::Connectors;
use instrukt_core::config::{Admitted, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::interfaces::CompanyRelayPass;
use instrukt_runner::keyed;
use instrukt_runner::runtime::conversations::ConversationSource;
use instrukt_runner::runtime::{Presence, RuntimeAdapter};
use instrukt_runner::sessions::SessionsError;
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::Timestamp;
use std::collections::BTreeSet;
use std::sync::PoisonError;

use super::_availability::{retire_notices, unavailable};
use super::_delivery::{
    accepted, adopt_replacement, clear_rejected_send, next_message, record_delivery_failure,
    require_source, schedule_earlier_failures, send_reconciliation_due, send_unknown,
    stale_acknowledgement,
};
use super::_store::{
    COMPANY_OWNED_ANSWERS, COMPANY_OWNED_MESSAGES, Conversation, Message, Stage,
    clear_idle_answer_failures, conversations, lock, open_messages, prune_superseded_failures,
    relay_stages, relay_work_filter, reread, set_failure, set_message_failure,
};
use super::_turns::{harvest, observe};
use super::{ConversationError, LOG_TARGET};
use _workers::{CheckedDelivery, DeliveryCheck};

mod _workers;

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
pub(super) fn relay(
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
