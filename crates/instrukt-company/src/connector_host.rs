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
    Incoming, Kind, Place, RefusalReason, SecretResolver, Sender, Status,
};
use instrukt_core::config::{
    Admitted, AdmittedSecrets, ConnectorAdmit, Current, ReplyMode, SessionScope,
};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName};
use instrukt_outside::Wait;
use instrukt_runner::state::RunnerState;
use jiff::Timestamp;
use serde_yaml_ng::Value;

use crate::conversations::{self, ConnectionState, Refusal};

/// The target every log statement of the host carries: the module the host
/// and Slack shared before the cut, so the same settings select it.
const LOG_TARGET: &str = "instrukt_runner::connectors";

/// How often the working pass looks at what is unfinished.
const STATUS_TICK: Duration = Duration::from_secs(1);

/// How long retained intake work waits between passes, and before retrying
/// a message whose failure carried no wait.
const RETRY: Duration = Duration::from_secs(2);

/// The pause between one connection's end and the next opening.
const RECONNECT: Duration = Duration::from_secs(1);

/// How long a whole loop (admission, the connection, read-back, an intake
/// pass) waits after a failure that carries no wait: a local failure or a
/// defect. A fixed supervision recheck, not a backoff.
const RECHECK: Duration = Duration::from_secs(60);

/// When a whole loop tries again after a failure: at the outside gate's
/// moment when the failure carried a wait, otherwise after [`RECHECK`].
fn after_failure(wait: Option<&Wait>, now: Instant) -> Instant {
    wait.map_or(now + RECHECK, Wait::until)
}

/// Whether the host clock has reached a stored moment.
fn due(until: Timestamp, now: Timestamp) -> bool {
    now >= until
}

/// Sleep until `until` or `stop`, in slices that observe it.
fn sleep_until(until: Instant, stop: &dyn Fn() -> bool) {
    while Instant::now() < until && !stop() {
        std::thread::sleep(Duration::from_millis(100).min(until - Instant::now()));
    }
}

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
    fn ready(&self, _admitted: &Admitted, _company: &ProjectSlug, _key: &str) -> bool {
        true
    }
    fn set_ready(&self, _company: &ProjectSlug, _key: &str, _identity: Option<ConnectorIdentity>) {}

    /// The time now on the runner's clock.
    fn now(&self) -> Timestamp;
}

/// The kinds built into this process, each built once when the runner
/// starts, and the runner's clock.
#[derive(Clone, PartialEq)]
pub struct ConnectorIdentity {
    entry: Value,
    secrets: AdmittedSecrets,
}

fn identity(admitted: &Admitted, company: &ProjectSlug, key: &str) -> Option<ConnectorIdentity> {
    let company = admitted.company(company)?;
    let declared = company
        .connectors
        .iter()
        .find(|connector| connector.key == key)?;
    Some(ConnectorIdentity {
        entry: declared.entry.clone(),
        secrets: company.secrets.clone(),
    })
}

type ConnectorKey = (ProjectSlug, String);

pub struct Directory {
    kinds: Vec<Box<dyn Kind>>,
    clock: Clock,
    ready: std::sync::Mutex<std::collections::HashMap<ConnectorKey, ConnectorIdentity>>,
}

impl Directory {
    pub fn new(kinds: Vec<Box<dyn Kind>>, clock: Clock) -> Self {
        Self {
            kinds,
            clock,
            ready: Default::default(),
        }
    }
}

