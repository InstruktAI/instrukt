//! Workflow service over the work record:
//! - factual ledger (`instrukt todo show` / `instrukt todo work`),
//! - deliberate pickup (`instrukt todo work <slug>`),
//! - runner dispatch assignment (`WorkflowAssignments`), and
//! - core's [`FlavorRoles`] for docs validation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use instrukt_core::config::{
    self, ApprovalGate as ConfigGate, ConfigError, Configuration, Env, Module,
};
use instrukt_core::docs::Position;
use instrukt_core::git::{self, GitError};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef, TodoSlug};
use instrukt_core::interfaces::{FlavorMembership, FlavorRoles, InterfaceError};
use instrukt_runner::interfaces::{
    Assignment, ClaimOutcome, Claimant, Claims, Lane, ProjectRef, TodoKind, WorkInterfaceError,
    WorkflowAssignments, WorkflowInterfaceError,
};

use crate::evidence::{TodoEvidence, TodoStatus, WorkEvidence};

use super::_flavor::{Flavor, FlavorError};
use super::_observe::{ObservationError, observe};
use super::_provider::{
    Approvers, PrerequisiteStanding, TodoLedger, derive_ledger, finalize_handoff_pending,
    progress_fingerprint, standings, task_approval_satisfied, waiting_for_human_approval,
};

/// What deliberate pickup did about the execution claim before reading facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pickup {
    /// The calling session now holds the claim, recorded and reflected.
    Claimed,
    /// The calling session already held it.
    AlreadyHeld,
    /// The claim is recorded locally; GitHub could not be reached, so it is
    /// pending reflection.
    NotReflected,
    /// The todo is not active: the call is read-only and records nothing.
    ReadOnly(TodoStatus),
}

/// The factual result of `instrukt todo work <slug>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkEntry {
    pub project: ProjectSlug,
    pub todo: TodoSlug,
    pub pickup: Pickup,
    pub ledger: TodoLedger,
}

