//! Converge only machine infrastructure demanded by adopted projects. Inspection
//! shares the demand model and executes no infrastructure mutation steps.
//! instrukt's services are found by the names this home gives them; nothing
//! records what installation wrote.
mod _demand;
pub(crate) mod _launch;
mod _package;
mod _proposals;
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
    #[serde(rename = "runtime project")]
    RuntimeProject,
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
            Self::RuntimeProject => "runtime project",
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
    /// The service is loaded and its process is alive; it may be waiting
    /// for the runner lock.
    Running,
    /// There is demand, and its service is missing, stale, stopped,
    /// unloaded or cannot be inspected.
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
    /// Why it is unmet or obsolete, or a prerequisite it still lacks while
    /// it runs, such as starting at boot on Linux.
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
    /// The company hosted here, whose conversations section the report
    /// shows; none shows none.
    pub company: Option<&'a dyn crate::interfaces::CompanyPorts>,
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
    if request.scheduler == Scheduler::Launchd
        && request.wake.locations.launch_agents
            != request
                .home
                .machine_user_home()
                .join("Library/LaunchAgents")
    {
        report.notices.push("alternate LaunchAgents directory is loaded explicitly; it is not automatically scanned at login".into());
    }
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
    if !request.check {
        // An earlier release's record of what installation wrote: the names
        // this home gives its services replace it.
        match instrukt_core::fsx::remove_file(
            request.home.state_dir().join("installation/owned.json"),
        ) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => report.finding("installation record", error),
        }
    }
    if request.module.is_none()
        && let Err(error) = _services::withdraw_legacy(request, demand.complete, &mut report)
    {
        report.finding("earlier registrations", error);
    }
    if request.module.is_none() && !request.conversations {
        admission_findings(request, &mut report);
    }
    if request.module.is_none() {
        runner_service(request, &demand, &mut report);
    }
    runtime_projects(request, &demand, &mut report);
    if request.conversations {
        fleet_permissions(request, &demand, &global, &mut report);
        boot_prerequisite(request, &demand, &mut report);
        return Ok(report);
    }
    if request.module != Some(Module::History)
        && (request.module != Some(Module::Events) || !demand.events.is_empty())
    {
        let required = !demand.projects().is_empty() && global.ticks_enabled()?;
        let projects = demand.projects();
        let tick = _services::tick(request);
        if let Err(error) = _services::reconcile(
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
            if let Err(error) = _redis::ensure(request, &demand, &mut report) {
                report.finding("Redis", error);
            }
        } else if demand.redis_references.is_empty()
            && demand.events.is_empty()
            && let Err(error) = _services::withdraw_named(
                request,
                ResourceKind::Redis,
                demand.complete,
                &mut report,
            )
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
            && let Err(error) = receiver(request, &demand, &mut report)
        {
            report.finding("receiver", error);
        }
    }
    fleet_permissions(request, &demand, &global, &mut report);
    if request.module.is_none()
        && let Err(error) = _services::report_unknown(request, &mut report)
    {
        report.finding("registrations", error);
    }
    boot_prerequisite(request, &demand, &mut report);
    Ok(report)
}

