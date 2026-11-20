//! The connector host of `instrukt run` (`slack-connector` and
//! `conversations` specs): everything around a connector's operations,
//! whatever its kind. One connection per declared connector, recorded and
//! reopened when it drops; each message a connector hands over is retained
//! before it is acknowledged; deciding who is answered, reading what
//! accompanies a message, posting refusals and keeping the working indicator
//! follow from the retained record, retried until done. A connector is
//! opened from the admitted generation current at each pass, its secrets
//! resolved from that generation: no connector reads a company's files.
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use instrukt_connector::{
    Admission, Captured, Clock, Connector, ConnectorError, ContextSnapshot, Declared, Inbox,
    Incoming, Kind, Place, SecretResolver, Sender, Status,
};
use instrukt_core::config::{Admitted, AdmittedSecrets, Current};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use jiff::Timestamp;
use serde_yaml_ng::Value;

use crate::conversations::{self, ConnectionState, Refusal};

/// The target every log statement of the host carries: the module the host
/// and Slack shared before the cut, so the same settings select it.
const LOG_TARGET: &str = "instrukt_runner::connectors";

/// How often the working pass looks at what is unfinished.
const STATUS_TICK: Duration = Duration::from_secs(1);

/// The longest wait before retrying intake work that failed.
const MOST_BACKOFF: Duration = Duration::from_secs(60);

/// How long retained intake work waits between passes, and how long it first
/// waits after a pass that failed; each further failure in a row doubles
/// that wait, up to [`MOST_BACKOFF`].
const RETRY: Duration = Duration::from_secs(2);

/// The connectors the host reaches: one company's, opened from a generation,
/// and the runner's clock.
pub trait Connectors: Send + Sync {
    /// The `key` connector of `company`, as `admitted` declares it and with
    /// the secrets it resolved.
    fn open(
        &self,
        admitted: &Admitted,
        company: &ProjectSlug,
        key: &str,
    ) -> Result<Box<dyn Connector>, ConnectorError>;
    /// The time now on the runner's clock.
    fn now(&self) -> Timestamp;
}

/// The kinds built into this process, each built once when the runner
/// starts, and the runner's clock.
pub struct Directory {
    kinds: Vec<Box<dyn Kind>>,
    clock: Clock,
}

impl Directory {
    pub fn new(kinds: Vec<Box<dyn Kind>>, clock: Clock) -> Self {
        Self { kinds, clock }
    }
}

impl Connectors for Directory {
    fn open(
        &self,
        admitted: &Admitted,
        company: &ProjectSlug,
        key: &str,
    ) -> Result<Box<dyn Connector>, ConnectorError> {
        let not_admitted = || ConnectorError {
            cause: format!("the {key} connector of company `{company}` is not admitted"),
        };
        let admitted_company = admitted.company(company).ok_or_else(not_admitted)?;
        let declared = admitted_company
            .connectors
            .iter()
            .find(|declared| declared.key == key)
            .ok_or_else(not_admitted)?;
        let kind = self
            .kinds
            .iter()
            .find(|kind| kind.declaration().key == key)
            .ok_or_else(|| ConnectorError {
                cause: format!("this instrukt has no {key} connector"),
            })?;
        kind.open(
            company.as_str(),
            &declared_values(&declared.entry),
            &AdmittedResolver(&admitted_company.secrets),
        )
    }

    fn now(&self) -> Timestamp {
        (self.clock)()
    }
}

/// A declared entry's fields as values: text as text, whole numbers as
/// counts; what a field means is its kind's declaration.
fn declared_values(entry: &Value) -> Declared {
    let mut declared = Declared::default();
    for (name, value) in entry.as_mapping().into_iter().flatten() {
        let Some(name) = name.as_str() else {
            continue;
        };
        if let Some(text) = value.as_str() {
            declared.insert_text(name, text);
        } else if let Some(count) = value.as_u64() {
            declared.insert_count(name, count);
        }
    }
    declared
}

/// A company's secrets as its generation admitted them.
struct AdmittedResolver<'a>(&'a AdmittedSecrets);

