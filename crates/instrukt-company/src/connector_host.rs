//! The connector host of `instrukt run` (`slack-connector` and
//! `conversations` specs): hosts and admits declared company connectors,
//! manages their tasks and connection state, runs each connector's
//! listen/reconnect loop, and delegates per-connection supervision workers to
//! `_workers`. Each connector is opened from the admitted generation current
//! at each pass, its secrets resolved from that generation: no connector reads
//! a company's files.
mod _workers;

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::Sender as AsyncSender;
use tokio::sync::{Notify, oneshot, watch};
use tokio::task::{JoinHandle, block_in_place};

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

/// A host shutdown closes the watch or sets its value once.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|requested| *requested).await;
}

async fn sleep_until(until: Instant) {
    tokio::time::sleep_until(until.into()).await;
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

/// Close the exit channel before waking supervision, including on unwind.
struct ExitNotice {
    connection: Arc<Connection>,
    exited: Option<oneshot::Sender<()>>,
    wake: AsyncSender<()>,
}

impl Drop for ExitNotice {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let connection = &self.connection;
            connection
                .connectors
                .set_ready(&connection.company, &connection.key, None);
            connection.state(
                ConnectionState::Failed,
                Some("the connector's task stopped unexpectedly"),
            );
        }
        drop(self.exited.take());
        let _ = self.wake.try_send(());
    }
}

struct Hosted {
    company: ProjectSlug,
    key: String,
    task: JoinHandle<()>,
    exited: oneshot::Receiver<()>,
    identity: ConnectorIdentity,
    stop: watch::Sender<bool>,
}

struct Connection {
    home: Home,
    current: Current,
    connectors: Arc<dyn Connectors>,
    accepted: Arc<conversations::Accepted>,
    wake: AsyncSender<()>,
    company: ProjectSlug,
    key: String,
}

impl Connection {
    fn open(&self) -> Result<Box<dyn Connector>, ConnectorError> {
        self.connectors
            .open(&self.current.get(), &self.company, &self.key)
    }

    fn state(&self, status: ConnectionState, failure: Option<&str>) {
        block_in_place(|| state(&self.home, &self.company, &self.key, status, failure));
    }
}

/// The connections `instrukt run` holds open, supervised by the run loop.
pub struct Connections {
    runtime: &'static Runtime,
    home: Home,
    current: Current,
    connectors: Arc<dyn Connectors>,
    accepted: Arc<conversations::Accepted>,
    wake: AsyncSender<()>,
    supervise: AsyncSender<()>,
    hosted: Vec<Hosted>,
    observed: std::collections::HashSet<ConnectorKey>,
    published: Vec<(String, String)>,
}

