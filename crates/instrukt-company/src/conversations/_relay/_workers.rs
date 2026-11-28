//! Per-conversation native dispatch, live state polling, and presence checks for the relay.
use crate::connector_host::Connectors;
use instrukt_core::config::{Admitted, Env, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::keyed::{self, RuntimeInvocation};
use instrukt_runner::runtime::{
    FollowupAssignment, Presence, RuntimeAdapter, RuntimeError, RuntimeStart, SendEvidence, Thread,
};
use instrukt_runner::sessions::{SessionState, SessionsError};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use super::super::_availability::unavailable;
use super::super::_delivery::{
    LOCK_WAIT, NativeDelivery, deliver_one, fresh_rollover_brief, next_message,
};
use super::super::_intake::conversation_title;
use super::super::_store::{
    COMPANY_OWNED_MESSAGES, Conversation, Message, lock, open_messages, reread, set_failure,
    set_message_failure,
};
use super::super::{ConversationError, LOG_TARGET};

pub const ANSWER_OPERATOR_BOUND: SignedDuration = SignedDuration::from_mins(5);

/// Per-conversation native work. A check or state read takes no dispatch or
/// conversation lock. Once cleared to dispatch, only that conversation's
/// worker can send, start, succeed, or roll over until it returns.
pub(in crate::conversations) struct RelayWorkers {
    pub(in crate::conversations) connectors: Arc<dyn Connectors>,
    wake: tokio::sync::mpsc::Sender<()>,
    state_reads: Mutex<BTreeMap<String, StateReadJob>>,
    work: Mutex<BTreeMap<String, DeliveryWork>>,
    answer_anomalies: Mutex<BTreeSet<String>>,
    terminal_anomalies: Mutex<BTreeSet<(String, String, String, String)>>,
    env: Env,
}

struct StateReadJob {
    session: SessionRef,
    reply: Receiver<Result<Thread, RuntimeError>>,
    invalidated: bool,
}

pub(super) enum PolledState {
    Pending,
    Ready(Box<Result<Thread, RuntimeError>>),
}

#[derive(Clone)]
pub(super) struct DeliveryCandidate {
    pub(super) session: SessionRef,
    pub(super) conversation: String,
    pub(super) message: String,
    pub(super) head_conversation: Option<String>,
    pub(super) head: Option<String>,
    pub(super) boundary: Option<String>,
    pub(super) attempted: Option<Timestamp>,
    pub(super) invalidated: bool,
}

enum DeliveryWork {
    Checking {
        candidate: Box<DeliveryCandidate>,
        reply: Receiver<Result<DeliveryCheck, SessionsError>>,
    },
    Dispatching(Receiver<Result<(), ConversationError>>),
}

pub(in crate::conversations) enum DeliveryCheck {
    Accepted,
    Presence(Presence),
    Ready,
    Busy,
    RetrySend,
    RejectedSend,
    UnconfirmedSend,
}

pub(in crate::conversations) enum CheckedDelivery {
    Idle,
    Checking(Box<DeliveryCandidate>),
    Checked {
        candidate: Box<DeliveryCandidate>,
        result: Result<DeliveryCheck, Box<SessionsError>>,
    },
    Dispatching,
    DispatchFailed,
    /// The dispatch found the bound thread idle in another runtime project.
    Misplaced,
}

pub(super) struct CheckArgs<'a> {
    pub(super) adapter: Arc<dyn RuntimeAdapter>,
    pub(super) c: &'a Conversation,
    pub(super) session: &'a SessionRef,
    pub(super) next: &'a Message,
    pub(super) head: Option<&'a Message>,
    pub(super) checkout: std::path::PathBuf,
}

pub(super) struct DispatchArgs<'a> {
    pub(super) home: &'a Home,
    pub(super) admitted: &'a Admitted,
    pub(super) adapter: Arc<dyn RuntimeAdapter>,
    pub(super) c: Conversation,
    pub(super) conversation: String,
    pub(super) message: String,
}

impl RelayWorkers {
    pub(in crate::conversations) fn new(
        connectors: Arc<dyn Connectors>,
        wake: tokio::sync::mpsc::Sender<()>,
        env: Env,
    ) -> Self {
        Self {
            connectors,
            wake,
            env,
            state_reads: Mutex::default(),
            work: Mutex::default(),
            answer_anomalies: Mutex::default(),
            terminal_anomalies: Mutex::default(),
        }
    }