impl Connectors for Directory {
    fn open(
        &self,
        admitted: &Admitted,
        company: &ProjectSlug,
        key: &str,
    ) -> Result<Box<dyn Connector>, ConnectorError> {
        let not_admitted = || {
            ConnectorError::new(format!(
                "the {key} connector of company `{company}` is not admitted"
            ))
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
            .find(|kind| kind.declaration().key == declared.kind)
            .ok_or_else(|| {
                ConnectorError::new(format!(
                    "this instrukt has no {} connector kind",
                    declared.kind
                ))
            })?;
        kind.open(
            company.as_str(),
            key,
            &declared_values(&declared.entry),
            &AdmittedResolver(&admitted_company.secrets),
        )
    }

    fn ready(&self, admitted: &Admitted, company: &ProjectSlug, key: &str) -> bool {
        let ready = self
            .ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ready
            .get(&(company.clone(), key.to_owned()))
            .is_some_and(|known| identity(admitted, company, key).as_ref() == Some(known))
    }

    fn set_ready(&self, company: &ProjectSlug, key: &str, identity: Option<ConnectorIdentity>) {
        let mut ready = self
            .ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (company.clone(), key.to_owned());
        if let Some(identity) = identity {
            ready.insert(key, identity);
        } else {
            ready.remove(&key);
        }
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
        self.0.resolve(reference).map(str::to_owned).ok_or_else(|| {
            ConnectorError::new(format!(
                "`{reference}` is not among the company's admitted secrets"
            ))
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
    identity: ConnectorIdentity,
    stop: Arc<AtomicBool>,
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
    wake: SyncSender<()>,
    supervise: SyncSender<()>,
    hosted: Vec<Hosted>,
    observed: std::collections::HashSet<ConnectorKey>,
    published: Vec<(String, String)>,
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
            wake,
            supervise,
            hosted: Vec::new(),
            observed: Default::default(),
            published: Vec::new(),
        };
        connections.check();
        Ok(connections)
    }

    fn spawn(
        &self,
        company: &ProjectSlug,
        key: &str,
        identity: ConnectorIdentity,
        stop: Arc<AtomicBool>,
    ) -> Option<std::thread::JoinHandle<()>> {
        let home = self.home.clone();
        let current = self.current.clone();
        let connectors = Arc::clone(&self.connectors);
        let accepted = Arc::clone(&self.accepted);
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
                admit_connector(&home, reach, &company, &key, &stop);
                if !stop.load(Ordering::Acquire) {
                    connectors.set_ready(&company, &key, Some(identity));
                    let _ = wake.try_send(());
                    connection(&home, reach, &company, &key, &stop, &wake);
                }
                connectors.set_ready(&company, &key, None);
            });
        match spawned {
            Ok(thread) => Some(thread),
            Err(error) => {
                tracing::error!(target: LOG_TARGET, event = "connector_start_failed", error = %error);
                None
            }
        }
    }

    /// Reconcile hosted connections with the admitted generation.
    pub fn check(&mut self) {
        let admitted = self.current.get();
        let declared = conversations::demand(&self.home).unwrap_or_default();
        self.observed.retain(|(company, key)| {
            declared
                .iter()
                .any(|(owner, entry)| owner == company && entry.key == *key)
        });
        for (company, declared) in declared {
            if self
                .observed
                .insert((company.clone(), declared.key.clone()))
                && let Err(error) = conversations::read_back::observe(
                    &self.home,
                    &company,
                    &declared.key,
                    self.connectors.now(),
                )
            {
                self.observed.remove(&(company, declared.key));
                tracing::warn!(target: LOG_TARGET, event = "connector_read_back_unrecorded", %error);
            }
        }
        let demand = conversations::admitted_demand(&admitted);
        for hosted in &self.hosted {
            if identity(&admitted, &hosted.company, &hosted.key).as_ref() != Some(&hosted.identity)
            {
                self.connectors
                    .set_ready(&hosted.company, &hosted.key, None);
                hosted.stop.store(true, Ordering::Release);
            }
        }
        self.hosted.retain_mut(|hosted| {
            if hosted
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
            {
                return true;
            }
            self.connectors
                .set_ready(&hosted.company, &hosted.key, None);
            if let Some(thread) = hosted.thread.take()
                && thread.join().is_err()
            {
                state(
                    &self.home,
                    &hosted.company,
                    &hosted.key,
                    ConnectionState::Failed,
                    Some("the connector's worker stopped unexpectedly; restarted"),
                );
            }
            false
        });
        for (company, key) in demand {
            if self
                .hosted
                .iter()
                .any(|hosted| hosted.company == company && hosted.key == key)
            {
                continue;
            }
            let Some(identity) = identity(&admitted, &company, &key) else {
                continue;
            };
            let stop = Arc::new(AtomicBool::new(false));
            let thread = self.spawn(&company, &key, identity.clone(), Arc::clone(&stop));
            self.hosted.push(Hosted {
                company,
                key,
                thread,
                identity,
                stop,
            });
        }
        let hosted: Vec<_> = self
            .hosted
            .iter()
            .filter(|hosted| !hosted.stop.load(Ordering::Acquire))
            .map(|hosted| (hosted.company.to_string(), hosted.key.clone()))
            .collect();
        if hosted != self.published {
            match instrukt_runner::runner::publish_hosted(&self.home, &hosted) {
                Ok(()) => self.published = hosted,
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "connector_hosting_unpublished", %error)
                }
            }
        }
    }
}

