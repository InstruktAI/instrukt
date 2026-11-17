//! Converge only machine infrastructure demanded by adopted projects. Inspection
//! shares the demand and ownership model and executes no infrastructure mutation steps.
mod _demand;
pub(crate) mod _launch;
mod _package;
mod _redis;
mod _services;
use crate::install::{Scheduler, WakeUp};
use _demand::Demand;
use instrukt_blueprint::{Graphify, GraphifyState, graphify_version};
use instrukt_core::{
    config::{Configuration, Env, Module},
    fsx::LockGuard,
    home::Home,
};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum InstallationError {
    #[error("cannot read adopted-project demand: {0}")]
    Demand(String),
    #[error("cannot read infrastructure ownership: {0}")]
    Ownership(String),
    #[error("{resource} inspection failed: {detail}")]
    Inspection {
        resource: &'static str,
        detail: String,
    },
    #[error(transparent)]
    Configuration(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Filesystem(#[from] instrukt_core::fsx::FsError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
}
fn inspection_error(resource: &'static str, error: impl std::fmt::Display) -> InstallationError {
    InstallationError::Inspection {
        resource,
        detail: error.to_string(),
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum ResourceKind {
    #[serde(rename = "wake-up")]
    WakeUp,
    #[serde(rename = "redis")]
    Redis,
    #[serde(rename = "receiver")]
    Receiver,
    #[serde(rename = "local Redis")]
    LocalRedis,
    #[serde(rename = "graphify")]
    Graphify,
    #[serde(rename = "runner service")]
    Runner,
}
impl ResourceKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::WakeUp => "wake-up",
            Self::Redis => "redis",
            Self::Receiver => "receiver",
            Self::LocalRedis => "local Redis",
            Self::Graphify => "graphify",
            Self::Runner => "runner service",
        }
    }
}
impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Installed,
    Reused,
    Repaired,
    Withdrawn,
    ExternallySupplied,
    Unmet,
}
impl std::fmt::Display for Disposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Installed => "installed",
            Self::Reused => "reused",
            Self::Repaired => "repaired",
            Self::Withdrawn => "withdrawn",
            Self::ExternallySupplied => "externally_supplied",
            Self::Unmet => "unmet",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TickMode {
    NoDemand,
    Operator,
    Launchd,
    Systemd,
    Cron,
    MissingOrBroken,
}
impl std::fmt::Display for TickMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoDemand => "no_demand",
            Self::Operator => "operator",
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::Cron => "cron",
            Self::MissingOrBroken => "missing_or_broken",
        })
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    pub name: ResourceKind,
    pub disposition: Disposition,
    pub paths: Vec<PathBuf>,
    pub projects: Vec<String>,
    pub detail: String,
}
/// The runner service's state (`module-installation`, "Runner service").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// A live owned service process runs the desired installed executable
    /// and configuration; it may be waiting for the runner lock.
    Running,
    /// There is demand, and its service is missing, stale, stopped, in
    /// conflict with a definition instrukt does not own, or cannot be
    /// inspected.
    Unmet,
    /// An owned service remains and there is no demand.
    Obsolete,
    /// There is no demand and no owned service.
    NoDemand,
}

