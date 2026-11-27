//! Per-conversation native dispatch and presence checks for the relay.
use crate::connector_host::Connectors;
use instrukt_core::config::{Admitted, Env, SessionScope};
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::state_db::rusqlite::params;
use instrukt_runner::keyed::{self, RuntimeInvocation};
use instrukt_runner::runtime::conversations::ConversationSource;
use instrukt_runner::runtime::{Presence, RuntimeAdapter, RuntimeStart, SendEvidence};
use instrukt_runner::sessions::{SessionState, SessionsError};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use super::super::_availability::unavailable;
use super::super::_delivery::{
    LOCK_WAIT, NativeDelivery, OpeningSource, deliver_one, next_message, opening_position,
};
use super::super::_intake::conversation_title;
use super::super::_store::{
    COMPANY_OWNED_MESSAGES, Conversation, Message, lock, open_messages, reread, set_failure,
};
use super::super::{ConversationError, LOG_TARGET};

const ANSWER_OPERATOR_BOUND: SignedDuration = SignedDuration::from_mins(5);

/// Per-conversation native work. A check takes no dispatch or conversation
/// lock. Once cleared to dispatch, only that conversation's worker can send
/// or start a turn until it returns.
pub(in crate::conversations) struct RelayWorkers {
    pub(in crate::conversations) connectors: Arc<dyn Connectors>,
    wake: tokio::sync::mpsc::Sender<()>,
    probes: Mutex<BTreeMap<String, MissingProbe>>,
    absence_reads: Mutex<BTreeMap<String, AbsenceRead>>,
    work: Mutex<BTreeMap<String, DeliveryWork>>,
    answer_anomalies: Mutex<BTreeSet<String>>,
    env: Env,
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
    pub(super) source_presence: Presence,
    pub(super) source_identity: Option<String>,
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
            probes: Mutex::default(),
            absence_reads: Mutex::default(),
            work: Mutex::default(),
            answer_anomalies: Mutex::default(),
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
        let due = state.read(|conn, path| match c.session_scope {
            SessionScope::Conversation => conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3
                    AND superseded_at IS NULL AND (
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
                        AND superseded_at IS NULL AND {COMPANY_OWNED_MESSAGES} AND (
                            (state='failed' AND failed_turns>=2)
                            OR ((state='sent' AND ?3)
                                AND julianday(unavailable_since)<=julianday(?4))
                        )
                    )"
                ),
                params![c.company.as_str(), c.connector, unread, cutoff],
                |row| row.get::<_, bool>(0),
            ),
        }.map_err(state::sql(path)))?.unwrap_or(false);
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
                    }))) => CheckedDelivery::Misplaced,
                    _ => CheckedDelivery::DispatchFailed,
                };
                work.remove(key);
                result
            }
            None => CheckedDelivery::Idle,
        }
    }

    pub(super) fn update_check_source(&self, c: &Conversation, observed: Option<Presence>) {
        if let Some(DeliveryWork::Checking { candidate, .. }) = self
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&c.semantic_key)
        {
            candidate.invalidated |= c.source_identity != candidate.source_identity
                || c.session.as_ref() != Some(&candidate.session)
                || observed != Some(candidate.source_presence);
        }
    }

    // The check carries every fact its worker thread needs, read by the
    // pass that starts it; grouping them would only rename the list.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn check(
        &self,
        adapter: &dyn RuntimeAdapter,
        c: &Conversation,
        session: &SessionRef,
        next: &Message,
        head: Option<&Message>,
        source_presence: Presence,
        checkout: std::path::PathBuf,
    ) {
        let runtime = adapter.runtime();
        let key = c.semantic_key.clone();
        let target = session.clone();
        let attempted = next.attempted;
        let command_id = next.dispatch_id.clone();
        let input = next.content.clone();
        let reply = self.spawn(move || {
            let adapter = instrukt_runner::runtime::adapter(runtime);
            if attempted.is_some() {
                let Some(input) = input else {
                    return Ok(DeliveryCheck::UnconfirmedSend);
                };
                return Ok(
                    match adapter.observe_send(&target, &input, command_id.as_deref())? {
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
            let thread = adapter.read_state(&target)?;
            if thread.presence != Presence::Present {
                return Ok(DeliveryCheck::Presence(thread.presence));
            }
            // A thread in another runtime project is never served, busy or
            // not: its outcome says whether it can roll over now.
            adapter.verify_thread_project(&checkout, &thread, Some(&key))?;
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
                        source_presence,
                        source_identity: c.source_identity.clone(),
                        invalidated: false,
                    }),
                    reply,
                },
            );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch(
        &self,
        home: &Home,
        admitted: &Admitted,
        adapter: &dyn RuntimeAdapter,
        source: &dyn ConversationSource,
        c: Conversation,
        conversation: String,
        message: String,
    ) {
        let opening = if c.session.is_none() && c.request.is_none() {
            opening_position(source).map(|position| {
                Some(OpeningSource {
                    identity: source.identity().to_owned(),
                    position,
                })
            })
        } else {
            Ok(None)
        };
        let identity = source.identity().to_owned();
        let env = self.env.clone();
        let key = c.semantic_key.clone();
        let home = home.clone();
        let admitted = admitted.clone();
        let runtime = adapter.runtime();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || {
            let state = RunnerState::new(&home);
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
                    unavailable(&state, &c, &[(conversation, message)], connectors.now())?;
                    return Err(error.into());
                }
            };
            let _lock = match lock(&home, &c) {
                Ok(lock) => lock,
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "conversation_lock_unavailable", company = %c.company, connector = %c.connector, key = %c.semantic_key, %error);
                    unavailable(&state, &c, &[(conversation, message)], connectors.now())?;
                    return Err(error);
                }
            };
            let Some(current) = reread(&state, &c)?.filter(|current| current.session == c.session)
            else {
                return Ok(());
            };
            let adapter = instrukt_runner::runtime::adapter(runtime);
            let clock = || connectors.now();
            let delivery = NativeDelivery { home: &home, identity: &identity, env: &env, clock: &clock };
            if let Err(error) = delivery.check_source(adapter.as_ref(), &state, &current) {
                set_failure(&state, &current, Some(&error.to_string()))?;
                return Err(error);
            }
            let _key = current.session.as_ref().map(|_| {
                keyed::KeyGuard::claim(
                    &invocation,
                    adapter.as_ref(),
                    &current.semantic_key,
                    None,
                    connectors.now(),
                )
            }).transpose()?;
            if let Some(session) = &current.session
                && keyed::replacement_binding(&home, adapter.as_ref(), &current.semantic_key, session)?
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
                opening,
                &current,
                next,
                &delivery,
            )
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    pub(super) fn succeed(&self, home: &Home, adapter: &dyn RuntimeAdapter, c: Conversation) {
        let home = home.clone();
        let runtime = adapter.runtime();
        let key = c.semantic_key.clone();
        let connectors = Arc::clone(&self.connectors);
        let reply = self.spawn(move || -> Result<(), ConversationError> {
            let _lock = lock(&home, &c)?;
            let state = RunnerState::new(&home);
            let Some(current) = reread(&state, &c)?.filter(|current| current.session == c.session)
            else {
                return Ok(());
            };
            let invocation = RuntimeInvocation::acquire(&home, LOCK_WAIT)?;
            let adapter = instrukt_runner::runtime::adapter(runtime);
            let title=state.read(|conn,path|conn.query_row("SELECT display_title FROM connected_conversations WHERE company=?1 AND connector=?2 AND conversation=?3",params![c.company.as_str(),c.connector,c.conversation],|row|row.get::<_,Option<String>>(0)).map_err(state::sql(path)))?.flatten().unwrap_or_else(||conversation_title(c.company.as_str(),None,""));
            let mut request=match current.request.as_deref() {Some(raw)=>serde_json::from_str::<RuntimeStart>(raw)?,None=>keyed::request(&home,adapter.as_ref(),&current.semantic_key)?};
            let original=request.clone();
            request.title=title;
            let session=current.session.as_ref().ok_or_else(||ConversationError::Refused("successor has no owner-retained native ID".into()))?;
            if let Err(error)=keyed::succeed(adapter.as_ref(),&instrukt_runner::runtime::FollowupAssignment{bindings:None,original_request:Some(&original),invocation:&invocation,request:&request,session,worktree_path:None,project:None,now:connectors.now()}) {
                set_failure(&state, &current, Some(&error.to_string()))?;
                let waiting = open_messages(&state, &current)?
                    .into_iter()
                    .map(|message| (message.conversation, message.message))
                    .collect::<Vec<_>>();
                unavailable(&state, &current, &waiting, connectors.now())?;
                return Err(error.into());
            }
            Ok(())
        });
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, DeliveryWork::Dispatching(reply));
    }

    fn spawn<T: Send + 'static>(&self, run: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
        let (send, reply) = mpsc::channel();
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new().spawn(move || {
            let _ = send.send(run());
            let _ = wake.try_send(());
        });
        reply
    }
}