/// `instrukt todo work` or ledger inspection could not answer.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("{dir} is not inside a git repository")]
    NotInRepository { dir: PathBuf },
    #[error("the repository at {root} declares no project; run `instrukt init`")]
    NoProject { root: PathBuf },
    #[error("{project} has no todo `{todo}`")]
    UnknownTodo {
        project: ProjectSlug,
        todo: TodoSlug,
    },
    #[error(
        "picking up `{todo}` records a claim for the calling session, and no configured runtime's session variable is set; run it from an agent session"
    )]
    NoSession { todo: TodoSlug },
    #[error("`{todo}` is claimed elsewhere; leave it to the holder")]
    ClaimedElsewhere { todo: TodoSlug },
    #[error("cannot load the workflow flavor: {0}")]
    Flavor(#[from] FlavorError),
    #[error(transparent)]
    Work(#[from] WorkInterfaceError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Observation(#[from] ObservationError),
    #[error("the runtime session `{value}` is not a session reference: {message}")]
    Session { value: String, message: String },
}

/// Workflow service over work's record.
pub struct Workflow {
    home: Home,
    evidence: Arc<dyn WorkEvidence>,
    env: Env,
}

struct Reading {
    config: Configuration,
    flavor: Flavor,
    project: ProjectRef,
}

impl Workflow {
    pub fn new(home: Home, evidence: Arc<dyn WorkEvidence>, env: Env) -> Self {
        Self {
            home,
            evidence,
            env,
        }
    }

    /// Read-only factual ledger for `todo` (`instrukt todo show <slug>`).
    pub fn ledger(&self, cwd: &Path, todo: &TodoSlug) -> Result<TodoLedger, WorkflowError> {
        let reading = self.read_project_for_ledger(cwd)?;
        let evidence = self.todo_evidence(&reading.project, todo)?;
        self.build_ledger(&reading.config, &reading.project, &evidence)
    }

    /// Read-only factual ledger derived directly from an already-loaded
    /// [`crate::work::TodoView`], avoiding a second work-session open or
    /// GitHub round-trip during `instrukt todo show`.
    pub fn ledger_for_view(
        &self,
        cwd: &Path,
        view: &crate::work::TodoView,
    ) -> Result<TodoLedger, WorkflowError> {
        let reading = self.read_project_for_ledger(cwd)?;
        let prereq_standings: Vec<(TodoSlug, PrerequisiteStanding)> = view
            .prerequisites
            .iter()
            .map(|p| {
                let standing = match p.status {
                    Some(TodoStatus::Delivered) => PrerequisiteStanding::Delivered,
                    Some(_) => PrerequisiteStanding::Undelivered,
                    None => PrerequisiteStanding::NotFound,
                };
                (p.slug.clone(), standing)
            })
            .collect();
        let evidence = crate::work::_ports::to_evidence(view.record.clone(), Vec::new());
        let observed = observe(&self.home, &reading.project.root, &evidence, &[])?;
        let approvers = approvers_of(&reading.config)?;
        Ok(derive_ledger(
            &evidence,
            &observed,
            approvers,
            prereq_standings,
        ))
    }

    /// Deliberate pickup (`instrukt todo work <slug>`): records the calling
    /// session's claim when active, then returns the current factual ledger.
    pub fn pickup(
        &self,
        cwd: &Path,
        todo: &TodoSlug,
        claims: &dyn Claims,
    ) -> Result<WorkEntry, WorkflowError> {
        let started = Instant::now();
        let reading = self.read_project(cwd)?;
        let evidence = self.todo_evidence(&reading.project, todo)?;
        let pickup = match evidence.status {
            TodoStatus::Active => {
                let session = self.session(&reading.config, todo)?;
                match claims.record_claim(&reading.project, todo, &Claimant::Session(session))? {
                    ClaimOutcome::ClaimedElsewhere => {
                        return Err(WorkflowError::ClaimedElsewhere { todo: todo.clone() });
                    }
                    ClaimOutcome::Claimed => Pickup::Claimed,
                    ClaimOutcome::AlreadyHeld => Pickup::AlreadyHeld,
                    ClaimOutcome::NotReflected => Pickup::NotReflected,
                }
            }
            status => Pickup::ReadOnly(status),
        };
        let evidence = match pickup {
            Pickup::ReadOnly(_) => evidence,
            _ => self.todo_evidence(&reading.project, todo)?,
        };
        let ledger = self.build_ledger(&reading.config, &reading.project, &evidence)?;
        tracing::debug!(
            event = "workflow_pickup",
            project = %reading.project.slug,
            todo = %todo,
            pickup = ?pickup,
            duration_ms = started.elapsed().as_millis() as u64,
        );
        Ok(WorkEntry {
            project: reading.project.slug,
            todo: todo.clone(),
            pickup,
            ledger,
        })
    }

    fn read_project(&self, cwd: &Path) -> Result<Reading, WorkflowError> {
        let root = git::worktree_root(cwd)?.ok_or_else(|| WorkflowError::NotInRepository {
            dir: cwd.to_path_buf(),
        })?;
        let primary = primary_worktree(&root)?;
        let config = Configuration::load(&self.home, Some(&primary))?;
        let slug = config.project()?.ok_or_else(|| WorkflowError::NoProject {
            root: primary.clone(),
        })?;
        self.reading(
            config,
            ProjectRef {
                slug,
                root: primary,
            },
        )
    }

    /// Reads project configuration without requiring a published workflow
    /// flavor (so `todo show` still reports factual ledger state in minimal
    /// test setups).
    fn read_project_for_ledger(&self, cwd: &Path) -> Result<LedgerProject, WorkflowError> {
        let root = git::worktree_root(cwd)?.ok_or_else(|| WorkflowError::NotInRepository {
            dir: cwd.to_path_buf(),
        })?;
        let primary = primary_worktree(&root)?;
        let config = Configuration::load(&self.home, Some(&primary))?;
        let slug = config.project()?.ok_or_else(|| WorkflowError::NoProject {
            root: primary.clone(),
        })?;
        Ok(LedgerProject {
            config,
            project: ProjectRef {
                slug,
                root: primary,
            },
        })
    }

    fn reading(
        &self,
        config: Configuration,
        project: ProjectRef,
    ) -> Result<Reading, WorkflowError> {
        let flavor = Flavor::load(&published_workflow(&self.home, &config)?)?;
        Ok(Reading {
            config,
            flavor,
            project,
        })
    }

    fn todo_evidence(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<TodoEvidence, WorkflowError> {
        self.evidence
            .evidence(project, todo)?
            .ok_or_else(|| WorkflowError::UnknownTodo {
                project: project.slug.clone(),
                todo: todo.clone(),
            })
    }

    fn session(
        &self,
        config: &Configuration,
        todo: &TodoSlug,
    ) -> Result<SessionRef, WorkflowError> {
        config
            .caller(&self.env)?
            .ok_or_else(|| WorkflowError::NoSession { todo: todo.clone() })
    }

    fn build_ledger(
        &self,
        config: &Configuration,
        project: &ProjectRef,
        evidence: &TodoEvidence,
    ) -> Result<TodoLedger, WorkflowError> {
        let others = related_evidence(self.evidence.as_ref(), project, evidence)?;
        let observed = observe(&self.home, &project.root, evidence, &others)?;
        let prereqs = standings(&evidence.prerequisites, |slug| {
            self.evidence.evidence(project, slug)
        })?;
        let approvers = approvers_of(config)?;
        Ok(derive_ledger(evidence, &observed, approvers, prereqs))
    }
}

struct LedgerProject {
    config: Configuration,
    project: ProjectRef,
}

fn approvers_of(config: &Configuration) -> Result<Approvers, ConfigError> {
    Ok(Approvers {
        prd: config.approver(ConfigGate::Prd)?,
        task: config.approver(ConfigGate::Task)?,
    })
}

impl WorkflowAssignments for Workflow {
    fn assignment(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<Assignment>, WorkflowInterfaceError> {
        let config = Configuration::load(&self.home, Some(&project.root)).map_err(|err| {
            WorkflowInterfaceError::Flavor {
                message: err.to_string(),
            }
        })?;
        let reading = self.reading(config, project.clone()).map_err(|err| {
            WorkflowInterfaceError::Flavor {
                message: err.to_string(),
            }
        })?;
        let Some(evidence) = self.evidence.evidence(project, todo)? else {
            return Ok(None);
        };
        if evidence.status != TodoStatus::Active || evidence.hold.is_some() {
            return Ok(None);
        }
        let prereqs = standings(&evidence.prerequisites, |slug| {
            self.evidence.evidence(project, slug)
        })?;
        if !prereqs
            .iter()
            .all(|(_, standing)| *standing == PrerequisiteStanding::Delivered)
        {
            return Ok(None);
        }
        let approvers =
            approvers_of(&reading.config).map_err(|err| WorkflowInterfaceError::Flavor {
                message: err.to_string(),
            })?;
        // Human-owned approval gate: when unclaimed and waiting on the human,
        // do not re-dispatch until the approval arrives.
        if evidence.claim.is_none()
            && waiting_for_human_approval(&evidence, approvers.prd, approvers.task)
        {
            return Ok(None);
        }
        let others = related_evidence(self.evidence.as_ref(), project, &evidence)?;
        let observed = observe(&self.home, &project.root, &evidence, &others).map_err(|err| {
            WorkflowInterfaceError::Observation {
                todo: todo.clone(),
                message: err.to_string(),
            }
        })?;
        // Finalized handoff already pending for `integrate`: do not re-dispatch
        // `todo-orchestrator` while unclaimed and awaiting integration.
        if evidence.claim.is_none() && finalize_handoff_pending(&evidence, &observed) {
            return Ok(None);
        }
        let state = progress_fingerprint(&evidence, &observed);
        if evidence.kind == TodoKind::Task && task_approval_satisfied(&evidence, approvers.task) {
            return Ok(Some(Assignment {
                lane: Lane::TaskRun,
                orchestrator: None,
                state,
            }));
        }
        Ok(Some(Assignment {
            lane: Lane::Work,
            orchestrator: Some(reading.flavor.orchestrator().clone()),
            state,
        }))
    }
}

/// Every todo whose evidence `evidence` depends on: prerequisites by slug
/// and grounding dependencies by durable identity.
pub fn related_evidence(
    store: &dyn WorkEvidence,
    project: &ProjectRef,
    evidence: &TodoEvidence,
) -> Result<Vec<TodoEvidence>, WorkInterfaceError> {
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    for prereq in &evidence.prerequisites {
        if let crate::evidence::Prerequisite::Todo(slug) = prereq
            && let Some(other) = store.evidence(project, slug)?
            && ids.insert(other.id.clone())
        {
            out.push(other);
        }
    }
    for version in evidence.artifacts.iter().filter_map(|a| a.versions.last()) {
        for dep in &version.grounding.dependencies {
            if let Some(id) = &dep.id
                && !ids.contains(id)
                && let Some(other) = store.evidence_by_id(project, id)?
                && ids.insert(other.id.clone())
            {
                out.push(other);
            }
        }
    }
    Ok(out)
}

fn published_workflow(home: &Home, config: &Configuration) -> Result<PathBuf, ConfigError> {
    Ok(home.domain_dir(&config.workflow_domain()?).join("workflow"))
}

fn primary_worktree(root: &Path) -> Result<PathBuf, GitError> {
    let listing = git::run(root, ["worktree", "list", "--porcelain"])?;
    Ok(listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map_or_else(|| root.to_path_buf(), PathBuf::from))
}

/// Core's flavor-role lookup for docs validation: the roles declared by the
/// selected workflow flavor's `workflow.yaml`.
#[derive(Debug, Clone)]
pub struct SelectedFlavorRoles {
    home: Home,
}

impl SelectedFlavorRoles {
    pub fn new(home: Home) -> Self {
        Self { home }
    }
}

impl FlavorRoles for SelectedFlavorRoles {
    fn declared(&self, project_root: &Path) -> Result<FlavorMembership, InterfaceError> {
        let failed = |message: String| InterfaceError {
            module: "todos",
            message,
        };
        let config = Configuration::load(&self.home, Some(project_root))
            .map_err(|err| failed(err.to_string()))?;
        let domain = config
            .workflow_domain()
            .map_err(|err| failed(err.to_string()))?;
        let authored = project_root
            .join(Position::of_domain(&domain).dir())
            .join("workflow");
        let dir = if authored.is_dir() {
            authored
        } else {
            self.home.domain_dir(&domain).join("workflow")
        };
        Flavor::load(&dir)
            .map(|flavor| FlavorMembership {
                roles: flavor.roles().to_vec(),
                dispatched_procedures: Vec::new(),
            })
            .map_err(|err| failed(err.to_string()))
    }
}

/// Whether `modules.todos` is on for a caller in `cwd`.
pub fn todos_enabled(home: &Home, cwd: &Path) -> Result<bool, WorkflowError> {
    let root = git::worktree_root(cwd)?;
    Ok(config::module_enabled(
        home,
        root.as_deref(),
        Module::Todos,
    )?)
}
