//! Interfaces core declares for answers only an optional module has.
//!
//! Core never depends on an optional module (architecture overview,
//! "Modules and crates"). Where a core context needs such an answer, it
//! declares the question here; the CLI wires the module's implementation
//! into [`CoreWiring`] only when that module is switched on. A `None` slot
//! means the module is off, and the core context behaves as the spec says
//! for that case.
//!
//! Implementors: [`TodoExistence`] — work (`instrukt_todos::work`);
//! [`FlavorRoles`] — workflow (`instrukt_todos::workflow`); [`FlavorSchema`]
//! — workflow's `WorkflowSchema`, always wired; [`GraphGeneration`] —
//! blueprint (`instrukt_blueprint`); [`ConfiguredAgentRuntimes`] — the
//! selected runtime's adapter (`instrukt_runner::runtime`). The CLI fills
//! each slot from its module's wiring file (`crates/instrukt-cli/src/wiring/`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ids::{CommitSha, ProjectSlug, ProviderInstanceId, RoleName, SnippetId, TodoSlug};

/// Caller identity supplied by the explicitly selected runtime adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    pub session: crate::ids::SessionRef,
    pub agent: Option<String>,
}
/// Recognize native caller context without performing runtime I/O.
pub trait CallerIdentityLookup: Send + Sync {
    fn lookup_caller(&self, env: &crate::config::Env) -> Option<CallerIdentity>;
    fn caller_variables(&self) -> &'static [&'static str];
}

/// Recorded session role, supplied by the assignment owners at composition.
pub trait SessionRoleLookup: Send + Sync {
    fn role(&self, session: &crate::ids::SessionRef) -> Result<Option<RoleName>, InterfaceError>;
}

/// An optional module could not answer a core question.
#[derive(Debug, thiserror::Error)]
#[error("{module}: {message}")]
pub struct InterfaceError {
    /// The answering module (`todos`, `blueprint`).
    pub module: &'static str,
    pub message: String,
}

/// Whether a todo slug names an existing todo of a project (work, through
/// the `todos` module). Used by docs for pending-doc slugs and by repository
/// for `Todo:` / `Planned-Todo:` trailers.
pub trait TodoExistence: Send + Sync {
    fn todo_exists(&self, project: &ProjectSlug, slug: &TodoSlug) -> Result<bool, InterfaceError>;
}

/// The roles and dispatched procedures of the selected workflow flavor
/// (workflow, through the `todos` module).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlavorMembership {
    /// `roles:` declared by the selected workflow's `actions.yaml`.
    pub roles: Vec<RoleName>,
    /// Procedures dispatched by the selected workflow's actions.
    pub dispatched_procedures: Vec<SnippetId>,
}

/// The roles and dispatched procedures of the selected workflow flavor
/// (workflow, through the `todos` module). Used by docs to validate
/// procedure roles.
pub trait FlavorRoles: Send + Sync {
    /// The declared roles and dispatched procedures for the project at
    /// `project_root`, read from the flavor its configuration selects.
    fn declared(&self, project_root: &Path) -> Result<FlavorMembership, InterfaceError>;
}

/// One problem in an authored flavor's workflow files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlavorFinding {
    /// The file, relative to the `workflow/` directory.
    pub file: PathBuf,
    /// Where in the file (a key path), empty for the whole file.
    pub location: String,
    pub message: String,
}

/// What a valid flavor declares that docs validation checks against the
/// procedures (`workflow-definition`, "Flavor source").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlavorDeclarations {
    /// `roles:` in declaration order.
    pub roles: Vec<RoleName>,
    /// Each flow's declared `orchestrator` role, by flow name.
    pub orchestrators: Vec<(String, RoleName)>,
    /// Each action whose command names a procedure, with that procedure.
    pub action_procedures: Vec<(String, SnippetId)>,
}

/// The outcome of checking one authored flavor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlavorCheck {
    /// The files satisfy the workflow schema.
    Valid(FlavorDeclarations),
    /// The schema findings; nothing is declared until they are fixed.
    Invalid(Vec<FlavorFinding>),
}

/// The workflow schema an authored flavor (`docs/domain/<domain>/workflow/`)
/// is validated against (workflow, through the todos crate). Docs
/// validation checks these files whether or not the `todos` module is on,
/// because the publishing project authors them either way; the CLI always
/// wires it.
pub trait FlavorSchema: Send + Sync {
    fn check(&self, workflow_dir: &Path) -> Result<FlavorCheck, InterfaceError>;
}

/// How a remote-main revision reached this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainAdvance {
    /// This machine pushed the revision to main (pre-push).
    Push,
    /// A pull moved local main to the revision (post-merge, post-rewrite).
    Pull,
}

