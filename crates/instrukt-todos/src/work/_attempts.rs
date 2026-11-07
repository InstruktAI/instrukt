//! Record native shared assignments and release them with their completion evidence.
use std::path::{Path, PathBuf};

use instrukt_core::{
    git,
    ids::{CommitSha, RoleName, SessionRef, TodoSlug},
};
use instrukt_runner::{
    interfaces::{AssignmentKind, ProjectRef},
    keyed,
};
use serde::{Deserialize, Serialize};

use super::_session::Session;
use super::{Attempt, Binding, Outcome, TodoRecord, Work, WorkError};
use crate::evidence::{PreparationReview, TodoStatus, artifact_names};

fn head(root: &Path, revision: &str) -> Result<CommitSha, WorkError> {
    CommitSha::parse(
        git::run(
            root,
            ["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
        )?
        .trim(),
    )
    .map_err(|e| WorkError::refused(e.to_string()))
}
/// Fetch before a new phase is placed. A retained remote work branch is continuation
/// of the existing phase; otherwise only freshly fetched canonical main supplies its base.
pub(super) fn phase_base(
    root: &Path,
    branch: &str,
    preparation: bool,
) -> Result<(String, CommitSha), WorkError> {
    git::run(root, ["fetch", "--prune", "origin"])?;
    let main = instrukt_core::github::default_branch(root)?;
    let remote = format!("origin/{branch}");
    let from = if !preparation
        && git::succeeds(
            root,
            [
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{remote}"),
            ],
        )? {
        remote
    } else {
        format!("origin/{main}")
    };
    let base = head(root, &from)?;
    Ok((from, base))
}
fn clean(root: &Path) -> Result<(), WorkError> {
    if !git::run(root, ["status", "--porcelain=v1"])?.is_empty() {
        return Err(WorkError::refused(
            "shared workspace has uncommitted changes; recover the outstanding attempt in place",
        ));
    }
    for name in [
        "MERGE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        let metadata = root.join(git::run(root, ["rev-parse", "--git-path", name])?);
        if metadata.try_exists().map_err(|source| WorkError::Io {
            path: metadata.clone(),
            source,
        })? {
            return Err(WorkError::refused(
                "shared workspace has an unfinished Git operation",
            ));
        }
    }
    Ok(())
}
fn canonical(root: &Path) -> Result<PathBuf, WorkError> {
    root.canonicalize().map_err(|source| WorkError::Io {
        path: root.into(),
        source,
    })
}
fn orchestrator(s: &Session<'_>, index: usize) -> Result<(), WorkError> {
    let caller = s.caller_session()?.ok_or_else(|| {
        WorkError::refused("this operation requires the claim-holding orchestrator session")
    })?;
    let machine = s.machine()?;
    let record = &s.items[index].record;
    if !record
        .claim
        .as_ref()
        .is_some_and(|c| c.machine == machine && c.session.as_ref() == Some(&caller))
    {
        return Err(WorkError::refused(
            "claimed elsewhere; only the claim-holding orchestrator may reserve or bind a role",
        ));
    }
    if record.status != TodoStatus::Active || record.hold.is_some() {
        return Err(WorkError::refused(
            "native assignment requires active, unheld work",
        ));
    }
    Ok(())
}
fn prepared(record: &TodoRecord) -> Result<(), WorkError> {
    let target = record
        .current_digest(artifact_names::IMPLEMENTATION_PLAN)
        .ok_or_else(|| WorkError::refused("current approved preparation is required"))?;
    super::_stamps::supporting_review(record, PreparationReview::PlanReview, target)
        .map_err(WorkError::refused)?;
    Ok(())
}
pub(super) fn preparation_integrated(record: &TodoRecord, root: &Path) -> Result<(), WorkError> {
    let retained = record
        .bindings
        .iter()
        .rev()
        .find(|b| b.attempt.lane == Some(instrukt_runner::interfaces::Lane::Prepare))
        .and_then(|b| b.attempt.branch.as_deref());
    let derived = format!("{}/prepare", record.slug);
    let branch = retained.unwrap_or(&derived);
    let local = git::succeeds(
        root,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    let remote = git::succeeds(
        root,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )?;
    let source = if local {
        Some(head(root, branch)?)
    } else {
        record.stamps.iter().rev().find_map(|s| match &s.fact {
            crate::evidence::StampFact::PreparationHeadReviewed { head, .. } => Some(head.clone()),
            _ => None,
        })
    };
    if let Some(source) = source {
        let main = instrukt_core::github::default_branch(root)?;
        let remote_main = format!("refs/remotes/origin/{main}");
        let reference = if git::succeeds(root, ["show-ref", "--verify", "--quiet", &remote_main])? {
            remote_main
        } else {
            main
        };
        if !git::succeeds(
            root,
            ["merge-base", "--is-ancestor", source.as_str(), &reference],
        )? {
            return Err(WorkError::refused(
                "preparation documentation awaits integration to main; no build workspace is admitted",
            ));
        }
    }
    if local || remote {
        return Err(WorkError::refused(format!(
            "preparation cleanup remains owed for `{branch}`; the integrator must remove its worktree and local/remote branch before work starts"
        )));
    }
    Ok(())
}
fn native_selection(
    s: &Session<'_>,
    record: &TodoRecord,
    procedure: Option<&str>,
) -> Result<Option<instrukt_core::config::AgentSelection>, WorkError> {
    let dispatch = procedure
        .map(|id| id.parse::<instrukt_core::ids::SnippetId>())
        .transpose()
        .map_err(|e| WorkError::refused(e.to_string()))?
        .as_ref()
        .map(|id| instrukt_runner::launch::procedure(s.home(), id))
        .transpose()
        .map_err(|e| WorkError::refused(e.to_string()))?;
    let (global, tier) = instrukt_runner::agents::global_order(&s.config);
    let order = instrukt_runner::agents::agent_order(
        record.agent.as_ref(),
        dispatch.as_ref().and_then(|p| p.dispatch.as_ref()),
        &global,
        tier,
    );
    let selection = instrukt_runner::agents::selections(&s.config, s.home(), &order, s.now)
        .map_err(|e| WorkError::refused(e.to_string()))?
        .into_iter()
        .next();
    Ok(selection)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeAssignment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<instrukt_core::config::AgentSelection>,
    pub runtime: instrukt_core::config::Runtime,
    pub dispatch_allowed: bool,
    pub brief: String,
    pub procedure: Option<String>,
    pub args: Option<String>,
    pub role: RoleName,
    pub attempt: u64,
    pub semantic_key: String,
    pub title: String,
    pub worktree: PathBuf,
    pub branch: String,
    pub base: CommitSha,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoppedChild {
    pub session: SessionRef,
    pub stopped: bool,
    pub source: String,
    pub worktree: PathBuf,
    pub branch: String,
    pub head: CommitSha,
}

pub struct AttemptRequest<'a> {
    pub todo: &'a TodoSlug,
    pub role: &'a RoleName,
    pub procedure: Option<&'a str>,
    pub args: Option<&'a str>,
    pub brief: &'a str,
}
pub struct NativeBinding<'a> {
    pub todo: &'a TodoSlug,
    pub role: &'a RoleName,
    pub attempt: u64,
    pub session: &'a SessionRef,
    pub worktree: Option<&'a Path>,
    pub branch: &'a str,
    pub base: &'a CommitSha,
}
pub struct AttemptResolution<'a> {
    pub todo: &'a TodoSlug,
    pub role: &'a RoleName,
    pub attempt: u64,
    pub reason: &'a str,
    pub evidence: Option<&'a StoppedChild>,
}
/// Supplied native recorded-workspace evidence for deliberate orchestrator entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverPlacement {
    pub session: SessionRef,
    pub key: String,
    pub worktree: PathBuf,
    pub branch: String,
    pub head: CommitSha,
    pub source: String,
}
pub struct WorkspaceRequest<'a> {
    pub todo: &'a TodoSlug,
    pub role: &'a RoleName,
    pub cwd: &'a Path,
    pub preparation: bool,
    pub evidence: Option<&'a DriverPlacement>,
}

