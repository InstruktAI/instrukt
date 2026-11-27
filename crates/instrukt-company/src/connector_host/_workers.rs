//! Per-connection supervision workers of the connector host: admission intake,
//! context capture, read-back scanning, working status, unavailable notices,
//! and private MCP sign-in DM delivery, together with their outside-gate
//! pacing.
mod _sign_in;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use instrukt_connector::{
    Admission, Captured, Connector, ConnectorError, ContextSnapshot, Place, RefusalReason, Sender,
    Status,
};
use instrukt_core::config::ConnectorAdmit;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_outside::Wait;
use instrukt_runner::state::RunnerState;
use jiff::Timestamp;

use crate::conversations::{self, Refusal};

use super::{Connectors, LOG_TARGET, Reach, after_failure, connector_admit, connector_policy};

/// How often the working pass looks at what is unfinished.
const STATUS_TICK: Duration = Duration::from_secs(1);

/// How long retained intake work waits between passes, and before retrying
/// a message whose failure carried no wait.
const RETRY: Duration = Duration::from_secs(2);

/// Whether the host clock has reached a stored moment.
fn due(until: Timestamp, now: Timestamp) -> bool {
    now >= until
}

/// The calls the outside gate answered with a wait, each owed until that
/// wait's moment: the part that made a call honours the wait it was handed,
/// also when a pass is woken early, whatever the wait's extent.
struct Owed<K>(HashMap<K, Wait>);

impl<K> Default for Owed<K> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<K: Eq + std::hash::Hash> Owed<K> {
    /// The wait `call` is still owed at `now`, if any; a passed wait is
    /// forgotten.
    fn pending(&mut self, call: &K, now: Instant) -> Option<Wait> {
        match self.0.get(call) {
            Some(wait) if now < wait.until() => Some(wait.clone()),
            Some(_) => {
                self.0.remove(call);
                None
            }
            None => None,
        }
    }

    /// What the outside gate answered `call` with: its wait, or none.
    fn answered(&mut self, call: K, wait: Option<&Wait>) {
        match wait {
            Some(wait) => {
                self.0.insert(call, wait.clone());
            }
            None => {
                self.0.remove(&call);
            }
        }
    }
}

/// A per-message call of intake.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IntakeCall {
    /// Asking the provider about the sender of `(conversation, message)`.
    Admit(String, String),
    /// Posting the fixed reply to `(conversation, sender)`.
    TurnAway(String, String),
}

/// Narrow wake handle handed to the connection's listen loop so incoming
/// activity can nudge the intake and read-back workers without exposing
/// internal channels.
#[derive(Clone, Copy)]
pub(super) struct WorkerNudges<'a> {
    intake: &'a SyncSender<()>,
    read_back: &'a SyncSender<()>,
}

impl WorkerNudges<'_> {
    pub(super) fn intake_sender(&self) -> SyncSender<()> {
        self.intake.clone()
    }

    pub(super) fn read_back_sender(&self) -> SyncSender<()> {
        self.read_back.clone()
    }
}

struct Ended<'a>(&'a AtomicBool);
impl Drop for Ended<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Run `run` alongside the live connector's scoped supervision workers,
/// propagating intake-worker exit through the stop predicate and joining every
/// worker before returning.
pub(super) fn supervise(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &AtomicBool,
    relay_wake: &tokio::sync::mpsc::Sender<()>,
    run: impl FnOnce(WorkerNudges<'_>, &dyn Fn() -> bool),
) {
    // Intake runs beside the connection, so reading what accompanies a
    // message or posting a refusal never holds back the next
    // acknowledgement. Its end ends this connection, which the run loop then
    // restarts. The working indicator has a worker of its own beside both.
    let ended = AtomicBool::new(false);
    let (nudge, nudges) = mpsc::sync_channel::<()>(1);
    let (admitted, admissions) = mpsc::sync_channel::<()>(1);
    let (context_wake, context_wakes) = mpsc::sync_channel::<()>(1);
    let (read_back, scans) = mpsc::sync_channel::<()>(1);
    let ended = &ended;
    let context_nudge = &context_wake;
    let scan_nudge = &nudge;
    let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ended = Ended(ended);
            let mut owed = Owed::default();
            intake_worker(&quit, &nudges, || {
                let Some(admit) = connector_admit(&reach.current.get(), company, key) else {
                    return IntakeProgress::default();
                };
                intake_pass(home, reach, company, key, |connector| {
                    admit_messages(
                        home,
                        company,
                        key,
                        admit,
                        connector.as_ref(),
                        &admitted,
                        context_nudge,
                        reach.connectors.as_ref(),
                        &mut owed,
                    )
                })
            });
        });
        scope.spawn(move || {
            let _ended = Ended(ended);
            let mut contexts = ContextReads::default();
            intake_worker(&quit, &context_wakes, || {
                intake_pass(home, reach, company, key, |connector| {
                    capture_contexts(
                        home,
                        company,
                        key,
                        &Arc::from(connector),
                        Wakes {
                            relay: relay_wake,
                            context: context_nudge,
                        },
                        reach.connectors,
                        &mut contexts,
                    )
                })
            });
        });
        scope.spawn(move || {
            status_worker(home, reach, company, key, &quit, &admissions);
        });
        scope.spawn(move || {
            notice_worker(home, reach, company, key, &quit);
        });
        scope.spawn(move || {
            _sign_in::worker(home, reach, company, key, &quit);
        });
        scope.spawn(move || {
            read_back_worker(home, reach, company, key, &quit, &scans, scan_nudge);
        });
        let workers = WorkerNudges {
            intake: &nudge,
            read_back: &read_back,
        };
        run(workers, &quit);
    });
}