/// The runtime projects of the execution roots the selected modules start
/// sessions in (`module-installation`, "Runtime projects of execution
/// roots"), under one rule for company homes and project checkouts alike:
/// a missing one is created, or reported under `--check`; none is ever
/// withdrawn. With no root selected, the runtime is not asked.
fn runtime_projects(request: &Request<'_>, demand: &Demand, report: &mut Report) {
    use crate::runtime::ExecutionRootMode;
    use _demand::RuntimeProjectModule;
    let selected = |module: RuntimeProjectModule| {
        if request.conversations {
            return module == RuntimeProjectModule::Conversations;
        }
        match request.module {
            None => true,
            Some(Module::Todos) => module == RuntimeProjectModule::Todos,
            Some(Module::Events) => module == RuntimeProjectModule::Events,
            Some(_) => false,
        }
    };
    // One row per canonical root, naming every home that demands it.
    let mut roots: Vec<(PathBuf, PathBuf, Vec<String>)> = Vec::new();
    for entry in demand
        .runtime_projects
        .iter()
        .filter(|entry| selected(entry.module))
    {
        let canonical = entry
            .checkout
            .canonicalize()
            .unwrap_or_else(|_| entry.checkout.clone());
        match roots.iter_mut().find(|(_, root, _)| *root == canonical) {
            Some((_, _, labels)) => {
                if !labels.contains(&entry.label) {
                    labels.push(entry.label.clone());
                }
            }
            None => roots.push((entry.checkout.clone(), canonical, vec![entry.label.clone()])),
        }
    }
    if roots.is_empty() {
        return;
    }
    let adapter =
        match Configuration::load_global(request.home).and_then(|global| global.runner_runtime()) {
            Ok(runtime) => crate::runtime::adapter(runtime),
            Err(error) => {
                for (root, _, labels) in &roots {
                    report.resource(
                        ResourceKind::RuntimeProject,
                        Disposition::Unmet,
                        vec![root.clone()],
                        labels,
                        error.to_string(),
                    );
                    report.finding(format!("runtime project {}", root.display()), &error);
                }
                return;
            }
        };
    let mode = if request.check {
        ExecutionRootMode::Check
    } else {
        ExecutionRootMode::Ensure
    };
    for (root, _, labels) in roots {
        let name = format!("runtime project {}", root.display());
        let (disposition, detail) = match adapter.execution_root(&root, mode) {
            Ok(status) => match status.project_id {
                Some(project) if status.created => (
                    Disposition::Installed,
                    format!("created runtime project {project}"),
                ),
                Some(project) => (Disposition::Reused, format!("runtime project {project}")),
                None => {
                    let cause =
                        "no runtime project is rooted here; run `instrukt install`".to_owned();
                    report.finding(&name, &cause);
                    (Disposition::Unmet, cause)
                }
            },
            Err(error) => {
                let cause = error.to_string();
                report.finding(&name, &cause);
                (Disposition::Unmet, cause)
            }
        };
        report.resource(
            ResourceKind::RuntimeProject,
            disposition,
            vec![root],
            &labels,
            detail,
        );
    }
}

/// The admission findings that withhold a responsibility on this computer
/// though the rest of the layer is admitted: event responses switched on
/// without the event transport they are carried on (`modules.events`), and
/// automatic review switched on without what it needs
/// (`review.automatic`), and a declaration of regenerable output that is not
/// admitted, which withholds worktree reclamation for its project. On a
/// check, the two identities' readiness is asked for every project admitted
/// for automatic review.
fn admission_findings(request: &Request<'_>, report: &mut Report) {
    use instrukt_core::config::FindingKind;
    let Ok(candidate) = instrukt_core::config::Admitted::load(request.home) else {
        return;
    };
    // Reclamation is reported withheld for the projects admission withheld
    // it for, with the findings that did.
    let withheld: Vec<std::path::PathBuf> = candidate
        .generation
        .projects()
        .iter()
        .filter(|project| !project.reclamation)
        .map(|project| {
            project
                .checkout
                .join(instrukt_core::config::CONFIG_FILE_NAME)
        })
        .collect();
    for finding in &candidate.findings {
        let label = format!("{} `{}`", finding.document.display(), finding.key);
        if matches!(
            finding.kind,
            FindingKind::EventsNeedTransport | FindingKind::AutomaticReviewNotReady
        ) {
            report.finding(label, &finding.message);
        } else if instrukt_core::config::withholds_thread_retention(finding) {
            report.finding(
                label,
                format!(
                    "{}; thread retention is withheld on this computer while the runner's other work runs",
                    finding.message
                ),
            );
        } else if instrukt_core::config::withholds_reclamation(finding)
            && withheld.contains(&finding.document)
        {
            report.finding(
                label,
                format!(
                    "{}; reclaiming the declared output of this project's idle worktrees is withheld on this computer, and a delivered todo's worktree is still released",
                    finding.message
                ),
            );
        }
    }
    // The proposal pull-request credential is checked where it enters, on
    // install and on its check alike, for every company home admitted for
    // automatic review.
    _proposals::readiness(request, &candidate, report);
    if !request.check {
        return;
    }
    for project in candidate
        .generation
        .projects()
        .iter()
        .filter(|project| project.review)
    {
        let repository: Result<String, String> = project
            .config
            .github_api_url()
            .map_err(|error| error.to_string())
            .and_then(|api_url| {
                instrukt_core::github::repository(&project.config, api_url, &project.checkout)
                    .map_err(|error| error.to_string())
            });
        let label = format!(
            "{} `{}`",
            project
                .checkout
                .join(instrukt_core::config::CONFIG_FILE_NAME)
                .display(),
            instrukt_core::config::REVIEW_AUTOMATIC_KEY
        );
        let repository = match repository {
            Ok(repository) => repository,
            Err(error) => {
                report.finding(label, error);
                continue;
            }
        };
        use instrukt_core::interfaces::ReviewReadiness;
        match request.github.review_readiness(&repository, request.now) {
            ReviewReadiness::Met { .. } => {}
            ReviewReadiness::Unmet(message) => report.finding(label, message),
            ReviewReadiness::CannotAsk(cause) => report.finding(
                label,
                format!("GitHub cannot be asked whether automatic review's identities are ready on {repository}: {cause}"),
            ),
        }
    }
}