/// The runner service as an installation or inspection found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub state: ServiceState,
    /// Why it is unmet or obsolete.
    pub cause: Option<String>,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub resources: Vec<Resource>,
    pub findings: Vec<String>,
    pub notices: Vec<String>,
    /// The runner service, when this convergence considered it.
    #[serde(skip)]
    pub runner_service: Option<ServiceStatus>,
}
impl Report {
    pub fn healthy(&self) -> bool {
        self.findings.is_empty()
    }
    fn finding(&mut self, name: impl std::fmt::Display, error: impl std::fmt::Display) {
        tracing::warn!(event="installation_unmet",resource=%name,error=%error);
        self.findings.push(format!("{name}: {error}"));
    }
    fn resource(
        &mut self,
        name: ResourceKind,
        disposition: Disposition,
        paths: Vec<PathBuf>,
        projects: &[String],
        detail: impl Into<String>,
    ) {
        tracing::info!(
            event = "installation_resource",
            resource = %name,
            disposition = %disposition
        );
        self.resources.push(Resource {
            name,
            disposition,
            paths,
            projects: projects.to_vec(),
            detail: detail.into(),
        });
    }
}
#[derive(Clone)]
pub struct Request<'a> {
    pub home: &'a Home,
    pub cwd: &'a std::path::Path,
    pub env: &'a Env,
    pub scheduler: Scheduler,
    pub wake: WakeUp<'a>,
    pub check: bool,
    pub module: Option<Module>,
    /// Limit convergence to conversations' dependencies: the runner service.
    pub conversations: bool,
    pub github: &'a dyn crate::interfaces::GitHubReadiness,
    pub now: jiff::Timestamp,
}
/// A failed resource leaves completed resources recorded, so a rerun repairs
/// observed drift rather than trusting the previous overall outcome.
pub fn converge(request: &Request<'_>) -> Result<Report, InstallationError> {
    let _lock = if request.check {
        None
    } else {
        Some(LockGuard::acquire(
            &request.home.locks_dir().join("installation.lock"),
            std::time::Duration::from_secs(30),
        )?)
    };
    let mut report = Report::default();
    let demand = _demand::read(request.home, request.cwd, &mut report)
        .map_err(|e| InstallationError::Demand(e.to_string()))?;
    if request.check
        && !request.conversations
        && (request.module.is_none() || request.module == Some(Module::Todos))
    {
        for work in &demand.work {
            if let Err(error) =
                request
                    .github
                    .reflection_ready(&work.repository, work.board.as_ref(), request.now)
            {
                let detail = match &error {
                    instrukt_core::github::AppError::RateLimited { until: Some(until) } => {
                        format!("{error}; retry at {until}")
                    }
                    _ => error.to_string(),
                };
                report.finding(
                    format!("project {} GitHub reflection", work.project),
                    detail,
                );
            }
        }
    }
    let global = Configuration::load_global(request.home)?;
    if let Some(notice) =
        instrukt_core::config::inspect_layer(request.home, &instrukt_core::config::Layer::Global)?
            .migration
    {
        report.notices.push(notice);
    }
    if (request.module.is_none() && !request.conversations)
        || request.module == Some(Module::Blueprint)
    {
        graphify(request, &demand.blueprint, &mut report);
    }
    if request.module == Some(Module::Blueprint) {
        return Ok(report);
    }
    let mut owned = _services::Owned::load(request.home)
        .map_err(|e| InstallationError::Ownership(e.to_string()))?;
    if !request.check {
        owned
            .persist_migration(request.home)
            .map_err(|e| InstallationError::Ownership(e.to_string()))?;
    }
    if request.module.is_none() {
        runner_service(request, &mut owned, &demand, &mut report);
    }
    if request.conversations {
        return Ok(report);
    }
    if request.module != Some(Module::History)
        && (request.module != Some(Module::Events) || !demand.events.is_empty())
    {
        let required = !demand.projects().is_empty() && global.ticks_enabled()?;
        let projects = demand.projects();
        let tick = _services::tick(request);
        if let Err(error) = owned.reconcile(
            request,
            &tick,
            required,
            demand.complete,
            &projects,
            &mut report,
        ) {
            report.finding("wake-up", error);
        }
        if !projects.is_empty() && !global.ticks_enabled()? {
            report.resource(
                ResourceKind::WakeUp,
                Disposition::ExternallySupplied,
                vec![],
                &projects,
                "ticks off: operator supplies scheduling",
            );
        }
    }
    if request.module != Some(Module::Todos) {
        if (request.module != Some(Module::History) && !demand.events.is_empty())
            || (request.module != Some(Module::Events) && !demand.history_owned_redis.is_empty())
        {
            if let Err(error) = _redis::ensure(request, &mut owned, &demand, &mut report) {
                report.finding("Redis", error);
            }
        } else if demand.redis_references.is_empty()
            && demand.events.is_empty()
            && let Err(error) =
                owned.withdraw_named(request, ResourceKind::Redis, demand.complete, &mut report)
        {
            report.finding("Redis", error);
        }
        if request.module != Some(Module::Events)
            && !demand.history.is_empty()
            && let Err(error) = _redis::history(request, &demand.history, &mut report)
        {
            report.finding("history Redis", error);
        }
        if request.module != Some(Module::History)
            && let Err(error) = receiver(request, &mut owned, &demand, &mut report)
        {
            report.finding("receiver", error);
        }
    }
    Ok(report)
}
/// The owned service that keeps `instrukt run` running while a company
/// home declares a connector and conversations are on; withdrawn when that
/// demand ends. Its state is recorded in the report as it was found.
fn runner_service(
    request: &Request<'_>,
    owned: &mut _services::Owned,
    demand: &Demand,
    report: &mut Report,
) {
    let desired = _services::runner(request);
    let paths = desired.files.keys().cloned().collect::<Vec<_>>();
    let status = |state, cause: Option<String>| ServiceStatus {
        state,
        cause,
        paths: paths.clone(),
    };
    let found = if !demand.conversation_blockers.is_empty() {
        let cause = demand.conversation_blockers.join("; ");
        report.finding(ResourceKind::Runner, &cause);
        status(ServiceState::Unmet, Some(cause))
    } else if demand.conversations.is_empty() {
        if !owned.registered(ResourceKind::Runner) {
            status(ServiceState::NoDemand, None)
        } else {
            let check = request.check;
            match owned.withdraw_named(request, ResourceKind::Runner, demand.complete, report) {
                Ok(()) if check => status(
                    ServiceState::Obsolete,
                    Some("no company declares a connector while conversations is on".into()),
                ),
                Ok(()) => status(ServiceState::NoDemand, None),
                Err(error) => {
                    let cause = error.to_string();
                    report.finding(ResourceKind::Runner, &cause);
                    status(ServiceState::Obsolete, Some(cause))
                }
            }
        }
    } else if request.scheduler == Scheduler::Crontab {
        report.resource(
            ResourceKind::Runner,
            Disposition::Unmet,
            vec![],
            &demand.conversations,
            "no service manager keeps a process running",
        );
        let cause = "conversations need a service manager that keeps `instrukt run` running: launchd on macOS or a systemd user manager on Linux; cron cannot keep a process running";
        report.finding(ResourceKind::Runner, cause);
        status(ServiceState::Unmet, Some(cause.into()))
    } else {
        if request.scheduler == Scheduler::SystemdUser
            && let Err(cause) = lingering()
        {
            report.finding(ResourceKind::Runner, cause);
        }
        match owned.runner_service(request, &desired, &demand.conversations, report) {
            Ok(None) => status(ServiceState::Running, None),
            Ok(Some(cause)) => status(ServiceState::Unmet, Some(cause)),
            Err(error) => {
                let cause = error.to_string();
                report.finding(ResourceKind::Runner, &cause);
                status(ServiceState::Unmet, Some(cause))
            }
        }
    };
    report.runner_service = Some(found);
}