fn read_back_worker(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    quit: &dyn Fn() -> bool,
    scans: &Receiver<()>,
    scan_nudge: &SyncSender<()>,
) {
    if let Err(error) =
        conversations::read_back::observe(home, company, key, reach.connectors.now())
    {
        tracing::warn!(target: LOG_TARGET, event = "connector_read_back_unrecorded", company = %company, error = %error);
    }
    // A scan runs when a connection wakes it, when its recheck is
    // due, or when the stored moment of its wait passes on the host
    // clock, the clock that moment was stored on. A live pause the
    // service named no moment for is a recheck on the monotonic
    // source; the scan after it skips that stored moment.
    let mut recheck: Option<Instant> = None;
    let mut waiting_until: Option<Timestamp> = None;
    let mut live_pause: Option<(Timestamp, Instant)> = None;
    while !quit() {
        let event = scans.recv_timeout(Duration::from_millis(200));
        if quit() {
            break;
        }
        let woken = event.is_ok()
            || recheck.is_some_and(|at| Instant::now() >= at)
            || waiting_until.is_some_and(|until| due(until, reach.connectors.now()));
        if !woken {
            continue;
        }
        let waited_out = live_pause
            .filter(|(_, pause)| Instant::now() >= *pause)
            .map(|(at, _)| at);
        let admitted_gen = reach.current.get();
        let result = connector_policy(&admitted_gen, company, key).and_then(
            |(role, _, session_scope, reply_mode)| {
                reach
                    .connectors
                    .open(&admitted_gen, company, key)
                    .and_then(|connector| {
                        conversations::read_back::scan(
                            home,
                            company,
                            key,
                            &role,
                            session_scope,
                            reply_mode,
                            connector.as_ref(),
                            &|| reach.connectors.now(),
                            quit,
                            &|| {
                                let _ = scan_nudge.try_send(());
                            },
                            waited_out,
                        )
                    })
            },
        );
        match result {
            Err(error) => {
                if error.wait.is_none() {
                    tracing::warn!(target: LOG_TARGET, event = "connector_read_back_failed", company = %company, connector = key, error = %error);
                }
                recheck = Some(after_failure(error.wait.as_ref(), Instant::now()));
                waiting_until = None;
                live_pause = None;
            }
            Ok(conversations::read_back::Scanned::Done { catch_up }) => {
                recheck = catch_up.then(Instant::now);
                waiting_until = None;
                live_pause = None;
            }
            Ok(conversations::read_back::Scanned::Waiting {
                until,
                pause: Some(pause),
            }) => {
                recheck = Some(pause);
                waiting_until = None;
                live_pause = Some((until, pause));
            }
            Ok(conversations::read_back::Scanned::Waiting { until, pause: None }) => {
                // Woken during its own live pause, the read keeps it.
                match live_pause.filter(|(at, _)| *at == until) {
                    Some((_, pause)) => {
                        recheck = Some(pause);
                        waiting_until = None;
                    }
                    None => {
                        recheck = None;
                        waiting_until = Some(until);
                        live_pause = None;
                    }
                }
            }
        }
    }
}

/// Wake the relay on capture and intake when a context read completes.
#[derive(Clone, Copy)]
struct Wakes<'a> {
    relay: &'a tokio::sync::mpsc::Sender<()>,
    context: &'a SyncSender<()>,
}

/// The company's retained intake work, until `stop`: run when a retained
/// message nudges it, and otherwise at the moment its last pass gave
/// ([`next_pass`]).
fn intake_worker(
    stop: &dyn Fn() -> bool,
    nudges: &Receiver<()>,
    mut pass: impl FnMut() -> IntakeProgress,
) {
    while !stop() {
        let next = next_pass(&pass(), Instant::now());
        while Instant::now() < next && !stop() {
            if nudges.recv_timeout(Duration::from_millis(100)).is_ok() {
                break;
            }
        }
    }
}

/// When intake passes again: [`RETRY`] after a clean pass, while reading
/// context and after a message failed without a wait; at the earliest
/// moment the outside gate gave, never sooner than [`RETRY`], when every
/// failure carried a wait; [`super::RECHECK`] after a pass that failed locally.
fn next_pass(progress: &IntakeProgress, now: Instant) -> Instant {
    let retry = now + RETRY;
    match progress.failures {
        Failures::Local => after_failure(None, now),
        _ if progress.reading_context => retry,
        Failures::Clean | Failures::Unwaited => retry,
        Failures::Waits(earliest) => retry.max(earliest),
    }
}