/// On Linux every owned unit is a systemd user unit that has to run without
/// a login; one finding names those that need the user's service manager to
/// start at boot when it does not. A wake-up registered with cron needs no
/// such setting, and instrukt never changes it.
fn boot_prerequisite(request: &Request<'_>, demand: &_demand::Demand, report: &mut Report) {
    if request.scheduler != Scheduler::SystemdUser {
        return;
    }
    let units: Vec<&str> = [
        (ResourceKind::Runner, "the runner service"),
        (ResourceKind::Receiver, "the receiver's socket and service"),
        (ResourceKind::Redis, "the private local Redis service"),
        (ResourceKind::WakeUp, "the minute timer"),
    ]
    .into_iter()
    .filter(|(kind, _)| {
        (*kind == ResourceKind::Runner && !demand.conversations.is_empty())
            || _services::registered(request, *kind)
    })
    .map(|(_, unit)| unit)
    .collect();
    if units.is_empty() {
        return;
    }
    if let Err(cause) = lingering() {
        report.finding(
            "user service manager",
            format!("{cause}; it is needed by {}", units.join(", ")),
        );
    }
}
/// Where a cluster connection is configured and the computer has a demand
/// this run covers that keeps a runner or a cluster reader — history,
/// events, an MCP server with sign-in, the minute wake-up or the runner
/// service — the cluster user must permit the computer fleet's query
/// (`project/instrukt/spec/feature/runner/module-installation`).
fn fleet_permissions(
    request: &Request<'_>,
    demand: &Demand,
    global: &Configuration,
    report: &mut Report,
) {
    let demanded = || -> anyhow::Result<bool> {
        if global.cluster_redis_url()?.is_none() {
            return Ok(false);
        }
        let history = !demand.history.is_empty();
        let events = !demand.events.is_empty()
            || (global.module_enabled(Module::Events)? && global.ingress_url()?.is_some());
        let sign_in = !demand.sign_in.is_empty();
        let wake_up = !demand.projects().is_empty() && global.ticks_enabled()?;
        let service = !demand.conversations.is_empty();
        Ok(if request.conversations {
            service
        } else {
            match request.module {
                None => history || events || sign_in || wake_up || service,
                Some(Module::History) => history,
                Some(Module::Events) => events || sign_in,
                Some(Module::Todos) => wake_up,
                Some(Module::Blueprint) => false,
            }
        })
    };
    match demanded() {
        Ok(false) => {}
        Ok(true) => {
            if let Err(error) = _redis::fleet(request) {
                report.finding("fleet Redis", error);
            }
        }
        Err(error) => report.finding("fleet Redis", error),
    }
}