/// One remote-main revision handed from a hook run to graph generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRequest {
    pub project: ProjectSlug,
    /// The repository's primary worktree.
    pub project_root: PathBuf,
    pub sha: CommitSha,
    pub advance: MainAdvance,
}

/// Graph generation for a revision of main (blueprint, through the
/// `blueprint` module). Repository's hook runs call it; it records the
/// revision and returns without waiting for the graph.
pub trait GraphGeneration: Send + Sync {
    fn start_generation(&self, request: &GenerationRequest) -> Result<(), InterfaceError>;
}

/// How the adapter selected an instruction target home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionHomeSource {
    NativeVariable,
    Default,
    Unknown,
}

impl InstructionHomeSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::NativeVariable => "native_variable",
            Self::Default => "default",
            Self::Unknown => "unknown",
        }
    }
}

/// A limitation of instruction relocation or receipt, independent of file presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionAssurance {
    InstructionRelocationUnverified,
    RuntimeHomeDiffers,
}

impl InstructionAssurance {
    pub fn name(self) -> &'static str {
        match self {
            Self::InstructionRelocationUnverified => "instruction_relocation_unverified",
            Self::RuntimeHomeDiffers => "runtime_home_differs",
        }
    }
}

/// One enabled agent instance of the selected runtime, with what the
/// runtime's adapter resolved for it. Publication reads only these facts: no
/// runtime settings or environment reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredInstance {
    pub instance: ProviderInstanceId,
    /// The open driver/selection name, including adapter-supplied compatibility aliases.
    pub runtime: String,
    /// Established native loader family; an open selection name alone proves no loader.
    pub native_runtime: Option<String>,
    /// The registry agent an ACP-registry instance runs.
    pub registry_agent: Option<String>,
    /// The home the instance reads its user-scope files from, when its
    /// native variable or documented default establishes one.
    pub home: Option<PathBuf>,
    /// Each folder in which a link to the instance's file must exist.
    pub link_folders: Vec<PathBuf>,
    /// A home or link folder the instance's settings name relative to the
    /// runtime's own working directory, which the runtime does not report:
    /// the instance's home cannot be known, so `home` is `None`, it has no
    /// link folders, and publication writes nothing for it.
    pub unknown_home: Option<PathBuf>,
    pub home_source: InstructionHomeSource,
    pub assurance: Vec<InstructionAssurance>,
}

/// What the selected runtime reported about its configured agent instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The enabled instances, each with its resolved home.
    Reported(Vec<ConfiguredInstance>),
    /// The runtime could not be asked; `cause` says why.
    Unreachable { runtime: String, cause: String },
}

/// The agent instances the selected runtime has configured (runner, through
/// the selected runtime's adapter). Publication chooses the global agent
/// files' targets from the answer.
pub trait ConfiguredAgentRuntimes: Send + Sync {
    fn configured(&self, home: &crate::home::Home, env: &crate::config::Env) -> Answer;
}

/// The optional-module implementations the CLI wired for this invocation.
/// Built once in the CLI's composition root; core contexts receive it by
/// reference.
#[derive(Clone, Default)]
pub struct CoreWiring {
    pub caller_identity: Option<Arc<dyn CallerIdentityLookup>>,
    pub github_actors: Option<Arc<dyn GitHubActors>>,
    pub session_role: Option<Arc<dyn SessionRoleLookup>>,
    pub todo_existence: Option<Arc<dyn TodoExistence>>,
    pub flavor_roles: Option<Arc<dyn FlavorRoles>>,
    pub flavor_schema: Option<Arc<dyn FlavorSchema>>,
    pub graph_generation: Option<Arc<dyn GraphGeneration>>,
    pub configured_runtimes: Option<Arc<dyn ConfiguredAgentRuntimes>>,
}

impl std::fmt::Debug for CoreWiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreWiring")
            .field("caller_identity", &self.caller_identity.is_some())
            .field("github_actors", &self.github_actors.is_some())
            .field("session_role", &self.session_role.is_some())
            .field("todo_existence", &self.todo_existence.is_some())
            .field("flavor_roles", &self.flavor_roles.is_some())
            .field("flavor_schema", &self.flavor_schema.is_some())
            .field("graph_generation", &self.graph_generation.is_some())
            .field("configured_runtimes", &self.configured_runtimes.is_some())
            .finish()
    }
}

/// Setup-verified stable actor facts for local author attribution. No repository
/// or installation authorization is implied by this capability.
pub trait GitHubActors: Send + Sync {
    fn actor(&self, role: &str) -> Result<crate::github::Bot, crate::github::AppError>;
}