fn intake_pass(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    pass: impl FnOnce(Box<dyn Connector>) -> Result<IntakeProgress, conversations::ConversationError>,
) -> IntakeProgress {
    let failed = |error: conversations::ConversationError| {
        tracing::warn!(target: LOG_TARGET, event = "connector_intake_failed", company = %company, connector = key, %error);
        IntakeProgress::local()
    };
    match reach.connectors.open(&reach.current.get(), company, key) {
        Ok(connector) => pass(connector).unwrap_or_else(failed),
        Err(error) => {
            if let Err(record) =
                conversations::intake_failed(home, company, key, &error.to_string())
            {
                return failed(record);
            }
            IntakeProgress::local()
        }
    }
}

/// Evaluate eligibility on its own cadence. Each member and conversation
/// has at most one delivery in flight, and provider reads run independently.
fn notice_worker(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &dyn Fn() -> bool,
) {
    std::thread::scope(|scope| {
        let mut pending = BTreeMap::new();
        while !stop() {
            pending
                .retain(|_, task: &mut std::thread::ScopedJoinHandle<'_, ()>| !task.is_finished());
            let now = reach.connectors.now();
            match conversations::notices(home, company, key, now) {
                Ok(notices) => {
                    for notice in notices {
                        let member = (notice.conversation.clone(), notice.sender.clone());
                        if let std::collections::btree_map::Entry::Vacant(entry) =
                            pending.entry(member)
                        {
                            match std::thread::Builder::new().spawn_scoped(scope, move || {
                                deliver_notice(home, reach, company, key, &notice);
                            }) {
                                Ok(task) => {
                                    entry.insert(task);
                                }
                                Err(error) => {
                                    tracing::warn!(target: LOG_TARGET, event = "conversation_notice_worker_unstarted", company = %company, connector = key, %error)
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "conversation_notice_failed", company = %company, connector = key, %error)
                }
            }
            let until = Instant::now() + conversations::INTAKE_INTERVAL;
            while Instant::now() < until && !stop() {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    });
}

/// Decide durably, then make one best-effort provider post without an answer lock.
fn deliver_notice(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    notice: &conversations::Notice,
) {
    let connector = match reach.connectors.open(&reach.current.get(), company, key) {
        Ok(connector) => connector,
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "conversation_notice_failed", company = %company, connector = key, error = %error);
            return;
        }
    };
    let step = conversations::notice_step(
        home,
        company,
        key,
        notice,
        reach.accepted,
        reach.connectors.now(),
    );
    let result = match step {
        Ok(conversations::NoticeStep::Post) => {
            let outcome = instrukt_outside::runtime::block_on(
                connector.unavailable_notice(&notice.conversation, &notice.sender),
            )
            .map_err(|error| error.to_string())
            .and_then(|res| res.map_err(|error| error.to_string()));
            tracing::info!(target: LOG_TARGET, event = "conversation_notice_posted", company = %company, connector = key, accepted = outcome.is_ok());
            conversations::notice_posted(
                home,
                company,
                key,
                notice,
                outcome,
                reach.connectors.now(),
            )
        }
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        tracing::warn!(target: LOG_TARGET, event = "conversation_notice_failed", company = %company, connector = key, error = %error);
    }
}

/// What a received message's record becomes from what the connector says
/// about its sender and the connector's `admit` policy.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Admitted, with what the provider gave about the sender.
    Admitted(Sender),
    /// Refused: the sender receives the fixed reply for this reason.
    Refused(RefusalReason),
    /// It waits, with this cause and the outside gate's wait when there is
    /// one.
    Waiting(ConnectorError),
}

fn next_record(admission: Admission, admit: ConnectorAdmit) -> Next {
    match admission {
        Admission::Admitted(sender) => {
            if admit == ConnectorAdmit::AdminsOnly && !sender.is_admin {
                Next::Refused(RefusalReason::NotAdmin)
            } else {
                Next::Admitted(sender)
            }
        }
        Admission::NotOwn => Next::Refused(match admit {
            ConnectorAdmit::Members => RefusalReason::NotOwn,
            ConnectorAdmit::AdminsOnly => RefusalReason::NotAdmin,
        }),
        Admission::Undecided { cause, wait } => Next::Waiting(ConnectorError { cause, wait }),
    }
}

/// The refusals still owed: those whose reply was not posted.
fn owed(refusals: Vec<(Refusal, bool)>) -> Vec<Refusal> {
    refusals
        .into_iter()
        .filter_map(|(refusal, posted)| (!posted).then_some(refusal))
        .collect()
}

enum ContextReply {
    Readable(ContextSnapshot),
    Finished(Result<Captured, ConnectorError>),
}

struct ContextFlight {
    replies: Receiver<ContextReply>,
    readable: Option<(ContextSnapshot, Instant)>,
}

#[derive(Default)]
/// The context reads in flight, and the reads of `(conversation, message)`
/// the outside gate answered with a wait, owed until its moment.
struct ContextReads(
    BTreeMap<(String, String), ContextFlight>,
    Owed<(String, String)>,
);

impl ContextReads {
    fn capture(
        &mut self,
        connector: &Arc<dyn Connector>,
        item: &conversations::Unread,
        wake: &SyncSender<()>,
        exclusions: impl FnOnce() -> Result<
            std::collections::HashSet<String>,
            conversations::ConversationError,
        >,
    ) -> Option<Result<Captured, ConnectorError>> {
        use std::collections::btree_map::Entry;
        let key = (item.conversation.clone(), item.message.clone());
        match self.0.entry(key) {
            Entry::Occupied(mut slot) => {
                let flight = slot.get_mut();
                loop {
                    match flight.replies.try_recv() {
                        Ok(ContextReply::Readable(snapshot)) => {
                            flight.readable = Some((snapshot, Instant::now() + RETRY));
                        }
                        Ok(ContextReply::Finished(result)) => {
                            slot.remove();
                            return Some(result);
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            slot.remove();
                            return Some(Err(ConnectorError::new(
                                "the context reader stopped before reporting its result",
                            )));
                        }
                    }
                }
                if flight
                    .readable
                    .as_ref()
                    .is_some_and(|(_, ready)| Instant::now() >= *ready)
                {
                    let (snapshot, _) = slot.remove().readable?;
                    return Some(Ok(snapshot()));
                }
            }
            Entry::Vacant(slot) => {
                let excluded = match exclusions() {
                    Ok(excluded) => excluded,
                    Err(error) => {
                        return Some(Err(ConnectorError::new(error.to_string())));
                    }
                };
                let connector = Arc::clone(connector);
                let item = item.clone();
                let wake = wake.clone();
                let (send, replies) = mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("conversation-context".into())
                    .spawn(move || {
                        let result = instrukt_outside::runtime::block_on(connector.capture(
                            &item.conversation,
                            &item.message,
                            &item.text,
                            &excluded,
                            &|snapshot| {
                                if send.send(ContextReply::Readable(snapshot)).is_ok() {
                                    let _ = wake.try_send(());
                                }
                            },
                        ))
                        .map_err(|e| ConnectorError::new(e.to_string()))
                        .and_then(|res| res);
                        if send.send(ContextReply::Finished(result)).is_ok() {
                            let _ = wake.try_send(());
                        }
                    });
                if let Err(error) = spawned {
                    return Some(Err(ConnectorError::new(format!(
                        "cannot start context read: {error}"
                    ))));
                }
                slot.insert(ContextFlight {
                    replies,
                    readable: None,
                });
            }
        }
        None
    }
}

/// What one intake pass's failures were.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Failures {
    #[default]
    Clean,
    /// Every failed message carried a wait; the earliest's deadline.
    Waits(Instant),
    /// A message failed without a wait.
    Unwaited,
    /// The pass itself failed.
    Local,
}

impl Failures {
    /// One more failed message, with the outside gate's wait when it had one.
    fn add(&mut self, wait: Option<&Wait>) {
        *self = match (*self, wait.map(Wait::until)) {
            (Self::Local, _) => Self::Local,
            (Self::Unwaited, _) | (_, None) => Self::Unwaited,
            (Self::Clean, Some(until)) => Self::Waits(until),
            (Self::Waits(earliest), Some(until)) => Self::Waits(earliest.min(until)),
        };
    }
}

#[derive(Debug, Default)]
struct IntakeProgress {
    failures: Failures,
    reading_context: bool,
}

impl IntakeProgress {
    fn local() -> Self {
        Self {
            failures: Failures::Local,
            reading_context: false,
        }
    }
}

/// Sender decisions and refusals can call Slack. Context reads and relay
/// handoff run on a different worker.
#[allow(clippy::too_many_arguments)]
fn admit_messages(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    admit: ConnectorAdmit,
    connector: &dyn Connector,
    status: &SyncSender<()>,
    context: &SyncSender<()>,
    clock: &dyn Connectors,
    owed: &mut Owed<IntakeCall>,
) -> Result<IntakeProgress, conversations::ConversationError> {
    let mut progress = IntakeProgress::default();
    let now = Instant::now();
    for unread in conversations::unread(home, company, key)?
        .into_iter()
        .filter(|item| !item.admitted)
    {
        let Some(sender) = unread.sender.as_deref() else {
            progress.failures.add(None);
            conversations::waiting(home, company, key, &unread, "the message has no sender")?;
            continue;
        };
        let call = IntakeCall::Admit(unread.conversation.clone(), unread.message.clone());
        if let Some(wait) = owed.pending(&call, now) {
            progress.failures.add(Some(&wait));
            continue;
        }
        let admitted =
            instrukt_outside::runtime::block_on(connector.admit(sender)).unwrap_or_else(|e| {
                Admission::Undecided {
                    cause: e.to_string(),
                    wait: None,
                }
            });
        let next = next_record(admitted, admit);
        owed.answered(
            call,
            match &next {
                Next::Waiting(error) => error.wait.as_ref(),
                _ => None,
            },
        );
        match next {
            Next::Admitted(admitted) => {
                conversations::admitted(home, company, key, &unread, &admitted, clock.now())?;
                let _ = status.try_send(());
                let _ = context.try_send(());
            }
            Next::Refused(reason) => {
                conversations::refused(
                    home,
                    company,
                    key,
                    &unread,
                    sender,
                    reason,
                    Timestamp::now(),
                )?;
            }
            Next::Waiting(error) => {
                progress.failures.add(error.wait.as_ref());
                conversations::waiting(home, company, key, &unread, &error.to_string())?;
            }
        }
    }
    for refusal in self::owed(conversations::refusals(home, company, key)?) {
        let call = IntakeCall::TurnAway(refusal.conversation.clone(), refusal.sender.clone());
        if let Some(wait) = owed.pending(&call, now) {
            progress.failures.add(Some(&wait));
            continue;
        }
        let outcome = instrukt_outside::runtime::block_on(
            connector.turn_away(&refusal.conversation, refusal.reason),
        )
        .map_err(|e| ConnectorError::new(e.to_string()))
        .and_then(|res| res);
        owed.answered(
            call,
            outcome.as_ref().err().and_then(|error| error.wait.as_ref()),
        );
        if let Err(error) = &outcome {
            progress.failures.add(error.wait.as_ref());
        }
        let outcome = outcome.map_err(|error| error.to_string());
        conversations::refusal_posted(home, company, key, &refusal, outcome, Timestamp::now())?;
    }
    Ok(progress)
}

fn capture_contexts(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    connector: &Arc<dyn Connector>,
    wakes: Wakes<'_>,
    clock: &Arc<dyn Connectors>,
    contexts: &mut ContextReads,
) -> Result<IntakeProgress, conversations::ConversationError> {
    let mut progress = IntakeProgress::default();
    let now = Instant::now();
    for unread in conversations::unread(home, company, key)?
        .into_iter()
        .filter(|item| item.admitted)
    {
        let call = (unread.conversation.clone(), unread.message.clone());
        if let Some(wait) = contexts.1.pending(&call, now) {
            progress.failures.add(Some(&wait));
            continue;
        }
        let captured = contexts.capture(connector, &unread, wakes.context, || {
            conversations::context_exclusions(
                &RunnerState::new(home),
                company,
                key,
                &unread.conversation,
            )
            .map_err(Into::into)
        });
        if let Some(result) = &captured {
            let wait = result.as_ref().err().and_then(|error| error.wait.as_ref());
            contexts.1.answered(call, wait);
        }
        match captured {
            Some(Ok(captured)) => {
                conversations::captured(home, company, key, &unread, Ok(captured), clock.now())?;
                let _ = wakes.relay.try_send(());
                continue;
            }
            Some(Err(error)) => {
                progress.failures.add(error.wait.as_ref());
                conversations::captured(
                    home,
                    company,
                    key,
                    &unread,
                    Err(error.to_string()),
                    clock.now(),
                )?;
            }
            None => progress.reading_context = true,
        }
    }
    Ok(progress)
}

/// What one status shows for the unfinished messages it serves: the
/// unavailable text while every one of them has its interval open;
/// otherwise the newest message the agent has written in the running turn,
/// or the working text before it has one.
fn shown(statuses: &[&Status]) -> Status {
    if !statuses.is_empty()
        && statuses
            .iter()
            .all(|status| **status == Status::Unavailable)
    {
        return Status::Unavailable;
    }
    statuses
        .iter()
        .find_map(|status| match status {
            Status::Update(update) => Some(Status::Update(update.clone())),
            _ => None,
        })
        .unwrap_or(Status::Working)
}

/// What one status pass does: the places to set (unfinished work whose
/// status was not set yet, shows something else now, or was set `refresh`
/// ago or more) and the places to clear (set by this process, with no
/// unfinished work left).
#[derive(Debug, PartialEq, Eq)]
struct StatusPass {
    set: Vec<(Place, Status)>,
    clear: Vec<Place>,
}

fn status_pass(
    wanted: &BTreeMap<Place, Status>,
    attempted: &HashMap<Place, (Instant, Status)>,
    owed: &mut Owed<Place>,
    now: Instant,
    refresh: Option<Duration>,
) -> StatusPass {
    let set = wanted
        .iter()
        .filter(|(place, _)| owed.pending(place, now).is_none())
        .filter(|(place, status)| {
            attempted.get(*place).is_none_or(|(at, shown)| {
                shown != *status
                    || refresh
                        .is_some_and(|interval| now.saturating_duration_since(*at) >= interval)
            })
        })
        .map(|(place, status)| (place.clone(), status.clone()))
        .collect();
    let mut clear: Vec<Place> = attempted
        .keys()
        .filter(|place| !wanted.contains_key(*place))
        .cloned()
        .collect();
    clear.retain(|place| owed.pending(place, now).is_none());
    clear.sort();
    StatusPass { set, clear }
}

/// The status for one company, a courtesy kept on a best-effort basis apart
/// from intake and the relay, so no status call holds a message. A plain
/// periodic pass reads the unfinished admitted messages with what each one's
/// status shows and does what [`status_pass`] decides. A message's admission
/// wakes the pass, so its first status comes promptly. A call that fails or
/// is skipped is left to the next regular setting; nothing is made up for
/// and nothing set by an earlier run is cleared.
fn status_worker(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &dyn Fn() -> bool,
    admitted: &Receiver<()>,
) {
    let mut attempted: HashMap<Place, (Instant, Status)> = HashMap::new();
    let mut owed = Owed::default();
    let mut failing: Option<String> = None;
    while !stop() {
        let pass = reach
            .connectors
            .open(&reach.current.get(), company, key)
            .map_err(|error| error.to_string())
            .and_then(|connector| {
                let unfinished =
                    conversations::unfinished(home, company, key).map_err(|e| e.to_string())?;
                Ok((connector, unfinished))
            });
        match pass {
            Ok((connector, unfinished)) => {
                if let Some(cause) = failing.take() {
                    tracing::info!(target: LOG_TARGET, event = "connector_status_recovered", company = %company, cause = %cause);
                }
                keep(
                    connector.as_ref(),
                    company,
                    key,
                    &unfinished,
                    &mut attempted,
                    &mut owed,
                );
            }
            // A continuing cause is logged once, when it starts or changes.
            Err(cause) => {
                if failing.as_deref() != Some(cause.as_str()) {
                    tracing::warn!(target: LOG_TARGET, event = "connector_status_unprepared", company = %company, error = %cause);
                    failing = Some(cause);
                }
            }
        }
        let until = Instant::now() + STATUS_TICK;
        while Instant::now() < until && !stop() {
            if admitted.recv_timeout(Duration::from_millis(100)).is_ok() {
                break;
            }
        }
    }
}

/// What each place with unfinished work shows, where `connector` places
/// each message.
fn wanted(
    connector: &dyn Connector,
    unfinished: &[conversations::Unfinished],
) -> BTreeMap<Place, Status> {
    let mut served: BTreeMap<Place, Vec<&Status>> = BTreeMap::new();
    for message in unfinished {
        served
            .entry(connector.place(&message.conversation, &message.message))
            .or_default()
            .push(&message.status);
    }
    served
        .into_iter()
        .map(|(place, statuses)| (place, shown(&statuses)))
        .collect()
}

/// One status pass on `connector`: set what stays and is due or changed,
/// clear once what no longer stays. Each call's outcome is only logged.
fn keep(
    connector: &dyn Connector,
    company: &ProjectSlug,
    key: &str,
    unfinished: &[conversations::Unfinished],
    attempted: &mut HashMap<Place, (Instant, Status)>,
    owed: &mut Owed<Place>,
) {
    let wanted = wanted(connector, unfinished);
    let now = Instant::now();
    let refresh = connector.limits().working_refresh;
    let pass = status_pass(&wanted, attempted, owed, now, refresh);
    // A call the outside gate answered with a wait stays owed until that
    // wait's moment; a later pass makes it then.
    for (place, status) in pass.set {
        let wait = call(connector, company, key, &place, Some(&status));
        if wait.is_none() {
            attempted.insert(place.clone(), (now, status));
        }
        owed.answered(place, wait.as_ref());
    }
    for place in pass.clear {
        let wait = call(connector, company, key, &place, None);
        if wait.is_none() {
            attempted.remove(&place);
        }
        owed.answered(place, wait.as_ref());
    }
}

/// Show `status` at `place`, best effort; the outside gate's wait when it
/// answered with one, which leaves the call owed.
fn call(
    connector: &dyn Connector,
    company: &ProjectSlug,
    key: &str,
    place: &Place,
    status: Option<&Status>,
) -> Option<Wait> {
    match instrukt_outside::runtime::block_on(connector.status(place, status))
        .map_err(|e| ConnectorError::new(e.to_string()))
        .and_then(|res| res)
    {
        Ok(()) => None,
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, event = "connector_status_skipped", company = %company, connector = key, error = %error);
            error.wait
        }
    }
}

