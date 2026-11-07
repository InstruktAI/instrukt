//! Workflow against the record: `instrukt todo prepare|work` (deliberate
//! pickup, then the next obligation), the runner's
//! [`WorkflowAssignments`], and core's [`FlavorRoles`]
//! (`docs/project/spec/domain/workflow.md`, "Flavor-facing contract").
//!
//! Each call loads the selected flavor, reads the todo's evidence through
//! work's [`WorkEvidence`], observes the repository, derives facts and
//! evaluates. Nothing is stored; the next call derives again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use instrukt_core::config::{
    self, ApprovalGate as ConfigGate, ConfigError, Configuration, Env, Module,
};
use instrukt_core::docs::Position;
use instrukt_core::git::{self, GitError};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, SnippetId, TodoSlug};
use instrukt_core::index::{GlobalIndex, IndexError};
use instrukt_core::interfaces::{FlavorRoles, InterfaceError};
use instrukt_runner::interfaces::{
    AssignedEffect, AssignedObligation, Assignment, ClaimOutcome, Claimant, Claims, Lane,
    ProjectRef, Responsibility, TodoKind, WorkInterfaceError, WorkflowAssignments,
    WorkflowInterfaceError,
};

use crate::evidence::{TodoEvidence, TodoStatus, WorkEvidence};

use super::_derive::{Derivation, Responsible, Subject};
use super::_facts::{FactError, Facts};
use super::_flavor::{Flavor, FlavorError};
use super::_lane::derive_lane;
use super::_observe::{ObservationError, observe};
use super::_provider::{Approvers, ProviderInput, derive_facts, standings};
use super::_render::{ConfigLookup, ConfigLookupError};

/// The flows `instrukt todo prepare|work` evaluate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// `workflow/prepare.yaml`.
    Prepare,
    /// `workflow/work.yaml`.
    Work,
}

impl Flow {
    /// The flow's name: the stem of its definition file.
    pub fn as_str(self) -> &'static str {
        match self {
            Flow::Prepare => "prepare",
            Flow::Work => "work",
        }
    }
}

/// What the call did about the execution claim before evaluating.
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

/// The next obligation `instrukt todo prepare|work` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Next {
    pub project: ProjectSlug,
    pub todo: TodoSlug,
    pub pickup: Pickup,
    pub derivation: Derivation,
    /// The responsible role, when the obligation's party is known: the
    /// orchestrator for a step of its own, or the `role` the docs index
    /// records for the procedure its command names.
    pub responsible_role: Option<RoleName>,
}

