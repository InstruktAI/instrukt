//! Record native shared assignments and release them with their completion evidence.
use std::path::{Path, PathBuf};

use instrukt_core::{
    git,
    ids::{CommitSha, RoleName, SessionRef, TodoSlug},
    machine::MachineId,
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
/// Fetch before a todo workspace is placed. A retained remote work branch
/// continues that branch; if a legacy `<slug>/prepare` branch exists (remote or
/// local) with un-integrated commits not yet on `origin/<main>`, `<slug>/work`
/// starts from it so in-flight preparation work is never lost; otherwise freshly
/// fetched canonical main supplies its base.
pub(super) fn phase_base(root: &Path, branch: &str) -> Result<(String, CommitSha), WorkError> {
    git::run(root, ["fetch", "--prune", "origin"])?;
    let main = instrukt_core::github::default_branch(root)?;
    let origin_main = format!("origin/{main}");
    let remote = format!("origin/{branch}");
    let has_ref = |refname: &str| git::succeeds(root, ["show-ref", "--verify", "--quiet", refname]);
    let is_unintegrated = |candidate: &str| -> Result<bool, WorkError> {
        Ok(!git::succeeds(
            root,
            ["merge-base", "--is-ancestor", candidate, &origin_main],
        )?)
    };
    let from = if has_ref(&format!("refs/remotes/{remote}"))? {
        remote
    } else if let Some(slug) = branch.strip_suffix("/work") {
        let legacy_remote = format!("origin/{slug}/prepare");
        let legacy_local = format!("{slug}/prepare");
        if has_ref(&format!("refs/remotes/{legacy_remote}"))? && is_unintegrated(&legacy_remote)? {
            legacy_remote
        } else if has_ref(&format!("refs/heads/{legacy_local}"))? && is_unintegrated(&legacy_local)?
        {
            legacy_local
        } else {
            origin_main
        }
    } else {
        origin_main
    };
    let base = head(root, &from)?;
    Ok((from, base))
}
pub(super) fn clean(root: &Path) -> Result<(), WorkError> {
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
/// Whether `assignment`, just reserved on item `index`, may be
/// returned with `dispatch_allowed`: the row the state database holds now,
/// never the session's copy, must still stand on the calling
/// orchestrator's claim, active and unheld, with the reservation unbound,
/// and in the same snapshot no withdrawal of the todo may be open. A change
/// another command made meanwhile, or a withdrawal opened meanwhile,
/// refuses; the reservation stays recorded, with nothing started by this
/// call.
fn dispatch_admitted(
    s: &Session<'_>,
    index: usize,
    assignment: &NativeAssignment,
) -> Result<(), WorkError> {
    let slug = s.items[index].record.slug.clone();
    let not_admitted = |why: &str| {
        WorkError::refused(format!(
            "attempt {} of `{}` on `{slug}` is recorded, but {why}; it is not admitted for dispatch, so start nothing for it. See where the todo and this attempt stand with `instrukt todo show {slug}` before acting on it",
            assignment.attempt, assignment.role
        ))
    };
    let Some((live, withdrawing)) = s.confirmed(index)? else {
        return Err(not_admitted("the todo is gone from this computer's record"));
    };
    if withdrawing {
        return Err(not_admitted(
            "the todo's withdrawal is releasing its working resources",
        ));
    }
    let record = &live.record;
    let caller = s.caller_session()?;
    let machine = s.machine()?;
    let claimed = record
        .claim
        .as_ref()
        .is_some_and(|c| c.machine == machine && caller.is_some() && c.session == caller);
    if !claimed {
        return Err(not_admitted("the todo is claimed elsewhere now"));
    }
    if record.status != TodoStatus::Active || record.hold.is_some() {
        return Err(not_admitted("the todo is no longer active and unheld"));
    }
    let reserved = record.bindings.iter().any(|b| {
        b.attempt.semantic_key == assignment.semantic_key
            && b.attempt.released_at.is_none()
            && b.session.is_none()
    });
    if !reserved {
        return Err(not_admitted(
            "the todo's record no longer holds the reservation",
        ));
    }
    Ok(())
}

/// Invariant 3: enforce configured `approvals.prd` before implementation or
/// finalization for features and any todo carrying a `prd` artifact.
fn require_prd_approval(
    record: &TodoRecord,
    approver: instrukt_core::config::Approver,
) -> Result<(), WorkError> {
    let evidence = super::_ports::to_evidence(record.clone(), Vec::new());
    if !crate::workflow::prd_approval_satisfied(&evidence, approver) {
        let by = match approver {
            instrukt_core::config::Approver::Human => "human",
            instrukt_core::config::Approver::Peer => "peer",
        };
        return Err(WorkError::refused(format!(
            "PRD approval is required before implementation or finalization; record it with `instrukt todo stamp {} --prd-approved --by {by}`",
            record.slug
        )));
    }
    Ok(())
}
/// Invariant 3: enforce configured `approvals.task` before executing a task firing.
pub(super) fn require_task_approval(
    record: &TodoRecord,
    approver: instrukt_core::config::Approver,
) -> Result<(), WorkError> {
    let evidence = super::_ports::to_evidence(record.clone(), Vec::new());
    if !crate::workflow::task_approval_satisfied(&evidence, approver) {
        let by = match approver {
            instrukt_core::config::Approver::Human => "human",
            instrukt_core::config::Approver::Peer => "peer",
        };
        return Err(WorkError::refused(format!(
            "task approval is required before starting a task firing; record it with `instrukt todo stamp {} --task-approved --by {by}`",
            record.slug
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
    let order = instrukt_runner::agents::role_order(
        &s.config,
        record.agent.as_ref(),
        dispatch.as_ref().and_then(|p| p.role.as_deref()),
        None,
    )
    .map_err(|e| WorkError::refused(e.to_string()))?;
    let selection = instrukt_runner::agents::selections(&s.config, s.home(), &order, s.now)
        .map_err(|e| WorkError::refused(e.to_string()))?
        .into_iter()
        .next();
    let runtime = s.config.runner_runtime()?;
    Ok(selection
        .map(|selection| instrukt_runner::runtime::adapter(runtime).native_selection(&selection)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeAssignment {
    pub dispatcher: Option<SessionRef>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<CommitSha>,
}

/// The native session of the role's most recent earlier attempt that has a
/// bound session on the reservation's runtime, with the selection and the
/// dispatching session that attempt retained where it holds them. That attempt can be older than the role's
/// latest attempt when later attempts never bound or bound on another runtime.
/// Whether the session is still usable is native evidence, not part of this
/// fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriorSession {
    pub session: SessionRef,
    pub attempt: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<instrukt_core::config::AgentSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatcher: Option<SessionRef>,
}

/// A reserved role attempt as its reservation replies: the retained native
/// inputs and, derived from the record's bindings when the reply is made, the
/// role's prior session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReservedAttempt {
    #[serde(flatten)]
    pub assignment: NativeAssignment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_session: Option<PriorSession>,
}

/// The highest-numbered worker attempt of `role` below `attempt` whose binding
/// carries a session of `runtime`. A worker binding the state migration carried
/// forward records its kind as unspecified and retains no assignment.
fn prior_session(
    bindings: &[Binding],
    role: &RoleName,
    attempt: u64,
    runtime: instrukt_core::config::Runtime,
) -> Option<PriorSession> {
    bindings
        .iter()
        .filter(|b| {
            &b.role == role
                && b.attempt.number < attempt
                && matches!(
                    b.attempt.assignment,
                    AssignmentKind::Child | AssignmentKind::Workspace | AssignmentKind::Unspecified
                )
        })
        .filter_map(|b| {
            let session = b
                .session
                .as_ref()
                .filter(|s| s.runtime.as_str() == runtime.name())?;
            Some((b, session))
        })
        .max_by_key(|(b, _)| b.attempt.number)
        .map(|(b, session)| {
            let assignment = b.attempt.native_assignment.as_ref();
            PriorSession {
                session: session.clone(),
                attempt: b.attempt.number,
                selection: assignment.and_then(|a| a.selection.clone()),
                dispatcher: assignment.and_then(|a| a.dispatcher.clone()),
            }
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoppedChild {
    pub session: SessionRef,
    pub stopped: bool,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<CommitSha>,
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
    pub branch: Option<&'a str>,
    pub base: Option<&'a CommitSha>,
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
    pub title: String,
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
    pub evidence: Option<&'a DriverPlacement>,
}

impl Work {
    /// Reserve immutable native dispatch inputs without runtime I/O.
    pub fn prepare_attempt(
        &self,
        project: &ProjectRef,
        request: &AttemptRequest<'_>,
    ) -> Result<ReservedAttempt, WorkError> {
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
        let runtime = s.config.runner_runtime()?;
        let record = &s.items[index].record;
        if let Some(active) = record.bindings.iter().find(|b| b.is_open_role_attempt()) {
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
            let prior = prior_session(&record.bindings, role, active.attempt.number, runtime);
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
                dispatch_admitted(&s, index, &result)?;
            }
            return Ok(ReservedAttempt {
                assignment: result,
                prior_session: prior,
            });
        }
        let is_task = record.kind == instrukt_runner::interfaces::TodoKind::Task;
        let (lane, worktree, branch, base, assignment_kind) = if is_task {
            if role.as_str() != "task-preparer" {
                return Err(WorkError::refused(
                    "only task-preparer may reserve a worktree-less attempt on a task todo",
                ));
            }
            (
                instrukt_runner::interfaces::Lane::Work,
                None,
                None,
                None,
                AssignmentKind::Child,
            )
        } else {
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
                .unwrap_or(instrukt_runner::interfaces::Lane::Work);
            if matches!(role.as_str(), "builder" | "finalizer") {
                let prd_approver = s
                    .config
                    .approver(instrukt_core::config::ApprovalGate::Prd)?;
                require_prd_approval(record, prd_approver)?;
            }
            let path = driver
                .worktree
                .clone()
                .ok_or_else(|| WorkError::refused("todo workspace missing"))?;
            let branch = driver
                .attempt
                .branch
                .clone()
                .ok_or_else(|| WorkError::refused("todo branch missing"))?;
            clean(&path)?;
            if git::run(&path, ["branch", "--show-current"])? != branch {
                return Err(WorkError::refused(
                    "shared workspace is not on the recorded branch",
                ));
            }
            let canon = canonical(&path)?;
            let base = head(&path, "HEAD")?;
            (
                lane,
                Some(canon),
                Some(branch),
                Some(base),
                AssignmentKind::Workspace,
            )
        };
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
        let prior = prior_session(&record.bindings, role, number, runtime);
        let selection = native_selection(&s, record, procedure)?;
        let assignment = NativeAssignment {
            dispatcher: s.caller_session()?,
            selection,
            runtime,
            dispatch_allowed: true,
            brief: brief.into(),
            procedure: procedure.map(str::to_owned),
            args: args.map(str::to_owned),
            role: role.clone(),
            attempt: number,
            semantic_key: key.clone(),
            title: instrukt_runner::runtime::thread_title(
                &format!("{role} · {}", record.slug),
                &key,
            ),
            worktree: worktree.clone(),
            branch: branch.clone(),
            base: base.clone(),
        };
        let binding = Binding {
            role: role.clone(),
            session: None,
            worktree,
            attempt: Attempt {
                assignment: assignment_kind,
                lane: Some(lane),
                number,
                semantic_key: key,
                branch,
                base_head: base,
                runtime: Some(runtime),
                native_assignment: Some(assignment.clone()),
                ..Default::default()
            },
        };
        s.commit("native role reserved", |s| {
            s.items[index].record.bindings.push(binding);
            Ok(())
        })?;
        dispatch_admitted(&s, index, &assignment)?;
        Ok(ReservedAttempt {
            assignment,
            prior_session: prior,
        })
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
        let runtime = s.config.runner_runtime()?;
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
        if binding.session.as_ref().is_some_and(|id| id != session) {
            return Err(WorkError::refused(
                "native identity or shared placement differs from the reservation",
            ));
        }
        if let Some(expected_worktree) = &assignment.worktree {
            let path = worktree.unwrap_or(expected_worktree);
            if canonical(path)? != *expected_worktree
                || assignment.branch.as_deref() != branch
                || assignment.base.as_ref() != base
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
            if Some(head(path, "HEAD")?).as_ref() != base
                || Some(git::run(path, ["branch", "--show-current"])?.as_str()) != branch
            {
                return Err(WorkError::refused(
                    "native child must bind before work at the captured clean head",
                ));
            }
        } else {
            if worktree.is_some() || branch.is_some() || base.is_some() {
                return Err(WorkError::refused(
                    "worktree-less task assignment takes no worktree, branch, or base",
                ));
            }
            if binding.session.as_ref() == Some(session) {
                return Ok(());
            }
            if binding.attempt.released_at.is_some() {
                return Err(WorkError::refused("attempt has been released"));
            }
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
                let placement_matches = match &assignment.worktree {
                    Some(expected_wt) => {
                        let wt_ok = binding.attempt.released_at.is_some()
                            || e.worktree
                                .as_ref()
                                .map(|p| canonical(p))
                                .transpose()?
                                .as_ref()
                                == Some(expected_wt);
                        wt_ok && e.branch == assignment.branch
                    }
                    None => e.worktree.is_none() && e.branch.is_none() && e.head.is_none(),
                };
                if !e.stopped
                    || e.source.trim().is_empty()
                    || binding.session.as_ref() != Some(&e.session)
                    || !placement_matches
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
                if let Some(wt) = &e.worktree {
                    clean(wt)?;
                    if Some(head(wt, "HEAD")?) != e.head
                        || Some(git::run(wt, ["branch", "--show-current"])?) != e.branch
                    {
                        return Err(WorkError::refused(
                            "failed child workspace evidence differs from current shared placement",
                        ));
                    }
                }
                next.attempt.failure_reason = Some(reason.into());
                next.attempt.failure_head = e.head.clone();
                next.attempt.failure_workspace = Some(encoded);
                next.attempt.failed_at = Some(s.now);
                next.attempt.released_at = Some(s.now);
            }
        }
        s.commit("native attempt resolved", |s| {
            s.items[index].record.bindings[pos] = next.clone();
            Ok(())
        })?;
        Ok(Outcome {
            value: next,
            reflection: s.status(index),
            docs_changed: vec![],
        })
    }
}

/// Pure binding selection for `StampRequest::DiagnosisSettled`: resolves the
/// active or matching-replay `diagnoser` workspace attempt, enforces caller
/// separation from the bound peer and claim-holder authority while claimed, and
/// returns `Ok(None)` when Step 1a settles without a diagnoser reservation.
fn diagnosis_settlement_binding(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
) -> Result<Option<usize>, WorkError> {
    let Some((index, b)) = record.bindings.iter().enumerate().rev().find(|(_, b)| {
        b.role.as_str() == "diagnoser"
            && matches!(
                b.attempt.assignment,
                AssignmentKind::Child | AssignmentKind::Workspace
            )
    }) else {
        return Ok(None);
    };
    let current_diagnosis = record.current_digest(artifact_names::DIAGNOSIS);
    if b.attempt.released_at.is_some() {
        let matches_current = b.attempt.completion_sequence.is_some_and(|sequence| {
            record.stamps.iter().any(|stamp| {
                stamp.sequence == sequence
                    && matches!(
                        &stamp.fact,
                        crate::evidence::StampFact::DiagnosisSettled { diagnosis }
                            if current_diagnosis == Some(diagnosis)
                    )
            })
        });
        return Ok(matches_current.then_some(index));
    }
    let bound = b.session.as_ref().ok_or_else(|| {
        WorkError::refused("diagnoser attempt must be bound before settling its diagnosis")
    })?;
    if caller == Some(bound) {
        return Err(WorkError::refused(
            "diagnoser cannot settle its own diagnosis; the driving session stamps --diagnosis-settled",
        ));
    }
    if let Some(claim) = &record.claim
        && claim.session.as_ref() != caller
    {
        return Err(WorkError::refused(
            "only the claim-holding orchestrator may settle a bound diagnoser attempt",
        ));
    }
    Ok(Some(index))
}

/// Pure binding selection for `StampRequest::DriftReviewed`: resolves the
/// active bound `drift-analyzer` attempt (or its released attempt before any
/// later child/workspace work starts, for multi-artifact drift stamps and
/// idempotent replay), enforces caller identity when an attempt is reserved,
/// and returns `Ok(None)` when no `drift-analyzer` attempt is bound.
fn drift_review_binding(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
) -> Result<Option<usize>, WorkError> {
    if let Some((index, b)) = record.bindings.iter().enumerate().rev().find(|(_, b)| {
        b.role.as_str() == "drift-analyzer"
            && matches!(
                b.attempt.assignment,
                AssignmentKind::Child | AssignmentKind::Workspace
            )
            && b.attempt.released_at.is_none()
    }) {
        let bound = b.session.as_ref().ok_or_else(|| {
            WorkError::refused("drift-analyzer attempt must be bound before recording drift review")
        })?;
        if caller != Some(bound) {
            return Err(WorkError::refused(
                "only the bound drift-analyzer session may complete its active drift review attempt",
            ));
        }
        return Ok(Some(index));
    }
    let Some(caller) = caller else {
        return Ok(None);
    };
    let Some((index, _)) = record.bindings.iter().enumerate().rev().find(|(_, b)| {
        b.role.as_str() == "drift-analyzer"
            && matches!(
                b.attempt.assignment,
                AssignmentKind::Child | AssignmentKind::Workspace
            )
            && b.session.as_ref() == Some(caller)
    }) else {
        return Ok(None);
    };
    if record.bindings[index + 1..].iter().any(|later| {
        matches!(
            later.attempt.assignment,
            AssignmentKind::Child | AssignmentKind::Workspace
        )
    }) {
        return Err(WorkError::refused(
            "superseded drift-analyzer attempt cannot stamp after later work began",
        ));
    }
    Ok(Some(index))
}

/// The artifact a preparation doc writer owes for its role, or `None` when
/// the role is not a preparation doc writer.
pub(super) fn writer_artifact(role: &str) -> Option<&'static str> {
    match role {
        "intaker" => Some(artifact_names::PRD),
        "architect" => Some(artifact_names::ARCHITECTURE),
        "planner" => Some(artifact_names::IMPLEMENTATION_PLAN),
        _ => None,
    }
}

/// What a completion stamp request completes: the one place a completion
/// request's admission facts live (`stamps`, which role completes which
/// request; `state-database`, completion and release per role). A new
/// completion request is one arm of [`completion`].
pub(super) struct Completion {
    /// How admission finds the attempt the request completes.
    completes: Completes,
    /// The attempt only reads the shared workspace, so its head must still
    /// be the one captured when the attempt began.
    read_only: bool,
    /// The caller must be a reviewer independent of the orchestrator and
    /// the writers.
    independent: bool,
    /// What a repeat of the request on an already released attempt does.
    pub(super) repeat: Repeat,
}

/// How a completion request finds the attempt it completes.
enum Completes {
    /// The caller's latest bound attempt of one of `roles`; `unbound` says
    /// what a caller without one gets.
    CallerRole {
        roles: &'static [&'static str],
        unbound: Unbound,
    },
    /// The active or matching-replay `diagnoser` attempt, settled by the
    /// driving session ([`diagnosis_settlement_binding`]).
    Diagnoser,
    /// The active, or still current released, `drift-analyzer` attempt
    /// ([`drift_review_binding`]).
    DriftAnalyzer,
}

/// What a caller without a matching role binding gets.
#[derive(Clone, Copy)]
enum Unbound {
    /// The record-only path: no attempt completes, and the request's own
    /// admission judges the caller.
    RecordOnly,
    /// Refusal: the request completes the caller's bound attempt or nothing.
    Refused,
}

/// What a repeat of a completion request on a released attempt does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Repeat {
    /// The attempt's retained completion stamp answers it.
    Retained,
    /// The request's own admission judges it: a drift review stamps each
    /// drifted artifact under one attempt, and a disposition replays only an
    /// exact repeat.
    Admitted,
}

/// The completion descriptor of `request`, or `None` when the request
/// completes no attempt.
pub(super) fn completion(request: &super::StampRequest) -> Option<Completion> {
    use super::StampRequest;
    let completion = match request {
        StampRequest::RoleComplete => Completion {
            completes: Completes::CallerRole {
                roles: &[
                    "setup",
                    "finalizer",
                    "intaker",
                    "architect",
                    "planner",
                    "task-preparer",
                ],
                unbound: Unbound::Refused,
            },
            read_only: false,
            independent: false,
            repeat: Repeat::Retained,
        },
        StampRequest::BuildComplete => Completion {
            completes: Completes::CallerRole {
                roles: &["builder"],
                unbound: Unbound::Refused,
            },
            read_only: false,
            independent: false,
            repeat: Repeat::Retained,
        },
        StampRequest::BuildVerdict { verdict } => Completion {
            completes: Completes::CallerRole {
                roles: &["build-reviewer"],
                unbound: Unbound::Refused,
            },
            read_only: true,
            independent: matches!(verdict, crate::evidence::BuildVerdict::Approve),
            repeat: Repeat::Retained,
        },
        StampRequest::DeferralsProcessed => Completion {
            completes: Completes::CallerRole {
                roles: &["deferrer"],
                unbound: Unbound::Refused,
            },
            read_only: true,
            independent: false,
            repeat: Repeat::Retained,
        },
        // Deliberately recorded preparation reviews without native dispatch
        // remain admissible on the record-only path.
        StampRequest::PreparationVerdict { phase, .. } => Completion {
            completes: Completes::CallerRole {
                roles: match phase {
                    PreparationReview::PrdReview => &["prd-reviewer"],
                    PreparationReview::ArchitectureReview => &["architecture-reviewer"],
                    PreparationReview::PlanReview => &["plan-reviewer"],
                },
                unbound: Unbound::RecordOnly,
            },
            read_only: true,
            independent: false,
            repeat: Repeat::Retained,
        },
        // A disposition from a caller with no verifier binding takes the
        // record-only path, where its admission judges the caller.
        StampRequest::VerificationDisposition { .. } => Completion {
            completes: Completes::CallerRole {
                roles: &[crate::workflow::VERIFIER_ROLE],
                unbound: Unbound::RecordOnly,
            },
            read_only: true,
            independent: false,
            repeat: Repeat::Admitted,
        },
        StampRequest::DiagnosisSettled => Completion {
            completes: Completes::Diagnoser,
            read_only: true,
            independent: false,
            repeat: Repeat::Retained,
        },
        StampRequest::DriftReviewed { .. } => Completion {
            completes: Completes::DriftAnalyzer,
            read_only: true,
            independent: false,
            repeat: Repeat::Admitted,
        },
        StampRequest::Approval { .. }
        | StampRequest::CarriedApproval { .. }
        | StampRequest::IntentChecked
        | StampRequest::Kickback { .. }
        | StampRequest::ArchitectureSkipped { .. }
        | StampRequest::DiagnosisUnresolved { .. }
        | StampRequest::BuildStarted
        | StampRequest::FinalizeReady
        | StampRequest::TaskFiringStarted { .. }
        | StampRequest::TaskOutcome { .. } => return None,
    };
    Some(completion)
}

/// The caller's latest bound attempt of one of `roles`; a caller without
/// one takes the record-only path or is refused, as `unbound` says.
fn caller_role_binding(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
    roles: &[&str],
    unbound: Unbound,
) -> Result<Option<usize>, WorkError> {
    let Some(caller) = caller else {
        return match unbound {
            Unbound::RecordOnly => Ok(None),
            Unbound::Refused => Err(WorkError::refused(
                "completion requires a bound role session",
            )),
        };
    };
    let found = record
        .bindings
        .iter()
        .enumerate()
        .rev()
        .find(|(_, b)| b.session.as_ref() == Some(caller) && roles.contains(&b.role.as_str()))
        .map(|(index, _)| index);
    match (found, unbound) {
        (Some(index), _) => Ok(Some(index)),
        (None, Unbound::RecordOnly) => Ok(None),
        (None, Unbound::Refused) => Err(WorkError::refused("caller has no bound completion role")),
    }
}

/// Match completion to the caller's reserved shared workspace: the index of
/// the attempt `request` completes, or `None` when it completes none.
pub(super) fn completion_binding(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
    cwd: &Path,
    request: &super::StampRequest,
) -> Result<Option<usize>, WorkError> {
    let Some(completion) = completion(request) else {
        return Ok(None);
    };
    let selected = match completion.completes {
        Completes::Diagnoser => diagnosis_settlement_binding(record, caller)?,
        Completes::DriftAnalyzer => drift_review_binding(record, caller)?,
        Completes::CallerRole { roles, unbound } => {
            caller_role_binding(record, caller, roles, unbound)?
        }
    };
    let Some(index) = selected else {
        return Ok(None);
    };
    let b = &record.bindings[index];
    if completion.independent {
        let caller =
            caller.ok_or_else(|| WorkError::refused("completion requires a bound role session"))?;
        let tuples: Vec<(RoleName, AssignmentKind, Option<&SessionRef>)> = record
            .bindings
            .iter()
            .map(|item| {
                (
                    item.role.clone(),
                    item.attempt.assignment,
                    item.session.as_ref(),
                )
            })
            .collect();
        if !crate::workflow::is_independent_reviewer(
            record.claim.as_ref().and_then(|c| c.session.as_ref()),
            &tuples,
            &crate::evidence::Actor::Session(caller.clone()),
            None,
        ) {
            return Err(WorkError::refused(
                "build approval requires an independent reviewer distinct from the orchestrator and writers",
            ));
        }
    }
    if b.worktree.is_none()
        && record.kind == instrukt_runner::interfaces::TodoKind::Task
        && b.attempt.assignment == AssignmentKind::Child
    {
        if b.attempt.released_at.is_some() && b.attempt.completion_sequence.is_none() {
            return Err(WorkError::refused(
                "released attempt has no matching completion",
            ));
        }
        return Ok(Some(index));
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
    if completion.read_only && b.attempt.base_head.as_ref() != Some(&current) {
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
        return Ok(Some(index));
    }
    if b.role.as_str() == "finalizer"
        && head(
            path,
            &format!(
                "refs/remotes/origin/{}",
                record
                    .branch
                    .as_deref()
                    .or(b.attempt.branch.as_deref())
                    .ok_or_else(|| WorkError::refused("todo branch missing"))?
            ),
        )? != current
    {
        return Err(WorkError::refused(
            "finalizer completion needs matching published todo head",
        ));
    }
    if let Some(artifact) = writer_artifact(b.role.as_str())
        && record.current_digest(artifact).is_none()
    {
        return Err(WorkError::refused(format!(
            "writer completion requires a version of its {artifact} artifact"
        )));
    }
    Ok(Some(index))
}

/// The caller's live driver binding on the todo: the binding the
/// orchestrator records a Tier 0 skip or hotfix deferral under, with no
/// verifier attempt.
pub(super) fn live_driver(record: &TodoRecord, caller: &SessionRef) -> Option<usize> {
    record
        .bindings
        .iter()
        .enumerate()
        .rev()
        .find(|(_, b)| {
            b.session.as_ref() == Some(caller)
                && b.attempt.assignment == AssignmentKind::Driver
                && b.attempt.released_at.is_none()
        })
        .map(|(index, _)| index)
}

/// The workspace a record-only disposition is written over
/// (`delivery-cycle`, "Live verification disposition"): no workspace
/// attempt open on the todo's worktree, whose finding it would race; the
/// caller in the driver binding's shared worktree; that tree clean, so the
/// head it binds is the candidate.
pub(super) fn record_only_workspace(
    record: &TodoRecord,
    driver: usize,
    cwd: &Path,
) -> Result<(), WorkError> {
    if let Some(open) = record.bindings.iter().find(|b| b.is_open_role_attempt()) {
        return Err(WorkError::refused(format!(
            "the {}/{} attempt is open on the todo's worktree; a disposition is recorded after that attempt's release or its recorded failure",
            open.role, open.attempt.number
        )));
    }
    let path = record.bindings[driver]
        .worktree
        .as_ref()
        .ok_or_else(|| WorkError::refused("shared workspace missing"))?;
    if canonical(
        &git::worktree_root(cwd)?.ok_or_else(|| WorkError::refused("caller outside worktree"))?,
    )? != canonical(path)?
    {
        return Err(WorkError::refused(
            "a disposition is recorded from the todo's shared worktree, whose head is the candidate; run it there",
        ));
    }
    // `clean`'s own refusal names an outstanding attempt, which this path
    // has none of; its other failures pass through unchanged.
    clean(path).map_err(|err| match err {
        WorkError::Refused(_) => WorkError::refused(
            "the shared workspace has uncommitted changes or an unfinished Git operation; commit or restore them before recording a disposition",
        ),
        other => other,
    })
}

pub(super) fn readiness(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
    cwd: &Path,
    root: &Path,
    prd_approver: instrukt_core::config::Approver,
) -> Result<(), WorkError> {
    let caller = caller.ok_or_else(|| {
        WorkError::refused(
            "readiness requires the claim-holding orchestrator or retained finalizer",
        )
    })?;
    // Invariant 1: no active unreleased workspace/child attempt.
    if record.bindings.iter().any(|b| b.is_open_role_attempt()) {
        return Err(WorkError::refused(
            "outstanding workspace attempt blocks readiness",
        ));
    }
    // Invariant 3: configured PRD approval must hold when required.
    require_prd_approval(record, prd_approver)?;
    // Admissible callers: a retained completed finalizer OR the claim-holding driver orchestrator.
    let finalizer = record.bindings.iter().rev().find(|b| {
        b.session.as_ref() == Some(caller)
            && b.role.as_str() == "finalizer"
            && b.attempt.completion_sequence.is_some()
            && b.attempt.failed_at.is_none()
    });
    let driver = if finalizer.is_none()
        && record
            .claim
            .as_ref()
            .is_some_and(|c| c.session.as_ref() == Some(caller))
    {
        record.bindings.iter().rev().find(|b| {
            b.session.as_ref() == Some(caller)
                && b.attempt.assignment == AssignmentKind::Driver
                && b.attempt.released_at.is_none()
        })
    } else {
        None
    };
    let b = finalizer.or(driver).ok_or_else(|| {
        WorkError::refused(
            "readiness requires the claim-holding orchestrator or a completed finalizer",
        )
    })?;
    let path = b
        .worktree
        .as_ref()
        .ok_or_else(|| WorkError::refused("shared workspace missing"))?;
    let candidate = head(path, "HEAD")?;
    let branch = record
        .branch
        .as_deref()
        .or(b.attempt.branch.as_deref())
        .ok_or_else(|| WorkError::refused("todo branch missing"))?;
    clean(path)?;
    let finalizer_head_ok =
        finalizer.is_none_or(|f| f.attempt.completed_head.as_ref() == Some(&candidate));
    if canonical(
        &git::worktree_root(cwd)?.ok_or_else(|| WorkError::refused("caller outside worktree"))?,
    )? != canonical(path)?
        || head(root, branch)? != candidate
        || head(root, &format!("refs/remotes/origin/{branch}"))? != candidate
        || !finalizer_head_ok
    {
        return Err(WorkError::refused(
            "readiness requires matching clean shared, local and published candidate",
        ));
    }
    // Invariant 2, the independent exact-SHA handoff gate: an independent
    // reviewer's APPROVE stamp and a current verification disposition that
    // admits handoff, both bound to this exact commit SHA.
    if !crate::workflow::readiness_approved(record, root, &candidate, caller) {
        return Err(WorkError::refused(
            "readiness requires an independent reviewer's APPROVE stamp bound to this exact commit SHA and current review-findings",
        ));
    }
    crate::workflow::handoff_admitted(record, &candidate, caller).map_err(|why| {
        WorkError::refused(format!(
            "readiness requires a current verification disposition admitting handoff at this exact commit SHA — an independent verifier's pass, or the claim-holding orchestrator's tier_0_skip or hotfix_deferred: {why}"
        ))
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceEntry {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<instrukt_core::config::AgentSelection>,
    pub key: String,
    pub from: String,
    pub branch: String,
    pub base: CommitSha,
    pub worktree: Option<PathBuf>,
    pub admitted: bool,
}
fn claimed_elsewhere() -> WorkError {
    WorkError::refused(
        "work is already claimed; await its verified startup binding or resume its current orchestrator",
    )
}

/// The todo's recorded orchestrator: its latest driver binding for `role`
/// that has not failed, released or not. A later binding, including an
/// unbound runner reservation, supersedes an earlier session's.
fn recorded_driver<'a>(record: &'a TodoRecord, role: &RoleName) -> Option<&'a Binding> {
    record.bindings.iter().rev().find(|b| {
        super::_ports::matches_driver_role(&b.role, role)
            && b.attempt.assignment == AssignmentKind::Driver
            && b.attempt.failed_at.is_none()
    })
}

/// Whether `b` is `caller`'s own placement: bound to that session, in the
/// registered worktree `path` (compared canonically, so a symlinked stored
/// path matches) on `branch`.
fn own_placement(
    b: &Binding,
    caller: Option<&SessionRef>,
    path: Option<&Path>,
    branch: &str,
) -> Result<bool, WorkError> {
    let (Some(caller), Some(path), Some(tree)) = (caller, path, b.worktree.as_deref()) else {
        return Ok(false);
    };
    Ok(b.session.as_ref() == Some(caller)
        && b.attempt.branch.as_deref() == Some(branch)
        && tree.is_dir()
        && canonical(tree)? == canonical(path)?)
}

/// What the record alone decides about an orchestrator's workspace entry,
/// before any Git inspection: the role's driver bindings, and whether the
/// caller would take up this computer's unassigned runner claim.
struct DriverClaim<'a> {
    record: &'a TodoRecord,
    role: &'a RoleName,
    caller: Option<&'a SessionRef>,
    /// The caller would take up this computer's unassigned runner claim,
    /// which admits only the todo's recorded orchestrator.
    takeover: bool,
    /// The role's latest unreleased driver binding.
    existing: Option<&'a Binding>,
    /// The todo's recorded orchestrator, released or not.
    recorded: Option<&'a Binding>,
}

impl DriverClaim<'_> {
    /// The phase branch: the current driver's, else the recorded driver's,
    /// else the work branch derived from `todo`.
    fn branch(&self, todo: &TodoSlug) -> String {
        self.existing
            .or(self.recorded)
            .and_then(|b| b.attempt.branch.clone())
            .unwrap_or_else(|| format!("{todo}/work"))
    }
}

/// The claim gate of an orchestrator's workspace entry. A claim admits its
/// own session on its computer. This computer's unassigned claim — the
/// runner's pickup — admits only the todo's recorded orchestrator, which
/// [`admit_driver`] decides once the placement is known.
fn driver_claim<'a>(
    record: &'a TodoRecord,
    role: &'a RoleName,
    machine: &MachineId,
    caller: Option<&'a SessionRef>,
) -> Result<DriverClaim<'a>, WorkError> {
    let takeover = match &record.claim {
        None => false,
        Some(claim) if claim.machine == *machine && caller.is_some() => {
            if claim.session.as_ref() == caller {
                false
            } else if claim.session.is_none() {
                true
            } else {
                return Err(claimed_elsewhere());
            }
        }
        Some(_) => return Err(claimed_elsewhere()),
    };
    let existing = record.bindings.iter().rev().find(|b| {
        super::_ports::matches_driver_role(&b.role, role)
            && b.attempt.assignment == AssignmentKind::Driver
            && b.attempt.released_at.is_none()
    });
    Ok(DriverClaim {
        record,
        role,
        caller,
        takeover,
        existing,
        recorded: recorded_driver(record, role),
    })
}

/// Where the phase branch's registered linked worktree is, relative to the
/// caller's worktree.
struct Placement {
    branch: String,
    /// The branch's registered linked worktree, if any.
    path: Option<PathBuf>,
    /// The caller's worktree root.
    root: PathBuf,
    /// The caller stands in the branch's registered worktree.
    in_workspace: bool,
}

/// Resolve `branch`'s registered linked worktree in `project_root` and
/// whether `cwd` stands in it.
fn placement(project_root: &Path, branch: String, cwd: &Path) -> Result<Placement, WorkError> {
    let listing = git::run(project_root, ["worktree", "list", "--porcelain"])?;
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
    let root =
        git::worktree_root(cwd)?.ok_or_else(|| WorkError::refused("caller outside a worktree"))?;
    let in_workspace = match &path {
        Some(path) => canonical(path)? == canonical(&root)?,
        None => false,
    };
    Ok(Placement {
        branch,
        path,
        root,
        in_workspace,
    })
}

/// How an orchestrator's workspace entry is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// Not admitted: the entry describes the phase for native placement.
    Describe,
    /// Admitted on native placement evidence; the entry records a new
    /// driver binding.
    Place,
    /// Admitted as the session already bound in its own placement;
    /// `reopen` reopens the recorded orchestrator's released binding.
    Bound { reopen: bool },
}

/// An admission decision with the attempt the entry continues or opens.
struct EntryAdmission<'a> {
    admission: Admission,
    /// The driver binding the entry continues, if any.
    driver: Option<&'a Binding>,
    number: u64,
    key: String,
}

/// Admit an orchestrator into the phase's placement under `runtime`.
/// Placement evidence must match the recorded phase assignment. Without
/// it, a caller in the workspace is admitted only as the session already
/// bound there or as the todo's recorded orchestrator resuming its own
/// placement — the one caller that may take up this computer's unassigned
/// runner claim; a caller outside it is described the phase to place.
fn admit_driver<'a>(
    claim: DriverClaim<'a>,
    placement: &Placement,
    evidence: Option<&DriverPlacement>,
    runtime: instrukt_core::config::Runtime,
    project: &str,
) -> Result<EntryAdmission<'a>, WorkError> {
    let DriverClaim {
        record,
        role,
        caller,
        takeover,
        existing,
        recorded,
    } = claim;
    let Placement {
        branch,
        path,
        root,
        in_workspace,
    } = placement;
    // The recorded orchestrator resumes in its own placement: its
    // binding was bound from verified native placement, so the same
    // session in the same worktree, branch and runtime needs no new proof.
    let resumable = match recorded {
        Some(b) if *in_workspace && own_placement(b, caller, path.as_deref(), branch)? => {
            b.attempt.runtime == Some(runtime)
        }
        _ => false,
    };
    if takeover && !resumable {
        return Err(claimed_elsewhere());
    }
    // A recorded orchestrator of another driver role (a retired one) that
    // resumes its own placement with evidence would be recorded in the
    // requested role beside its own binding, and its session would then
    // resolve to no assignment. Its door is the reopen of that binding.
    if let Some(held) = recorded
        .filter(|b| evidence.is_some() && resumable && existing.is_none() && b.role != *role)
    {
        return Err(WorkError::refused(format!(
            "this session is the todo's recorded orchestrator on `{}`; placement evidence would also record it as `{role}`, a second assignment it could not resolve, so nothing was written. Enter again from this worktree without --placement-evidence: that reopens its recorded binding",
            held.attempt.semantic_key
        )));
    }
    // Without evidence, a released recorded binding is reopened rather
    // than followed by a new attempt, as dispatch resumes a retained driver.
    let reopen = evidence.is_none() && existing.is_none() && resumable;
    let driver = if reopen { recorded } else { existing };
    let number = driver.map(|b| b.attempt.number).unwrap_or_else(|| {
        record
            .bindings
            .iter()
            .filter(|b| b.role == *role)
            .map(|b| b.attempt.number)
            .max()
            .unwrap_or(0)
            + 1
    });
    let key = driver
        .map(|b| b.attempt.semantic_key.clone())
        .unwrap_or_else(|| keyed::todo_key(project, record.id.as_str(), role.as_str(), number));
    let already_bound = *in_workspace
        && match driver {
            Some(b) => own_placement(b, caller, path.as_deref(), branch)?,
            None => false,
        };
    let admission = if let Some(proof) = evidence {
        let caller = caller.ok_or_else(|| {
            WorkError::refused(
                "placement evidence must be recorded from its selected-runtime caller",
            )
        })?;
        if !in_workspace
            || &proof.session != caller
            || proof.key != key
            || proof.branch != *branch
            || proof.source.trim().is_empty()
            || path.as_ref().map(|p| canonical(p)).transpose()? != Some(canonical(&proof.worktree)?)
            || head(root, "HEAD")? != proof.head
        {
            return Err(WorkError::refused(
                "native placement evidence differs from the recorded phase assignment",
            ));
        }
        if driver
            .and_then(|b| b.session.as_ref())
            .is_some_and(|bound| bound != caller)
        {
            return Err(WorkError::refused("phase is bound to another orchestrator"));
        }
        clean(root)?;
        if already_bound {
            Admission::Bound { reopen }
        } else {
            Admission::Place
        }
    } else if already_bound {
        Admission::Bound { reopen }
    } else if *in_workspace {
        return Err(WorkError::refused(
            "Git cwd alone does not bind an orchestrator; supply --placement-evidence with native session, key, title, recorded worktree, branch, head and source",
        ));
    } else {
        Admission::Describe
    };
    Ok(EntryAdmission {
        admission,
        driver,
        number,
        key,
    })
}