impl Work {
    /// Reserve immutable native dispatch inputs without runtime I/O.
    pub fn prepare_attempt(
        &self,
        project: &ProjectRef,
        request: &AttemptRequest<'_>,
    ) -> Result<NativeAssignment, WorkError> {
        let AttemptRequest {
            todo,
            role,
            procedure,
            args,
            brief,
        } = *request;
        if brief.trim().is_empty() {
            return Err(WorkError::refused("an assignment needs a nonempty brief"));
        }
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        orchestrator(&s, index)?;
        let runtime = s
            .config
            .runner_runtime()?
            .ok_or_else(|| WorkError::refused("no runtime selected"))?;
        let record = &s.items[index].record;
        if let Some(active) = record.bindings.iter().find(|b| {
            matches!(
                b.attempt.assignment,
                AssignmentKind::Child | AssignmentKind::Workspace
            ) && b.attempt.released_at.is_none()
        }) {
            let assignment = active.attempt.native_assignment.as_ref().ok_or_else(|| {
                WorkError::refused("outstanding native assignment has no retained inputs")
            })?;
            if &active.role != role
                || assignment.procedure.as_deref() != procedure
                || assignment.args.as_deref() != args
                || assignment.brief != brief
            {
                return Err(WorkError::refused(
                    "an outstanding role attempt blocks changed inputs or another workspace role",
                ));
            }
            if assignment.runtime != runtime {
                return Err(WorkError::refused(
                    "outstanding reservation belongs to a different selected runtime",
                ));
            }
            let mut result = assignment.clone();
            result.dispatch_allowed = active.attempt.no_start_reason.is_some();
            if result.dispatch_allowed {
                result.selection = native_selection(&s, record, procedure)?;
                let key = result.semantic_key.clone();
                s.commit("native dispatch retry admitted", |s| {
                    if let Some(b) = s.items[index]
                        .record
                        .bindings
                        .iter_mut()
                        .find(|b| b.attempt.semantic_key == key)
                    {
                        b.attempt.no_start_reason = None;
                        b.attempt.native_assignment = Some(result.clone());
                    }
                    Ok(())
                })?;
                s.flush()?;
            }
            return Ok(result);
        }
        let driver = record
            .bindings
            .iter()
            .rev()
            .find(|b| {
                b.attempt.assignment == AssignmentKind::Driver
                    && b.attempt.released_at.is_none()
                    && b.session == record.claim.as_ref().and_then(|c| c.session.clone())
            })
            .ok_or_else(|| {
                WorkError::refused("claim-holding orchestrator has no recorded phase workspace")
            })?;
        let lane = driver
            .attempt
            .lane
            .ok_or_else(|| WorkError::refused("driver phase is unknown"))?;
        let work = lane == instrukt_runner::interfaces::Lane::Work;
        if work {
            prepared(record)?;
            preparation_integrated(record, &project.root)?;
        }
        if role.as_str() == "setup" && !work {
            return Err(WorkError::refused("setup belongs to the work phase"));
        }
        let path = driver
            .worktree
            .clone()
            .ok_or_else(|| WorkError::refused("phase workspace missing"))?;
        let branch = driver
            .attempt
            .branch
            .clone()
            .ok_or_else(|| WorkError::refused("phase branch missing"))?;
        if matches!(role.as_str(), "builder" | "fixer")
            && (!record
                .stamps
                .iter()
                .any(|s| matches!(s.fact, crate::evidence::StampFact::BuildStarted { .. }))
                || record
                    .current_digest(artifact_names::QUALITY_CHECKLIST)
                    .is_none())
        {
            return Err(WorkError::refused(
                "builder admission needs build-start and quality-checklist",
            ));
        }
        clean(&path)?;
        if git::run(&path, ["branch", "--show-current"])? != branch {
            return Err(WorkError::refused(
                "shared workspace is not on the recorded branch",
            ));
        }
        let number = record
            .bindings
            .iter()
            .filter(|b| &b.role == role)
            .map(|b| b.attempt.number)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| WorkError::refused("role attempt counter exhausted"))?;
        let key = keyed::todo_key(
            project.slug.as_str(),
            record.id.as_str(),
            role.as_str(),
            number,
        );
        let selection = native_selection(&s, record, procedure)?;
        let assignment = NativeAssignment {
            selection,
            runtime,
            dispatch_allowed: true,
            brief: brief.into(),
            procedure: procedure.map(str::to_owned),
            args: args.map(str::to_owned),
            role: role.clone(),
            attempt: number,
            semantic_key: key.clone(),
            title: key.clone(),
            worktree: canonical(&path)?,
            branch: branch.clone(),
            base: head(&path, "HEAD")?,
        };
        let binding = Binding {
            role: role.clone(),
            session: None,
            worktree: Some(assignment.worktree.clone()),
            attempt: Attempt {
                assignment: AssignmentKind::Workspace,
                lane: Some(lane),
                number,
                semantic_key: key,
                branch: Some(branch),
                base_head: Some(assignment.base.clone()),
                runtime: Some(runtime),
                native_assignment: Some(assignment.clone()),
                ..Default::default()
            },
        };
        s.commit("native role reserved", |s| {
            s.items[index].record.bindings.push(binding);
            Ok(())
        })?;
        s.flush()?;
        Ok(assignment)
    }
    /// Bind the native result only after verifying the reserved shared placement.
    pub fn bind_attempt(
        &self,
        project: &ProjectRef,
        request: &NativeBinding<'_>,
    ) -> Result<(), WorkError> {
        let NativeBinding {
            todo,
            role,
            attempt,
            session,
            worktree,
            branch,
            base,
        } = *request;
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        orchestrator(&s, index)?;
        if s.caller_session()?.as_ref() == Some(session) {
            return Err(WorkError::refused(
                "native child identity must differ from its coordinating parent",
            ));
        }
        let runtime = s
            .config
            .runner_runtime()?
            .ok_or_else(|| WorkError::refused("no runtime selected"))?;
        if session.runtime.as_str() != runtime.name() {
            return Err(WorkError::refused(
                "child identity belongs to another runtime",
            ));
        }
        let binding = s.items[index]
            .record
            .bindings
            .iter()
            .find(|b| &b.role == role && b.attempt.number == attempt)
            .ok_or_else(|| WorkError::refused("no reserved role attempt"))?;
        let assignment = binding
            .attempt
            .native_assignment
            .as_ref()
            .ok_or_else(|| WorkError::refused("attempt is not a native reservation"))?;
        if assignment.runtime != runtime {
            return Err(WorkError::refused(
                "selected runtime differs from the reserved assignment",
            ));
        }
        let path = worktree.unwrap_or(&assignment.worktree);
        if binding.session.as_ref().is_some_and(|id| id != session)
            || canonical(path)? != assignment.worktree
            || assignment.branch != branch
            || &assignment.base != base
        {
            return Err(WorkError::refused(
                "native identity or shared placement differs from the reservation",
            ));
        }
        if binding.session.as_ref() == Some(session) {
            return Ok(());
        }
        if binding.attempt.released_at.is_some() {
            return Err(WorkError::refused("attempt has been released"));
        }
        clean(path)?;
        if head(path, "HEAD")? != *base || git::run(path, ["branch", "--show-current"])? != branch {
            return Err(WorkError::refused(
                "native child must bind before work at the captured clean head",
            ));
        }
        s.commit("native child bound", |s| {
            let b = s.items[index]
                .record
                .bindings
                .iter_mut()
                .find(|b| &b.role == role && b.attempt.number == attempt)
                .ok_or_else(|| WorkError::refused("reserved attempt missing"))?;
            b.session = Some(session.clone());
            Ok(())
        })?;
        s.flush()?;
        Ok(())
    }
    pub fn resolve_native_attempt(
        &self,
        project: &ProjectRef,
        request: &AttemptResolution<'_>,
    ) -> Result<Outcome<Binding>, WorkError> {
        let AttemptResolution {
            todo,
            role,
            attempt,
            reason,
            evidence,
        } = *request;
        if reason.trim().is_empty() {
            return Err(WorkError::refused(
                "attempt resolution needs a nonempty reason",
            ));
        }
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        orchestrator(&s, index)?;
        let pos = s.items[index]
            .record
            .bindings
            .iter()
            .position(|b| &b.role == role && b.attempt.number == attempt)
            .ok_or_else(|| WorkError::refused("no reserved role attempt"))?;
        let binding = &s.items[index].record.bindings[pos];
        let assignment = binding
            .attempt
            .native_assignment
            .as_ref()
            .ok_or_else(|| WorkError::refused("not a native assignment"))?;
        let mut next = binding.clone();
        match evidence {
            None => {
                if binding.session.is_some() || binding.attempt.released_at.is_some() {
                    return Err(WorkError::refused(
                        "a bound or released child cannot be declared not started",
                    ));
                }
                next.attempt.no_start_reason = Some(reason.into());
            }
            Some(e) => {
                if !e.stopped
                    || e.source.trim().is_empty()
                    || binding.session.as_ref() != Some(&e.session)
                    || (binding.attempt.released_at.is_none()
                        && canonical(&e.worktree)? != assignment.worktree)
                    || e.branch != assignment.branch
                {
                    return Err(WorkError::refused(
                        "failure requires native stopped-child evidence matching the binding",
                    ));
                }
                let encoded =
                    serde_json::to_string(e).map_err(|e| WorkError::refused(e.to_string()))?;
                if binding.attempt.released_at.is_some() {
                    if binding.attempt.failure_workspace.as_deref() != Some(&encoded)
                        || binding.attempt.failure_reason.as_deref() != Some(reason)
                    {
                        return Err(WorkError::refused(
                            "failure replay differs from retained evidence",
                        ));
                    }
                    return Ok(Outcome {
                        value: binding.clone(),
                        reflection: s.status(index),
                        docs_changed: vec![],
                    });
                }
                clean(&e.worktree)?;
                if head(&e.worktree, "HEAD")? != e.head
                    || git::run(&e.worktree, ["branch", "--show-current"])? != e.branch
                {
                    return Err(WorkError::refused(
                        "failed child workspace evidence differs from current shared placement",
                    ));
                }
                next.attempt.failure_reason = Some(reason.into());
                next.attempt.failure_head = Some(e.head.clone());
                next.attempt.failure_workspace = Some(encoded);
                next.attempt.failed_at = Some(s.now);
                next.attempt.released_at = Some(s.now);
            }
        }
        s.commit("native attempt resolved", |s| {
            s.items[index].record.bindings[pos] = next.clone();
            Ok(())
        })?;
        s.flush()?;
        Ok(Outcome {
            value: next,
            reflection: s.status(index),
            docs_changed: vec![],
        })
    }
}

