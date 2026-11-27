//! The connector host of `instrukt run` (`slack-connector` and
//! `conversations` specs): hosts and admits declared company connectors,
//! manages their OS threads and connection state, runs each connector's
//! listen/reconnect loop, and delegates per-connection supervision workers to
//! `_workers`. Each connector is opened from the admitted generation current
//! at each pass, its secrets resolved from that generation: no connector reads
//! a company's files.
mod _workers;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use instrukt_connector::{
    Clock, Connector, ConnectorError, Declared, Inbox, Incoming, Kind, SecretResolver,
};
use instrukt_core::config::{
    Admitted, AdmittedSecrets, ConnectorAdmit, Current, ReplyMode, SessionScope,
};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName};
use instrukt_outside::Wait;
use jiff::Timestamp;
use serde_yaml_ng::Value;

use crate::conversations::{self, ConnectionState};

/// The target every log statement of the host carries: the module the host
/// and Slack shared before the cut, so the same settings select it.
const LOG_TARGET: &str = "instrukt_runner::connectors";

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
    workers: _workers::WorkerNudges<'a>,
    clock: &'a dyn Connectors,
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
        self.workers.nudge_intake();
        Ok(())
    }

    fn connected(&self) {
        self.workers.nudge_read_back();
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
    _workers::supervise(home, reach, company, key, stop, wake, |workers, quit| {
        while !quit() {
            state(home, company, key, ConnectionState::Connecting, None);
            let inbox = HostInbox {
                home,
                current: reach.current,
                company,
                key,
                workers,
                clock: reach.connectors.as_ref(),
            };
            let failure = reach
                .connectors
                .open(&reach.current.get(), company, key)
                .and_then(|connector| connector.listen(&inbox, quit))
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
            sleep_until(until, quit);
        }
    });
    if stop.load(Ordering::Acquire) {
        state(home, company, key, ConnectionState::Stopped, None);
    }
}

/// A wait the outside gate gives `after` past `base` on its monotonic source.
#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_loop_retries_at_the_wait_or_after_the_recheck() {
        let now = Instant::now();
        let wait = wait_after(now, Duration::from_secs(7));
        assert_eq!(after_failure(Some(&wait), now), wait.until());
        assert_eq!(after_failure(None, now), now + RECHECK);
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