#[cfg(test)]
mod test_connector {
    //! An in-memory connector whose facts each test sets: its limits and
    //! where it places a message's status. It records nothing.
    use instrukt_connector::{
        Admission, Captured, Connector, ConnectorError, Inbox, Limits, Place, Status,
    };

    pub(super) struct TestConnector {
        pub limits: Limits,
        /// Places every message of a conversation at the conversation, or
        /// each message at itself.
        pub per_message: bool,
    }

    #[async_trait::async_trait]
    impl Connector for TestConnector {
        fn limits(&self) -> Limits {
            self.limits
        }
        async fn listen(
            &self,
            _: std::sync::Arc<dyn Inbox>,
            _: tokio::sync::watch::Receiver<bool>,
        ) -> Result<(), ConnectorError> {
            Ok(())
        }
        async fn admit(&self, _: &str) -> Admission {
            Admission::NotOwn
        }
        async fn capture(
            &self,
            _: &str,
            _: &str,
            text: &str,
            _: &std::collections::HashSet<String>,
            _: &(dyn Fn(instrukt_connector::ContextSnapshot) + Send + Sync),
        ) -> Result<Captured, ConnectorError> {
            Ok(Captured {
                display: None,
                text: text.to_owned(),
                accompanying: Default::default(),
            })
        }
        async fn answer(
            &self,
            _: &str,
            _: &str,
        ) -> Result<instrukt_connector::AnswerReceipt, instrukt_connector::AnswerError> {
            Ok(instrukt_connector::AnswerReceipt::default())
        }
        async fn turn_away(
            &self,
            _: &str,
            _: instrukt_connector::RefusalReason,
        ) -> Result<(), ConnectorError> {
            Ok(())
        }
        async fn unavailable_notice(&self, _: &str, _: &str) -> Result<(), ConnectorError> {
            Ok(())
        }
        fn place(&self, conversation: &str, message: &str) -> Place {
            if self.per_message {
                Place(conversation.to_owned(), message.to_owned())
            } else {
                Place(conversation.to_owned(), conversation.to_owned())
            }
        }
        async fn status(&self, _: &Place, _: Option<&Status>) -> Result<(), ConnectorError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::time::{Duration, Instant};