impl SecretResolver for AdmittedResolver<'_> {
    fn resolve(&self, reference: &str) -> Result<String, ConnectorError> {
        self.0
            .resolve(reference)
            .map(str::to_owned)
            .ok_or_else(|| ConnectorError {
                cause: format!("`{reference}` is not among the company's admitted secrets"),
            })
    }
}

/// Wakes the runner's supervision when a connection thread ends, whether it
/// returned or unwound.
struct ExitNotice(SyncSender<()>);
impl Drop for ExitNotice {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

struct Hosted {
    company: ProjectSlug,
    key: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Where a connection reaches its company: the admitted generation current
/// at each pass, and the connectors that open from it.
#[derive(Clone, Copy)]
struct Reach<'a> {
    current: &'a Current,
    connectors: &'a Arc<dyn Connectors>,
    accepted: &'a conversations::Accepted,
}

/// The connections `instrukt run` holds open, supervised by the run loop.
pub struct Connections {
    home: Home,
    current: Current,
    connectors: Arc<dyn Connectors>,
    accepted: Arc<conversations::Accepted>,
    stop: Arc<AtomicBool>,
    wake: SyncSender<()>,
    supervise: SyncSender<()>,
    hosted: Vec<Hosted>,
}

impl Connections {
    /// Open one connection per declared connector. `wake` tells the relay
    /// that retained work arrived; `supervise` tells the run loop that a
    /// connection thread ended.
    pub fn start(
        home: &Home,
        current: &Current,
        connectors: Arc<dyn Connectors>,
        accepted: Arc<conversations::Accepted>,
        wake: SyncSender<()>,
        supervise: SyncSender<()>,
    ) -> Result<Self, conversations::ConversationError> {
        let mut connections = Self {
            home: home.clone(),
            current: current.clone(),
            connectors,
            accepted,
            stop: Arc::new(AtomicBool::new(false)),
            wake,
            supervise,
            hosted: Vec::new(),
        };
        let demand = conversations::admitted_demand(&current.get());
        let hosted: Vec<(String, String)> = demand
            .iter()
            .map(|(company, key)| (company.to_string(), key.clone()))
            .collect();
        if let Err(error) = instrukt_runner::runner::publish_hosted(home, &hosted) {
            tracing::warn!(target: LOG_TARGET, event = "connector_hosting_unpublished", error = %error);
        }
        for (company, key) in demand {
            let thread = connections.spawn(&company, &key);
            connections.hosted.push(Hosted {
                company,
                key,
                thread,
            });
        }
        Ok(connections)
    }

    fn spawn(&self, company: &ProjectSlug, key: &str) -> Option<std::thread::JoinHandle<()>> {
        let home = self.home.clone();
        let current = self.current.clone();
        let connectors = Arc::clone(&self.connectors);
        let accepted = Arc::clone(&self.accepted);
        let stop = Arc::clone(&self.stop);
        let wake = self.wake.clone();
        let notice = ExitNotice(self.supervise.clone());
        let company = company.clone();
        let key = key.to_owned();
        let spawned = std::thread::Builder::new()
            .name(format!("{key}-{company}"))
            .spawn(move || {
                let _notice = notice;
                let reach = Reach {
                    current: &current,
                    connectors: &connectors,
                    accepted: &accepted,
                };
                connection(&home, reach, &company, &key, &stop, &wake);
            });
        match spawned {
            Ok(thread) => Some(thread),
            Err(error) => {
                tracing::error!(target: LOG_TARGET, event = "connector_start_failed", error = %error);
                None
            }
        }
    }

