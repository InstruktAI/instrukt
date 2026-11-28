//! Interfaces core declares for answers only an optional module has.
//!
//! Core never depends on an optional module (architecture overview,
//! "Modules and crates"). Where a core context needs such an answer, it
//! declares the question here; the CLI wires the module's implementation
//! into [`CoreWiring`] only when that module is switched on. A `None` slot
//! means the module is off, and the core context behaves as the spec says
//! for that case.
//!
//! Implementors: [`TodoExistence`] and [`CommitExemption`] — work
//! (`instrukt_todos::work`);
//! [`FlavorRoles`] — workflow (`instrukt_todos::workflow`); [`FlavorSchema`]
//! — workflow's `WorkflowSchema`, always wired; [`GraphGeneration`] —
//! blueprint (`instrukt_blueprint`); [`ConfiguredAgentRuntimes`] — the
//! selected runtime's adapter (`instrukt_runner::runtime`). The CLI fills
//! each slot from its module's wiring file (`crates/instrukt-cli/src/wiring/`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ids::{CommitSha, ProjectSlug, ProviderInstanceId, RoleName, TodoSlug};

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

/// Records a commit's exemption from independent review through work's
/// admission of review evidence (`project/instrukt/spec/feature/work/github-apps`,
/// Admission of review evidence), for the project the caller stands in. Used
/// by repository for an author's no-review routing choice once the commit
/// exists (`project/instrukt/spec/feature/repository/attributed-commit`,
/// Review routing).
pub trait CommitExemption: Send + Sync {
    fn exempt(
        &self,
        commit: &CommitSha,
        basis: &str,
        caller: Option<&crate::ids::SessionRef>,
    ) -> Result<(), InterfaceError>;
}

/// The roles of the selected workflow flavor (workflow, through the `todos`
/// module).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlavorMembership {
    /// `roles:` declared by the selected workflow's `workflow.yaml`.
    pub roles: Vec<RoleName>,
}

/// The roles of the selected workflow flavor (workflow, through the `todos`
/// module). Used by docs to validate procedure roles.
pub trait FlavorRoles: Send + Sync {
    /// The declared roles for the project at `project_root`, read from the
    /// flavor its configuration selects.
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
    /// The flavor's declared `orchestrator` role, by manifest name.
    pub orchestrators: Vec<(String, RoleName)>,
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

/// The company's recording of a commit just made as a knowledge proposal
/// (`project/instrukt/spec/domain/company`, Knowledge proposal). The
/// `post-commit` hook run invokes it synchronously with the commit's
/// repository root and the hook's environment, which is the committing
/// session's; it decides everything, reports on `stderr`, the commit's
/// standard error, and never fails the commit.
pub trait ProposalRecording: Send + Sync {
    fn record(
        &self,
        home: &crate::home::Home,
        root: &Path,
        env: &crate::config::Env,
        stderr: &mut dyn std::io::Write,
    );
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
    pub commit_exemption: Option<Arc<dyn CommitExemption>>,
    pub flavor_roles: Option<Arc<dyn FlavorRoles>>,
    pub flavor_schema: Option<Arc<dyn FlavorSchema>>,
    pub graph_generation: Option<Arc<dyn GraphGeneration>>,
    pub configured_runtimes: Option<Arc<dyn ConfiguredAgentRuntimes>>,
    pub proposal_recording: Option<Arc<dyn ProposalRecording>>,
}

impl std::fmt::Debug for CoreWiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreWiring")
            .field("caller_identity", &self.caller_identity.is_some())
            .field("github_actors", &self.github_actors.is_some())
            .field("session_role", &self.session_role.is_some())
            .field("todo_existence", &self.todo_existence.is_some())
            .field("commit_exemption", &self.commit_exemption.is_some())
            .field("flavor_roles", &self.flavor_roles.is_some())
            .field("flavor_schema", &self.flavor_schema.is_some())
            .field("graph_generation", &self.graph_generation.is_some())
            .field("configured_runtimes", &self.configured_runtimes.is_some())
            .field("proposal_recording", &self.proposal_recording.is_some())
            .finish()
    }
}