/// Match completion to the caller's reserved shared workspace.
pub(super) fn completion_binding(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
    cwd: &Path,
    request: &super::StampRequest,
) -> Result<Option<(usize, bool)>, WorkError> {
    use super::StampRequest;
    let roles: &[&str] = match request {
        StampRequest::PreparationHeadVerdict { .. } => &["commit-reviewer"],
        StampRequest::RoleComplete => &["setup", "finalizer", "intaker", "architect", "planner"],
        StampRequest::BuildComplete => &["builder", "fixer"],
        StampRequest::BuildVerdict { .. } => &["build-reviewer"],
        StampRequest::DeferralsProcessed => &["deferrer"],
        StampRequest::PreparationVerdict { phase, .. } => match phase {
            PreparationReview::PrdReview => &["prd-reviewer"],
            PreparationReview::ArchitectureReview => &["architecture-reviewer"],
            PreparationReview::PlanReview => &["plan-reviewer"],
        },
        _ => return Ok(None),
    };
    if caller.is_none() && matches!(request, StampRequest::PreparationVerdict { .. }) {
        return Ok(None);
    }
    let caller =
        caller.ok_or_else(|| WorkError::refused("completion requires a bound role session"))?;
    let Some((index, b)) = record
        .bindings
        .iter()
        .enumerate()
        .rev()
        .find(|(_, b)| b.session.as_ref() == Some(caller) && roles.contains(&b.role.as_str()))
    else {
        // Existing deliberately recorded preparation reviews without native dispatch remain admissible.
        if matches!(request, StampRequest::PreparationVerdict { .. }) {
            return Ok(None);
        }
        return Err(WorkError::refused("caller has no bound completion role"));
    };
    if matches!(request, StampRequest::PreparationHeadVerdict { .. })
        && record.bindings.iter().any(|writer| {
            writer.session.as_ref() == Some(caller)
                && matches!(writer.role.as_str(), "intaker" | "architect" | "planner")
        })
    {
        return Err(WorkError::refused(
            "preparation head review requires an independent reviewer",
        ));
    }
    let path = b
        .worktree
        .as_ref()
        .ok_or_else(|| WorkError::refused("shared placement missing"))?;
    if canonical(
        &git::worktree_root(cwd)?
            .ok_or_else(|| WorkError::refused("caller is outside a worktree"))?,
    )? != canonical(path)?
    {
        return Err(WorkError::refused(
            "caller is outside its bound shared worktree",
        ));
    }
    clean(path)?;
    let current = head(path, "HEAD")?;
    if b.attempt.branch.as_deref() != Some(git::run(path, ["branch", "--show-current"])?.as_str()) {
        return Err(WorkError::refused(
            "completion branch differs from the shared assignment",
        ));
    }
    let readonly = matches!(
        request,
        StampRequest::BuildVerdict { .. }
            | StampRequest::DeferralsProcessed
            | StampRequest::PreparationVerdict { .. }
            | StampRequest::PreparationHeadVerdict { .. }
    );
    if readonly && b.attempt.base_head.as_ref() != Some(&current) {
        return Err(WorkError::refused(
            "read-only attempt changed from its captured head",
        ));
    }
    if b.attempt.released_at.is_some() {
        if b.attempt.completion_sequence.is_none()
            || b.attempt.completed_head.as_ref() != Some(&current)
        {
            return Err(WorkError::refused(
                "released attempt has no matching completion",
            ));
        }
        return Ok(Some((index, true)));
    }
    if b.role.as_str() == "finalizer"
        && head(
            path,
            &format!(
                "refs/remotes/origin/{}",
                record
                    .branch
                    .as_deref()
                    .ok_or_else(|| WorkError::refused("todo branch missing"))?
            ),
        )? != current
    {
        return Err(WorkError::refused(
            "finalizer completion needs matching published todo head",
        ));
    }
    if matches!(b.role.as_str(), "intaker" | "architect" | "planner") {
        let artifact = match b.role.as_str() {
            "intaker" => artifact_names::PRD,
            "architect" => artifact_names::ARCHITECTURE,
            _ => artifact_names::IMPLEMENTATION_PLAN,
        };
        if record.current_digest(artifact).is_none()
            || b.attempt.base_head.as_ref() == Some(&current)
        {
            return Err(WorkError::refused(
                "writer completion requires its artifact and authored commit evidence",
            ));
        }
    }
    Ok(Some((index, true)))
}