fn admit_connector(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &AtomicBool,
) {
    let stop = || stop.load(Ordering::Acquire);
    while !stop() {
        let result = reach
            .connectors
            .open(&reach.current.get(), company, key)
            .and_then(|connector| connector.check_permissions());
        let error = match result {
            Ok(()) => return,
            Err(error) => error,
        };
        state(
            home,
            company,
            key,
            ConnectionState::Failed,
            Some(&error.to_string()),
        );
        sleep_until(after_failure(error.wait.as_ref(), Instant::now()), &stop);
    }
}

impl Drop for Connections {
    fn drop(&mut self) {
        for hosted in &self.hosted {
            hosted.stop.store(true, Ordering::Release);
        }
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

fn connector_policy(
    admitted: &Admitted,
    company: &ProjectSlug,
    key: &str,
) -> Result<(RoleName, ConnectorAdmit, SessionScope, ReplyMode), ConnectorError> {
    admitted
        .company(company)
        .and_then(|company| company.connectors.iter().find(|c| c.key == key))
        .map(|declared| {
            (
                declared.role.clone(),
                declared.admit,
                declared.session_scope,
                declared.reply_mode,
            )
        })
        .ok_or_else(|| {
            ConnectorError::new(format!(
                "connector `{key}` is not admitted for company `{company}`"
            ))
        })
}

fn connector_admit(
    admitted: &Admitted,
    company: &ProjectSlug,
    key: &str,
) -> Option<ConnectorAdmit> {
    admitted
        .company(company)
        .and_then(|company| company.connectors.iter().find(|c| c.key == key))
        .map(|declared| declared.admit)
}

/// The host's side of one connection: what it retains is recorded and the
/// intake nudged; the connection's state follows what the connector reports.
struct HostInbox<'a> {
    home: &'a Home,
    current: &'a Current,
    company: &'a ProjectSlug,
    key: &'a str,
    nudge: &'a SyncSender<()>,
    clock: &'a dyn Connectors,
    read_back: &'a SyncSender<()>,
}

impl Inbox for HostInbox<'_> {
    fn retain(&self, incoming: Incoming) -> Result<(), ConnectorError> {
        let (role, _, session_scope, reply_mode) =
            connector_policy(&self.current.get(), self.company, self.key)?;
        conversations::retain_with_policy(
            self.home,
            self.company,
            &incoming,
            &role,
            session_scope,
            reply_mode,
            self.clock.now(),
        )
        .map_err(|error| ConnectorError::new(error.to_string()))?;
        let _ = self.nudge.try_send(());
        Ok(())
    }

    fn connected(&self) {
        let _ = self.read_back.try_send(());
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
    let (read_back, scans) = std::sync::mpsc::sync_channel::<()>(1);
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
        scope.spawn(move || {
            sign_in_delivery_worker(home, reach, company, key, &quit);
        });
        let scan_nudge = &nudge;
        scope.spawn(move || {
            let quit = || stop.load(Ordering::Acquire) || ended.load(Ordering::Acquire);
            if let Err(error) = conversations::read_back::observe(home, company, key, reach.connectors.now()) {
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
                                    &quit,
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
        });
        while !quit() {
            state(home, company, key, ConnectionState::Connecting, None);
            let inbox = HostInbox {
                home,
                current: reach.current,
                company,
                key,
                nudge: &nudge,
                clock: reach.connectors.as_ref(),
                read_back: &read_back,
            };
            let failure = reach
                .connectors
                .open(&reach.current.get(), company, key)
                .and_then(|connector| connector.listen(&inbox, &quit))
                .err();
            if quit() {
                break;
            }
            let until = match &failure {
                None => Instant::now() + RECONNECT,
                Some(error) => {
                    if error.wait.is_none() {
                        tracing::warn!(target: LOG_TARGET, event = "connector_failed", company = %company, connector = key, error = %error);
                    }
                    state(
                        home,
                        company,
                        key,
                        ConnectionState::Failed,
                        Some(&error.to_string()),
                    );
                    after_failure(error.wait.as_ref(), Instant::now())
                }
            };
            sleep_until(until, &quit);
        }
    });
    if stop.load(Ordering::Acquire) {
        state(home, company, key, ConnectionState::Stopped, None);
    }
}