/// Whether the two identities automatic review needs can do their part on a
/// repository (`github-apps`, "Readiness for automatic review").
pub enum ReviewReadiness {
    /// Both credentials were issued with exactly their permissions; the
    /// observation credential is kept for the caller's reads.
    Met {
        observation: Arc<crate::github::Credential>,
    },
    /// An identity cannot do its part: the App, the repository, what it
    /// lacks and the repair, in plain words.
    Unmet(String),
    /// GitHub could not be asked; nothing is decided.
    CannotAsk(String),
}

/// Work's readiness answer for automatic review. It requests the two
/// credentials and writes no state.
pub trait AutomaticReviewReadiness: Send + Sync {
    fn review_readiness(&self, repository: &str, now: jiff::Timestamp) -> ReviewReadiness;
}

/// Work's readiness answer for the proposal pull-request credential: the
/// identity the `company-agent` role selects, with write access to pull
/// requests and read access to contents. It requests that one credential
/// and writes no state; a met answer carries it.
pub trait ProposalReadiness: Send + Sync {
    fn proposal_readiness(&self, repository: &str, now: jiff::Timestamp) -> ReviewReadiness;
}

/// What kind of account a GitHub login names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    /// An organisation.
    Organisation,
    /// A person's own account.
    Personal,
    /// No account of that login.
    Absent,
}

/// Work's answer to what kind of account a GitHub login names, read from
/// GitHub's public account record
/// (`project/instrukt/spec/feature/work/github-apps`, Company repositories).
pub trait GitHubAccounts: Send + Sync {
    fn account_kind(&self, login: &str) -> Result<AccountKind, InterfaceError>;
}

/// What creating a company's repository found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Creation {
    /// GitHub created the repository, empty.
    Created,
    /// A repository of that name already exists in the organisation.
    AlreadyExists,
}

/// Why work's GitHub boundary could not act on a company repository.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompanyRepositoryError {
    /// The project identity cannot administer repositories in the
    /// organisation: no installation there, or one without the permission.
    #[error(
        "the project App{} cannot create repositories in the GitHub organisation `{organisation}` ({cause}); install it on `{organisation}` with write access to repository administration (administration: write), then run the command again — the configuration skill's GitHub App setup, Company repositories, walks through it",
        app.as_deref().map(|slug| format!(" `{slug}`")).unwrap_or_default()
    )]
    NotReady {
        app: Option<String>,
        organisation: crate::ids::GitHubOrganisation,
        cause: String,
    },
    /// GitHub refused the operation or could not be asked.
    #[error("{0}")]
    Failed(String),
}

/// Work's GitHub boundary for company repositories, under the project
/// identity's installation on the owning organisation
/// (`project/instrukt/spec/feature/work/github-apps`, Company repositories).
/// `now` is the time credentials are issued at.
pub trait CompanyRepositories: Send + Sync {
    /// Whether the project identity can administer repositories in `org`:
    /// its credential requested with exactly that permission.
    fn ready(
        &self,
        org: &crate::ids::GitHubOrganisation,
        now: jiff::Timestamp,
    ) -> Result<(), CompanyRepositoryError>;
    /// Creates the private repository `org/repo`, empty.
    fn create(
        &self,
        org: &crate::ids::GitHubOrganisation,
        repo: &ProjectSlug,
        now: jiff::Timestamp,
    ) -> Result<Creation, CompanyRepositoryError>;
    /// Makes `login` an administrator of `org/repo`: at once for a member
    /// of `org`, by GitHub's invitation for anyone else.
    fn grant_admin(
        &self,
        org: &crate::ids::GitHubOrganisation,
        repo: &ProjectSlug,
        login: &crate::ids::GitHubLogin,
        now: jiff::Timestamp,
    ) -> Result<(), CompanyRepositoryError>;
    /// Sets the default branch of `org/repo` to `branch`.
    fn set_default_branch(
        &self,
        org: &crate::ids::GitHubOrganisation,
        repo: &ProjectSlug,
        branch: &str,
        now: jiff::Timestamp,
    ) -> Result<(), CompanyRepositoryError>;
}

/// Setup-verified stable actor facts for local author attribution. No repository
/// or installation authorization is implied by this capability.
pub trait GitHubActors: Send + Sync {
    fn actor(&self, role: &str) -> Result<crate::github::Bot, crate::github::AppError>;
}