    use instrukt_connector::{
        Admission, Connector, ConnectorError, Limits, Place, RefusalReason, Sender, Status,
    };
    use instrukt_core::config::ConnectorAdmit;
    use jiff::Timestamp;

    use crate::conversations::{self, Refusal};

    use super::super::{RECHECK, wait_after};
    use super::test_connector::TestConnector;
    use super::{
        Failures, IntakeProgress, Next, Owed, RETRY, due, next_pass, next_record, owed, shown,
        status_pass, wanted,
    };

    fn connector(per_message: bool) -> TestConnector {
        TestConnector {
            limits: Limits {
                message_chars: 100,
                working_refresh: Some(Duration::from_secs(30)),
            },
            per_message,
        }
    }

    fn unfinished(messages: &[(&str, &str, Status)]) -> Vec<conversations::Unfinished> {
        messages
            .iter()
            .map(
                |(conversation, message, status)| conversations::Unfinished {
                    conversation: (*conversation).to_owned(),
                    message: (*message).to_owned(),
                    sender: Some("USTEVEN".to_owned()),
                    since: None,
                    status: status.clone(),
                },
            )
            .collect()
    }

    fn place(connector: &TestConnector, conversation: &str, message: &str) -> Place {
        connector.place(conversation, message)
    }

    #[test]
    fn the_status_pass_sets_new_changed_and_due_places_and_clears_finished_ones() {
        let connector = connector(false);
        let base = Instant::now();
        let refresh = connector.limits.working_refresh;
        let wanted = wanted(
            &connector,
            &unfinished(&[
                ("C1", "1", Status::Working),
                ("C1", "2", Status::Working),
                ("C2", "3", Status::Working),
            ]),
        );
        // Nothing set yet: each place with unfinished work is set once.
        let first = status_pass(
            &wanted,
            &HashMap::new(),
            &mut Owed::default(),
            base,
            refresh,
        );
        assert_eq!(
            first.set,
            [
                (place(&connector, "C1", "1"), Status::Working),
                (place(&connector, "C2", "3"), Status::Working)
            ]
        );
        assert!(first.clear.is_empty());
        let mut attempted: HashMap<Place, (Instant, Status)> = first
            .set
            .iter()
            .map(|(place, status)| (place.clone(), (base, status.clone())))
            .collect();
        // Before the interval nothing is set again.
        let early = status_pass(
            &wanted,
            &attempted,
            &mut Owed::default(),
            base + Duration::from_secs(29),
            refresh,
        );
        assert!(early.set.is_empty());
        // A place whose state changed is set at the next pass.
        let changed = BTreeMap::from([(place(&connector, "C1", "1"), Status::Unavailable)]);
        let next = status_pass(
            &changed,
            &attempted,
            &mut Owed::default(),
            base + Duration::from_secs(1),
            refresh,
        );
        assert_eq!(
            next.set,
            [(place(&connector, "C1", "1"), Status::Unavailable)]
        );
        // At the interval a place that still has work is set again; one
        // whose work finished is cleared.
        let later = BTreeMap::from([(place(&connector, "C1", "1"), Status::Working)]);
        let due = status_pass(
            &later,
            &attempted,
            &mut Owed::default(),
            base + Duration::from_secs(30),
            refresh,
        );
        assert_eq!(due.set, [(place(&connector, "C1", "1"), Status::Working)]);
        assert_eq!(due.clear, [place(&connector, "C2", "3")]);
        // Without a refresh interval a set place is never set again.
        attempted.insert(place(&connector, "C1", "1"), (base, Status::Working));
        let once = status_pass(
            &later,
            &attempted,
            &mut Owed::default(),
            base + Duration::from_secs(600),
            None,
        );
        assert!(once.set.is_empty());
    }

