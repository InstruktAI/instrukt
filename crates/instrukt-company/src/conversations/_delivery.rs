//! Bind and deliver addressed messages to their native sessions.
use instrukt_connector::Accompanying;
use instrukt_core::config::{Admitted, Agents, COMPANY_AGENT_ROLE, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::{RoleName, SessionRef, SnippetId};
use instrukt_core::index::{GlobalIndex, IndexEntry, RoleProcedure};
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::keyed::{self, RuntimeInvocation};
use instrukt_runner::runtime::{
    DispatchError, Presence, RuntimeAdapter, RuntimeAssignment, RuntimeError, RuntimeStart,
};
use instrukt_runner::sessions::SessionsError;
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use std::path::PathBuf;

use super::_availability::{retire_notices, unavailable};
use super::_envelopes::framed;
use super::_intake::{context_exclusions, conversation_title};
use super::_store::{
    COMPANY_OWNED_MESSAGES, Conversation, Message, Stage, remember_delivery, set_message_failure,
};
use super::envelopes::Span;
use super::{ConversationError, LOG_TARGET, envelopes};

/// The home and clock under which this native delivery was prepared.
pub(super) struct NativeDelivery<'a> {
    pub(super) home: &'a Home,
    pub(super) clock: &'a (dyn Fn() -> Timestamp + Sync),
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
pub(super) fn opening_procedure<'a>(
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
pub(super) fn opening_procedure_for_role<'a>(
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
    let reply_line = match c.session_scope {
        SessionScope::Conversation => {
            "\nReply mode: explicit (raw turn output is not posted; send replies with `instrukt company reply --text <text>`, and optionally publish short active-turn progress with `instrukt company status --text <text>`)"
        }
        SessionScope::Company => {
            "\nReply mode: explicit (raw turn output is not posted; send replies with `instrukt company reply --conversation <id> --text <text>`, and optionally publish short active-turn progress with `instrukt company status --conversation <id> --text <text>`)"
        }
    };
    Ok(instrukt_runner::control::Control::Assignment.frame(&format!(
        "instrukt conversation assignment\nProcedure: {procedure}\nRole: {role}\nResolved role capabilities: {capabilities}\nCompany: {}\nConnector: {}\n{scope_line}{reply_line}\nLoad `instrukt docs get {procedure} --closure` before handling the member's message.\n",
        c.company, c.connector
    )))
}