struct AbsenceRead {
    session: SessionRef,
    reply: Receiver<Option<Presence>>,
    delay_secs: i64,
    invalidated: bool,
}

enum MissingProbe {
    Backoff {
        retry_at: Timestamp,
        delay_secs: i64,
    },
    Confirmed(Presence),
    LiveUntil(Timestamp),
}

impl RelayWorkers {
    // A projection change can invalidate the result, but only the worker's
    // completion releases the conversation's slot for another native read.
    pub(super) fn absence_in_flight(
        &self,
        c: &Conversation,
        missing: Option<&SessionRef>,
        now: Timestamp,
    ) -> bool {
        let mut reads = self
            .absence_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(read) = reads.get_mut(&c.semantic_key) else {
            return false;
        };
        read.invalidated |= Some(&read.session) != missing;
        let observation = match read.reply.try_recv() {
            Ok(observation) => observation,
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) => return true,
        };
        if !read.invalidated {
            let state = if let Some(presence) = observation {
                if presence != Presence::Present {
                    tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_confirmed", session = %read.session, ?presence);
                }
                Self::observation(presence, now)
            } else {
                let session = &read.session;
                let retry_at = now + jiff::SignedDuration::from_secs(read.delay_secs);
                tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_retry", %session, %retry_at);
                MissingProbe::Backoff {
                    retry_at,
                    delay_secs: (read.delay_secs * 2).min(30),
                }
            };
            self.probes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(read.session.to_string(), state);
        }
        reads.remove(&c.semantic_key);
        false
    }

    pub(super) fn confirm_missing(
        &self,
        adapter: &dyn RuntimeAdapter,
        c: &Conversation,
        session: &SessionRef,
        now: Timestamp,
    ) -> Option<Presence> {
        if self.absence_in_flight(c, Some(session), now) {
            return None;
        }
        let id = session.to_string();
        let delay_secs = match self
            .probes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            Some(MissingProbe::Confirmed(presence)) => return Some(*presence),
            Some(MissingProbe::LiveUntil(retry_at)) if now < *retry_at => {
                return Some(Presence::Present);
            }
            Some(MissingProbe::Backoff {
                retry_at,
                delay_secs,
            }) if now >= *retry_at => *delay_secs,
            Some(MissingProbe::Backoff { .. }) => return None,
            _ => 2,
        };
        let runtime = adapter.runtime();
        let session = session.clone();
        let target = session.clone();
        tracing::debug!(target: LOG_TARGET, event = "conversation_presence_probe_started", %session);
        let reply = self.spawn(move || {
            instrukt_runner::runtime::adapter(runtime)
                .read_state(&target)
                .ok()
                .map(|thread| thread.presence)
        });
        self.absence_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                c.semantic_key.clone(),
                AbsenceRead {
                    session,
                    reply,
                    delay_secs,
                    invalidated: false,
                },
            );
        None
    }

    fn observation(presence: Presence, now: Timestamp) -> MissingProbe {
        match presence {
            // A live CLI thread can disappear while the projection is
            // still missing. Refresh this result if no delivery checks it.
            Presence::Present => MissingProbe::LiveUntil(now + jiff::SignedDuration::from_secs(2)),
            _ => MissingProbe::Confirmed(presence),
        }
    }

    pub(super) fn confirmed_presence(
        &self,
        session: &SessionRef,
        presence: Presence,
        now: Timestamp,
    ) {
        self.probes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session.to_string(), Self::observation(presence, now));
    }

    pub(super) fn source_present(&self, session: &SessionRef) {
        self.probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&session.to_string());
    }
}