    pub(super) fn poll_state(
        &self,
        adapter: Arc<dyn RuntimeAdapter>,
        c: &Conversation,
        session: &SessionRef,
    ) -> PolledState {
        let mut reads = self
            .state_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(job) = reads.get_mut(&c.semantic_key) {
            if &job.session != session {
                job.invalidated = true;
            }
            match job.reply.try_recv() {
                Err(TryRecvError::Empty) => return PolledState::Pending,
                Ok(result) => {
                    let invalidated = job.invalidated;
                    reads.remove(&c.semantic_key);
                    if !invalidated {
                        return PolledState::Ready(Box::new(result));
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    reads.remove(&c.semantic_key);
                }
            }
        }
        let target = session.clone();
        let reply = self.spawn(move || adapter.read_state(&target));
        reads.insert(
            c.semantic_key.clone(),
            StateReadJob {
                session: session.clone(),
                reply,
                invalidated: false,
            },
        );
        PolledState::Pending
    }

    pub(super) fn invalidate_state_read(&self, c: &Conversation) {
        if let Some(job) = self
            .state_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&c.semantic_key)
        {
            job.invalidated = true;
        }
    }

    pub(super) fn report_terminal_anomaly(
        &self,
        admitted: &Admitted,
        c: &Conversation,
        conversation: &str,
        message: &str,
    ) {
        let dedup_key = (
            c.company.to_string(),
            c.connector.clone(),
            conversation.to_owned(),
            message.to_owned(),
        );
        if self
            .terminal_anomalies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(dedup_key)
        {
            let reference = c
                .session
                .as_ref()
                .map_or_else(|| c.semantic_key.clone(), ToString::to_string);
            let config = admitted.global().clone();
            let env = self.env.clone();
            drop(
                self.spawn(move || {
                    instrukt_runner::anomaly::notify(&config, &env, &reference, None)
                }),
            );
        }
    }

