//! Converge only machine infrastructure demanded by adopted projects. Inspection
//! shares the demand and ownership model and executes no infrastructure mutation steps.
mod _demand;
pub(crate) mod _launch;
mod _package;
mod _redis;
mod _services;
use crate::install::{Scheduler, WakeUp};
use _demand::Demand;
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
}
impl ResourceKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::WakeUp => "wake-up",
            Self::Redis => "redis",
            Self::Receiver => "receiver",
            Self::LocalRedis => "local Redis",
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
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub resources: Vec<Resource>,
    pub findings: Vec<String>,
    pub notices: Vec<String>,
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
    if request.check && request.module != Some(Module::Events) {
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
    let mut owned = _services::Owned::load(request.home)
        .map_err(|e| InstallationError::Ownership(e.to_string()))?;
    if !request.check {
        owned
            .persist_migration(request.home)
            .map_err(|e| InstallationError::Ownership(e.to_string()))?;
    }
    let global = Configuration::load_global(request.home)?;
    if let Some(notice) =
        instrukt_core::config::inspect_layer(request.home, &instrukt_core::config::Layer::Global)?
            .migration
    {
        report.notices.push(notice);
    }
    if request.module != Some(Module::Events) || !demand.events.is_empty() {
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
        if !demand.events.is_empty() {
            if let Err(error) = _redis::ensure(request, &mut owned, &demand, &mut report) {
                report.finding("Redis", error);
            }
        } else if demand.redis_references.is_empty()
            && let Err(error) =
                owned.withdraw_named(request, ResourceKind::Redis, demand.complete, &mut report)
        {
            report.finding("Redis", error);
        }
        if let Err(error) = receiver(request, &mut owned, &demand, &mut report) {
            report.finding("receiver", error);
        }
    }
    Ok(report)
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
        let ingress = config.ingress()?;
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
                "/hooks/inbound/project/{}/{domain}",
                record.project
            ));
            source
                .resolve(request.home)
                .map_err(|e| anyhow::anyhow!("project {} source {domain}: {e}", record.project))?;
        }
        projects.push(record.project.to_string());
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
        serde_json::json!({"enabled_projects":demand.events,"transport":redis,"infrastructure":infrastructure,"findings":report.findings,"activation_failures":failures,"links":crate::links::status(request.home).map_err(|e|inspection_error("conversation links",e))?}),
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