/// `instrukt todo prepare|work` could not answer.
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
    /// Deliberate pickup records the claim for the calling session.
    #[error(
        "picking up `{todo}` records a claim for the calling session, and no configured runtime's session variable is set; run it from an agent session"
    )]
    NoSession { todo: TodoSlug },
    #[error("`{todo}` is claimed elsewhere; leave it to the holder")]
    ClaimedElsewhere { todo: TodoSlug },
    #[error("cannot load the workflow flavor: {0}")]
    Flavor(#[from] FlavorError),
    #[error("cannot evaluate `{todo}`: {message}")]
    Evaluation { todo: TodoSlug, message: String },
    #[error(transparent)]
    Work(#[from] WorkInterfaceError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Observation(#[from] ObservationError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error("the runtime session `{value}` is not a session reference: {message}")]
    Session { value: String, message: String },
    #[error("the fact provider supplied an invalid fact: {0}")]
    Fact(#[from] FactError),
}

/// Workflow over work's record. Constructed by the CLI with work's
/// evidence and the invocation's environment (for configuration
/// placeholders and the calling session).
pub struct Workflow {
    home: Home,
    evidence: Arc<dyn WorkEvidence>,
    env: Env,
}

/// One todo's derivation inputs, read for one call.
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

    /// `instrukt todo prepare|work <todo>` from `cwd`: when the todo is
    /// active, records the calling session's claim through `claims` (a
    /// claim held elsewhere refuses), then evaluates `flow` on current
    /// facts. A todo that is not active is read only.
    pub fn next(
        &self,
        cwd: &Path,
        flow: Flow,
        todo: &TodoSlug,
        claims: &dyn Claims,
    ) -> Result<Next, WorkflowError> {
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
        // The claim changes `claim.held`: derive from the record as it is
        // now.
        let evidence = match pickup {
            Pickup::ReadOnly(_) => evidence,
            _ => self.todo_evidence(&reading.project, todo)?,
        };
        let facts = self.facts(&reading, &evidence)?;
        let derivation = self.evaluate(&reading, flow, &evidence, facts)?;
        let index = GlobalIndex::load(&self.home)?;
        let responsible_role =
            derivation
                .obligation
                .as_ref()
                .and_then(|obligation| match &obligation.responsible {
                    Responsible::Orchestrator(role) | Responsible::Role(role) => Some(role.clone()),
                    Responsible::Procedure(procedure) => role_in(index.as_ref(), procedure),
                });
        tracing::debug!(
            event = "workflow_next",
            project = %reading.project.slug,
            todo = %todo,
            flow = flow.as_str(),
            pickup = ?pickup,
            state = %derivation.state,
            action = derivation.obligation.as_ref().map(|o| o.action.as_str()),
            duration_ms = started.elapsed().as_millis() as u64,
        );
        Ok(Next {
            project: reading.project.slug.clone(),
            todo: todo.clone(),
            pickup,
            derivation,
            responsible_role,
        })
    }

    /// The configuration, flavor and project of the repository containing
    /// `cwd`, addressed by its primary worktree.
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

    /// The calling session: the active runtime and its session variable,
    /// as an attributed commit resolves it.
    fn session(
        &self,
        config: &Configuration,
        todo: &TodoSlug,
    ) -> Result<SessionRef, WorkflowError> {
        config
            .caller(&self.env)?
            .ok_or_else(|| WorkflowError::NoSession { todo: todo.clone() })
    }

    /// The facts for `evidence` from the record and the repository.
    fn facts(&self, reading: &Reading, evidence: &TodoEvidence) -> Result<Facts, WorkflowError> {
        let others = related_evidence(self.evidence.as_ref(), &reading.project, evidence)?;
        facts_with(&self.home, reading, evidence, &others)
    }

    fn evaluate(
        &self,
        reading: &Reading,
        flow: Flow,
        evidence: &TodoEvidence,
        facts: Facts,
    ) -> Result<Derivation, WorkflowError> {
        evaluate_with(&self.env, reading, flow, evidence, facts)
    }

    /// The lane `evidence`'s todo is owed, with the derivation of that
    /// lane's flow; `None` when the todo is not active.
    fn lane(
        &self,
        reading: &Reading,
        evidence: &TodoEvidence,
        index: Option<&GlobalIndex>,
    ) -> Result<Option<(Lane, Derivation)>, WorkflowError> {
        if evidence.status != TodoStatus::Active {
            return Ok(None);
        }
        let facts = self.facts(reading, evidence)?;
        let Some((lane, work, prepare)) = lane_with(&self.env, reading, evidence, facts, index)?
        else {
            return Err(WorkflowError::Evaluation {
                todo: evidence.slug.clone(),
                message: "the flavor defines no `prepare` flow".to_owned(),
            });
        };
        Ok(Some(match lane {
            Lane::Prepare => (lane, prepare),
            Lane::Work | Lane::TaskRun => (lane, work),
        }))
    }

    /// Whether the prerequisites of `evidence` admit `lane`
    /// (`dependencies-and-holds`, "Prerequisites"): execution needs every
    /// prerequisite delivered; preparation needs a repository-work
    /// prerequisite prepared or delivered, and a task prerequisite
    /// delivered.
    fn admitted(
        &self,
        reading: &Reading,
        evidence: &TodoEvidence,
        lane: &Lane,
        index: Option<&GlobalIndex>,
    ) -> Result<bool, WorkflowError> {
        for prerequisite in &evidence.prerequisites {
            let crate::evidence::Prerequisite::Todo(slug) = prerequisite else {
                return Ok(false);
            };
            let Some(other) = self.evidence.evidence(&reading.project, slug)? else {
                return Ok(false);
            };
            if other.status == TodoStatus::Delivered {
                continue;
            }
            let prepared = *lane == Lane::Prepare
                && other.kind != TodoKind::Task
                && self
                    .lane(reading, &other, index)?
                    .is_some_and(|(other_lane, _)| other_lane != Lane::Prepare);
            if !prepared {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn assign(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<Assignment>, WorkflowError> {
        let evidence = self.todo_evidence(project, todo)?;
        if evidence.status != TodoStatus::Active {
            return Ok(None);
        }
        let config = Configuration::load(&self.home, Some(&project.root))?;
        let reading = self.reading(config, project.clone())?;
        let index = GlobalIndex::load(&self.home)?;
        let Some((lane, derivation)) = self.lane(&reading, &evidence, index.as_ref())? else {
            return Ok(None);
        };
        if !self.admitted(&reading, &evidence, &lane, index.as_ref())? {
            return Ok(None);
        }
        let orchestrator = match lane {
            Lane::Prepare => reading.flavor.orchestrator(Flow::Prepare.as_str()).cloned(),
            Lane::Work => reading.flavor.orchestrator(Flow::Work.as_str()).cloned(),
            Lane::TaskRun => None,
        };
        let obligation = match derivation.obligation {
            Some(obligation) => Some(assigned(todo, obligation)?),
            None => None,
        };
        Ok(Some(Assignment {
            lane,
            orchestrator,
            state: derivation.state.to_string(),
            obligation,
        }))
    }
}

impl WorkflowAssignments for Workflow {
    fn assignment(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<Assignment>, WorkflowInterfaceError> {
        let started = Instant::now();
        let assignment = self.assign(project, todo).map_err(|err| match err {
            WorkflowError::Work(err) => WorkflowInterfaceError::Work(err),
            WorkflowError::Flavor(err) => WorkflowInterfaceError::Flavor {
                message: err.to_string(),
            },
            WorkflowError::Evaluation { todo, message } => {
                WorkflowInterfaceError::Evaluation { todo, message }
            }
            other => WorkflowInterfaceError::Evaluation {
                todo: todo.clone(),
                message: other.to_string(),
            },
        })?;
        tracing::debug!(
            event = "workflow_assignment",
            project = %project.slug,
            todo = %todo,
            lane = ?assignment.as_ref().map(|a| &a.lane),
            state = assignment.as_ref().map(|a| a.state.as_str()),
            duration_ms = started.elapsed().as_millis() as u64,
        );
        Ok(assignment)
    }
}

/// The related todos' evidence a derivation for `evidence` reads besides
/// its own, read through `work`: each prerequisite by slug, and each todo
/// its artifacts' current grounding recorded a dependency on by durable
/// identity (a slug may since name another todo). Each todo once.
pub fn related_evidence(
    work: &dyn WorkEvidence,
    project: &ProjectRef,
    evidence: &TodoEvidence,
) -> Result<Vec<TodoEvidence>, WorkInterfaceError> {
    let mut others: Vec<TodoEvidence> = Vec::new();
    let mut add = |other: Option<TodoEvidence>| {
        if let Some(other) = other
            && !others.iter().any(|known| known.id == other.id)
        {
            others.push(other);
        }
    };
    for prerequisite in &evidence.prerequisites {
        if let crate::evidence::Prerequisite::Todo(slug) = prerequisite {
            add(work.evidence(project, slug)?);
        }
    }
    // Every grounding a derivation compares: each artifact's current
    // version, and each drift judgment's assessed evidence, whose
    // dependencies may no longer be prerequisites at all.
    let versions = evidence.artifacts.iter().flat_map(|artifact| {
        artifact
            .versions
            .last()
            .into_iter()
            .flat_map(|version| version.grounding.dependencies.iter())
    });
    let judged = evidence
        .stamps
        .iter()
        .filter_map(|stamp| match &stamp.fact {
            crate::evidence::StampFact::DriftReviewed { grounding, .. } => {
                Some(grounding.dependencies.iter())
            }
            _ => None,
        })
        .flatten();
    let mut ids: Vec<&crate::work::TodoId> = Vec::new();
    for id in versions.chain(judged).filter_map(|d| d.id.as_ref()) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    for id in ids {
        add(work.evidence_by_id(project, id)?);
    }
    Ok(others)
}

/// Whether `evidence`'s todo has completed preparation, as the current
/// derivation of the flavor its project selects says: preparation is
/// complete when the todo's lane is no longer preparation (the work lane,
/// or for a task its run). `project_root` is the project's primary
/// worktree; `others` is the evidence of the todos [`related_evidence`]
/// reads for it.
///
/// `Ok(None)` when no answer can be derived: the project selects no
/// published flavor, or the flavor defines no `prepare` or `work` flow. Reads the
/// home, configuration and repository and records nothing, so work can
/// ask before it opens its write session.
pub fn preparation_complete(
    home: &Home,
    project_root: &Path,
    evidence: &TodoEvidence,
    others: &[TodoEvidence],
) -> Result<Option<bool>, WorkflowError> {
    let config = Configuration::load(home, Some(project_root))?;
    let Some(slug) = config.project()? else {
        return Ok(None);
    };
    let dir = published_workflow(home, &config)?;
    if !dir.is_dir() {
        return Ok(None);
    }
    let reading = Reading {
        flavor: Flavor::load(&dir)?,
        config,
        project: ProjectRef {
            slug,
            root: project_root.to_path_buf(),
        },
    };
    let facts = facts_with(home, &reading, evidence, others)?;
    let index = GlobalIndex::load(home)?;
    let env = Env::default();
    Ok(lane_with(&env, &reading, evidence, facts, index.as_ref())?
        .map(|(lane, _, _)| lane != Lane::Prepare))
}

/// The facts for `evidence` from the record, `others` (the related todos'
/// evidence) and the repository.
fn facts_with(
    home: &Home,
    reading: &Reading,
    evidence: &TodoEvidence,
    others: &[TodoEvidence],
) -> Result<Facts, WorkflowError> {
    let prerequisites = standings(evidence, |slug| {
        Ok::<_, WorkflowError>(others.iter().find(|other| &other.slug == slug).cloned())
    })?;
    let observed = observe(home, &reading.project.root, evidence, others)?;
    let approvers = Approvers {
        prd: reading.config.approver(ConfigGate::Prd)?,
        task: reading.config.approver(ConfigGate::Task)?,
    };
    Ok(derive_facts(&ProviderInput {
        evidence,
        prerequisites: &prerequisites,
        approvers,
        observed: &observed,
    })?)
}

fn evaluate_with(
    env: &Env,
    reading: &Reading,
    flow: Flow,
    evidence: &TodoEvidence,
    facts: Facts,
) -> Result<Derivation, WorkflowError> {
    let subject = Subject {
        project: reading.project.slug.clone(),
        slug: evidence.slug.clone(),
        facts,
    };
    let lookup = ConfigPaths {
        config: &reading.config,
        env,
    };
    reading
        .flavor
        .evaluate(flow.as_str(), &subject, &lookup)
        .map_err(|err| WorkflowError::Evaluation {
            todo: evidence.slug.clone(),
            message: err.to_string(),
        })
}

/// The lane for `facts`, with the work and prepare derivations; `None`
/// when the flavor defines no `prepare` flow.
fn lane_with(
    env: &Env,
    reading: &Reading,
    evidence: &TodoEvidence,
    facts: Facts,
    index: Option<&GlobalIndex>,
) -> Result<Option<(Lane, Derivation, Derivation)>, WorkflowError> {
    let subject = Subject {
        project: reading.project.slug.clone(),
        slug: evidence.slug.clone(),
        facts,
    };
    let lookup = ConfigPaths {
        config: &reading.config,
        env,
    };
    let role_of = |procedure: &SnippetId| role_in(index, procedure);
    derive_lane(&reading.flavor, evidence.kind, &subject, &lookup, &role_of).map_err(|err| {
        WorkflowError::Evaluation {
            todo: evidence.slug.clone(),
            message: err.to_string(),
        }
    })
}

/// The rendered obligation the runner receives; a command, brief or
/// effect that cannot be rendered is not executable.
fn assigned(
    todo: &TodoSlug,
    obligation: super::_derive::Obligation,
) -> Result<AssignedObligation, WorkflowError> {
    let unrendered =
        |what: &str, failure: super::_render::RenderFailure| WorkflowError::Evaluation {
            todo: todo.clone(),
            message: format!(
                "the {what} of `{}` cannot be rendered: {failure}",
                obligation.action
            ),
        };
    let command = obligation
        .command
        .clone()
        .map_err(|failure| unrendered("command", failure))?;
    let brief = obligation
        .brief
        .clone()
        .map_err(|failure| unrendered("instructions", failure))?;
    let mut effects = Vec::with_capacity(obligation.effects.len());
    for effect in &obligation.effects {
        effects.push(AssignedEffect {
            name: effect.name.to_string(),
            command: effect
                .command
                .clone()
                .map_err(|failure| unrendered("effect command", failure))?,
        });
    }
    let dispatch_instruction = obligation
        .dispatch_instruction
        .map(|value| value.map_err(|failure| unrendered("native dispatch", failure)))
        .transpose()?;
    Ok(AssignedObligation {
        dispatch_instruction,
        action: obligation.action.to_string(),
        responsible: match obligation.responsible {
            Responsible::Procedure(procedure) => Responsibility::Procedure(procedure),
            Responsible::Orchestrator(role) => Responsibility::Orchestrator(role),
            Responsible::Role(role) => Responsibility::Role(role),
        },
        command,
        brief,
        effects,
    })
}

/// The `role` the docs index records for `procedure`.
fn role_in(index: Option<&GlobalIndex>, procedure: &SnippetId) -> Option<RoleName> {
    index?
        .snippets
        .iter()
        .find(|entry| &entry.id == procedure)
        .and_then(|entry| entry.role.as_deref())
        .and_then(|role| RoleName::parse(role).ok())
}

/// The selected flavor's published `workflow/` directory.
fn published_workflow(home: &Home, config: &Configuration) -> Result<PathBuf, ConfigError> {
    Ok(home.domain_dir(&config.workflow_domain()?).join("workflow"))
}

/// The primary worktree of the repository whose worktree is `root`: the
/// project's checkout, also when called from a todo worktree.
fn primary_worktree(root: &Path) -> Result<PathBuf, GitError> {
    let listing = git::run(root, ["worktree", "list", "--porcelain"])?;
    Ok(listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map_or_else(|| root.to_path_buf(), PathBuf::from))
}

/// Configuration paths in placeholders, resolved as `instrukt config get`
/// resolves them.
struct ConfigPaths<'a> {
    config: &'a Configuration,
    env: &'a Env,
}

impl ConfigLookup for ConfigPaths<'_> {
    fn lookup(&self, path: &str) -> Result<Option<String>, ConfigLookupError> {
        match self.config.get_str(path, self.env) {
            Ok(value) => Ok(Some(value)),
            Err(ConfigError::MissingKey { .. }) => Ok(None),
            Err(err) => Err(ConfigLookupError(err.to_string())),
        }
    }
}

/// Core's flavor-role lookup for docs validation: the roles of the flavor
/// the project's configuration selects. A project that authors that flavor
/// (`docs/domain/<domain>/workflow/`, or `docs/general/workflow/`) is validated against its own
/// authored roles, so a commit that changes them can be published; any
/// other project reads the published flavor.
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
    fn declared_roles(&self, project_root: &Path) -> Result<Vec<RoleName>, InterfaceError> {
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
            .map(|flavor| flavor.roles().to_vec())
            .map_err(|err| failed(err.to_string()))
    }
}

/// Whether `modules.todos` is on for a caller in `cwd`: the project layer
/// of the repository containing it over the global layer.
pub fn todos_enabled(home: &Home, cwd: &Path) -> Result<bool, WorkflowError> {
    let root = git::worktree_root(cwd)?;
    Ok(config::module_enabled(
        home,
        root.as_deref(),
        Module::Todos,
    )?)
}

#[cfg(test)]
mod tests {
    use instrukt_core::config::CURRENT_SCHEMA_VERSION;
    use instrukt_runner::interfaces::TodoKind;

    use super::super::_testing::todo;
    use super::*;

    #[test]
    fn without_a_published_flavor_preparation_completion_has_no_answer() {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(home_dir.path().to_path_buf());
        let project = tempfile::tempdir().unwrap();
        std::fs::write(
            project.path().join("instrukt.yaml"),
            format!("schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\n"),
        )
        .unwrap();
        let evidence = todo(TodoKind::Change).build();
        let answer = preparation_complete(&home, project.path(), &evidence, &[]).unwrap();
        assert_eq!(answer, None);
    }
}