/// Wake the relay on capture and intake when a context read completes.
#[derive(Clone, Copy)]
struct Wakes<'a> {
    relay: &'a SyncSender<()>,
    context: &'a SyncSender<()>,
}

/// The company's retained intake work, until `stop`: run when a retained
/// message nudges it, and otherwise at the moment its last pass gave
/// ([`next_pass`]).
fn intake_worker(
    stop: &dyn Fn() -> bool,
    nudges: &std::sync::mpsc::Receiver<()>,
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
/// failure carried a wait; [`RECHECK`] after a pass that failed locally.
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

/// Dedicated worker delivering private MCP sign-in links and outcome notices
/// by direct message through the conversation's connector. Polls read-only via
/// `StateRead` (never taking the writer lock when idle) and never holds `StateDb`
/// across connector network calls.
fn sign_in_delivery_worker(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &dyn Fn() -> bool,
) {
    let mut sent_links = std::collections::HashSet::<String>::new();
    let mut sent_outcomes = std::collections::HashSet::<String>::new();
    while !stop() {
        let now = reach.connectors.now();
        deliver_pending_sign_in_dms(
            home,
            reach,
            company,
            key,
            now,
            &mut sent_links,
            &mut sent_outcomes,
        );
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until && !stop() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Sanitize an OAuth error code for plain-text direct messages per `mcp-access`:
/// up to 64 printable RFC 6749 NQCHAR bytes (`0x21..=0x7E`, excluding `"` and `\`),
/// or `"unknown"` otherwise.
fn sanitize_oauth_error_code(raw: Option<&str>) -> &str {
    match raw.map(str::trim) {
        Some(code)
            if !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|b| (0x21..=0x7E).contains(&b) && b != b'"' && b != b'\\') =>
        {
            code
        }
        _ => "unknown",
    }
}

struct PendingLink {
    state_digest: String,
    server: String,
    recipient: String,
    link: String,
    expires_at: String,
}

struct PendingOutcome {
    state_digest: String,
    server: String,
    recipient: String,
    state: String,
    failure: Option<String>,
}

fn deliver_pending_sign_in_dms(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    now: Timestamp,
    sent_links: &mut std::collections::HashSet<String>,
    sent_outcomes: &mut std::collections::HashSet<String>,
) {
    use instrukt_connector::DirectMessageError;
    use instrukt_core::state_db::rusqlite::params;
    use instrukt_core::state_db::{StateDb, StateDbError, StateRead};

    let inspection = home.for_inspection();
    let (pending_links, pending_outcomes): (Vec<PendingLink>, Vec<PendingOutcome>) =
        match StateRead::open(&inspection) {
            Ok(Some(read)) => {
                let links = match read.conn().prepare(
                    "SELECT state_digest, server, recipient, link, expires_at
                     FROM mcp_sign_ins
                     WHERE company = ?1 AND connector = ?2 AND state = 'open'
                       AND link_sent_at IS NULL AND link IS NOT NULL",
                ) {
                    Ok(mut stmt) => match stmt.query_map(params![company.as_str(), key], |r| {
                        Ok(PendingLink {
                            state_digest: r.get(0)?,
                            server: r.get(1)?,
                            recipient: r.get(2)?,
                            link: r.get(3)?,
                            expires_at: r.get(4)?,
                        })
                    }) {
                        Ok(rows) => {
                            let mut out = Vec::new();
                            for row in rows {
                                match row {
                                    Ok(item) => out.push(item),
                                    Err(error) => tracing::warn!(
                                        target: LOG_TARGET,
                                        event = "mcp_sign_in_link_row_error",
                                        company = %company,
                                        connector = key,
                                        %error
                                    ),
                                }
                            }
                            out
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: LOG_TARGET,
                                event = "mcp_sign_in_link_query_failed",
                                company = %company,
                                connector = key,
                                %error
                            );
                            Vec::new()
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            target: LOG_TARGET,
                            event = "mcp_sign_in_link_prepare_failed",
                            company = %company,
                            connector = key,
                            %error
                        );
                        Vec::new()
                    }
                };
                let outcomes = match read.conn().prepare(
                    "SELECT state_digest, server, recipient, state, failure
                     FROM mcp_sign_ins
                     WHERE company = ?1 AND connector = ?2
                       AND state IN ('completed', 'refused', 'declined', 'failed')
                       AND told_at IS NULL",
                ) {
                    Ok(mut stmt) => match stmt.query_map(params![company.as_str(), key], |r| {
                        Ok(PendingOutcome {
                            state_digest: r.get(0)?,
                            server: r.get(1)?,
                            recipient: r.get(2)?,
                            state: r.get(3)?,
                            failure: r.get(4)?,
                        })
                    }) {
                        Ok(rows) => {
                            let mut out = Vec::new();
                            for row in rows {
                                match row {
                                    Ok(item) => out.push(item),
                                    Err(error) => tracing::warn!(
                                        target: LOG_TARGET,
                                        event = "mcp_sign_in_outcome_row_error",
                                        company = %company,
                                        connector = key,
                                        %error
                                    ),
                                }
                            }
                            out
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: LOG_TARGET,
                                event = "mcp_sign_in_outcome_query_failed",
                                company = %company,
                                connector = key,
                                %error
                            );
                            Vec::new()
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            target: LOG_TARGET,
                            event = "mcp_sign_in_outcome_prepare_failed",
                            company = %company,
                            connector = key,
                            %error
                        );
                        Vec::new()
                    }
                };
                (links, outcomes)
            }
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    event = "mcp_sign_in_state_read_failed",
                    company = %company,
                    connector = key,
                    %error
                );
                return;
            }
        };

    // Prune local de-duplication sets once DB rows are settled.
    sent_links.retain(|digest| pending_links.iter().any(|p| p.state_digest == *digest));
    sent_outcomes.retain(|digest| pending_outcomes.iter().any(|p| p.state_digest == *digest));

    if pending_links.is_empty() && pending_outcomes.is_empty() {
        return;
    }

    let now_str = now.to_string();
    let connector = match reach.connectors.open(&reach.current.get(), company, key) {
        Ok(connector) => connector,
        Err(error) => {
            tracing::warn!(
                target: LOG_TARGET,
                event = "mcp_sign_in_connector_unavailable",
                company = %company,
                connector = key,
                error = %error
            );
            return;
        }
    };

    for item in pending_links {
        if item
            .expires_at
            .parse::<Timestamp>()
            .is_ok_and(|exp| exp <= now)
        {
            match StateDb::open(home) {
                Ok(mut db) => {
                    let path = db.path().to_path_buf();
                    if let Err(error) = db.write::<_, StateDbError>(|tx| {
                        tx.execute(
                            "UPDATE mcp_sign_ins
                             SET state = 'expired', closed_at = ?1, code_verifier = NULL, nonce = NULL, link = NULL
                             WHERE state_digest = ?2 AND state = 'open'",
                            params![now_str, item.state_digest],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                        Ok(())
                    }) {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_expire_write_failed", company = %company, %error);
                    }
                }
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_expire_open_failed", company = %company, %error);
                }
            }
            continue;
        }

        let already_sent = sent_links.contains(&item.state_digest);
        let send_outcome = if already_sent {
            Ok(())
        } else {
            let message = format!(
                "To let the agent use {} as you, sign in here. The link works once, for ten minutes: {}",
                item.server, item.link
            );
            connector
                .direct_message(&item.recipient, &message)
                .map(|_| ())
        };
        match send_outcome {
            Ok(()) => {
                sent_links.insert(item.state_digest.clone());
                match StateDb::open(home) {
                    Ok(mut db) => {
                        let path = db.path().to_path_buf();
                        if let Err(error) = db.write::<_, StateDbError>(|tx| {
                            tx.execute(
                                "UPDATE mcp_sign_ins
                                 SET link_sent_at = ?1, link = NULL
                                 WHERE state_digest = ?2 AND state = 'open'",
                                params![now_str, item.state_digest],
                            )
                            .map_err(StateDbError::sqlite(&path))?;
                            Ok(())
                        }) {
                            tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_sent_write_failed", company = %company, server = %item.server, %error);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_sent_open_failed", company = %company, server = %item.server, %error);
                    }
                }
            }
            // The outside gate reports the wait; the next poll retries.
            Err(DirectMessageError::Retryable(_)) => {}
            Err(DirectMessageError::Final(cause)) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    event = "mcp_sign_in_dm_undelivered",
                    company = %company,
                    connector = key,
                    server = %item.server,
                    cause = %cause
                );
                match StateDb::open(home) {
                    Ok(mut db) => {
                        let path = db.path().to_path_buf();
                        if let Err(error) = db.write::<_, StateDbError>(|tx| {
                            tx.execute(
                                "UPDATE mcp_sign_ins
                                 SET state = 'undelivered', failure = ?1, closed_at = ?2, code_verifier = NULL, nonce = NULL, link = NULL
                                 WHERE state_digest = ?3 AND state = 'open'",
                                params![cause, now_str, item.state_digest],
                            )
                            .map_err(StateDbError::sqlite(&path))?;
                            Ok(())
                        }) {
                            tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_undelivered_write_failed", company = %company, server = %item.server, %error);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_undelivered_open_failed", company = %company, server = %item.server, %error);
                    }
                }
            }
        }
    }

    for item in pending_outcomes {
        let already_sent = sent_outcomes.contains(&item.state_digest);
        let send_outcome = if already_sent {
            Ok(())
        } else {
            let text = match item.state.as_str() {
                "completed" => format!(
                    "You are signed in to {}. Ask the agent again to continue.",
                    item.server
                ),
                "refused" => format!(
                    "That sign-in to {} was not for your address and was not kept.",
                    item.server
                ),
                "declined" | "failed" => {
                    let code = sanitize_oauth_error_code(item.failure.as_deref());
                    format!("The sign-in to {} did not complete: {code}.", item.server)
                }
                _ => continue,
            };
            connector.direct_message(&item.recipient, &text).map(|_| ())
        };
        match send_outcome {
            Ok(()) => {
                sent_outcomes.insert(item.state_digest.clone());
                match StateDb::open(home) {
                    Ok(mut db) => {
                        let path = db.path().to_path_buf();
                        if let Err(error) = db.write::<_, StateDbError>(|tx| {
                            tx.execute(
                                "UPDATE mcp_sign_ins SET told_at = ?1 WHERE state_digest = ?2 AND told_at IS NULL",
                                params![now_str, item.state_digest],
                            )
                            .map_err(StateDbError::sqlite(&path))?;
                            Ok(())
                        }) {
                            tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_told_write_failed", company = %company, server = %item.server, %error);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_told_open_failed", company = %company, server = %item.server, %error);
                    }
                }
            }
            // The outside gate reports the wait; the next poll retries.
            Err(DirectMessageError::Retryable(_)) => {}
            Err(DirectMessageError::Final(cause)) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    event = "mcp_sign_in_outcome_dm_failed",
                    company = %company,
                    connector = key,
                    server = %item.server,
                    cause = %cause
                );
                if let Ok(mut db) = StateDb::open(home) {
                    let path = db.path().to_path_buf();
                    if let Err(error) = db.write::<_, StateDbError>(|tx| {
                        tx.execute(
                            "UPDATE mcp_sign_ins SET told_at = ?1 WHERE state_digest = ?2 AND told_at IS NULL",
                            params![now_str, item.state_digest],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                        Ok(())
                    }) {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_told_final_write_failed", company = %company, %error);
                    }
                }
            }
        }
    }
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
            let outcome = connector
                .unavailable_notice(&notice.conversation, &notice.sender)
                .map_err(|error| error.to_string());
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
    replies: std::sync::mpsc::Receiver<ContextReply>,
    readable: Option<(ContextSnapshot, Instant)>,
}