    /// Restart every connection whose thread ended unexpectedly, retaining
    /// the cause as that connector's failure.
    pub fn check(&mut self) {
        for index in 0..self.hosted.len() {
            let ended = self.hosted[index]
                .thread
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished);
            if !ended || self.stop.load(Ordering::Acquire) {
                continue;
            }
            let cause = match self.hosted[index]
                .thread
                .take()
                .map(std::thread::JoinHandle::join)
            {
                Some(Err(_)) => "the connection's intake panicked; restarted",
                _ => "the connection's intake stopped unexpectedly; restarted",
            };
            let hosted = &self.hosted[index];
            tracing::error!(target: LOG_TARGET, event = "connector_intake_stopped", company = %hosted.company, connector = %hosted.key, cause);
            state(
                &self.home,
                &hosted.company,
                &hosted.key,
                ConnectionState::Failed,
                Some(cause),
            );
            let thread = self.spawn(&hosted.company, &hosted.key);
            self.hosted[index].thread = thread;
        }
    }
}

impl Drop for Connections {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for hosted in &mut self.hosted {
            if let Some(thread) = hosted.thread.take()
                && thread.join().is_err()
            {
                tracing::error!(target: LOG_TARGET, event = "connector_thread_panicked");
            }
        }
    }
}

fn state(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    status: ConnectionState,
    failure: Option<&str>,
) {
    if let Err(error) = conversations::connector_state(home, company, key, status, failure) {
        tracing::warn!(target: LOG_TARGET, event = "connector_state_unrecorded", company = %company, error = %error);
    }
}

/// The host's side of one connection: what it retains is recorded and the
/// intake nudged; the connection's state follows what the connector reports.
struct HostInbox<'a> {
    home: &'a Home,
    company: &'a ProjectSlug,
    key: &'a str,
    nudge: &'a SyncSender<()>,
    clock: &'a dyn Connectors,
}

impl Inbox for HostInbox<'_> {
    fn retain(&self, incoming: Incoming) -> Result<(), ConnectorError> {
        conversations::retain(self.home, self.company, &incoming, self.clock.now()).map_err(
            |error| ConnectorError {
                cause: error.to_string(),
            },
        )?;
        let _ = self.nudge.try_send(());
        Ok(())
    }

    fn connected(&self) {
        state(
            self.home,
            self.company,
            self.key,
            ConnectionState::Connected,
            None,
        );
    }

    fn unretained(&self, cause: &str) {
        state(
            self.home,
            self.company,
            self.key,
            ConnectionState::Connected,
            Some(cause),
        );
    }
}