/// Whether the user's systemd manager starts at boot without a login, read
/// from the user's linger state; instrukt never changes it.
fn lingering() -> Result<(), String> {
    let uid = rustix::process::getuid().as_raw().to_string();
    let door = "the user's systemd manager does not start at boot without a login, so the runner service waits for one; enable it with `loginctl enable-linger`";
    match crate::process::Invocation::program(
        "loginctl",
        &["show-user", &uid, "--property=Linger", "--value"],
    )
    .run(std::time::Duration::from_secs(10))
    {
        Ok(result) if result.success && result.stdout.trim() == "yes" => Ok(()),
        Ok(result) if result.success => Err(door.into()),
        Ok(_) | Err(_) => Err(format!(
            "cannot read the user's linger state with loginctl; {door}"
        )),
    }
}
fn graphify(request: &Request<'_>, projects: &[String], report: &mut Report) {
    if projects.is_empty() {
        return;
    }
    let tool = match Graphify::new(request.home) {
        Ok(tool) => tool,
        Err(error) => {
            report.finding("Graphify", error);
            return;
        }
    };
    let state = tool.inspect();
    let disposition = match state {
        GraphifyState::Ready => Disposition::Reused,
        other if request.check => {
            report.finding(
                "Graphify",
                format!(
                    "expected graphify {}: {other:?}; run `instrukt install`",
                    graphify_version()
                ),
            );
            Disposition::Unmet
        }
        other => match tool.install() {
            Ok(()) if other == GraphifyState::Missing => Disposition::Installed,
            Ok(()) => Disposition::Repaired,
            Err(error) => {
                report.finding("Graphify", error);
                Disposition::Unmet
            }
        },
    };
    report.resource(
        ResourceKind::Graphify,
        disposition,
        vec![tool.binary()],
        projects,
        format!("release pin {}", graphify_version()),
    );
}