/// Build a fresh explicit-frontstage opening assignment brief for a successor
/// generation rolling over a pre-upgrade `turn_output` binding.
pub(super) fn fresh_rollover_brief(
    home: &Home,
    admitted: &Admitted,
    c: &Conversation,
) -> Result<String, ConversationError> {
    let index = GlobalIndex::load_readable(home)?.unwrap_or_default();
    let procedure = opening_procedure_for_role(&index, &c.role, admitted.agents())?
        .id
        .clone();
    assignment(admitted, c, &procedure)
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
            body.push_str(&format!(
                "Reply command: instrukt company reply --conversation {safe_conversation} --text <text>\n"
            ));
            let header = instrukt_runner::control::Control::Notification.frame(&body);
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

/// The runtime accepted `message` as its own turn, which ends its no-work
/// interval; a first message's acceptance also binds the conversation's
/// session and records Turn 1's deterministic launch `start_command_id`
/// (leaving `dispatch_write_at = NULL`).
pub(super) fn accepted(
    state: &RunnerState,
    c: &Conversation,
    conversation: &str,
    message: &str,
    boundary: Option<&str>,
    session: Option<&SessionRef>,
    start_command_id: Option<&str>,
) -> Result<bool, StateError> {
    let applied = state.write(|tx| {
        let path = state::db_path(tx);
        let changed = tx.execute(
            "UPDATE conversation_messages
             SET state='sent',failure=NULL,unavailable_since=NULL,retry_at=NULL,send_unknown_at=NULL,
                 observed_run_id=NULL,turn_started_at=NULL,
                 dispatch_command_id=coalesce(dispatch_command_id,?6)
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
               AND boundary=?5 AND attempted_at IS NOT NULL
               AND state IN ('pending','failed') AND superseded_at IS NULL",
            params![c.company.as_str(), c.connector, conversation, message, boundary, start_command_id],
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

pub(super) fn stale_acknowledgement(c: &Conversation, message: &str) {
    tracing::warn!(target: LOG_TARGET, event = "conversation_stale_acknowledgement", key = %c.semantic_key, message);
}

/// Initial backoff delay before retrying a failed addressed turn.
pub const FAILED_RETRY_DELAY: SignedDuration = SignedDuration::from_mins(2);
/// Maximum backoff delay between failed-turn retries.
pub const FAILED_RETRY_LIMIT: SignedDuration = SignedDuration::from_mins(15);
/// Maximum number of retries after the initial attempt (4 total attempts: initial + 3 retries).
pub const FAILED_RETRY_MAX_ATTEMPTS: u32 = 3;

const SEND_RECONCILE_BOUND: SignedDuration = SignedDuration::from_mins(5);

pub(super) fn retry_delay(previous_failures: u32) -> SignedDuration {
    (FAILED_RETRY_DELAY * (1_i32 << previous_failures.min(3))).min(FAILED_RETRY_LIMIT)
}

/// A failed turn with a ready retry still yields to new messages.
pub(super) fn next_message<'a>(
    open: &'a [Message],
    c: &Conversation,
    now: Timestamp,
) -> Option<&'a Message> {
    let available = || {
        open.iter()
            .filter(|message| !message.send_unknown && message.terminal_error_queued_at.is_none())
    };
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

pub(super) fn schedule_earlier_failures(
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
                   AND superseded_at IS NULL AND send_unknown_at IS NULL AND retry_at IS NULL
                   AND boundary IS NOT NULL AND terminal_error_queued_at IS NULL",
                params![c.company.as_str(), c.connector, c.conversation, retry],
            ),
            SessionScope::Company => tx.execute(
                &format!(
                    "UPDATE conversation_messages SET retry_at=?3,failed_turns=1
                     WHERE company=?1 AND connector=?2 AND state='failed'
                       AND superseded_at IS NULL AND send_unknown_at IS NULL AND retry_at IS NULL
                       AND boundary IS NOT NULL AND terminal_error_queued_at IS NULL
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
pub(super) fn deliver_one(
    invocation: &RuntimeInvocation,
    admitted: &Admitted,
    adapter: &dyn RuntimeAdapter,
    c: &Conversation,
    next: &Message,
    delivery: &NativeDelivery<'_>,
) -> Result<(), ConversationError> {
    let state = &RunnerState::new(invocation.home());
    let mut tracked = next.clone();
    let result = match prepare_delivery(state, c, next) {
        Ok(None) => return Ok(()),
        Err(error) => Err(error),
        Ok(Some(prepared)) => {
            tracked = prepared;
            match &c.session {
                None => start(invocation, admitted, adapter, c, &tracked, delivery),
                Some(session) => send(adapter, state, c, session, &mut tracked, delivery),
            }
        }
    };
    record_delivery_failure(adapter, state, c, &tracked, delivery.clock, &result)?;
    result
}

pub(super) fn record_delivery_failure(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    next: &Message,
    clock: &dyn Fn() -> Timestamp,
    result: &Result<(), ConversationError>,
) -> Result<(), ConversationError> {
    // A runtime failure is retained through the adapter's safe diagnostic,
    // never its raw native output.
    let failure = result.as_ref().err().map(|error| match error {
        ConversationError::Runtime(native) => adapter
            .diagnostic("conversation delivery", native)
            .to_string(),
        ConversationError::Sessions(SessionsError::Runtime(native)) => adapter
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

fn start(
    invocation: &RuntimeInvocation,
    admitted: &Admitted,
    adapter: &dyn RuntimeAdapter,
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
                title: state
                    .read(|conn, path| {
                        conn.query_row(
                            "SELECT display_title FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3",
                            params![c.company.as_str(), c.connector, c.conversation],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .map_err(state::sql(path))
                    })?
                    .flatten()
                    .unwrap_or(default_title),
                key: c.semantic_key.clone(),
                brief: format!("{}\n{input}", assignment(admitted, c, &procedure)?),
                worktree: None,
                launch_modes: None,
                native_launch: None,
                profiles: instrukt_runner::runtime::Profiles(Some(admitted.shared_agents())),
            };
            state.write::<_, StateError>(|tx| {
                let path = state::db_path(tx);
                tx.execute(
                    "UPDATE connected_conversations SET opening=?4,opening_conversation=?6,request=?5 WHERE company=?1 AND connector=?2 AND conversation=?3",
                    params![
                        c.company.as_str(),
                        c.connector,
                        c.conversation,
                        next.message,
                        serde_json::to_string(&request).map_err(|e| state::invalid(&path, e))?,
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
    let thread = keyed::start(
        adapter,
        &RuntimeAssignment {
            bindings: None,
            invocation,
            project: None,
            request: &request,
            existing_worktree: None,
            now,
        },
    )?;
    let start_cmd = keyed::start_command_id(state, adapter.runtime_name(), &c.semantic_key)?;
    accepted(
        state,
        c,
        &next.conversation,
        &next.message,
        next.boundary.as_deref(),
        Some(&thread.session),
        start_cmd.as_deref(),
    )?;
    tracing::info!(target: LOG_TARGET, event = "conversation_session_started", company = %c.company, connector = %c.connector, key = %c.semantic_key, session = %thread.session);
    Ok(())
}

fn is_rejected_send(
    adapter: &dyn RuntimeAdapter,
    session: &SessionRef,
    error: &ConversationError,
) -> bool {
    match error {
        ConversationError::Runtime(err)
        | ConversationError::Sessions(SessionsError::Runtime(err)) => {
            matches!(err, RuntimeError::CommandPreviouslyRejected { .. })
                || adapter.send_certainly_rejected(session, err)
        }
        _ => false,
    }
}

pub(super) fn send(
    adapter: &dyn RuntimeAdapter,
    state: &RunnerState,
    c: &Conversation,
    session: &SessionRef,
    next: &mut Message,
    delivery: &NativeDelivery<'_>,
) -> Result<(), ConversationError> {
    let clock = delivery.clock;
    let now = clock();
    let content = next
        .content
        .clone()
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
    if let Some(id) = next.dispatch_id.clone() {
        if let Err(error) =
            dispatch_frame(adapter, state, c, next, session, (&id, &content), delivery)
        {
            if is_rejected_send(adapter, session, &error)
                && let Some(boundary) = next.boundary.as_deref()
            {
                clear_rejected_send(state, c, &next.conversation, &next.message, boundary)?;
                next.dispatch_write_at = None;
                next.attempted = None;
                next.dispatch_id = None;
            }
            return Err(error);
        }
        accepted(
            state,
            c,
            &next.conversation,
            &next.message,
            next.boundary.as_deref(),
            None,
            None,
        )?;
        return Ok(());
    }
    let old_boundary = next
        .boundary
        .as_deref()
        .ok_or_else(|| ConversationError::Refused("the message has no delivery boundary".into()))?;
    let boundary = envelopes::delivery_boundary(&[&content])?;
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
            params![
                c.company.as_str(),
                c.connector,
                next.conversation,
                next.message,
                now.to_string(),
                content,
                boundary,
                old_boundary,
                id
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))
    })?;
    if recorded != 0 {
        next.boundary = Some(boundary.clone());
        next.content = Some(content.clone());
        next.attempted = Some(now);
        next.dispatch_id = Some(id.clone());
        if let Err(error) =
            dispatch_frame(adapter, state, c, next, session, (&id, &content), delivery)
        {
            if is_rejected_send(adapter, session, &error) {
                clear_rejected_send(state, c, &next.conversation, &next.message, &boundary)?;
                next.dispatch_write_at = None;
                next.attempted = None;
                next.dispatch_id = None;
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
            None,
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
            )
            .map_err(|err| RuntimeError::Refused(err.to_string()))?;
            marked = true;
        }
        Ok(())
    });
    match result {
        Ok(()) => Ok(()),
        Err(DispatchError::BeforeWrite(error) | DispatchError::BeforeFrame(error)) => {
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

pub(super) fn clear_rejected_send(
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
               AND state IN ('pending','failed') AND send_unknown_at IS NULL",
            params![
                c.company.as_str(),
                c.connector,
                conversation,
                message,
                boundary
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

pub(super) fn send_unknown(
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

pub(super) fn possible_dispatch_at(message: &Message) -> Option<Timestamp> {
    if message.dispatch_id.is_some() {
        message.dispatch_write_at
    } else {
        message.attempted
    }
}

pub(super) fn send_reconciliation_due(message: &Message, now: Timestamp) -> bool {
    !message.send_unknown
        && matches!(message.stage, Stage::Pending | Stage::Failed)
        && possible_dispatch_at(message).is_some_and(|at| now >= at + SEND_RECONCILE_BOUND)
}

/// How long the relay waits for a conversation's lock or the runtime's
/// dispatch lock before it leaves that work for its next pass.
pub(super) const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

pub(super) fn adopt_replacement(
    home: &Home,
    state: &RunnerState,
    adapter: &dyn RuntimeAdapter,
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
        let changed_session = successor != *current;
        state.write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            tx.execute(
                "UPDATE connected_conversations
                 SET session=?4,
                     needs_explicit_rollover=CASE WHEN ?5 THEN 0 ELSE needs_explicit_rollover END
                 WHERE company=?1 AND connector=?2 AND conversation=?3",
                params![
                    c.company.as_str(),
                    c.connector,
                    c.conversation,
                    successor.to_string(),
                    changed_session
                ],
            )
            .map_err(state::sql(&path))?;
            match c.session_scope {
                SessionScope::Conversation => {
                    tx.execute(
                        "UPDATE conversation_messages
                         SET state=?4,attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,
                             observed_run_id=NULL,turn_started_at=NULL,failure=NULL
                         WHERE company=?1 AND connector=?2 AND conversation=?3
                           AND state IN (?4,?5) AND send_unknown_at IS NULL",
                        params![
                            c.company.as_str(),
                            c.connector,
                            c.conversation,
                            Stage::Pending.as_str(),
                            Stage::Sent.as_str()
                        ],
                    )
                    .map_err(state::sql(&path))?;
                    tx.execute(
                        "UPDATE conversation_messages
                         SET attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,
                             observed_run_id=NULL,turn_started_at=NULL
                         WHERE company=?1 AND connector=?2 AND conversation=?3
                           AND state='failed' AND superseded_at IS NULL AND send_unknown_at IS NULL",
                        params![c.company.as_str(), c.connector, c.conversation],
                    )
                    .map_err(state::sql(&path))?;
                }
                SessionScope::Company => {
                    tx.execute(
                        &format!(
                            "UPDATE conversation_messages
                             SET state=?3,attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,
                                 observed_run_id=NULL,turn_started_at=NULL,failure=NULL
                             WHERE company=?1 AND connector=?2
                               AND state IN (?3,?4) AND send_unknown_at IS NULL AND {COMPANY_OWNED_MESSAGES}"
                        ),
                        params![
                            c.company.as_str(),
                            c.connector,
                            Stage::Pending.as_str(),
                            Stage::Sent.as_str()
                        ],
                    )
                    .map_err(state::sql(&path))?;
                    tx.execute(
                        &format!(
                            "UPDATE conversation_messages
                             SET attempted_at=NULL,dispatch_command_id=NULL,dispatch_write_at=NULL,
                                 observed_run_id=NULL,turn_started_at=NULL
                             WHERE company=?1 AND connector=?2
                               AND state='failed' AND superseded_at IS NULL AND send_unknown_at IS NULL
                               AND {COMPANY_OWNED_MESSAGES}"
                        ),
                        params![c.company.as_str(), c.connector],
                    )
                    .map_err(state::sql(&path))?;
                }
            }
            Ok(())
        })?;
        c.session = Some(successor);
        if changed_session {
            c.needs_explicit_rollover = false;
        }
    }
    Ok(None)
}