impl Connections {
    /// Open one connection per declared connector. `wake` tells the relay
    /// that retained work arrived; `supervise` tells the run loop that a
    /// connection task ended.
    pub fn start(
        home: &Home,
        current: &Current,
        connectors: Arc<dyn Connectors>,
        accepted: Arc<conversations::Accepted>,
        wake: AsyncSender<()>,
        supervise: AsyncSender<()>,
    ) -> Result<Self, conversations::ConversationError> {
        let runtime = instrukt_outside::runtime::runtime()
            .map_err(|error| ConnectorError::new(error.to_string()))?;
        let mut connections = Self {
            runtime,
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

    fn spawn(&self, company: ProjectSlug, key: String, identity: ConnectorIdentity) -> Hosted {
        let connection = Arc::new(Connection {
            home: self.home.clone(),
            current: self.current.clone(),
            connectors: Arc::clone(&self.connectors),
            accepted: Arc::clone(&self.accepted),
            wake: self.wake.clone(),
            company: company.clone(),
            key: key.clone(),
        });
        let (stop, mut shutdown) = watch::channel(false);
        let (exit, exited) = oneshot::channel();
        let notice = ExitNotice {
            connection: Arc::clone(&connection),
            exited: Some(exit),
            wake: self.supervise.clone(),
        };
        let admitted_identity = identity.clone();
        let task = self.runtime.spawn(async move {
            let _notice = notice;
            let admitted = tokio::select! {
                biased;
                () = stopped(&mut shutdown) => false,
                () = admit_connector(&connection) => true,
            };
            if admitted {
                // Serialize readiness publication with a concurrent stop.
                let stopping = shutdown.borrow();
                if !*stopping {
                    connection.connectors.set_ready(
                        &connection.company,
                        &connection.key,
                        Some(admitted_identity),
                    );
                    let _ = connection.wake.try_send(());
                }
            }
            if admitted {
                let result = _workers::supervise(Arc::clone(&connection), shutdown).await;
                connection
                    .connectors
                    .set_ready(&connection.company, &connection.key, None);
                match result {
                    Ok(()) => connection.state(ConnectionState::Stopped, None),
                    Err(error) => connection.state(ConnectionState::Failed, Some(&error)),
                }
            } else {
                connection
                    .connectors
                    .set_ready(&connection.company, &connection.key, None);
            }
        });
        Hosted {
            company,
            key,
            task,
            exited,
            identity,
            stop,
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
                hosted.stop.send_replace(true);
                self.connectors
                    .set_ready(&hosted.company, &hosted.key, None);
            }
        }
        self.hosted.retain_mut(|hosted| {
            if matches!(
                hosted.exited.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ) {
                return true;
            }
            self.connectors
                .set_ready(&hosted.company, &hosted.key, None);
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
            self.hosted.push(self.spawn(company, key, identity));
        }
        let hosted: Vec<_> = self
            .hosted
            .iter()
            .filter(|hosted| !*hosted.stop.borrow())
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

async fn admit_connector(connection: &Connection) {
    loop {
        let result = match connection.open() {
            Ok(connector) => connector.check_permissions().await,
            Err(error) => Err(error),
        };
        let error = match result {
            Ok(()) => return,
            Err(error) => error,
        };
        connection.state(ConnectionState::Failed, Some(&error.to_string()));
        sleep_until(after_failure(error.wait.as_ref(), Instant::now())).await;
    }
}

impl Drop for Connections {
    fn drop(&mut self) {
        for hosted in &self.hosted {
            hosted.stop.send_replace(true);
            self.connectors
                .set_ready(&hosted.company, &hosted.key, None);
        }
        let hosted = std::mem::take(&mut self.hosted);
        if hosted.is_empty() {
            return;
        }
        let finish = async move {
            for hosted in hosted {
                if let Err(error) = hosted.task.await {
                    tracing::error!(target: LOG_TARGET, event = "connector_task_failed", %error);
                }
            }
        };
        // Drain hosted tasks before returning whenever the caller can block:
        // synchronously, or via `block_in_place` on a multi-thread Tokio worker.
        match Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| self.runtime.block_on(finish));
            }
            Ok(_) => {
                self.runtime.spawn(finish);
            }
            Err(_) => {
                self.runtime.block_on(finish);
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
struct HostInbox {
    connection: Arc<Connection>,
    intake: Arc<Notify>,
    read_back: Arc<Notify>,
}

impl Inbox for HostInbox {
    fn retain(&self, incoming: Incoming) -> Result<(), ConnectorError> {
        let Connection {
            home,
            current,
            company,
            key,
            connectors,
            ..
        } = self.connection.as_ref();
        let (role, _, session_scope, reply_mode) = connector_policy(&current.get(), company, key)?;
        block_in_place(|| {
            conversations::retain_with_policy(
                home,
                company,
                &incoming,
                &role,
                session_scope,
                reply_mode,
                connectors.now(),
            )
        })
        .map_err(|error| ConnectorError::new(error.to_string()))?;
        self.intake.notify_one();
        Ok(())
    }

    fn connected(&self) {
        self.read_back.notify_one();
        self.connection.state(ConnectionState::Connected, None);
    }

    fn unretained(&self, cause: &str) {
        self.connection
            .state(ConnectionState::Connected, Some(cause));
    }
}

/// Keep one company's connection open until `stop`: a refused or dropped
/// connection is recorded and retried, and retained intake work continues
/// while it is down.
async fn listen(
    connection: &Arc<Connection>,
    mut shutdown: watch::Receiver<bool>,
    intake: Arc<Notify>,
    read_back: Arc<Notify>,
) {
    while !*shutdown.borrow() {
        connection.state(ConnectionState::Connecting, None);
        let inbox: Arc<dyn Inbox> = Arc::new(HostInbox {
            connection: Arc::clone(connection),
            intake: Arc::clone(&intake),
            read_back: Arc::clone(&read_back),
        });
        let result = match connection.open() {
            Ok(connector) => connector.listen(inbox, shutdown.clone()).await,
            Err(error) => Err(error),
        };
        if *shutdown.borrow() {
            break;
        }
        let until = match result {
            Ok(()) => Instant::now() + RECONNECT,
            Err(error) => {
                if error.wait.is_none() {
                    tracing::warn!(target: LOG_TARGET, event = "connector_failed", company = %connection.company, connector = %connection.key, error = %error);
                }
                connection.state(ConnectionState::Failed, Some(&error.to_string()));
                after_failure(error.wait.as_ref(), Instant::now())
            }
        };
        tokio::select! {
            () = stopped(&mut shutdown) => break,
            () = sleep_until(until) => {}
        }
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
