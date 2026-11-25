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
    /// GitHub round-trip for the subject todo during `instrukt todo show`
    /// while still loading related evidence and prerequisite standings identically
    /// to `todo work`.
    pub fn ledger_for_view(
        &self,
        cwd: &Path,
        view: &crate::work::TodoView,
    ) -> Result<TodoLedger, WorkflowError> {
        let reading = self.read_project_for_ledger(cwd)?;
        let evidence = crate::work::_ports::to_evidence(
            view.record.clone(),
            view.expanded_prerequisites.clone(),
        );
        self.build_ledger(&reading.config, &reading.project, &evidence)
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
        let approvers =
            approvers_of(&reading.config).map_err(|err| WorkflowInterfaceError::Flavor {
                message: err.to_string(),
            })?;
        let Some(published) = admitted(&evidence, approvers, || {
            let prereqs = standings(&evidence.prerequisites, |slug| {
                self.evidence.evidence(project, slug)
            })?;
            Ok::<_, WorkflowInterfaceError>(
                prereqs
                    .iter()
                    .all(|(_, standing)| *standing == PrerequisiteStanding::Delivered),
            )
        })?
        else {
            return Ok(None);
        };
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
                holder: published.holder,
                waits_on_person: published.waits_on_person,
            }));
        }
        Ok(Some(Assignment {
            lane: Lane::Work,
            orchestrator: Some(reading.flavor.orchestrator().clone()),
            state,
            holder: published.holder,
            waits_on_person: published.waits_on_person,
        }))
    }
}

/// What an assignment publishes about who drives the todo: the session
/// holding its execution claim, and whether it waits on a person.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Published {
    holder: Option<SessionRef>,
    waits_on_person: bool,
}