    #[test]
    fn the_status_pass_follows_where_the_connector_places_each_message() {
        let connector = connector(true);
        let wanted = wanted(
            &connector,
            &unfinished(&[("D1", "1", Status::Working), ("D1", "2", Status::Working)]),
        );
        let pass = status_pass(
            &wanted,
            &HashMap::new(),
            &mut Owed::default(),
            Instant::now(),
            None,
        );
        assert_eq!(pass.set.len(), 2);
    }

    #[test]
    fn one_status_says_unavailable_only_while_every_message_it_serves_is() {
        let update = Status::Update("Reading the release notes.".to_owned());
        assert_eq!(
            shown(&[&Status::Unavailable, &Status::Unavailable]),
            Status::Unavailable
        );
        assert_eq!(shown(&[&Status::Unavailable, &update]), update);
        assert_eq!(
            shown(&[&Status::Unavailable, &Status::Working]),
            Status::Working
        );
        assert_eq!(shown(&[&Status::Working, &update]), update);
    }

    #[test]
    fn a_status_call_answered_with_a_wait_is_not_made_again_before_its_moment() {
        let conn = connector(true);
        let waiting = place(&conn, "C1", "1.1");
        let other = place(&conn, "C1", "1.2");
        let wanted = BTreeMap::from([
            (waiting.clone(), Status::Working),
            (other.clone(), Status::Working),
        ]);
        let base = Instant::now();
        let mut owed = Owed::default();
        owed.answered(
            waiting.clone(),
            Some(&wait_after(base, Duration::from_secs(1))),
        );
        // A pass woken inside the wait makes no request for the waiting call
        // and still makes the other one.
        let early = status_pass(&wanted, &HashMap::new(), &mut owed, base, None);
        assert_eq!(early.set, [(other.clone(), Status::Working)]);
        let due = status_pass(
            &wanted,
            &HashMap::new(),
            &mut owed,
            base + Duration::from_secs(1),
            None,
        );
        assert_eq!(
            due.set,
            [(waiting, Status::Working), (other, Status::Working)]
        );
    }

