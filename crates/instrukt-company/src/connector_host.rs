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
    Admission, Clock, Connector, ConnectorError, Declared, Inbox, Incoming, Kind, Place,
    SecretResolver, Sender, Status,
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
    connectors: &'a dyn Connectors,
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
                    connectors: connectors.as_ref(),
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
}

impl Inbox for HostInbox<'_> {
    fn retain(&self, incoming: Incoming) -> Result<(), ConnectorError> {
        conversations::retain(self.home, self.company, &incoming, Timestamp::now()).map_err(
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
    let ended = &ended;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            struct Ended<'a>(&'a AtomicBool);
            impl Drop for Ended<'_> {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _ended = Ended(ended);
            intake_worker(
                home,
                reach,
                company,
                key,
                stop,
                Wakes {
                    relay: wake,
                    status: &admitted,
                },
                &nudges,
            );
        });
        scope.spawn(move || {
            let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
            status_worker(home, reach, company, key, &quit, &admissions);
        });
        scope.spawn(move || {
            let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
            notice_worker(home, reach, company, key, &quit);
        });
        let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
        let mut backoff = Duration::from_secs(1);
        while !quit() {
            state(home, company, key, ConnectionState::Connecting, None);
            let inbox = HostInbox {
                home,
                company,
                key,
                nudge: &nudge,
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
    status: &'a SyncSender<()>,
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
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &AtomicBool,
    wakes: Wakes<'_>,
    nudges: &std::sync::mpsc::Receiver<()>,
) {
    let mut failures: u32 = 0;
    let mut retry_at = Instant::now();
    let mut nudged = true;
    while !stop.load(Ordering::Acquire) {
        if nudged || Instant::now() >= retry_at {
            let clean = match reach.connectors.open(&reach.current.get(), company, key) {
                Ok(connector) => intake(home, company, key, connector.as_ref(), wakes, &|| {
                    reach.connectors.now()
                }),
                // A failure before any request stays with the outstanding work.
                Err(error) => {
                    if let Err(record) =
                        conversations::intake_failed(home, company, key, &error.cause)
                    {
                        tracing::warn!(target: LOG_TARGET, event = "connector_intake_failed", company = %company, connector = key, error = %record);
                    }
                    false
                }
            };
            failures = if clean { 0 } else { failures.saturating_add(1) };
            retry_at = Instant::now() + retry_wait(failures);
        }
        nudged = false;
        while Instant::now() < retry_at && !stop.load(Ordering::Acquire) {
            if nudges.recv_timeout(Duration::from_millis(100)).is_ok() {
                nudged = true;
                break;
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

/// Retained intake work of one company: decide for each received message,
/// from what the connector says about its sender now, whether it is
/// answered; read what accompanies each admitted one; post owed refusals. A
/// failure stays with its record and is retried; while the provider cannot
/// be asked, a message waits and nothing is delivered or replied. The relay
/// is woken for what became deliverable. Returns whether the pass ended
/// without a failure.
fn intake(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    connector: &dyn Connector,
    wakes: Wakes<'_>,
    now: &dyn Fn() -> Timestamp,
) -> bool {
    let mut clean = true;
    let result = (|| -> Result<(), conversations::ConversationError> {
        let mut readable = false;
        for unread in conversations::unread(home, company, key)? {
            if !unread.admitted {
                let Some(sender) = unread.sender.as_deref() else {
                    clean = false;
                    conversations::waiting(
                        home,
                        company,
                        key,
                        &unread,
                        "the message has no sender",
                    )?;
                    continue;
                };
                match next_record(connector.admit(sender)) {
                    Next::Admitted(admitted) => {
                        conversations::admitted(home, company, key, &unread, &admitted, now())?;
                        // Its working indicator is arranged now, not after
                        // the batch.
                        let _ = wakes.status.try_send(());
                    }
                    Next::Refused => {
                        conversations::refused(
                            home,
                            company,
                            key,
                            &unread,
                            sender,
                            Timestamp::now(),
                        )?;
                        continue;
                    }
                    Next::Waiting(cause) => {
                        clean = false;
                        conversations::waiting(home, company, key, &unread, &cause)?;
                        continue;
                    }
                }
            }
            let captured = connector.capture(
                &unread.conversation,
                &unread.message,
                unread.previous.as_deref(),
                &unread.text,
            );
            readable |= captured.is_ok();
            clean &= captured.is_ok();
            let (text, accompanying) = match captured {
                Ok(captured) => (captured.text, Ok(captured.accompanying)),
                Err(error) => (String::new(), Err(error.cause)),
            };
            conversations::captured(home, company, key, &unread, &text, accompanying)?;
        }
        if readable {
            let _ = wakes.relay.try_send(());
        }
        // Replies owed, those decided now among them.
        for refusal in owed(conversations::refusals(home, company, key)?) {
            let outcome = connector
                .turn_away(&refusal.conversation)
                .map_err(|error| error.cause);
            clean &= outcome.is_ok();
            conversations::refusal_posted(home, company, key, &refusal, outcome, Timestamp::now())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        clean = false;
        tracing::warn!(target: LOG_TARGET, event = "connector_intake_failed", company = %company, connector = key, error = %error);
    }
    clean
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
