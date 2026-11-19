//! The connector host of `instrukt run` (`slack-connector` and
//! `conversations` specs): everything around a connector's operations,
//! whatever its kind. One connection per declared connector, recorded and
//! reopened when it drops; each message a connector hands over is retained
//! before it is acknowledged; deciding who is answered, reading what
//! accompanies a message, posting refusals and keeping the working indicator
//! follow from the retained record, retried until done. A connector is
//! opened from the admitted generation current at each pass, its secrets
//! resolved from that generation: no connector reads a company's files.
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use instrukt_connector::{
    Admission, Clock, Connector, ConnectorError, Declared, Inbox, Incoming, Kind, Place,
    SecretResolver, Sender,
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
}

/// The connections `instrukt run` holds open, supervised by the run loop.
pub struct Connections {
    home: Home,
    current: Current,
    connectors: Arc<dyn Connectors>,
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
        wake: SyncSender<()>,
        supervise: SyncSender<()>,
    ) -> Result<Self, conversations::ConversationError> {
        let mut connections = Self {
            home: home.clone(),
            current: current.clone(),
            connectors,
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
                Ok(connector) => intake(home, company, key, connector.as_ref(), wakes),
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
                        conversations::admitted(home, company, key, &unread, &admitted)?;
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

/// What one working pass does: the places to set (unfinished work whose
/// indicator was not set yet, or was set `refresh` ago or more) and the
/// places to clear (set by this process, with no unfinished work left).
#[derive(Debug, PartialEq, Eq)]
struct WorkingPass {
    set: Vec<Place>,
    clear: Vec<Place>,
}

fn working_pass(
    wanted: &[Place],
    attempted: &HashMap<Place, Instant>,
    now: Instant,
    refresh: Option<Duration>,
) -> WorkingPass {
    let wanted: BTreeSet<&Place> = wanted.iter().collect();
    let set = wanted
        .iter()
        .filter(|place| {
            attempted.get(**place).is_none_or(|at| {
                refresh.is_some_and(|interval| now.saturating_duration_since(*at) >= interval)
            })
        })
        .map(|place| (*place).clone())
        .collect();
    let mut clear: Vec<Place> = attempted
        .keys()
        .filter(|place| !wanted.contains(place))
        .cloned()
        .collect();
    clear.sort();
    WorkingPass { set, clear }
}

/// The working indicator for one company, a courtesy kept on a best-effort
/// basis apart from intake and the relay, so no indicator call holds a
/// message. A plain periodic pass reads the places with unfinished admitted
/// messages and does what [`working_pass`] decides. A message's admission
/// wakes the pass, so its first indicator comes promptly. A call that fails
/// or is skipped is left to the next regular setting; nothing is made up for
/// and nothing set by an earlier run is cleared.
fn status_worker(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &dyn Fn() -> bool,
    admitted: &std::sync::mpsc::Receiver<()>,
) {
    let mut attempted: HashMap<Place, Instant> = HashMap::new();
    let mut failing: Option<String> = None;
    while !stop() {
        let pass = reach
            .connectors
            .open(&reach.current.get(), company, key)
            .map_err(|error| error.cause)
            .and_then(|connector| {
                let working =
                    conversations::working(home, company, key).map_err(|e| e.to_string())?;
                Ok((connector, working))
            });
        match pass {
            Ok((connector, working)) => {
                if let Some(cause) = failing.take() {
                    tracing::info!(target: LOG_TARGET, event = "connector_status_recovered", company = %company, cause = %cause);
                }
                keep(connector.as_ref(), company, key, &working, &mut attempted);
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

/// One working pass on `connector`: set what stays and is due, clear once
/// what no longer stays. Each call's outcome is only logged.
fn keep(
    connector: &dyn Connector,
    company: &ProjectSlug,
    key: &str,
    working: &[(String, String)],
    attempted: &mut HashMap<Place, Instant>,
) {
    let wanted: Vec<Place> = working
        .iter()
        .map(|(conversation, message)| connector.place(conversation, message))
        .collect();
    let now = Instant::now();
    let pass = working_pass(&wanted, attempted, now, connector.limits().working_refresh);
    for place in pass.set {
        attempted.insert(place.clone(), now);
        call(connector, company, key, &place, true);
    }
    for place in pass.clear {
        attempted.remove(&place);
        call(connector, company, key, &place, false);
    }
}

fn call(connector: &dyn Connector, company: &ProjectSlug, key: &str, place: &Place, on: bool) {
    if let Err(error) = connector.working(place, on) {
        tracing::debug!(target: LOG_TARGET, event = "connector_status_skipped", company = %company, connector = key, error = %error);
    }
}

#[cfg(test)]
mod test_connector {
    //! An in-memory connector whose facts each test sets: its limits and
    //! where it places a message's working indicator. It records nothing.
    use instrukt_connector::{
        Admission, Captured, Connector, ConnectorError, Inbox, Limits, Place,
    };

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
        fn answer(&self, _: &str, _: &str) -> Result<(), ConnectorError> {
            Ok(())
        }
        fn own_posts(&self, _: &str, _: &str) -> Result<Vec<String>, ConnectorError> {
            Ok(Vec::new())
        }
        fn turn_away(&self, _: &str) -> Result<(), ConnectorError> {
            Ok(())
        }
        fn place(&self, conversation: &str, message: &str) -> Place {
            if self.per_message {
                Place(conversation.to_owned(), message.to_owned())
            } else {
                Place(conversation.to_owned(), conversation.to_owned())
            }
        }
        fn working(&self, _: &Place, _: bool) -> Result<(), ConnectorError> {
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
                working_refresh: Some(Duration::from_secs(30)),
            },
            per_message,
        }
    }

    fn places(connector: &TestConnector, working: &[(&str, &str)]) -> Vec<Place> {
        working
            .iter()
            .map(|(conversation, message)| connector.place(conversation, message))
            .collect()
    }

    #[test]
    fn the_working_pass_sets_new_and_due_places_and_clears_finished_ones() {
        let connector = connector(false);
        let base = Instant::now();
        let refresh = connector.limits.working_refresh;
        let wanted = places(&connector, &[("C1", "1"), ("C1", "2"), ("C2", "3")]);
        // Nothing set yet: each place with unfinished work is set once.
        let first = working_pass(&wanted, &HashMap::new(), base, refresh);
        assert_eq!(first.set, places(&connector, &[("C1", "1"), ("C2", "3")]));
        assert!(first.clear.is_empty());
        let mut attempted: HashMap<Place, Instant> = first
            .set
            .iter()
            .map(|place| (place.clone(), base))
            .collect();
        // Before the interval nothing is set again.
        let early = working_pass(&wanted, &attempted, base + Duration::from_secs(29), refresh);
        assert!(early.set.is_empty());
        // At the interval a place that still has work is set again; one
        // whose work finished is cleared.
        let later = places(&connector, &[("C1", "1")]);
        let due = working_pass(&later, &attempted, base + Duration::from_secs(30), refresh);
        assert_eq!(due.set, later);
        assert_eq!(due.clear, places(&connector, &[("C2", "3")]));
        // Without a refresh interval a set place is never set again.
        attempted.insert(later[0].clone(), base);
        let once = working_pass(&later, &attempted, base + Duration::from_secs(600), None);
        assert!(once.set.is_empty());
    }

    #[test]
    fn the_working_pass_follows_where_the_connector_places_each_message() {
        let connector = connector(true);
        let wanted = places(&connector, &[("D1", "1"), ("D1", "2")]);
        let pass = working_pass(&wanted, &HashMap::new(), Instant::now(), None);
        assert_eq!(pass.set.len(), 2);
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