    #[test]
    fn intake_after_waits_only_passes_at_the_later_of_retry_and_the_earliest_wait() {
        let now = Instant::now();
        let mut failures = Failures::default();
        failures.add(Some(&wait_after(now, Duration::from_secs(30))));
        failures.add(Some(&wait_after(now, Duration::from_secs(10))));
        let progress = IntakeProgress {
            failures,
            reading_context: false,
        };
        assert_eq!(next_pass(&progress, now), now + Duration::from_secs(10));
        let mut soon = Failures::default();
        soon.add(Some(&wait_after(now, Duration::from_millis(500))));
        let progress = IntakeProgress {
            failures: soon,
            reading_context: false,
        };
        assert_eq!(next_pass(&progress, now), now + RETRY);
    }

    #[test]
    fn a_stored_moment_is_due_once_the_host_clock_reaches_it() {
        let until: Timestamp = "2026-11-20T10:02:00Z".parse().unwrap();
        let micro = jiff::SignedDuration::from_micros(1);
        assert!(due(until, until));
        assert!(due(until, until + micro));
        assert!(!due(until, until - micro));
    }

    #[test]
    fn an_unwaited_message_failure_retries_at_retry_and_a_failed_pass_at_the_recheck() {
        let now = Instant::now();
        let mut failures = Failures::default();
        failures.add(Some(&wait_after(now, Duration::from_secs(30))));
        failures.add(None);
        let progress = IntakeProgress {
            failures,
            reading_context: false,
        };
        assert_eq!(next_pass(&progress, now), now + RETRY);
        assert_eq!(next_pass(&IntakeProgress::local(), now), now + RECHECK);
    }