/// Keep one company's connection open until `stop`: a refused or dropped
/// connection is recorded and retried, and retained intake work continues
/// while it is down.
fn connection(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &AtomicBool,
    wake: &SyncSender<()>,
) {
    // Intake runs beside the connection, so reading what accompanies a
    // message or posting a refusal never holds back the next
    // acknowledgement. Its end ends this connection, which the run loop then
    // restarts. The working indicator has a worker of its own beside both.
    let ended = AtomicBool::new(false);
    let (nudge, nudges) = std::sync::mpsc::sync_channel::<()>(1);
    let (admitted, admissions) = std::sync::mpsc::sync_channel::<()>(1);
    let (context_wake, context_wakes) = std::sync::mpsc::sync_channel::<()>(1);
    let ended = &ended;
    let context_nudge = &context_wake;
    struct Ended<'a>(&'a AtomicBool);
    impl Drop for Ended<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ended = Ended(ended);
            intake_worker(&quit, &nudges, reach.connectors.as_ref(), || {
                intake_pass(home, reach, company, key, |connector| {
                    admit_messages(
                        home,
                        company,
                        key,
                        connector.as_ref(),
                        &admitted,
                        context_nudge,
                        reach.connectors.as_ref(),
                    )
                })
            });
        });
        scope.spawn(move || {
            let _ended = Ended(ended);
            let mut contexts = ContextReads::default();
            intake_worker(&quit, &context_wakes, reach.connectors.as_ref(), || {
                intake_pass(home, reach, company, key, |connector| {
                    capture_contexts(
                        home,
                        company,
                        key,
                        &Arc::from(connector),
                        Wakes {
                            relay: wake,
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
        let mut backoff = Duration::from_secs(1);
        while !quit() {
            state(home, company, key, ConnectionState::Connecting, None);
            let inbox = HostInbox {
                home,
                company,
                key,
                nudge: &nudge,
                clock: reach.connectors.as_ref(),
            };
            let failure = reach
                .connectors
                .open(&reach.current.get(), company, key)
                .and_then(|connector| connector.listen(&inbox, &quit))
                .err();
            if quit() {
                break;
            }
            if let Some(error) = &failure {
                tracing::warn!(target: LOG_TARGET, event = "connector_failed", company = %company, connector = key, error = %error);
                state(
                    home,
                    company,
                    key,
                    ConnectionState::Failed,
                    Some(&error.cause),
                );
            }
            let until = Instant::now() + backoff;
            while Instant::now() < until && !quit() {
                std::thread::sleep(Duration::from_millis(100));
            }
            backoff = if failure.is_some() {
                (backoff * 2).min(Duration::from_secs(60))
            } else {
                Duration::from_secs(1)
            };
        }
    });
    if stop.load(Ordering::Acquire) {
        state(home, company, key, ConnectionState::Stopped, None);
    }
}

/// Whom intake wakes: the relay, for what became deliverable, and the
/// working pass, for a message just admitted.
#[derive(Clone, Copy)]
struct Wakes<'a> {
    relay: &'a SyncSender<()>,
    context: &'a SyncSender<()>,
}

/// How long intake waits before its next pass after `failures` failed
/// passes in a row: [`RETRY`] after a clean pass and after the first
/// failure, doubling with each further one up to [`MOST_BACKOFF`].
fn retry_wait(failures: u32) -> Duration {
    if failures == 0 {
        RETRY
    } else {
        RETRY
            .saturating_mul(1 << (failures - 1).min(5))
            .min(MOST_BACKOFF)
    }
}

/// The company's retained intake work, until `stop`: run when a retained
/// message nudges it, every [`RETRY`] while it runs clean, and after a
/// failure once its wait ([`retry_wait`]) has passed.
fn intake_worker(
    stop: &dyn Fn() -> bool,
    nudges: &std::sync::mpsc::Receiver<()>,
    clock: &dyn Connectors,
    mut pass: impl FnMut() -> IntakeProgress,
) {
    let mut failures: u32 = 0;
    while !stop() {
        let progress = pass();
        if !progress.clean {
            failures = failures.saturating_add(1);
        } else if !progress.reading_context {
            failures = 0;
        }
        let wait = if progress.reading_context {
            RETRY
        } else {
            retry_wait(failures)
        };
        let retry_at = Instant::now() + wait;
        while Instant::now() < retry_at
            && progress
                .next_deadline
                .is_none_or(|deadline| clock.now() < deadline)
            && !stop()
        {
            if nudges.recv_timeout(Duration::from_millis(100)).is_ok() {
                break;
            }
        }
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
        IntakeProgress {
            clean: false,
            ..Default::default()
        }
    };
    match reach.connectors.open(&reach.current.get(), company, key) {
        Ok(connector) => pass(connector).unwrap_or_else(failed),
        Err(error) => {
            if let Err(record) = conversations::intake_failed(home, company, key, &error.cause) {
                return failed(record);
            }
            IntakeProgress {
                clean: false,
                ..Default::default()
            }
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
    // A fixture can request a pass after it advances its operator clock.
    // This signal is independent of the clock used in lock-wait scenarios.
    let wake_file = cfg!(debug_assertions)
        .then(|| std::env::var_os("INSTRUKT_TEST_NOTICE_WAKE_FILE"))
        .flatten();
    let wake = || wake_file.as_ref().and_then(|path| std::fs::read(path).ok());
    std::thread::scope(|scope| {
        let mut pending = BTreeMap::new();
        while !stop() {
            let current_wake = wake();
            pending
                .retain(|_, task: &mut std::thread::ScopedJoinHandle<'_, ()>| !task.is_finished());
            let now = reach.connectors.now();
            if let Err(error) =
                conversations::expire_unconfirmed_answers(home, company, key, reach.accepted, now)
            {
                tracing::warn!(target: LOG_TARGET, event = "answer_expiry_failed", company = %company, connector = key, %error);
            }
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
                if wake() != current_wake {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
}

/// Read back the post outside the conversation lock, then decide and post
/// under the lock shared with answers.
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
            tracing::warn!(target: LOG_TARGET, event = "conversation_notice_failed", company = %company, connector = key, error = %error.cause);
            return;
        }
    };
    let text = connector.unavailable_notice(&notice.sender);
    let reconciliation = if notice.posting {
        let read_at = reach.connectors.now();
        let Some(post_id) = &notice.post_id else {
            tracing::warn!(target: LOG_TARGET, event = "conversation_notice_missing_post_identity", company = %company, connector = key);
            return;
        };
        match connector.post_receipt(&notice.conversation, &notice.since_message, post_id) {
            Ok(receipt) => Some((read_at, receipt)),
            // Not knowing whether it landed, nothing is posted now.
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "conversation_notice_unread", company = %company, connector = key, error = %error.cause);
                return;
            }
        }
    } else {
        None
    };
    let lock = conversations::post_lock(company, key, &notice.conversation);
    let _held = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let step = conversations::notice_step(
        home,
        company,
        key,
        notice,
        reconciliation,
        reach.accepted,
        reach.connectors.now(),
    );
    let result = match step {
        Ok(conversations::NoticeStep::Post(post_id)) => {
            let outcome = connector
                .answer(&notice.conversation, &text, &post_id)
                .map_err(|error| error.cause);
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
    // Append only after the decision and any resulting post have completed,
    // so a fixture holding a receipt can wait for this member's own task.
    if cfg!(debug_assertions)
        && result.is_ok()
        && let Some(path) = std::env::var_os("INSTRUKT_TEST_NOTICE_DECISIONS")
        && let Ok(mut file) =
            instrukt_core::fsx::open(std::fs::OpenOptions::new().create(true).append(true), path)
    {
        use std::io::Write as _;
        let decision = format!("{}\t{}\n", notice.conversation, notice.sender);
        let _ = file.write_all(decision.as_bytes());
    }
    if let Err(error) = result {
        tracing::warn!(target: LOG_TARGET, event = "conversation_notice_failed", company = %company, connector = key, error = %error);
    }
}

/// What a received message's record becomes from what the connector says
/// about its sender.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Admitted, with what the provider gave about the sender.
    Admitted(Sender),
    /// Refused: the sender receives the fixed reply.
    Refused,
    /// It waits, with this cause.
    Waiting(String),
}

fn next_record(admission: Admission) -> Next {
    match admission {
        Admission::Admitted(sender) => Next::Admitted(sender),
        Admission::NotOwn => Next::Refused,
        Admission::Undecided(cause) => Next::Waiting(cause),
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
    Readable(ContextSnapshot, Timestamp, Instant),
    Finished(Result<Captured, ConnectorError>, Timestamp),
}

struct ContextFlight {
    replies: std::sync::mpsc::Receiver<ContextReply>,
    readable: Option<(ContextSnapshot, Instant)>,
}

#[derive(Default)]
struct ContextReads {
    active: BTreeMap<(String, String), ContextFlight>,
}

impl ContextReads {
    fn capture(
        &mut self,
        connector: &Arc<dyn Connector>,
        item: &conversations::Unread,
        clock: &Arc<dyn Connectors>,
        wake: &SyncSender<()>,
        deadline: Timestamp,
    ) -> Option<Result<Captured, ConnectorError>> {
        use std::sync::mpsc::{self, TryRecvError};
        let key = (item.conversation.clone(), item.message.clone());
        let now = clock.now();
        if let Some(flight) = self.active.get_mut(&key) {
            loop {
                match flight.replies.try_recv() {
                    Ok(ContextReply::Readable(snapshot, at, ready)) if at < deadline => {
                        flight.readable = Some((snapshot, ready + RETRY));
                    }
                    Ok(ContextReply::Readable(..)) => {}
                    Ok(ContextReply::Finished(Ok(captured), at)) if at < deadline => {
                        self.active.remove(&key);
                        return Some(Ok(captured));
                    }
                    Ok(ContextReply::Finished(outcome, finished)) => {
                        let readable = self.active.remove(&key).and_then(|flight| flight.readable);
                        if let Some((snapshot, _)) = readable {
                            return Some(Ok(snapshot()));
                        }
                        if now >= deadline || finished >= deadline {
                            return Some(Ok(connector.without_context(&item.text)));
                        }
                        return Some(outcome);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        let readable = self.active.remove(&key).and_then(|flight| flight.readable);
                        if let Some((snapshot, _)) = readable {
                            return Some(Ok(snapshot()));
                        }
                        if now < deadline {
                            return Some(Err(ConnectorError {
                                cause: "the context reader stopped before reporting its result"
                                    .into(),
                            }));
                        }
                        break;
                    }
                }
            }
        }
        let enrichment_done = self
            .active
            .get(&key)
            .and_then(|flight| flight.readable.as_ref())
            .is_some_and(|(_, ready)| Instant::now() >= *ready);
        if now >= deadline || enrichment_done {
            let readable = self.active.remove(&key).and_then(|flight| flight.readable);
            return Some(Ok(readable.map_or_else(
                || connector.without_context(&item.text),
                |(snapshot, _)| snapshot(),
            )));
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = self.active.entry(key) {
            let connector = Arc::clone(connector);
            let clock = Arc::clone(clock);
            let item = item.clone();
            let wake = wake.clone();
            let (send, replies) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("conversation-context".into())
                .spawn(move || {
                    let readable = |snapshot| {
                        if send.send(ContextReply::Readable(snapshot, clock.now(), Instant::now())).is_ok() {
                            let _ = wake.try_send(());
                        }
                    };
                    let result = connector.capture(
                        &item.conversation, &item.message, item.previous.as_deref(), &item.text, &readable,
                    );
                    if send.send(ContextReply::Finished(result, clock.now())).is_err() {
                        tracing::debug!(target: LOG_TARGET, event = "conversation_context_late_result", message = %item.message);
                    }
                    let _ = wake.try_send(());
                });
            if let Err(error) = spawned {
                return Some(Err(ConnectorError {
                    cause: format!("cannot start context read: {error}"),
                }));
            }
            slot.insert(ContextFlight {
                replies,
                readable: None,
            });
        }
        None
    }
}

struct IntakeProgress {
    clean: bool,
    reading_context: bool,
    next_deadline: Option<Timestamp>,
}

impl Default for IntakeProgress {
    fn default() -> Self {
        Self {
            clean: true,
            reading_context: false,
            next_deadline: None,
        }
    }
}

/// Sender decisions and refusals can call Slack. Context expiry and relay
/// handoff run on a different worker.
fn admit_messages(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    connector: &dyn Connector,
    status: &SyncSender<()>,
    context: &SyncSender<()>,
    clock: &dyn Connectors,
) -> Result<IntakeProgress, conversations::ConversationError> {
    let mut progress = IntakeProgress::default();
    for unread in conversations::unread(home, company, key)?
        .into_iter()
        .filter(|item| !item.admitted)
    {
        let Some(sender) = unread.sender.as_deref() else {
            progress.clean = false;
            conversations::waiting(home, company, key, &unread, "the message has no sender")?;
            continue;
        };
        match next_record(connector.admit(sender)) {
            Next::Admitted(admitted) => {
                conversations::admitted(home, company, key, &unread, &admitted, clock.now())?;
                let _ = status.try_send(());
                let _ = context.try_send(());
            }
            Next::Refused => {
                conversations::refused(home, company, key, &unread, sender, Timestamp::now())?;
            }
            Next::Waiting(cause) => {
                progress.clean = false;
                conversations::waiting(home, company, key, &unread, &cause)?;
            }
        }
    }
    for refusal in owed(conversations::refusals(home, company, key)?) {
        let outcome = connector
            .turn_away(&refusal.conversation)
            .map_err(|error| error.cause);
        progress.clean &= outcome.is_ok();
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
    #[cfg(debug_assertions)]
    let now = clock.now();
    for unread in conversations::unread(home, company, key)?
        .into_iter()
        .filter(|item| item.admitted)
    {
        let deadline = unread
            .arrived_at
            .checked_add(conversations::CONTEXT_BOUND)
            .unwrap_or(Timestamp::MAX);
        match contexts.capture(connector, &unread, clock, wakes.context, deadline) {
            Some(Ok(captured)) => {
                conversations::captured(
                    home,
                    company,
                    key,
                    &unread,
                    &captured.text,
                    Ok(captured.accompanying),
                )?;
                let _ = wakes.relay.try_send(());
                continue;
            }
            Some(Err(error)) => {
                progress.clean = false;
                conversations::captured(home, company, key, &unread, "", Err(error.cause))?;
            }
            None => progress.reading_context = true,
        }
        progress.next_deadline = Some(
            progress
                .next_deadline
                .map_or(deadline, |next| next.min(deadline)),
        );
    }
    #[cfg(debug_assertions)]
    if instrukt_core::fsx::fence_active()
        && let Some(path) = std::env::var_os("INSTRUKT_TEST_CONTEXT_PASS")
    {
        let _ = instrukt_core::fsx::atomic_write(
            std::path::Path::new(&path),
            now.to_string().as_bytes(),
        );
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
    now: Instant,
    refresh: Option<Duration>,
) -> StatusPass {
    let set = wanted
        .iter()
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
    admitted: &std::sync::mpsc::Receiver<()>,
) {
    let mut attempted: HashMap<Place, (Instant, Status)> = HashMap::new();
    let mut failing: Option<String> = None;
    while !stop() {
        let pass = reach
            .connectors
            .open(&reach.current.get(), company, key)
            .map_err(|error| error.cause)
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
) {
    let wanted = wanted(connector, unfinished);
    let now = Instant::now();
    let pass = status_pass(&wanted, attempted, now, connector.limits().working_refresh);
    for (place, status) in pass.set {
        call(connector, company, key, &place, Some(&status));
        attempted.insert(place, (now, status));
    }
    for place in pass.clear {
        attempted.remove(&place);
        call(connector, company, key, &place, None);
    }
}

fn call(
    connector: &dyn Connector,
    company: &ProjectSlug,
    key: &str,
    place: &Place,
    status: Option<&Status>,
) {
    if let Err(error) = connector.status(place, status) {
        tracing::debug!(target: LOG_TARGET, event = "connector_status_skipped", company = %company, connector = key, error = %error);
    }
}

#[cfg(test)]
mod test_connector {
    //! An in-memory connector whose facts each test sets: its limits and
    //! where it places a message's status. It records nothing.
    use instrukt_connector::{
        Admission, Captured, Connector, ConnectorError, Inbox, Limits, Place, Status,
    };
    use jiff::Timestamp;

    pub(super) struct TestConnector {
        pub limits: Limits,
        /// Places every message of a conversation at the conversation, or
        /// each message at itself.
        pub per_message: bool,
    }

    impl Connector for TestConnector {
        fn limits(&self) -> Limits {
            self.limits
        }
        fn listen(&self, _: &dyn Inbox, _: &dyn Fn() -> bool) -> Result<(), ConnectorError> {
            Ok(())
        }
        fn admit(&self, _: &str) -> Admission {
            Admission::NotOwn
        }
        fn capture(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            text: &str,
            _: &dyn Fn(instrukt_connector::ContextSnapshot),
        ) -> Result<Captured, ConnectorError> {
            Ok(Captured {
                text: text.to_owned(),
                accompanying: Default::default(),
            })
        }
        fn answer(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<Timestamp>, instrukt_connector::AnswerError> {
            Ok(None)
        }
        fn post_receipt(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<Timestamp>, ConnectorError> {
            Ok(None)
        }
        fn legacy_answer_receipt(
            &self,
            _: &str,
            _: Timestamp,
            _: &str,
        ) -> Result<Option<Timestamp>, ConnectorError> {
            Ok(None)
        }
        fn turn_away(&self, _: &str) -> Result<(), ConnectorError> {
            Ok(())
        }
        fn unavailable_notice(&self, member: &str) -> String {
            format!("{member}: unavailable")
        }
        fn place(&self, conversation: &str, message: &str) -> Place {
            if self.per_message {
                Place(conversation.to_owned(), message.to_owned())
            } else {
                Place(conversation.to_owned(), conversation.to_owned())
            }
        }
        fn status(&self, _: &Place, _: Option<&Status>) -> Result<(), ConnectorError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_connector::TestConnector;
    use super::*;
    use instrukt_connector::Limits;

    fn connector(per_message: bool) -> TestConnector {
        TestConnector {
            limits: Limits {
                message_chars: 100,
                answer_timeout: Duration::from_secs(30),
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
                    notice_eligible: true,
                    admission_position: None,
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
        let first = status_pass(&wanted, &HashMap::new(), base, refresh);
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
        let early = status_pass(&wanted, &attempted, base + Duration::from_secs(29), refresh);
        assert!(early.set.is_empty());
        // A place whose state changed is set at the next pass.
        let changed = BTreeMap::from([(place(&connector, "C1", "1"), Status::Unavailable)]);
        let next = status_pass(&changed, &attempted, base + Duration::from_secs(1), refresh);
        assert_eq!(
            next.set,
            [(place(&connector, "C1", "1"), Status::Unavailable)]
        );
        // At the interval a place that still has work is set again; one
        // whose work finished is cleared.
        let later = BTreeMap::from([(place(&connector, "C1", "1"), Status::Working)]);
        let due = status_pass(&later, &attempted, base + Duration::from_secs(30), refresh);
        assert_eq!(due.set, [(place(&connector, "C1", "1"), Status::Working)]);
        assert_eq!(due.clear, [place(&connector, "C2", "3")]);
        // Without a refresh interval a set place is never set again.
        attempted.insert(place(&connector, "C1", "1"), (base, Status::Working));
        let once = status_pass(&later, &attempted, base + Duration::from_secs(600), None);
        assert!(once.set.is_empty());
    }

    #[test]
    fn the_status_pass_follows_where_the_connector_places_each_message() {
        let connector = connector(true);
        let wanted = wanted(
            &connector,
            &unfinished(&[("D1", "1", Status::Working), ("D1", "2", Status::Working)]),
        );
        let pass = status_pass(&wanted, &HashMap::new(), Instant::now(), None);
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
    fn the_intake_wait_doubles_after_each_failure_up_to_a_minute() {
        assert_eq!(retry_wait(0), Duration::from_secs(2));
        assert_eq!(retry_wait(1), Duration::from_secs(2));
        assert_eq!(retry_wait(2), Duration::from_secs(4));
        assert_eq!(retry_wait(3), Duration::from_secs(8));
        assert_eq!(retry_wait(6), Duration::from_secs(60));
        assert_eq!(retry_wait(40), Duration::from_secs(60));
    }

    #[test]
    fn what_the_connector_says_of_a_sender_decides_the_next_record() {
        let sender = Sender {
            name: "Ada".into(),
            email: None,
        };
        assert_eq!(
            next_record(Admission::Admitted(sender.clone())),
            Next::Admitted(sender)
        );
        assert_eq!(next_record(Admission::NotOwn), Next::Refused);
        assert_eq!(
            next_record(Admission::Undecided("later".into())),
            Next::Waiting("later".into())
        );
    }

    #[test]
    fn only_refusals_not_yet_posted_are_owed() {
        let refusal = |conversation: &str| Refusal {
            conversation: conversation.into(),
            sender: "U1".into(),
        };
        let owed = owed(vec![(refusal("C1/1"), true), (refusal("C1/2"), false)]);
        assert_eq!(
            owed.iter()
                .map(|r| r.conversation.as_str())
                .collect::<Vec<_>>(),
            ["C1/2"]
        );
    }

    #[test]
    fn a_declared_entry_reaches_its_kind_as_text_and_counts() {
        let entry: Value =
            serde_yaml_ng::from_str("{app_token: '${A}', context_messages: 5}").unwrap();
        let declared = declared_values(&entry);
        assert_eq!(declared.text("app_token"), Some("${A}"));
        assert_eq!(declared.count("context_messages"), Some(5));
    }
}