fn receiver(
    request: &Request<'_>,
    owned: &mut _services::Owned,
    demand: &Demand,
    report: &mut Report,
) -> anyhow::Result<()> {
    let global = Configuration::load_global(request.home)?;
    let mut projects = Vec::new();
    let mut ingress_port = None;
    let mut health_route = None;
    for record in &demand.sources {
        let config = &record.config;
        // Ingress is a machine setting: a company document carries none.
        let ingress = global.ingress()?;
        // Check placement without minting an identity on a read-only path.
        let machine=std::fs::read_to_string(request.home.machine_id_file()).map_err(|_|anyhow::anyhow!("machine identity missing/unreadable; restore the recorded machine-id before receiver placement or withdrawal"))?;
        anyhow::ensure!(
            instrukt_core::machine::MachineId::parse(machine.trim()).is_some(),
            "invalid machine identity; restore the recorded machine-id before receiver placement"
        );
        if machine.trim() != ingress.computer.as_str() {
            continue;
        }
        for (domain, source) in config.webhooks()? {
            health_route = Some(format!(
                "/hooks/inbound/{}/{}/{domain}",
                record.kind, record.slug
            ));
            source
                .resolve(request.home, &record.secrets)
                .map_err(|e| anyhow::anyhow!("{} source {domain}: {e}", record.label()))?;
        }
        projects.push(match record.kind {
            "project" => record.slug.to_string(),
            kind => format!("{kind}:{}", record.slug),
        });
        ingress_port = Some(ingress.port);
    }
    if let Some(port) = ingress_port {
        let ingress = global.ingress()?;
        anyhow::ensure!(
            ingress.port == port,
            "projects disagree with machine ingress; align ingress.computer and port before install"
        );
        anyhow::ensure!(
            request.scheduler != Scheduler::Crontab,
            "receiver requires launchd or systemd user socket custody; cron supplies ticks only"
        );
        // Readers and publication must be verified before opening intake.
        anyhow::ensure!(
            !report.findings.iter().any(|f| f.starts_with("Redis")),
            "event transport/reader prerequisites are unmet; resolve Redis findings and rerun instrukt install"
        );
        let route = health_route
            .clone()
            .ok_or_else(|| anyhow::anyhow!("receiver has no valid health route"))?;
        let receiver = owned.receiver_definition(request, port, route)?;
        owned.reconcile(request, &receiver, true, demand.complete, &projects, report)?;
        if !request.check
            && let Some(route) = health_route
        {
            receiver_health(port, &route)?;
        }
    } else {
        owned.withdraw_named(request, ResourceKind::Receiver, demand.complete, report)?;
    }
    Ok(())
}
#[derive(Serialize)]
pub struct TickStatus {
    pub mode: TickMode,
    pub registration: Report,
    pub last_wakeup: Option<jiff::Timestamp>,
    pub stale: bool,
}
pub fn ticks_status(
    request: &Request<'_>,
    now: jiff::Timestamp,
) -> Result<TickStatus, InstallationError> {
    let mut registration = Report::default();
    let demand = _demand::read(request.home, request.cwd, &mut registration)
        .map_err(|e| InstallationError::Demand(e.to_string()))?;
    let enabled = !demand.projects().is_empty();
    let policy = Configuration::load_global(request.home)?.ticks_enabled()?;
    let mut owned = _services::Owned::load(request.home)
        .map_err(|e| InstallationError::Ownership(e.to_string()))?;
    let tick = _services::tick(request);
    let mut inspection = Request {
        check: true,
        ..request.clone()
    };
    inspection.module = None;
    if enabled && policy {
        owned
            .reconcile(
                &inspection,
                &tick,
                true,
                demand.complete,
                &demand.projects(),
                &mut registration,
            )
            .map_err(|e| inspection_error("wake-up", e))?;
    }
    let last = crate::ticks::last_wakeup(&crate::state::RunnerState::new(request.home))?;
    let stale = enabled && crate::ticks::status(last, now).stale;
    Ok(TickStatus {
        mode: if !enabled {
            TickMode::NoDemand
        } else if !policy {
            TickMode::Operator
        } else if registration.healthy() {
            match request.scheduler {
                Scheduler::Launchd => TickMode::Launchd,
                Scheduler::SystemdUser => TickMode::Systemd,
                Scheduler::Crontab => TickMode::Cron,
            }
        } else {
            TickMode::MissingOrBroken
        },
        registration,
        last_wakeup: last,
        stale,
    })
}
/// Read retained transport/activation facts without enabling events or creating groups.
pub fn events_status(request: &Request<'_>) -> Result<serde_json::Value, InstallationError> {
    let mut report = Report::default();
    let demand = _demand::read(request.home, request.cwd, &mut report)
        .map_err(|e| InstallationError::Demand(e.to_string()))?;
    let infrastructure = converge(&Request {
        check: true,
        module: Some(Module::Events),
        ..request.clone()
    })?;
    let runner_service = converge(&Request {
        check: true,
        module: None,
        conversations: true,
        ..request.clone()
    })?
    .runner_service;
    let configured = Configuration::load_global(request.home)?
        .get(None, request.env)?
        .get("redis")
        .is_some();
    let redis = if demand.events.is_empty() && !configured {
        vec![]
    } else {
        _redis::inspect(request, &mut report).map_err(|e| inspection_error("Redis", e))?
    };
    let state = crate::state::RunnerState::new(request.home);
    let failures=state.read(|conn,path| {
        let mut stmt=conn.prepare("SELECT event_id, state, failure FROM procedure_activations WHERE failure IS NOT NULL").map_err(crate::state::sql(path))?;
        let rows=stmt.query_map([],|row| Ok(serde_json::json!({"id":row.get::<_,String>(0)?,"state":row.get::<_,String>(1)?,"failure":row.get::<_,String>(2)?}))).map_err(crate::state::sql(path))?;
        rows.collect::<Result<Vec<_>,_>>().map_err(crate::state::sql(path))
    })?.unwrap_or_default();
    Ok(
        serde_json::json!({"enabled_projects":demand.events,"transport":redis,"infrastructure":infrastructure,"findings":report.findings,"activation_failures":failures,"links":crate::links::status(request.home).map_err(|e|inspection_error("conversation links",e))?,"conversations":crate::conversations::status(request.home).map_err(|e|inspection_error("conversations",e))?,"runner_service":runner_service}),
    )
}