    #[test]
    fn what_the_connector_says_of_a_sender_decides_the_next_record() {
        let sender = Sender {
            name: "Ada".into(),
            email: None,
            is_admin: false,
        };
        let admin = Sender {
            name: "Root".into(),
            email: None,
            is_admin: true,
        };
        assert_eq!(
            next_record(Admission::Admitted(sender.clone()), ConnectorAdmit::Members),
            Next::Admitted(sender.clone())
        );
        assert_eq!(
            next_record(Admission::NotOwn, ConnectorAdmit::Members),
            Next::Refused(RefusalReason::NotOwn)
        );
        assert_eq!(
            next_record(Admission::Admitted(sender), ConnectorAdmit::AdminsOnly),
            Next::Refused(RefusalReason::NotAdmin)
        );
        assert_eq!(
            next_record(Admission::NotOwn, ConnectorAdmit::AdminsOnly),
            Next::Refused(RefusalReason::NotAdmin)
        );
        assert_eq!(
            next_record(
                Admission::Admitted(admin.clone()),
                ConnectorAdmit::AdminsOnly
            ),
            Next::Admitted(admin)
        );
        assert_eq!(
            next_record(
                Admission::Undecided {
                    cause: "later".into(),
                    wait: None
                },
                ConnectorAdmit::Members
            ),
            Next::Waiting(ConnectorError::new("later"))
        );
    }

    #[test]
    fn only_refusals_not_yet_posted_are_owed() {
        let refusal = |conversation: &str| Refusal {
            conversation: conversation.into(),
            sender: "U1".into(),
            reason: RefusalReason::NotOwn,
        };
        let owed = owed(vec![(refusal("C1/1"), true), (refusal("C1/2"), false)]);
        assert_eq!(
            owed.iter()
                .map(|r| r.conversation.as_str())
                .collect::<Vec<_>>(),
            ["C1/2"]
        );
    }
}