    pub(super) fn report_answer_failure(
        &self,
        admitted: &Admitted,
        state: &RunnerState,
        c: &Conversation,
        unread: bool,
        now: Timestamp,
    ) -> Result<(), StateError> {
        let cutoff = (now - ANSWER_OPERATOR_BOUND).to_string();
        let due = state
            .read(|conn, path| {
                match c.session_scope {
                    SessionScope::Conversation => conn.query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3
                            AND superseded_at IS NULL AND terminal_error_queued_at IS NULL AND (
                                (state='failed' AND failed_turns>=2)
                                OR ((state='sent' AND ?4)
                                    AND julianday(unavailable_since)<=julianday(?5))
                            )
                        )",
                        params![c.company.as_str(), c.connector, c.conversation, unread, cutoff],
                        |row| row.get::<_, bool>(0),
                    ),
                    SessionScope::Company => conn.query_row(
                        &format!(
                            "SELECT EXISTS(
                                SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2
                                AND superseded_at IS NULL AND terminal_error_queued_at IS NULL AND {COMPANY_OWNED_MESSAGES} AND (
                                    (state='failed' AND failed_turns>=2)
                                    OR ((state='sent' AND ?3)
                                        AND julianday(unavailable_since)<=julianday(?4))
                                )
                            )"
                        ),
                        params![c.company.as_str(), c.connector, unread, cutoff],
                        |row| row.get::<_, bool>(0),
                    ),
                }
                .map_err(state::sql(path))
            })?
            .unwrap_or(false);
        if due
            && self
                .answer_anomalies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(c.semantic_key.clone())
        {
            let reference = c
                .session
                .as_ref()
                .map_or_else(|| c.semantic_key.clone(), ToString::to_string);
            let config = admitted.global().clone();
            let env = self.env.clone();
            drop(
                self.spawn(move || {
                    instrukt_runner::anomaly::notify(&config, &env, &reference, None)
                }),
            );
        }
        Ok(())
    }

    pub(super) fn is_dispatching(&self, c: &Conversation) -> bool {
        let mut work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        let key = &c.semantic_key;
        match work.get(key) {
            Some(DeliveryWork::Dispatching(reply)) => match reply.try_recv() {
                Err(TryRecvError::Empty) => true,
                Ok(result) => {
                    // Put back the completed result so `poll()` consumes it on this pass.
                    let (tx, rx) = std::sync::mpsc::channel();
                    let _ = tx.send(result);
                    work.insert(key.clone(), DeliveryWork::Dispatching(rx));
                    false
                }
                Err(TryRecvError::Disconnected) => false,
            },
            _ => false,
        }
    }

    pub(super) fn is_checking(&self, c: &Conversation) -> bool {
        let work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        matches!(
            work.get(&c.semantic_key),
            Some(DeliveryWork::Checking { .. })
        )
    }

    pub(super) fn poll(&self, c: &Conversation) -> CheckedDelivery {
        let mut work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        let key = &c.semantic_key;
        match work.get(key) {
            Some(DeliveryWork::Checking { candidate, reply }) => {
                let result = match reply.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => {
                        return CheckedDelivery::Checking(candidate.clone());
                    }
                    Err(TryRecvError::Disconnected) => {
                        work.remove(key);
                        return CheckedDelivery::Idle;
                    }
                };
                let check = CheckedDelivery::Checked {
                    candidate: candidate.clone(),
                    result: result.map_err(Box::new),
                };
                work.remove(key);
                check
            }
            Some(DeliveryWork::Dispatching(reply)) => {
                let result = match reply.try_recv() {
                    Err(TryRecvError::Empty) => return CheckedDelivery::Dispatching,
                    Ok(Ok(())) => CheckedDelivery::Idle,
                    Ok(Err(ConversationError::Sessions(SessionsError::ThreadMisplaced {
                        idle: true,
                        ..
                    })))
                    | Ok(Err(ConversationError::Runtime(RuntimeError::ThreadMisplaced {
                        idle: true,
                        ..
                    }))) => CheckedDelivery::Misplaced,
                    Ok(Err(error)) => {
                        tracing::warn!(target: LOG_TARGET, event = "conversation_worker_failed", key = %c.semantic_key, %error);
                        CheckedDelivery::DispatchFailed
                    }
                    Err(TryRecvError::Disconnected) => {
                        tracing::warn!(target: LOG_TARGET, event = "conversation_worker_disconnected", key = %c.semantic_key);
                        CheckedDelivery::DispatchFailed
                    }
                };
                work.remove(key);
                result
            }
            None => CheckedDelivery::Idle,
        }
    }

    pub(super) fn invalidate_check(&self, c: &Conversation) {
        if let Some(DeliveryWork::Checking { candidate, .. }) = self
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&c.semantic_key)
        {
            candidate.invalidated = true;
        }
    }

    pub(super) fn check(&self, args: CheckArgs<'_>) {
        let CheckArgs {
            adapter,
            c,
            session,
            next,
            head,
            checkout,
        } = args;
        let key = c.semantic_key.clone();
        let target = session.clone();
        let attempted = next.attempted;
        let command_id = next.dispatch_id.clone();
        let input = next.content.clone();
        let reply = self.spawn(move || {
            if attempted.is_some() {
                let Some(input) = input else {
                    return Ok(DeliveryCheck::UnconfirmedSend);
                };
                return Ok(
                    match adapter
                        .observe_send(&target, &input, command_id.as_deref())
                        .map_err(SessionsError::Runtime)?
                    {
                        SendEvidence::Accepted => DeliveryCheck::Accepted,
                        SendEvidence::Rejected => DeliveryCheck::RejectedSend,
                        SendEvidence::Presence(presence) => DeliveryCheck::Presence(presence),
                        SendEvidence::IdleWithoutInput if command_id.is_some() => {
                            DeliveryCheck::RetrySend
                        }
                        SendEvidence::IdleWithoutInput => DeliveryCheck::UnconfirmedSend,
                        SendEvidence::Unresolved => DeliveryCheck::UnconfirmedSend,
                    },
                );
            }
            let thread = adapter
                .read_state(&target)
                .map_err(SessionsError::Runtime)?;
            if thread.presence != Presence::Present {
                return Ok(DeliveryCheck::Presence(thread.presence));
            }
            // A thread in another runtime project is never served, busy or
            // not: its outcome says whether it can roll over now.
            adapter
                .verify_thread_project(&checkout, &thread, Some(&key))
                .map_err(SessionsError::Runtime)?;
            Ok(
                if matches!(
                    thread.state,
                    SessionState::InputReady | SessionState::EndedFailed
                ) {
                    DeliveryCheck::Ready
                } else {
                    DeliveryCheck::Busy
                },
            )
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                c.semantic_key.clone(),
                DeliveryWork::Checking {
                    candidate: Box::new(DeliveryCandidate {
                        session: session.clone(),
                        conversation: next.conversation.clone(),
                        message: next.message.clone(),
                        head_conversation: head.map(|head| head.conversation.clone()),
                        head: head.map(|head| head.message.clone()),
                        boundary: next.boundary.clone(),
                        attempted: next.attempted,
                        invalidated: false,
                    }),
                    reply,
                },
            );
    }

    pub(super) fn dispatch(&self, args: DispatchArgs<'_>) {
        let DispatchArgs {
            home,
            admitted,
            adapter,
            c,
            conversation,
            message,
        } = args;
        let key = c.semantic_key.clone();
        let home = home.clone();
        let admitted = admitted.clone();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || {
            let state = RunnerState::new(&home);
            let outcome = (|| -> Result<(), ConversationError> {
                let ready = if c.session.is_some() {
                    RuntimeInvocation::try_acquire(&home)
                } else {
                    Ok(None)
                };
                let invocation = match ready.and_then(|ready| match ready {
                    Some(invocation) => Ok(Some(invocation)),
                    None => RuntimeInvocation::acquire(&home, LOCK_WAIT).map(|invocation| {
                        // A bound thread needs a fresh readiness check after
                        // waiting for dispatch. Do not hold this lock across it.
                        c.session.is_none().then_some(invocation)
                    }),
                }) {
                    Ok(Some(invocation)) => invocation,
                    Ok(None) => return Ok(()),
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "conversation_dispatch_unavailable", error = %error);
                        unavailable(&state, &c, &[(conversation.clone(), message.clone())], connectors.now())?;
                        return Err(error.into());
                    }
                };
                let _lock = match lock(&home, &c) {
                    Ok(lock) => lock,
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "conversation_lock_unavailable", company = %c.company, connector = %c.connector, key = %c.semantic_key, %error);
                        unavailable(&state, &c, &[(conversation.clone(), message.clone())], connectors.now())?;
                        return Err(error);
                    }
                };
                let Some(current) =
                    reread(&state, &c)?.filter(|current| current.session == c.session)
                else {
                    return Ok(());
                };
                let clock = || connectors.now();
                let delivery = NativeDelivery {
                    home: &home,
                    clock: &clock,
                };
                let _key = current
                    .session
                    .as_ref()
                    .map(|_| {
                        keyed::KeyGuard::claim(
                            &invocation,
                            adapter.as_ref(),
                            &current.semantic_key,
                            None,
                            connectors.now(),
                        )
                    })
                    .transpose()?;
                if let Some(session) = &current.session
                    && keyed::replacement_binding(
                        &home,
                        adapter.as_ref(),
                        &current.semantic_key,
                        session,
                    )?
                    .is_some()
                {
                    return Ok(());
                }
                // Intake can change the head while delivery waits for the lock.
                let open = open_messages(&state, &current)?;
                let Some(next) = next_message(&open, &current, connectors.now())
                    .filter(|next| next.conversation == conversation && next.message == message)
                else {
                    return Ok(());
                };
                deliver_one(
                    &invocation,
                    &admitted,
                    adapter.as_ref(),
                    &current,
                    next,
                    &delivery,
                )
            })();
            if let Err(ref error) = outcome {
                let cause = match error {
                    ConversationError::Runtime(native) => adapter
                        .diagnostic("conversation delivery", native)
                        .to_string(),
                    ConversationError::Sessions(SessionsError::Runtime(native)) => adapter
                        .diagnostic("conversation delivery", native)
                        .to_string(),
                    other => other.to_string(),
                };
                tracing::warn!(target: LOG_TARGET, event = "conversation_dispatch_worker_error", company = %c.company, connector = %c.connector, key = %c.semantic_key, %cause);
                let _ = set_message_failure(&state, &c, &conversation, &message, Some(&cause));
                let _ = unavailable(&state, &c, &[(conversation, message)], connectors.now());
            }
            outcome
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    pub(super) fn succeed(
        &self,
        home: &Home,
        admitted: &Admitted,
        adapter: Arc<dyn RuntimeAdapter>,
        c: Conversation,
    ) {
        let home = home.clone();
        let admitted = admitted.clone();
        let key = c.semantic_key.clone();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || -> Result<(), ConversationError> {
            let state = RunnerState::new(&home);
            let outcome = (|| -> Result<(), ConversationError> {
                let _lock = lock(&home, &c)?;
                let Some(current) =
                    reread(&state, &c)?.filter(|current| current.session == c.session)
                else {
                    return Ok(());
                };
                let invocation = RuntimeInvocation::acquire(&home, LOCK_WAIT)?;
                let title = state
                    .read(|conn, path| {
                        conn.query_row(
                            "SELECT display_title FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3",
                            params![c.company.as_str(), c.connector, c.conversation],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .map_err(state::sql(path))
                    })?
                    .flatten()
                    .unwrap_or_else(|| conversation_title(c.company.as_str(), None, ""));
                let mut request = match current.request.as_deref() {
                    Some(raw) => serde_json::from_str::<RuntimeStart>(raw)?,
                    None => keyed::request(&home, adapter.as_ref(), &current.semantic_key)?,
                };
                let original = request.clone();
                request.title = title;
                let replace_brief = if current.needs_explicit_rollover {
                    request.brief = fresh_rollover_brief(&home, &admitted, &current)?;
                    true
                } else {
                    false
                };
                let session = current.session.as_ref().ok_or_else(|| {
                    ConversationError::Refused("successor has no owner-retained native ID".into())
                })?;
                keyed::succeed(
                    adapter.as_ref(),
                    &FollowupAssignment {
                        bindings: None,
                        original_request: Some(&original),
                        invocation: &invocation,
                        request: &request,
                        session,
                        worktree_path: None,
                        project: None,
                        now: connectors.now(),
                        replace_brief,
                    },
                )?;
                Ok(())
            })();
            if let Err(ref error) = outcome {
                let cause = error.to_string();
                tracing::warn!(target: LOG_TARGET, event = "conversation_succeed_failed", company = %c.company, connector = %c.connector, key = %c.semantic_key, %cause);
                let _ = set_failure(&state, &c, Some(&cause));
                if let Ok(waiting) = open_messages(&state, &c) {
                    let keys = waiting
                        .into_iter()
                        .map(|message| (message.conversation, message.message))
                        .collect::<Vec<_>>();
                    let _ = unavailable(&state, &c, &keys, connectors.now());
                }
            }
            outcome
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    pub(super) fn rollover(
        &self,
        home: &Home,
        admitted: &Admitted,
        adapter: Arc<dyn RuntimeAdapter>,
        c: Conversation,
    ) {
        let home = home.clone();
        let admitted = admitted.clone();
        let key = c.semantic_key.clone();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || -> Result<(), ConversationError> {
            let state = RunnerState::new(&home);
            let outcome = (|| -> Result<(), ConversationError> {
                let _lock = lock(&home, &c)?;
                let Some(current) = reread(&state, &c)?
                    .filter(|current| current.session == c.session && current.needs_explicit_rollover)
                else {
                    return Ok(());
                };
                let invocation = RuntimeInvocation::acquire(&home, LOCK_WAIT)?;
                let title = state
                    .read(|conn, path| {
                        conn.query_row(
                            "SELECT display_title FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3",
                            params![c.company.as_str(), c.connector, c.conversation],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .map_err(state::sql(path))
                    })?
                    .flatten()
                    .unwrap_or_else(|| conversation_title(c.company.as_str(), None, ""));
                let mut request = match current.request.as_deref() {
                    Some(raw) => serde_json::from_str::<RuntimeStart>(raw)?,
                    None => keyed::request(&home, adapter.as_ref(), &current.semantic_key)?,
                };
                let original = request.clone();
                request.title = title;
                request.brief = fresh_rollover_brief(&home, &admitted, &current)?;
                let session = current.session.as_ref().ok_or_else(|| {
                    ConversationError::Refused("rollover has no owner-retained native ID".into())
                })?;
                keyed::rollover_present_idle(
                    adapter.as_ref(),
                    &FollowupAssignment {
                        bindings: None,
                        original_request: Some(&original),
                        invocation: &invocation,
                        request: &request,
                        session,
                        worktree_path: None,
                        project: None,
                        now: connectors.now(),
                        replace_brief: true,
                    },
                )?;
                Ok(())
            })();
            if let Err(ref error) = outcome {
                let cause = error.to_string();
                tracing::warn!(target: LOG_TARGET, event = "conversation_rollover_failed", company = %c.company, connector = %c.connector, key = %c.semantic_key, %cause);
                let _ = set_failure(&state, &c, Some(&cause));
                if let Ok(waiting) = open_messages(&state, &c) {
                    let keys = waiting
                        .into_iter()
                        .map(|message| (message.conversation, message.message))
                        .collect::<Vec<_>>();
                    let _ = unavailable(&state, &c, &keys, connectors.now());
                }
            }
            outcome
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    fn spawn<T: Send + 'static>(&self, run: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
        let (send, reply) = mpsc::channel();
        let wake = self.wake.clone();
        if let Err(error) = std::thread::Builder::new().spawn(move || {
            let _ = send.send(run());
            let _ = wake.try_send(());
        }) {
            tracing::error!(target: LOG_TARGET, event = "conversation_worker_spawn_failed", %error);
        }
        reply
    }
}