/// Inspect retained failure identities and coordinates without exposing payloads.
pub fn dead_letters(request: &Request<'_>) -> Result<Vec<serde_json::Value>, InstallationError> {
    _redis::dead_letters(request).map_err(|e| inspection_error("dead letters", e))
}

fn receiver_health(port: u16, route: &str) -> anyhow::Result<()> {
    use std::io::{BufRead, Read, Write};
    let timeout = std::time::Duration::from_secs(3);
    let mut connection=std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127,0,0,1],port)),timeout).map_err(|_|anyhow::anyhow!("receiver loopback socket is unavailable; inspect the owned service and rerun instrukt install"))?;
    connection.set_read_timeout(Some(timeout))?;
    connection.set_write_timeout(Some(timeout))?;
    // Rejection exercises the real receiver without publishing a business event.
    connection.write_all(format!("POST {route} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())?;
    let mut response = std::io::BufReader::new(connection).take(128);
    let mut status = String::new();
    response.read_line(&mut status).map_err(|_|anyhow::anyhow!("receiver health timed out or failed; inspect the owned receiver and rerun instrukt install"))?;
    anyhow::ensure!(
        status.split_whitespace().nth(1) == Some("401"),
        "receiver health did not reject an unsigned request; inspect placement, source secrets and the owned service before rerunning instrukt install"
    );
    Ok(())
}