pub(super) fn readiness(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
    cwd: &Path,
    root: &Path,
) -> Result<(), WorkError> {
    let caller =
        caller.ok_or_else(|| WorkError::refused("readiness requires the retained finalizer"))?;
    if record.bindings.iter().any(|b| {
        b.attempt.assignment == AssignmentKind::Workspace
            && b.attempt.lane == Some(instrukt_runner::interfaces::Lane::Work)
            && b.attempt.released_at.is_none()
    }) {
        return Err(WorkError::refused(
            "outstanding workspace attempt blocks readiness",
        ));
    }
    let b = record
        .bindings
        .iter()
        .rev()
        .find(|b| {
            b.session.as_ref() == Some(caller)
                && b.role.as_str() == "finalizer"
                && b.attempt.completion_sequence.is_some()
                && b.attempt.failed_at.is_none()
        })
        .ok_or_else(|| WorkError::refused("readiness requires successful finalizer completion"))?;
    let path = b
        .worktree
        .as_ref()
        .ok_or_else(|| WorkError::refused("shared workspace missing"))?;
    let candidate = head(path, "HEAD")?;
    let branch = record
        .branch
        .as_deref()
        .ok_or_else(|| WorkError::refused("todo branch missing"))?;
    clean(path)?;
    if canonical(
        &git::worktree_root(cwd)?.ok_or_else(|| WorkError::refused("caller outside worktree"))?,
    )? != canonical(path)?
        || head(root, branch)? != candidate
        || head(root, &format!("refs/remotes/origin/{branch}"))? != candidate
        || b.attempt.completed_head.as_ref() != Some(&candidate)
    {
        return Err(WorkError::refused(
            "readiness requires matching clean shared, completed, local and published candidate",
        ));
    }
    if !crate::workflow::readiness_approved(record, root, &candidate, caller) {
        return Err(WorkError::refused(
            "readiness requires current independent build approval",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<instrukt_core::config::AgentSelection>,
    pub key: String,
    pub from: String,
    pub branch: String,
    pub base: CommitSha,
    pub worktree: Option<PathBuf>,
    pub admitted: bool,
}
impl Work {
    /// Describe a derived phase branch without creating it or switching the canonical checkout. A caller
    /// is admitted only in that branch's registered linked worktree.
    pub fn workspace_entry(
        &self,
        project: &ProjectRef,
        request: &WorkspaceRequest<'_>,
    ) -> Result<Option<WorkspaceEntry>, WorkError> {
        let WorkspaceRequest {
            todo,
            role,
            cwd,
            preparation,
            evidence,
        } = *request;
        let mut s = Session::open(self, project)?;
        let index = match s.find(todo, true) {
            Some(index) => {
                s.pull_through(index)?;
                index
            }
            None => s.enter(todo)?,
        };
        let machine = s.machine()?;
        let caller = s.caller_session()?;
        let record = &s.items[index].record;
        if record.status != TodoStatus::Active
            || record.kind == instrukt_runner::interfaces::TodoKind::Task
        {
            return Ok(None);
        }
        if !preparation {
            prepared(record)?;
        }
        if record.hold.is_some() {
            return Err(WorkError::refused("held work admits no orchestrator entry"));
        }
        if record.claim.as_ref().is_some_and(|claim| {
            claim.machine != machine
                || claim.session.is_none()
                || caller.is_none()
                || claim.session != caller
        }) {
            return Err(WorkError::refused(
                "work is already claimed; await its verified startup binding or resume its current orchestrator",
            ));
        }
        let lane = if preparation {
            instrukt_runner::interfaces::Lane::Prepare
        } else {
            instrukt_runner::interfaces::Lane::Work
        };
        let existing = record.bindings.iter().rev().find(|b| {
            b.role == *role
                && b.attempt.assignment == AssignmentKind::Driver
                && b.attempt.released_at.is_none()
        });
        let branch = existing
            .and_then(|b| b.attempt.branch.clone())
            .unwrap_or_else(|| format!("{todo}/{}", if preparation { "prepare" } else { "work" }));
        let default = instrukt_core::github::default_branch(&project.root)?;
        let listing = git::run(&project.root, ["worktree", "list", "--porcelain"])?;
        let path = listing
            .split("\n\n")
            .find(|entry| {
                entry
                    .lines()
                    .any(|line| line == format!("branch refs/heads/{branch}"))
            })
            .and_then(|entry| {
                entry
                    .lines()
                    .find_map(|line| line.strip_prefix("worktree "))
            })
            .map(PathBuf::from);
        let root = git::worktree_root(cwd)?
            .ok_or_else(|| WorkError::refused("caller outside a worktree"))?;
        let in_workspace = match &path {
            Some(path) => canonical(path)? == canonical(&root)?,
            None => false,
        };
        let number = existing.map(|b| b.attempt.number).unwrap_or_else(|| {
            record
                .bindings
                .iter()
                .filter(|b| b.role == *role)
                .map(|b| b.attempt.number)
                .max()
                .unwrap_or(0)
                + 1
        });
        let key = existing
            .map(|b| b.attempt.semantic_key.clone())
            .unwrap_or_else(|| {
                keyed::todo_key(
                    project.slug.as_str(),
                    record.id.as_str(),
                    role.as_str(),
                    number,
                )
            });
        let already_bound = in_workspace
            && existing.is_some_and(|b| {
                b.session.is_some()
                    && b.session == caller
                    && b.worktree.as_ref() == path.as_ref()
                    && b.attempt.branch.as_ref() == Some(&branch)
            });
        let admitted = if let Some(proof) = evidence {
            let caller = caller.as_ref().ok_or_else(|| {
                WorkError::refused(
                    "placement evidence must be recorded from its selected-runtime caller",
                )
            })?;
            if !in_workspace
                || &proof.session != caller
                || proof.key != key
                || proof.branch != branch
                || proof.source.trim().is_empty()
                || path.as_ref().map(|p| canonical(p)).transpose()?
                    != Some(canonical(&proof.worktree)?)
                || head(&root, "HEAD")? != proof.head
            {
                return Err(WorkError::refused(
                    "native placement evidence differs from the recorded phase assignment",
                ));
            }
            if existing
                .and_then(|b| b.session.as_ref())
                .is_some_and(|bound| bound != caller)
            {
                return Err(WorkError::refused("phase is bound to another orchestrator"));
            }
            clean(&root)?;
            true
        } else if already_bound {
            true
        } else if in_workspace {
            return Err(WorkError::refused(
                "Git cwd alone does not bind an orchestrator; supply --placement-evidence with native session, key, recorded worktree, branch, head and source",
            ));
        } else {
            false
        };
        if !admitted
            && git::succeeds(
                &project.root,
                [
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}"),
                ],
            )?
        {
            return Err(WorkError::refused(format!(
                "branch `{branch}` already exists; resume its recorded orchestrator or resolve its existing worktree before creation"
            )));
        }
        let (from, base) = if already_bound {
            (format!("origin/{default}"), head(&project.root, &branch)?)
        } else {
            let (from, base) = phase_base(&project.root, &branch, preparation)?;
            (
                from,
                if admitted {
                    head(&project.root, &branch)?
                } else {
                    base
                },
            )
        };
        if !preparation {
            preparation_integrated(record, &project.root)?;
        }
        if (!preparation && record.branch.as_ref() != Some(&branch)) || (admitted && !already_bound)
        {
            let runtime = s.config.runner_runtime()?;
            s.commit("todo workspace placement", |s| {
                let record = &mut s.items[index].record;
                if !preparation {
                    record.branch = Some(branch.clone());
                }
                if admitted && !already_bound {
                    record.bindings.push(Binding {
                        role: role.clone(),
                        session: caller.clone(),
                        worktree: path.clone(),
                        attempt: Attempt {
                            runtime,
                            native_placement: evidence.cloned(),
                            assignment: AssignmentKind::Driver,
                            lane: Some(lane),
                            number,
                            semantic_key: key.clone(),
                            branch: Some(branch.clone()),
                            base_head: Some(base.clone()),
                            ..Default::default()
                        },
                    });
                }
                Ok(())
            })?;
            s.flush()?;
        }
        let selection = if admitted {
            None
        } else {
            let domain = s.config.workflow_domain()?;
            let procedure = instrukt_core::index::GlobalIndex::load(s.home())
                .map_err(|error| WorkError::refused(error.to_string()))?
                .and_then(|index| {
                    index
                        .snippets
                        .into_iter()
                        .find(|entry| {
                            entry.role.as_deref() == Some(role.as_str())
                                && entry.domain == domain.as_str()
                        })
                        .map(|entry| entry.id.to_string())
                });
            native_selection(&s, &s.items[index].record, procedure.as_deref())?
        };
        Ok(Some(WorkspaceEntry {
            selection,
            key,
            from,
            branch,
            base,
            worktree: path,
            admitted,
        }))
    }
}