/// The owned service that keeps `instrukt run` running while a company
/// home declares a connector; withdrawn when that
/// demand ends. Its state is recorded in the report as it was found.
fn runner_service(request: &Request<'_>, demand: &Demand, report: &mut Report) {
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
    } else if demand.conversations.is_empty() && !demand.unreadable_homes.is_empty() {
        // An unreadable home may declare a connector: the demand is
        // unresolved and nothing is withdrawn on the strength of it.
        let cause = demand.unreadable_homes.join("; ");
        report.finding(ResourceKind::Runner, &cause);
        status(ServiceState::Unmet, Some(cause))
    } else if demand.conversations.is_empty() {
        if !_services::registered(request, ResourceKind::Runner) {
            status(ServiceState::NoDemand, None)
        } else {
            let check = request.check;
            match _services::withdraw_named(request, ResourceKind::Runner, demand.complete, report)
            {
                Ok(()) if check => status(
                    ServiceState::Obsolete,
                    Some("no company home declares a connector".into()),
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
        // The boot prerequisite is a cause of the service's status beside
        // whatever its process shows.
        // The boot prerequisite is reported once for every owned unit; here
        // it is only a cause of the service's status.
        let boot = if request.scheduler == Scheduler::SystemdUser {
            lingering().err()
        } else {
            None
        };
        let (state, cause) = match _services::runner_service(
            request,
            &desired,
            demand.complete,
            &demand.conversations,
            report,
        ) {
            Ok(None) => (ServiceState::Running, None),
            Ok(Some(cause)) => (ServiceState::Unmet, Some(cause)),
            Err(error) => {
                let cause = error.to_string();
                report.finding(ResourceKind::Runner, &cause);
                (ServiceState::Unmet, Some(cause))
            }
        };
        let cause = match (cause, boot) {
            (Some(cause), Some(boot)) => Some(format!("{cause}; {boot}")),
            (cause, boot) => cause.or(boot),
        };
        status(state, cause)
    };
    report.runner_service = Some(found);
}

/// Whether the user's systemd manager starts at boot without a login, read
/// from the user's linger state; instrukt never changes it.
fn lingering() -> Result<(), String> {
    let uid = rustix::process::getuid().as_raw().to_string();
    let door = "the user's systemd manager does not start at boot without a login, so instrukt's owned units wait for one; enable it with `loginctl enable-linger`";
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
                    "expected graphify {}: {other}; run `instrukt install`",
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

fn receiver(request: &Request<'_>, demand: &Demand, report: &mut Report) -> anyhow::Result<()> {
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
    if ingress_port.is_none()
        && global.module_enabled(Module::Events)?
        && global.ingress_url()?.is_some()
        && let Ok(ingress) = global.ingress()
    {
        let machine = std::fs::read_to_string(request.home.machine_id_file()).map_err(|_| {
            anyhow::anyhow!("machine identity missing/unreadable; restore the recorded machine-id before receiver placement or withdrawal")
        })?;
        anyhow::ensure!(
            instrukt_core::machine::MachineId::parse(machine.trim()).is_some(),
            "invalid machine identity; restore the recorded machine-id before receiver placement"
        );
        if machine.trim() == ingress.computer.as_str() {
            ingress_port = Some(ingress.port);
            health_route = Some("/hooks/inbound/sign-in/00000000000000000000000000000000".into());
            projects.push("ingress:url".into());
        }
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
        let receiver = _services::receiver_definition(request, port, route)?;
        _services::reconcile(request, &receiver, true, demand.complete, &projects, report)?;
        if !request.check
            && let Some(route) = health_route
        {
            receiver_health(port, &route)?;
        }
    } else {
        _services::withdraw_named(request, ResourceKind::Receiver, demand.complete, report)?;
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
    let tick = _services::tick(request);
    let mut inspection = Request {
        check: true,
        ..request.clone()
    };
    inspection.module = None;
    if enabled && policy {
        _services::reconcile(
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
fn retained_event_status(request: &Request<'_>) -> Result<serde_json::Value, InstallationError> {
    use instrukt_core::state_db::rusqlite::OptionalExtension as _;
    let state = crate::state::RunnerState::new(request.home);
    // What the runner keeps in its state is reported whatever the files say
    // now: an invalid machine layer must not hide why the runner holds on.
    let failures=state.read(|conn,path| {
        let mut stmt=conn.prepare("SELECT event_id, state, failure FROM procedure_activations WHERE failure IS NOT NULL AND state!='set_aside'").map_err(crate::state::sql(path))?;
        let rows=stmt.query_map([],|row| Ok(serde_json::json!({"id":row.get::<_,String>(0)?,"state":row.get::<_,String>(1)?,"failure":row.get::<_,String>(2)?}))).map_err(crate::state::sql(path))?;
        rows.collect::<Result<Vec<_>,_>>().map_err(crate::state::sql(path))
    })?.unwrap_or_default();
    // What the retained consumers hold whatever the files say now.
    let consumers = crate::consumers::status(request.home, &state)?;
    let stopped_executions = crate::executions::Executions::new(&state).stopped()?;
    let input_findings = state.read(|conn, path| {
        let sql = crate::state::sql(path);
        let mut stmt = conn.prepare("SELECT runtime,session,input_id,state,failure,evidence_source,evidence_at FROM runtime_inputs WHERE state='unconfirmed' OR (state='accepted' AND waiting=1 AND failure IS NOT NULL) ORDER BY rowid").map_err(&sql)?;
        stmt.query_map([], |row| Ok(serde_json::json!({
            "session":format!("{}:{}", row.get::<_,String>(0)?,row.get::<_,String>(1)?),
            "input_id":row.get::<_,String>(2)?,"state":row.get::<_,String>(3)?,"failure":row.get::<_,Option<String>>(4)?,"evidence_source":row.get::<_,Option<String>>(5)?,"evidence_at":row.get::<_,Option<String>>(6)?
        }))).map_err(&sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)
    })?.unwrap_or_default();
    let set_aside = state.read(|conn, path| {
        let sql = crate::state::sql(path);
        // A mirrored activation and its input account for one retained entry.
        let rows = "SELECT event_id AS id, accepted_at AS at, failure FROM procedure_activations WHERE state='set_aside'
            UNION ALL SELECT i.input_id, i.arrived_at, i.failure FROM runtime_inputs i WHERE i.state='set_aside'
            AND NOT EXISTS (SELECT 1 FROM standing_deliveries d WHERE d.input_id=i.input_id AND d.target=(i.runtime || ':' || i.session) AND d.state='set_aside')";
        let count: u64 = conn.query_row(&format!("SELECT count(*) FROM ({rows})"), [], |row| row.get(0)).map_err(&sql)?;
        let mut statement = conn.prepare(&format!("SELECT id,at,failure FROM ({rows}) ORDER BY julianday(at) DESC,id LIMIT ?1")).map_err(&sql)?;
        let recent = statement.query_map([crate::inputs::SET_ASIDE_STATUS_LIMIT], |row| Ok(serde_json::json!({
            "id":row.get::<_,String>(0)?,"at":row.get::<_,String>(1)?,"failure":row.get::<_,Option<String>>(2)?
        }))).map_err(&sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
        Ok(serde_json::json!({"count":count,"recent":recent}))
    })?.unwrap_or_else(|| serde_json::json!({"count":0,"recent":[]}));
    // The runner workers' current failures, while they last: the per-worker
    // failure files under `Home::runner_workers_dir` and `Home::computer_fleet_file`.
    let mut workers: Vec<serde_json::Value> = crate::state::read_worker_failures(request.home)?
        .into_iter()
        .map(crate::state::WorkerFailure::into_json)
        .collect();
    workers.extend(crate::computer_fleet::failure_status(request.home));
    workers.sort_by(|a, b| a["worker"].as_str().cmp(&b["worker"].as_str()));
    // The run loop's last skipped wake, until a wake is not skipped.
    let skipped_wake = state.read(|conn, path| {
        conn.query_row("SELECT at,cause FROM runner_skipped_wake WHERE id=1", [], |row| Ok(serde_json::json!({"at":row.get::<_,String>(0)?,"cause":row.get::<_,String>(1)?}))).optional().map_err(crate::state::sql(path))
    })?.flatten();
    // What the running runner last did with the configuration it admitted:
    // replaced its generation, or kept the one in force.
    let runner = state.read(|conn, path| {
        conn.query_row("SELECT at,outcome FROM runner_admission WHERE id=1", [], |row| Ok(serde_json::json!({"at":row.get::<_,String>(0)?,"outcome":row.get::<_,String>(1)?}))).optional().map_err(crate::state::sql(path))
    })?.flatten();
    let conversations = request
        .company
        .map(|company| company.status(request.home, request.now))
        .transpose()
        .map_err(|e| inspection_error("conversations", e))?
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({
        "runner": runner, "workers": workers, "skipped_wake": skipped_wake,
        "activation_failures": failures, "consumers": consumers,
        "stopped_executions": stopped_executions, "input_findings": input_findings,
        "set_aside": set_aside, "conversations": conversations
    }))
}

pub fn events_status(request: &Request<'_>) -> Result<serde_json::Value, InstallationError> {
    let home = request.home.for_inspection();
    let request = &Request {
        home: &home,
        ..request.clone()
    };
    let mut status = match retained_event_status(request) {
        Ok(status) => status,
        Err(InstallationError::State(crate::state::StateError::Db(
            error @ instrukt_core::state_db::StateDbError::Newer { .. },
        ))) => {
            serde_json::json!({"state_database": {"path": home.state_db_file(), "reason": error.to_string()}})
        }
        Err(error) => return Err(error),
    };
    let runner = status["runner"].take();
    if let Some(fields) = status.as_object_mut() {
        fields.remove("runner");
    }
    // This computer's own machine ID, as it reads it, for comparison with the
    // other computers; reading creates no ID.
    let computer = instrukt_core::machine::existing_machine_id(request.home)
        .map_err(|e| inspection_error("machine ID", e))?
        .map(|id| id.as_str().to_owned());
    // What the files admit now: each project's admitted modules, the findings
    // of what is left out, or why no generation can be admitted at all.
    let candidate = instrukt_core::config::Admitted::load(request.home);
    let notification_configured = candidate.as_ref().ok().and_then(|candidate| {
        candidate
            .generation
            .global()
            .runner_notify()
            .ok()
            .map(|notify| matches!(notify, instrukt_core::config::Notify::Command(_)))
    });
    let admission = match &candidate {
        Ok(candidate) => serde_json::json!({
            "error": null,
            "runner": runner,
            "projects": candidate.generation.projects().iter().map(|project| serde_json::json!({
                "project": project.slug.as_str(),
                "todos": project.todos,
                "events": project.events,
                "history": project.history,
            })).collect::<Vec<_>>(),
            "findings": candidate.findings.iter().map(|finding| serde_json::json!({
                "document": finding.document,
                "key": finding.key,
                "message": finding.message,
            })).collect::<Vec<_>>(),
        }),
        Err(error) => {
            serde_json::json!({"error": error.to_string(), "runner": runner, "projects": [], "findings": []})
        }
    };
    if candidate.is_err() {
        // Nothing read from the configuration can be shown: what depends on
        // it is absent, and the admission error says why.
        return Ok(merge_status(
            status,
            serde_json::json!({"computer":computer,"ingress":null,"enabled_projects":null,"transport":null,"infrastructure":null,"findings":[],"admission":admission,"notification_configured":notification_configured,"runner_service":null}),
        ));
    }
    let mut report = Report::default();
    if let Ok(candidate) = &candidate {
        _proposals::findings(request, candidate, &mut report);
    }
    let demand = _demand::read(request.home, request.cwd, &mut report)
        .map_err(|e| InstallationError::Demand(e.to_string()))?;
    let infrastructure = converge(&Request {
        check: true,
        module: Some(Module::Events),
        ..request.clone()
    })?;
    // Only the runner service's state is shown: it is read alone, without
    // asking the runtime about anything the status leaves out.
    let mut service = Report::default();
    runner_service(
        &Request {
            check: true,
            module: None,
            conversations: true,
            ..request.clone()
        },
        &demand,
        &mut service,
    );
    let runner_service = service.runner_service;
    let global = Configuration::load_global(request.home)?.get(None, request.env)?;
    let configured = global.get("redis").is_some();
    let redis = if demand.events.is_empty() && !configured {
        vec![]
    } else {
        _redis::inspect(request, &mut report).map_err(|e| inspection_error("Redis", e))?
    };
    // This computer's own two ingress settings, as it reads them.
    let setting = |key: &str| {
        global
            .get("ingress")
            .and_then(|ingress| ingress.get(key))
            .and_then(serde_yaml_ng::Value::as_str)
            .map(str::to_owned)
    };
    Ok(merge_status(
        status,
        serde_json::json!({"computer":computer,"ingress":{"computer":setting("computer"),"reactions":setting("reactions")},"enabled_projects":demand.events,"transport":redis,"infrastructure":infrastructure,"findings":report.findings,"admission":admission,"notification_configured":notification_configured,"runner_service":runner_service}),
    ))
}

fn merge_status(
    mut retained: serde_json::Value,
    mut configured: serde_json::Value,
) -> serde_json::Value {
    if let (Some(retained), Some(configured)) =
        (retained.as_object_mut(), configured.as_object_mut())
    {
        retained.append(configured);
    }
    retained
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