/// The assignment gates that need no repository observation, and the
/// values an admitted todo publishes; `None` when a gate leaves the todo
/// without an assignment. `prerequisites_delivered` reads the
/// prerequisites' evidence and runs only for an active todo the hold gate
/// lets through.
fn admitted<E>(
    evidence: &TodoEvidence,
    approvers: Approvers,
    prerequisites_delivered: impl FnOnce() -> Result<bool, E>,
) -> Result<Option<Published>, E> {
    let holder = evidence
        .claim
        .as_ref()
        .and_then(|claim| claim.session.clone());
    // A hold pauses pickup and ends no running work: a session that holds
    // the claim keeps its assignment through the hold.
    if evidence.status != TodoStatus::Active || (evidence.hold.is_some() && holder.is_none()) {
        return Ok(None);
    }
    if !prerequisites_delivered()? {
        return Ok(None);
    }
    // Human-owned approval gate: when unclaimed and waiting on the human,
    // do not re-dispatch until the approval arrives.
    if evidence.claim.is_none()
        && waiting_for_human_approval(evidence, approvers.prd, approvers.task)
    {
        return Ok(None);
    }
    Ok(Some(Published {
        holder,
        waits_on_person: crate::work::waits_on_person(evidence, approvers.prd, approvers.task),
    }))
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
    // Also load dependencies captured in drift-review stamps' assessed
    // grounding, whose dependencies may no longer be current prerequisites.
    for stamp in &evidence.stamps {
        if let crate::evidence::StampFact::DriftReviewed { grounding, .. } = &stamp.fact {
            for dep in &grounding.dependencies {
                if let Some(id) = &dep.id
                    && !ids.contains(id)
                    && let Some(other) = store.evidence_by_id(project, id)?
                    && ids.insert(other.id.clone())
                {
                    out.push(other);
                }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use instrukt_core::ids::{CommitSha, RoleName};
    use instrukt_runner::interfaces::TodoKind;

    use super::super::_testing::{digest, session, todo};
    use super::*;
    use crate::evidence::{
        Actor, ApprovalGate, ApprovedBy, ArtifactName, DependencyDigest, Grounding,
        PreparationReview, ReviewVerdict, StampFact, StampRecord, artifact_names,
    };

    struct MemStore(BTreeMap<crate::work::TodoId, TodoEvidence>);

    impl WorkEvidence for MemStore {
        fn evidence(
            &self,
            _project: &ProjectRef,
            slug: &TodoSlug,
        ) -> Result<Option<TodoEvidence>, WorkInterfaceError> {
            Ok(self.0.values().find(|e| &e.slug == slug).cloned())
        }

        fn evidence_by_id(
            &self,
            _project: &ProjectRef,
            id: &crate::work::TodoId,
        ) -> Result<Option<TodoEvidence>, WorkInterfaceError> {
            Ok(self.0.get(id).cloned())
        }
    }

    fn stamp(fact: StampFact, seq: u64) -> StampRecord {
        StampRecord {
            source_comment: None,
            fact,
            at: jiff::Timestamp::from_second(1_800_000_000 + seq as i64).unwrap(),
            actor: Actor::Session(session("reviewer")),
            model: None,
            eval_fingerprint: None,
            role: Some(RoleName::parse("prd-reviewer").unwrap()),
            human_involved: None,
            sequence: seq,
            model_provenance: Default::default(),
        }
    }

    #[test]
    fn related_evidence_includes_dependencies_from_drift_review_stamps() {
        let mut upstream = todo(TodoKind::Change).build();
        upstream.id = crate::work::TodoId::parse(&"22".repeat(16)).unwrap();
        upstream.slug = TodoSlug::parse("former-prereq").unwrap();

        let mut subject = todo(TodoKind::Feature).build();
        subject.stamps.push(stamp(
            StampFact::DriftReviewed {
                artifact: ArtifactName::new(artifact_names::PRD),
                examined: digest("prd-v1"),
                class: crate::evidence::DriftClass::Cosmetic,
                reason: "checked".into(),
                grounding: Grounding {
                    dependencies: vec![DependencyDigest {
                        id: Some(upstream.id.clone()),
                        todo: upstream.slug.clone(),
                        artifact: ArtifactName::new(artifact_names::INPUT),
                        digest: Some(digest("upstream-input")),
                    }],
                    ..Default::default()
                },
            },
            1,
        ));

        let mut map = BTreeMap::new();
        map.insert(upstream.id.clone(), upstream.clone());
        map.insert(subject.id.clone(), subject.clone());
        let store = MemStore(map);
        let project = ProjectRef {
            slug: ProjectSlug::parse("demo").unwrap(),
            root: PathBuf::from("/tmp/demo"),
        };

        let related = related_evidence(&store, &project, &subject).unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, upstream.id);
    }

    #[test]
    fn an_admitted_todo_names_its_claim_holder_and_whether_it_waits_on_a_person() {
        use instrukt_core::config::Approver;
        let approvers = |approver| Approvers {
            prd: approver,
            task: approver,
        };
        let admit = |evidence: &TodoEvidence, approver| {
            admitted(evidence, approvers(approver), || Ok::<_, ()>(true)).unwrap()
        };
        let holder = Some(session("orchestrator"));

        let at_prd_approval = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-v1")
            .artifact(artifact_names::PRD_REVIEW_FINDINGS, "ok")
            .stamped(StampFact::PreparationVerdict {
                phase: PreparationReview::PrdReview,
                verdict: ReviewVerdict::Approve,
                target: digest("prd-v1"),
                report: digest("ok"),
            })
            .claimed()
            .build();
        assert_eq!(
            admit(&at_prd_approval, Approver::Human),
            Some(Published {
                holder: holder.clone(),
                waits_on_person: true,
            })
        );
        assert_eq!(
            admit(&at_prd_approval, Approver::Peer),
            Some(Published {
                holder: holder.clone(),
                waits_on_person: false,
            })
        );

        let held_and_claimed = todo(TodoKind::Change).held().claimed().build();
        assert_eq!(
            admit(&held_and_claimed, Approver::Peer),
            Some(Published {
                holder,
                waits_on_person: true,
            })
        );

        let held_unclaimed = todo(TodoKind::Change).held().build();
        assert_eq!(admit(&held_unclaimed, Approver::Peer), None);
    }

    #[test]
    fn assignment_suppresses_human_approval_wait_and_finalized_handoff_until_kickback() {
        let dir = tempfile::tempdir().unwrap();
        let home_dir = dir.path().join("home");
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(home_dir.join(".instrukt/docs/sdlc/workflow")).unwrap();
        std::fs::write(
            home_dir.join(".instrukt/docs/sdlc/workflow/workflow.yaml"),
            "orchestrator: todo-orchestrator\nrequires: []\nroles: [todo-orchestrator]\n",
        )
        .unwrap();
        std::fs::write(
            home_dir.join(".instrukt/instrukt.yaml"),
            "schema_version: 1\nmodules:\n  todos: true\n",
        )
        .unwrap();
        std::fs::write(
            repo_dir.join("instrukt.yaml"),
            "schema_version: 1\nproject: demo\n",
        )
        .unwrap();
        git::run(&repo_dir, ["init", "-q"]).unwrap();
        git::run(&repo_dir, ["config", "user.email", "test@example.com"]).unwrap();
        git::run(&repo_dir, ["config", "user.name", "Test"]).unwrap();
        std::fs::write(repo_dir.join("README.md"), "hi\n").unwrap();
        git::run(&repo_dir, ["add", "README.md"]).unwrap();
        git::run(&repo_dir, ["commit", "-q", "-m", "init"]).unwrap();
        let head =
            CommitSha::parse(git::run(&repo_dir, ["rev-parse", "HEAD"]).unwrap().trim()).unwrap();
        git::run(&repo_dir, ["branch", "add-login/work"]).unwrap();

        let project = ProjectRef {
            slug: ProjectSlug::parse("demo").unwrap(),
            root: repo_dir.clone(),
        };
        let home = Home::at_user_home(home_dir);

        // 1. Unclaimed feature with approved prd_review waiting on human PRD approval -> suppressed.
        let mut waiting = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-v1")
            .artifact(artifact_names::PRD_REVIEW_FINDINGS, "ok")
            .build();
        waiting.stamps.push(stamp(
            StampFact::PreparationVerdict {
                phase: PreparationReview::PrdReview,
                verdict: ReviewVerdict::Approve,
                target: digest("prd-v1"),
                report: digest("ok"),
            },
            1,
        ));
        let mut map = BTreeMap::new();
        map.insert(waiting.id.clone(), waiting.clone());
        let wf = Workflow::new(home.clone(), Arc::new(MemStore(map)), Env::default());
        assert!(wf.assignment(&project, &waiting.slug).unwrap().is_none());

        // Once human approves PRD, runner dispatches todo-orchestrator again.
        waiting.stamps.push(stamp(
            StampFact::Approval {
                gate: ApprovalGate::Prd,
                target: digest("prd-v1"),
                by: ApprovedBy::Human,
                carried_from: None,
                reasoning: None,
            },
            2,
        ));
        let mut map = BTreeMap::new();
        map.insert(waiting.id.clone(), waiting.clone());
        let wf = Workflow::new(home.clone(), Arc::new(MemStore(map)), Env::default());
        let assigned = wf.assignment(&project, &waiting.slug).unwrap().unwrap();
        assert_eq!(assigned.lane, Lane::Work);

        // 2. FinalizeReady at current head -> suppressed while unclaimed awaiting integrate.
        waiting.branch = Some("add-login/work".into());
        waiting.stamps.push(stamp(
            StampFact::FinalizeReady {
                commit: head.clone(),
            },
            3,
        ));
        let mut map = BTreeMap::new();
        map.insert(waiting.id.clone(), waiting.clone());
        let wf = Workflow::new(home.clone(), Arc::new(MemStore(map)), Env::default());
        assert!(wf.assignment(&project, &waiting.slug).unwrap().is_none());

        // 3. Kickback at same head resets handoff -> runner wakes todo-orchestrator again!
        waiting.stamps.push(stamp(
            StampFact::Kickback {
                reason: "CI red".into(),
                input: digest("input"),
                invalidated_plan: None,
                invalidated_reviews: Vec::new(),
            },
            4,
        ));
        let mut map = BTreeMap::new();
        map.insert(waiting.id.clone(), waiting.clone());
        let wf = Workflow::new(home, Arc::new(MemStore(map)), Env::default());
        let woken = wf.assignment(&project, &waiting.slug).unwrap().unwrap();
        assert_eq!(woken.lane, Lane::Work);
    }
}