#[derive(Default)]
struct ContextReads(BTreeMap<(String, String), ContextFlight>);

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
        use std::sync::mpsc::{self, TryRecvError};
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
                        let result = connector.capture(
                            &item.conversation,
                            &item.message,
                            &item.text,
                            &excluded,
                            &|snapshot| {
                                if send.send(ContextReply::Readable(snapshot)).is_ok() {
                                    let _ = wake.try_send(());
                                }
                            },
                        );
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
) -> Result<IntakeProgress, conversations::ConversationError> {
    let mut progress = IntakeProgress::default();
    for unread in conversations::unread(home, company, key)?
        .into_iter()
        .filter(|item| !item.admitted)
    {
        let Some(sender) = unread.sender.as_deref() else {
            progress.failures.add(None);
            conversations::waiting(home, company, key, &unread, "the message has no sender")?;
            continue;
        };
        match next_record(connector.admit(sender), admit) {
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
    for refusal in owed(conversations::refusals(home, company, key)?) {
        let outcome = connector.turn_away(&refusal.conversation, refusal.reason);
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
    for unread in conversations::unread(home, company, key)?
        .into_iter()
        .filter(|item| item.admitted)
    {
        match contexts.capture(connector, &unread, wakes.context, || {
            conversations::context_exclusions(
                &RunnerState::new(home),
                company,
                key,
                &unread.conversation,
            )
            .map_err(Into::into)
        }) {
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
    // A call the outside gate answered with a wait stays owed, so the next
    // tick makes it once the wait is over.
    for (place, status) in pass.set {
        if !call(connector, company, key, &place, Some(&status)) {
            attempted.insert(place, (now, status));
        }
    }
    for place in pass.clear {
        if !call(connector, company, key, &place, None) {
            attempted.remove(&place);
        }
    }
}

/// Show `status` at `place`, best effort; whether the outside gate answered
/// with a wait, which leaves the call owed.
fn call(
    connector: &dyn Connector,
    company: &ProjectSlug,
    key: &str,
    place: &Place,
    status: Option<&Status>,
) -> bool {
    match connector.status(place, status) {
        Ok(()) => false,
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, event = "connector_status_skipped", company = %company, connector = key, error = %error);
            error.wait.is_some()
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
            text: &str,
            _: &std::collections::HashSet<String>,
            _: &dyn Fn(instrukt_connector::ContextSnapshot),
        ) -> Result<Captured, ConnectorError> {
            Ok(Captured {
                display: None,
                text: text.to_owned(),
                accompanying: Default::default(),
            })
        }
        fn answer(
            &self,
            _: &str,
            _: &str,
        ) -> Result<instrukt_connector::AnswerReceipt, instrukt_connector::AnswerError> {
            Ok(instrukt_connector::AnswerReceipt::default())
        }
        fn turn_away(
            &self,
            _: &str,
            _: instrukt_connector::RefusalReason,
        ) -> Result<(), ConnectorError> {
            Ok(())
        }
        fn unavailable_notice(&self, _: &str, _: &str) -> Result<(), ConnectorError> {
            Ok(())
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

    /// A wait the outside gate gives `after` past `base` on its monotonic
    /// source.
    fn wait_after(base: Instant, after: Duration) -> Wait {
        let gate = instrukt_outside::Gate::new(
            Arc::new(|| "2026-11-20T10:00:00Z".parse().unwrap()),
            Arc::new(move || base),
            instrukt_outside::Report::to(|_| {}),
        );
        let settings = instrukt_outside::Settings {
            backoff: instrukt_outside::Backoff::new(after, after).unwrap(),
            ..Default::default()
        };
        let key = instrukt_outside::Key::new("test", ["acme"]).item("method");
        let unavailable = instrukt_outside::Unavailable {
            reason: instrukt_outside::Reason::Unreachable,
            extent: instrukt_outside::Extent::Key,
            named: None,
        };
        match gate.attempt(&key, &settings, || {
            instrukt_outside::Tried::<(), ()>::Unavailable(unavailable, ())
        }) {
            instrukt_outside::Attempted::Wait { wait, .. } => wait,
            _ => panic!("a wait"),
        }
    }

    #[test]
    fn a_whole_loop_retries_at_the_wait_or_after_the_recheck() {
        let now = Instant::now();
        let wait = wait_after(now, Duration::from_secs(7));
        assert_eq!(after_failure(Some(&wait), now), wait.until());
        assert_eq!(after_failure(None, now), now + RECHECK);
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

    #[test]
    fn a_declared_entry_reaches_its_kind_as_text_and_counts() {
        let entry: Value =
            serde_yaml_ng::from_str("{app_token: '${A}', context_messages: 5}").unwrap();
        let declared = declared_values(&entry);
        assert_eq!(declared.text("app_token"), Some("${A}"));
        assert_eq!(declared.count("context_messages"), Some(5));
    }
}