impl Work {
    /// Describe a derived phase branch without creating it or switching the canonical checkout. A caller
    /// is admitted only in that branch's registered linked worktree: with native placement evidence,
    /// as the session already bound there, or as the todo's recorded orchestrator resuming its own
    /// released binding (which also takes up this computer's unassigned runner claim).
    pub fn workspace_entry(
        &self,
        project: &ProjectRef,
        request: &WorkspaceRequest<'_>,
    ) -> Result<Option<WorkspaceEntry>, WorkError> {
        let WorkspaceRequest {
            todo,
            role,
            cwd,
            evidence,
        } = *request;
        let mut s = Session::open(self, project)?;
        let index = match s.find(todo, true) {
            Some(index) => index,
            None => s.enter(todo)?,
        };
        let machine = s.machine()?;
        let caller = s.caller_session()?;
        // Resolving the caller already read the runner runtime.
        let runtime = s.config.runner_runtime()?;
        let record = &s.items[index].record;
        if record.status != TodoStatus::Active
            || record.kind == instrukt_runner::interfaces::TodoKind::Task
        {
            return Ok(None);
        }
        if record.hold.is_some() {
            return Err(WorkError::refused("held work admits no orchestrator entry"));
        }
        let claim = driver_claim(record, role, &machine, caller.as_ref())?;
        let branch = claim.branch(todo);
        let default = instrukt_core::github::default_branch(&project.root)?;
        let placement = placement(&project.root, branch, cwd)?;
        let entry = admit_driver(claim, &placement, evidence, runtime, project.slug.as_str())?;
        let title = entry
            .driver
            .and_then(|b| b.attempt.native_placement.as_ref())
            .map(|p| p.title.clone())
            .unwrap_or_else(|| {
                instrukt_runner::runtime::thread_title(&format!("{role} · {todo}"), &entry.key)
            });
        let EntryAdmission {
            admission,
            number,
            key,
            ..
        } = entry;
        let Placement { branch, path, .. } = placement;
        let lane = instrukt_runner::interfaces::Lane::Work;
        let admitted = admission != Admission::Describe;
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
        let (from, base) = if let Admission::Bound { .. } = admission {
            (format!("origin/{default}"), head(&project.root, &branch)?)
        } else {
            let (from, base) = phase_base(&project.root, &branch)?;
            (
                from,
                if admitted {
                    head(&project.root, &branch)?
                } else {
                    base
                },
            )
        };
        if admission == (Admission::Bound { reopen: true }) {
            s.commit("resume recorded driver", |s| {
                let binding = s.items[index]
                    .record
                    .bindings
                    .iter_mut()
                    .find(|b| b.attempt.semantic_key == key)
                    .ok_or_else(|| WorkError::refused("recorded phase disappeared"))?;
                binding.attempt.released_at = None;
                Ok(())
            })?;
        }
        if admission == Admission::Place {
            let now = s.now;
            s.commit("todo workspace placement", |s| {
                let record = &mut s.items[index].record;
                // Retire any unreleased legacy prepare-orchestrator driver binding
                // (and any unbound legacy work-orchestrator reservation) now that
                // the unified todo-orchestrator owns `<slug>/work`.
                for b in &mut record.bindings {
                    if b.attempt.assignment == AssignmentKind::Driver
                        && b.attempt.released_at.is_none()
                        && (b.role.as_str() == "prepare-orchestrator"
                            || (b.role.as_str() == "work-orchestrator" && b.session.is_none()))
                    {
                        b.attempt.released_at = Some(now);
                    }
                }
                record.bindings.push(Binding {
                    role: role.clone(),
                    session: caller.clone(),
                    worktree: path.clone(),
                    attempt: Attempt {
                        runtime: Some(runtime),
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
                Ok(())
            })?;
        }
        let selection = if admitted {
            None
        } else {
            let domain = s.config.workflow_domain()?;
            let procedure = instrukt_core::index::GlobalIndex::load_readable(s.home())
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
            title,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        Actor, ArtifactName, ContentDigest, Grounding, ModelProvenance, StampFact, StampRecord,
    };
    use crate::work::_model::SCHEMA;
    use crate::work::{Artifact, Claim, TodoId, Version};
    use instrukt_core::{ids::ProjectSlug, machine::MachineId};
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    fn session(name: &str) -> SessionRef {
        SessionRef::parse(&format!("t3:{name}")).unwrap()
    }

    fn base_record() -> TodoRecord {
        TodoRecord {
            schema: SCHEMA,
            id: TodoId::parse(&"11".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("bug-fix").unwrap(),
            prior_slugs: Vec::new(),
            kind: TodoKind::Bug,
            status: TodoStatus::Active,
            created: jiff::Timestamp::from_second(1).unwrap(),
            created_by: Actor::Person { name: "dev".into() },
            human_involved: None,
            revision: 1,
            sequence: 1,
            artifacts: vec![Artifact {
                name: ArtifactName::new(artifact_names::DIAGNOSIS),
                address: None,
                versions: vec![Version {
                    digest: ContentDigest::of("cause-v1"),
                    writer: Actor::Session(session("diagnoser-1")),
                    created: jiff::Timestamp::from_second(2).unwrap(),
                    derived_from: Vec::new(),
                    sequence: 1,
                    grounding: Grounding::default(),
                    remote: None,
                    content: None,
                }],
            }],
            stamps: Vec::new(),
            claim: Some(Claim {
                machine: MachineId::parse("0123456789abcdef").unwrap(),
                session: Some(session("driver-1")),
                at: jiff::Timestamp::from_second(1).unwrap(),
            }),
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
            label_actions: Vec::new(),
            after: Vec::new(),
            group: None,
            schedule: None,
            catch_up: CatchUp::Once,
            agent: None,
            branch: None,
            delivery: None,
            removal: None,
        }
    }

    fn diagnoser_binding(bound: Option<SessionRef>, released: Option<u64>) -> Binding {
        Binding {
            role: RoleName::parse("diagnoser").unwrap(),
            session: bound,
            worktree: None,
            attempt: Attempt {
                assignment: AssignmentKind::Workspace,
                number: 1,
                semantic_key: "demo/bug-fix:diagnoser:1".into(),
                completion_sequence: released,
                released_at: released.map(|_| jiff::Timestamp::from_second(3).unwrap()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn diagnosis_settlement_selects_bound_diagnoser_for_claimant() {
        let mut record = base_record();
        record
            .bindings
            .push(diagnoser_binding(Some(session("diagnoser-1")), None));
        let driver = session("driver-1");
        assert_eq!(
            diagnosis_settlement_binding(&record, Some(&driver)).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn diagnosis_settlement_refuses_diagnoser_self_settlement() {
        let mut record = base_record();
        let peer = session("diagnoser-1");
        record
            .bindings
            .push(diagnoser_binding(Some(peer.clone()), None));
        assert!(matches!(
            diagnosis_settlement_binding(&record, Some(&peer)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn diagnosis_settlement_refuses_other_caller_while_claimed() {
        let mut record = base_record();
        record
            .bindings
            .push(diagnoser_binding(Some(session("diagnoser-1")), None));
        let outsider = session("other-session");
        assert!(matches!(
            diagnosis_settlement_binding(&record, Some(&outsider)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn diagnosis_settlement_refuses_unbound_diagnoser_reservation() {
        let mut record = base_record();
        record.bindings.push(diagnoser_binding(None, None));
        let driver = session("driver-1");
        assert!(matches!(
            diagnosis_settlement_binding(&record, Some(&driver)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn diagnosis_settlement_returns_none_without_diagnoser_binding() {
        let record = base_record();
        let driver = session("driver-1");
        assert_eq!(
            diagnosis_settlement_binding(&record, Some(&driver)).unwrap(),
            None
        );
    }

    #[test]
    fn diagnosis_settlement_replay_selects_released_binding_for_matching_digest() {
        let mut record = base_record();
        let digest = record
            .current_digest(artifact_names::DIAGNOSIS)
            .unwrap()
            .clone();
        record.stamps.push(StampRecord {
            source_comment: None,
            fact: StampFact::DiagnosisSettled { diagnosis: digest },
            at: jiff::Timestamp::from_second(3).unwrap(),
            role: None,
            actor: Actor::Session(session("driver-1")),
            model: None,
            eval_fingerprint: None,
            human_involved: None,
            sequence: 2,
            model_provenance: ModelProvenance::default(),
        });
        record
            .bindings
            .push(diagnoser_binding(Some(session("diagnoser-1")), Some(2)));
        let driver = session("driver-1");
        assert_eq!(
            diagnosis_settlement_binding(&record, Some(&driver)).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn diagnosis_settlement_returns_none_when_released_binding_settled_an_older_digest() {
        let mut record = base_record();
        record.stamps.push(StampRecord {
            source_comment: None,
            fact: StampFact::DiagnosisSettled {
                diagnosis: ContentDigest::of("older-cause"),
            },
            at: jiff::Timestamp::from_second(3).unwrap(),
            role: None,
            actor: Actor::Session(session("driver-1")),
            model: None,
            eval_fingerprint: None,
            human_involved: None,
            sequence: 2,
            model_provenance: ModelProvenance::default(),
        });
        record
            .bindings
            .push(diagnoser_binding(Some(session("diagnoser-1")), Some(2)));
        let driver = session("driver-1");
        assert_eq!(
            diagnosis_settlement_binding(&record, Some(&driver)).unwrap(),
            None
        );
    }

    fn drift_binding(bound: Option<SessionRef>, released: Option<u64>) -> Binding {
        Binding {
            role: RoleName::parse("drift-analyzer").unwrap(),
            session: bound,
            worktree: None,
            attempt: Attempt {
                assignment: AssignmentKind::Child,
                number: 1,
                semantic_key: "demo/bug-fix:drift-analyzer:1".into(),
                completion_sequence: released,
                released_at: released.map(|_| jiff::Timestamp::from_second(3).unwrap()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn drift_review_selects_active_bound_analyzer_for_matching_caller() {
        let mut record = base_record();
        let analyzer = session("drift-1");
        record
            .bindings
            .push(drift_binding(Some(analyzer.clone()), None));
        assert_eq!(
            drift_review_binding(&record, Some(&analyzer)).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn drift_review_refuses_wrong_caller_while_analyzer_attempt_is_active() {
        let mut record = base_record();
        record
            .bindings
            .push(drift_binding(Some(session("drift-1")), None));
        let other = session("driver-1");
        assert!(matches!(
            drift_review_binding(&record, Some(&other)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn drift_review_refuses_unbound_analyzer_reservation() {
        let mut record = base_record();
        record.bindings.push(drift_binding(None, None));
        let caller = session("drift-1");
        assert!(matches!(
            drift_review_binding(&record, Some(&caller)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn drift_review_returns_none_without_analyzer_binding() {
        let record = base_record();
        let caller = session("driver-1");
        assert_eq!(drift_review_binding(&record, Some(&caller)).unwrap(), None);
    }

    #[test]
    fn drift_review_returns_none_without_caller_when_no_active_analyzer_exists() {
        let record = base_record();
        assert_eq!(drift_review_binding(&record, None).unwrap(), None);
    }

    #[test]
    fn drift_review_selects_released_analyzer_before_later_attempt_for_multi_artifact_stamps() {
        let mut record = base_record();
        let analyzer = session("drift-1");
        record
            .bindings
            .push(drift_binding(Some(analyzer.clone()), Some(4)));
        assert_eq!(
            drift_review_binding(&record, Some(&analyzer)).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn drift_review_refuses_released_analyzer_after_later_workspace_attempt_begins() {
        let mut record = base_record();
        let analyzer = session("drift-1");
        record
            .bindings
            .push(drift_binding(Some(analyzer.clone()), Some(4)));
        record.bindings.push(Binding {
            role: RoleName::parse("planner").unwrap(),
            session: Some(session("planner-2")),
            worktree: None,
            attempt: Attempt {
                assignment: AssignmentKind::Workspace,
                number: 2,
                semantic_key: "demo/bug-fix:planner:2".into(),
                ..Default::default()
            },
        });
        assert!(matches!(
            drift_review_binding(&record, Some(&analyzer)),
            Err(WorkError::Refused(_))
        ));
    }

    fn worker_binding(role: &str, number: u64, bound: Option<SessionRef>) -> Binding {
        Binding {
            role: RoleName::parse(role).unwrap(),
            session: bound,
            worktree: None,
            attempt: Attempt {
                assignment: AssignmentKind::Workspace,
                number,
                semantic_key: format!("demo/bug-fix:{role}:{number}"),
                ..Default::default()
            },
        }
    }

    #[test]
    fn prior_session_is_the_roles_latest_earlier_bound_attempt() {
        let bindings = [
            worker_binding("planner", 1, Some(session("planner-1"))),
            worker_binding("planner", 2, Some(session("planner-2"))),
            worker_binding("planner", 3, Some(session("planner-3"))),
            worker_binding("plan-reviewer", 4, Some(session("reviewer-4"))),
        ];
        let planner = RoleName::parse("planner").unwrap();
        let prior = prior_session(&bindings, &planner, 3, instrukt_core::config::Runtime::T3);
        assert_eq!(
            prior.map(|p| (p.session, p.attempt)),
            Some((session("planner-2"), 2))
        );
    }

    #[test]
    fn prior_session_is_absent_when_no_earlier_attempt_of_the_role_was_bound() {
        let bindings = [
            worker_binding("planner", 1, None),
            worker_binding("plan-reviewer", 1, Some(session("reviewer-1"))),
        ];
        let planner = RoleName::parse("planner").unwrap();
        assert_eq!(
            prior_session(&bindings, &planner, 2, instrukt_core::config::Runtime::T3),
            None
        );
    }

    #[test]
    fn prior_session_names_a_carried_forward_worker_binding_without_its_unrecorded_facts() {
        let planner = RoleName::parse("planner").unwrap();
        let mut migrated = worker_binding("planner", 1, Some(session("planner-1")));
        migrated.attempt.assignment = AssignmentKind::Unspecified;
        let prior = prior_session(&[migrated], &planner, 2, instrukt_core::config::Runtime::T3);
        assert_eq!(
            prior,
            Some(PriorSession {
                session: session("planner-1"),
                attempt: 1,
                selection: None,
                dispatcher: None,
            })
        );
    }

    #[test]
    fn prior_session_carries_the_dispatcher_its_attempt_recorded() {
        let planner = RoleName::parse("planner").unwrap();
        let mut bound = worker_binding("planner", 1, Some(session("planner-1")));
        bound.attempt.native_assignment = Some(NativeAssignment {
            dispatcher: Some(session("lead-1")),
            selection: None,
            runtime: instrukt_core::config::Runtime::T3,
            dispatch_allowed: true,
            brief: "Plan the change.".into(),
            procedure: None,
            args: None,
            role: planner.clone(),
            attempt: 1,
            semantic_key: "demo/bug-fix:planner:1".into(),
            title: "planner".into(),
            worktree: None,
            branch: None,
            base: None,
        });
        let prior = prior_session(&[bound], &planner, 2, instrukt_core::config::Runtime::T3);
        assert_eq!(prior.and_then(|p| p.dispatcher), Some(session("lead-1")));
    }

    /// A task record: its child attempts complete without a shared
    /// worktree, so admission decides before any Git inspection.
    fn task_record(bindings: Vec<Binding>) -> TodoRecord {
        TodoRecord {
            kind: TodoKind::Task,
            bindings,
            ..base_record()
        }
    }

    fn child_binding(role: &str, number: u64, bound: &str) -> Binding {
        let mut binding = worker_binding(role, number, Some(session(bound)));
        binding.attempt.assignment = AssignmentKind::Child;
        binding
    }

    fn admit(
        record: &TodoRecord,
        caller: Option<&str>,
        request: &crate::work::StampRequest,
    ) -> Result<Option<usize>, WorkError> {
        let caller = caller.map(session);
        completion_binding(record, caller.as_ref(), Path::new("/nonexistent"), request)
    }

    fn preparation_verdict(phase: PreparationReview) -> crate::work::StampRequest {
        crate::work::StampRequest::PreparationVerdict {
            phase,
            verdict: crate::evidence::ReviewVerdict::Approve,
        }
    }

    fn disposition() -> crate::work::StampRequest {
        crate::work::StampRequest::VerificationDisposition {
            disposition: crate::evidence::VerificationDisposition::Pass,
            follow_up: None,
        }
    }

    fn build_verdict(verdict: crate::evidence::BuildVerdict) -> crate::work::StampRequest {
        crate::work::StampRequest::BuildVerdict { verdict }
    }

    fn refused(result: Result<Option<usize>, WorkError>) -> bool {
        matches!(result, Err(WorkError::Refused(_)))
    }

    #[test]
    fn a_request_that_completes_no_attempt_binds_none_of_the_callers_attempts() {
        let record = task_record(vec![child_binding("builder", 1, "builder-1")]);
        assert_eq!(
            admit(
                &record,
                Some("builder-1"),
                &crate::work::StampRequest::FinalizeReady
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_completion_without_a_session_is_refused() {
        let record = task_record(Vec::new());
        assert!(refused(admit(
            &record,
            None,
            &crate::work::StampRequest::BuildComplete
        )));
    }

    #[test]
    fn a_record_only_completion_without_a_session_binds_no_attempt() {
        let record = task_record(Vec::new());
        assert_eq!(admit(&record, None, &disposition()).unwrap(), None);
    }

    #[test]
    fn a_session_without_the_requests_role_is_refused() {
        let record = task_record(vec![child_binding("builder", 1, "builder-1")]);
        assert!(refused(admit(
            &record,
            Some("builder-1"),
            &crate::work::StampRequest::DeferralsProcessed
        )));
    }

    #[test]
    fn a_record_only_completion_by_a_session_without_its_role_binds_no_attempt() {
        let record = task_record(vec![child_binding("builder", 1, "builder-1")]);
        assert_eq!(
            admit(
                &record,
                Some("builder-1"),
                &preparation_verdict(PreparationReview::PrdReview)
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_completion_completes_the_callers_latest_attempt_of_its_role() {
        let record = task_record(vec![
            child_binding("builder", 1, "worker"),
            child_binding("builder", 2, "worker"),
            child_binding("builder", 3, "other"),
        ]);
        assert_eq!(
            admit(
                &record,
                Some("worker"),
                &crate::work::StampRequest::BuildComplete
            )
            .unwrap(),
            Some(1)
        );
    }

    #[test]
    fn a_preparation_verdict_completes_the_reviewer_attempt_of_its_phase() {
        let record = task_record(vec![child_binding("plan-reviewer", 1, "reviewer-1")]);
        assert_eq!(
            admit(
                &record,
                Some("reviewer-1"),
                &preparation_verdict(PreparationReview::PlanReview)
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn a_build_approval_by_the_claim_holder_is_refused() {
        // `driver-1` holds the todo's claim (`base_record`).
        let record = task_record(vec![child_binding("build-reviewer", 1, "driver-1")]);
        assert!(refused(admit(
            &record,
            Some("driver-1"),
            &build_verdict(crate::evidence::BuildVerdict::Approve)
        )));
    }

    #[test]
    fn a_build_change_request_by_the_claim_holder_completes_its_attempt() {
        // `driver-1` holds the todo's claim (`base_record`).
        let record = task_record(vec![child_binding("build-reviewer", 1, "driver-1")]);
        assert_eq!(
            admit(
                &record,
                Some("driver-1"),
                &build_verdict(crate::evidence::BuildVerdict::ChangesRequested)
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn a_build_approval_by_an_independent_reviewer_completes_its_attempt() {
        let record = task_record(vec![child_binding("build-reviewer", 1, "reviewer-1")]);
        assert_eq!(
            admit(
                &record,
                Some("reviewer-1"),
                &build_verdict(crate::evidence::BuildVerdict::Approve)
            )
            .unwrap(),
            Some(0)
        );
    }

    fn released_builder(completion_sequence: Option<u64>) -> Binding {
        let mut released = child_binding("builder", 1, "builder-1");
        released.attempt.released_at = Some(jiff::Timestamp::from_second(4).unwrap());
        released.attempt.completion_sequence = completion_sequence;
        released
    }

    #[test]
    fn a_released_attempt_without_its_recorded_completion_is_refused() {
        let record = task_record(vec![released_builder(None)]);
        assert!(refused(admit(
            &record,
            Some("builder-1"),
            &crate::work::StampRequest::BuildComplete
        )));
    }

    #[test]
    fn a_released_attempt_with_its_recorded_completion_completes_again() {
        let record = task_record(vec![released_builder(Some(2))]);
        assert_eq!(
            admit(
                &record,
                Some("builder-1"),
                &crate::work::StampRequest::BuildComplete
            )
            .unwrap(),
            Some(0)
        );
    }

    /// A clean shared worktree on branch `work`: the head an attempt
    /// captured, and the head one further commit moved it to.
    fn shared_worktree() -> (tempfile::TempDir, CommitSha, CommitSha) {
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        for args in [
            &["init", "-q", "-b", "work"][..],
            &["config", "user.name", "Test"],
            &["config", "user.email", "test@example.com"],
            &["config", "commit.gpgsign", "false"],
            &["commit", "-q", "--allow-empty", "-m", "captured"],
        ] {
            git::run(dir, args.iter().copied()).unwrap();
        }
        let captured = head(dir, "HEAD").unwrap();
        git::run(dir, ["commit", "-q", "--allow-empty", "-m", "moved"]).unwrap();
        let moved = head(dir, "HEAD").unwrap();
        (repo, captured, moved)
    }

    /// A record whose one attempt of `role`, bound to `bound`, shares
    /// `tree` on branch `work` from the captured head `base`.
    fn shared_record(role: &str, bound: &str, tree: &Path, base: &CommitSha) -> TodoRecord {
        let mut binding = worker_binding(role, 1, Some(session(bound)));
        binding.worktree = Some(tree.to_path_buf());
        binding.attempt.branch = Some("work".into());
        binding.attempt.base_head = Some(base.clone());
        TodoRecord {
            bindings: vec![binding],
            ..base_record()
        }
    }

    #[test]
    fn a_read_only_completion_at_its_captured_head_completes_its_attempt() {
        let (repo, _, moved) = shared_worktree();
        let tree = repo.path();
        let record = shared_record("build-reviewer", "reviewer-1", tree, &moved);
        assert_eq!(
            completion_binding(
                &record,
                Some(&session("reviewer-1")),
                tree,
                &build_verdict(crate::evidence::BuildVerdict::ChangesRequested)
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn a_read_only_completion_is_refused_once_the_shared_head_moved_past_its_capture() {
        let (repo, captured, _) = shared_worktree();
        let tree = repo.path();
        let record = shared_record("build-reviewer", "reviewer-1", tree, &captured);
        assert!(refused(completion_binding(
            &record,
            Some(&session("reviewer-1")),
            tree,
            &build_verdict(crate::evidence::BuildVerdict::ChangesRequested)
        )));
    }

    #[test]
    fn a_writer_completes_at_the_head_its_own_commits_moved_to() {
        let (repo, captured, _) = shared_worktree();
        let tree = repo.path();
        let record = shared_record("builder", "builder-1", tree, &captured);
        assert_eq!(
            completion_binding(
                &record,
                Some(&session("builder-1")),
                tree,
                &crate::work::StampRequest::BuildComplete
            )
            .unwrap(),
            Some(0)
        );
    }

    /// A record holding both a bound `diagnoser` and a bound
    /// `drift-analyzer` attempt, so each request must reach its own selector.
    fn diagnoser_and_analyzer_record() -> TodoRecord {
        task_record(vec![
            child_binding("diagnoser", 1, "diagnoser-1"),
            child_binding("drift-analyzer", 2, "analyzer-1"),
        ])
    }

    #[test]
    fn a_diagnosis_settlement_by_the_claimant_completes_the_diagnoser_attempt() {
        // `driver-1` holds the todo's claim (`base_record`).
        assert_eq!(
            admit(
                &diagnoser_and_analyzer_record(),
                Some("driver-1"),
                &crate::work::StampRequest::DiagnosisSettled
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn a_drift_review_by_the_bound_analyzer_completes_its_attempt() {
        let drift = crate::work::StampRequest::DriftReviewed {
            artifact: ArtifactName::new(artifact_names::PRD),
            class: crate::evidence::DriftClass::Cosmetic,
            reason: "wording".into(),
        };
        assert_eq!(
            admit(&diagnoser_and_analyzer_record(), Some("analyzer-1"), &drift).unwrap(),
            Some(1)
        );
    }

    fn this_machine() -> MachineId {
        // The computer `base_record`'s claim is held on.
        MachineId::parse("0123456789abcdef").unwrap()
    }

    fn orchestrator_role() -> RoleName {
        RoleName::parse("todo-orchestrator").unwrap()
    }

    /// A todo-orchestrator driver attempt bound to `bound` in `tree` on
    /// branch `work`, recorded under the T3 runtime.
    fn driver_binding(bound: &str, tree: &Path, released: bool) -> Binding {
        Binding {
            role: orchestrator_role(),
            session: Some(session(bound)),
            worktree: Some(tree.to_path_buf()),
            attempt: Attempt {
                runtime: Some(instrukt_core::config::Runtime::T3),
                assignment: AssignmentKind::Driver,
                number: 1,
                semantic_key: "demo/bug-fix:todo-orchestrator:1".into(),
                branch: Some("work".into()),
                released_at: released.then(|| jiff::Timestamp::from_second(4).unwrap()),
                ..Default::default()
            },
        }
    }

    /// A record whose claim, when present, is this computer's unassigned
    /// runner claim.
    fn runner_claimed(mut record: TodoRecord) -> TodoRecord {
        if let Some(claim) = record.claim.as_mut() {
            claim.session = None;
        }
        record
    }

    /// The caller standing in `tree`, the registered worktree of `work`.
    fn in_workspace(tree: &Path) -> Placement {
        Placement {
            branch: "work".into(),
            path: Some(tree.to_path_buf()),
            root: tree.to_path_buf(),
            in_workspace: true,
        }
    }

    fn decide<'a>(
        record: &'a TodoRecord,
        role: &'a RoleName,
        caller: Option<&'a SessionRef>,
        placement: &Placement,
        evidence: Option<&DriverPlacement>,
    ) -> Result<EntryAdmission<'a>, WorkError> {
        let claim = driver_claim(record, role, &this_machine(), caller)?;
        admit_driver(
            claim,
            placement,
            evidence,
            instrukt_core::config::Runtime::T3,
            "demo",
        )
    }

    fn admission_of(result: Result<EntryAdmission<'_>, WorkError>) -> Option<Admission> {
        result.ok().map(|entry| entry.admission)
    }

    #[test]
    fn driver_claim_admits_the_claim_holders_own_session_without_takeover() {
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        let claim = driver_claim(&record, &role, &this_machine(), Some(&caller)).unwrap();
        assert!(!claim.takeover);
    }

    #[test]
    fn driver_claim_marks_this_computers_unassigned_claim_for_takeover() {
        let record = runner_claimed(base_record());
        let role = orchestrator_role();
        let caller = session("driver-1");
        let claim = driver_claim(&record, &role, &this_machine(), Some(&caller)).unwrap();
        assert!(claim.takeover);
    }

    #[test]
    fn driver_claim_refuses_a_claim_held_by_another_session() {
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("other-session");
        assert!(matches!(
            driver_claim(&record, &role, &this_machine(), Some(&caller)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn driver_claim_refuses_a_claim_held_on_another_computer() {
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        let elsewhere = MachineId::parse("fedcba9876543210").unwrap();
        assert!(matches!(
            driver_claim(&record, &role, &elsewhere, Some(&caller)),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn driver_claim_refuses_a_callerless_entry_into_claimed_work() {
        let record = base_record();
        let role = orchestrator_role();
        assert!(matches!(
            driver_claim(&record, &role, &this_machine(), None),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn driver_claim_branch_continues_the_recorded_drivers_branch() {
        let tree = tempfile::tempdir().unwrap();
        let mut record = base_record();
        record
            .bindings
            .push(driver_binding("driver-1", tree.path(), true));
        let role = orchestrator_role();
        let caller = session("driver-1");
        let claim = driver_claim(&record, &role, &this_machine(), Some(&caller)).unwrap();
        assert_eq!(claim.branch(&record.slug), "work");
    }

    #[test]
    fn driver_claim_branch_derives_the_work_branch_without_a_driver() {
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        let claim = driver_claim(&record, &role, &this_machine(), Some(&caller)).unwrap();
        assert_eq!(claim.branch(&record.slug), "bug-fix/work");
    }

    #[test]
    fn admission_describes_a_new_attempt_to_a_caller_outside_the_workspace() {
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        let outside = Placement {
            branch: "bug-fix/work".into(),
            path: None,
            root: PathBuf::from("/nonexistent"),
            in_workspace: false,
        };
        let entry = decide(&record, &role, Some(&caller), &outside, None).unwrap();
        assert_eq!(entry.admission, Admission::Describe);
        assert!(entry.driver.is_none());
        assert_eq!(entry.number, 1);
        assert_eq!(
            entry.key,
            keyed::todo_key("demo", record.id.as_str(), "todo-orchestrator", 1)
        );
    }

    #[test]
    fn admission_refuses_an_unbound_caller_in_the_workspace_without_evidence() {
        let (repo, _, _) = shared_worktree();
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        assert!(matches!(
            decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                None
            ),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn admission_admits_the_session_already_bound_in_its_placement() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        record
            .bindings
            .push(driver_binding("driver-1", repo.path(), false));
        let role = orchestrator_role();
        let caller = session("driver-1");
        let entry = decide(
            &record,
            &role,
            Some(&caller),
            &in_workspace(repo.path()),
            None,
        )
        .unwrap();
        assert_eq!(entry.admission, Admission::Bound { reopen: false });
        assert_eq!(entry.key, "demo/bug-fix:todo-orchestrator:1");
    }

    #[test]
    fn admission_reopens_the_recorded_orchestrators_released_binding() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        record
            .bindings
            .push(driver_binding("driver-1", repo.path(), true));
        let role = orchestrator_role();
        let caller = session("driver-1");
        let entry = decide(
            &record,
            &role,
            Some(&caller),
            &in_workspace(repo.path()),
            None,
        )
        .unwrap();
        assert_eq!(entry.admission, Admission::Bound { reopen: true });
        assert_eq!(entry.number, 1);
        assert_eq!(entry.key, "demo/bug-fix:todo-orchestrator:1");
    }

    #[test]
    fn admission_lets_the_recorded_orchestrator_take_up_the_unassigned_runner_claim() {
        let (repo, _, _) = shared_worktree();
        let mut record = runner_claimed(base_record());
        record
            .bindings
            .push(driver_binding("driver-1", repo.path(), true));
        let role = orchestrator_role();
        let caller = session("driver-1");
        assert_eq!(
            admission_of(decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                None
            )),
            Some(Admission::Bound { reopen: true })
        );
    }

    #[test]
    fn admission_refuses_a_runner_claim_takeover_by_another_session() {
        let (repo, _, _) = shared_worktree();
        let mut record = runner_claimed(base_record());
        record
            .bindings
            .push(driver_binding("driver-1", repo.path(), true));
        let role = orchestrator_role();
        let caller = session("other-session");
        assert!(matches!(
            decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                None
            ),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn admission_does_not_resume_a_recorded_binding_without_its_runtime() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        let mut legacy = driver_binding("driver-1", repo.path(), true);
        legacy.attempt.runtime = None;
        record.bindings.push(legacy);
        let role = orchestrator_role();
        let caller = session("driver-1");
        assert!(matches!(
            decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                None
            ),
            Err(WorkError::Refused(_))
        ));
    }

    /// Native placement evidence for `caller` in `tree` at its head, for
    /// the record's first new todo-orchestrator attempt.
    fn placement_evidence(record: &TodoRecord, caller: &str, tree: &Path) -> DriverPlacement {
        let key = keyed::todo_key("demo", record.id.as_str(), "todo-orchestrator", 1);
        DriverPlacement {
            title: "todo-orchestrator · bug-fix".into(),
            session: session(caller),
            key,
            worktree: tree.to_path_buf(),
            branch: "work".into(),
            head: head(tree, "HEAD").unwrap(),
            source: "native".into(),
        }
    }

    #[test]
    fn admission_places_a_caller_on_matching_native_evidence() {
        let (repo, _, _) = shared_worktree();
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        let proof = placement_evidence(&record, "driver-1", repo.path());
        assert_eq!(
            admission_of(decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                Some(&proof)
            )),
            Some(Admission::Place)
        );
    }

    #[test]
    fn admission_refuses_evidence_that_differs_from_the_recorded_assignment() {
        let (repo, _, _) = shared_worktree();
        let record = base_record();
        let role = orchestrator_role();
        let caller = session("driver-1");
        let mut proof = placement_evidence(&record, "driver-1", repo.path());
        proof.key = "demo/bug-fix:todo-orchestrator:9".into();
        assert!(matches!(
            decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                Some(&proof)
            ),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn admission_refuses_evidence_while_the_phase_is_bound_to_another_orchestrator() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        record
            .bindings
            .push(driver_binding("other-session", repo.path(), false));
        let role = orchestrator_role();
        let caller = session("driver-1");
        let mut proof = placement_evidence(&record, "driver-1", repo.path());
        proof.key = "demo/bug-fix:todo-orchestrator:1".into();
        assert!(matches!(
            decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                Some(&proof)
            ),
            Err(WorkError::Refused(_))
        ));
    }

    /// The released driver binding of the retired `work-orchestrator` role,
    /// bound to `bound` in `tree`.
    fn retired_driver_binding(bound: &str, tree: &Path) -> Binding {
        let mut binding = driver_binding(bound, tree, true);
        binding.role = RoleName::parse("work-orchestrator").unwrap();
        binding.attempt.semantic_key = "demo/bug-fix:work-orchestrator:1".into();
        binding
    }

    #[test]
    fn admission_refuses_evidence_from_the_retired_role_recorded_orchestrator() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        record
            .bindings
            .push(retired_driver_binding("driver-1", repo.path()));
        let role = orchestrator_role();
        let caller = session("driver-1");
        let proof = placement_evidence(&record, "driver-1", repo.path());
        assert!(matches!(
            decide(
                &record,
                &role,
                Some(&caller),
                &in_workspace(repo.path()),
                Some(&proof)
            ),
            Err(WorkError::Refused(_))
        ));
    }

    #[test]
    fn admission_reopens_the_retired_role_recorded_orchestrator_without_evidence() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        record
            .bindings
            .push(retired_driver_binding("driver-1", repo.path()));
        let role = orchestrator_role();
        let caller = session("driver-1");
        let entry = decide(
            &record,
            &role,
            Some(&caller),
            &in_workspace(repo.path()),
            None,
        )
        .unwrap();
        assert_eq!(entry.admission, Admission::Bound { reopen: true });
        assert_eq!(entry.key, "demo/bug-fix:work-orchestrator:1");
    }

    #[test]
    fn admission_refuses_evidence_without_a_caller() {
        let (repo, _, _) = shared_worktree();
        let mut record = base_record();
        record.claim = None;
        let role = orchestrator_role();
        let proof = placement_evidence(&record, "driver-1", repo.path());
        assert!(matches!(
            decide(
                &record,
                &role,
                None,
                &in_workspace(repo.path()),
                Some(&proof)
            ),
            Err(WorkError::Refused(_))
        ));
    }
}
