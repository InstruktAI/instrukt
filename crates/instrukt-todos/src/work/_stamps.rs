//! The admitted stamp forms and the evidence each binds (`stamps`,
//! "Mutation boundary" and "Stamp forms"; `approvals`, "Command
//! boundary"). A form resolves against the current record and the
//! caller's worktree into one [`StampFact`], or is refused naming what is
//! missing.

use std::path::Path;

use instrukt_core::git;
use instrukt_core::ids::{CommitSha, SessionRef};
use instrukt_runner::dispatch::TASK_RUNNER_ROLE;
use instrukt_runner::interfaces::TodoKind;
use jiff::Timestamp;

use super::_error::WorkError;
use super::_model::TodoRecord;
use crate::evidence::{
    Actor, ApprovalGate, ApprovedBy, ArtifactName, BuildVerdict, ContentDigest, DriftClass,
    PreparationReview, ReviewVerdict, StampFact, TaskResult, artifact_names,
};

/// One admitted stamp form, as `instrukt todo stamp <slug>` takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampRequest {
    /// `--verdict approve|needs_work --phase prd_review|architecture_review|plan_review`.
    PreparationVerdict {
        phase: PreparationReview,
        verdict: ReviewVerdict,
    },
    /// `--verdict approve|changes_requested --phase build_review`.
    BuildVerdict { verdict: BuildVerdict },
    /// `--prd-approved|--task-approved --by human|peer [--session
    /// <runtime:id>] [--reasoning <text>]`; a peer approval names its
    /// session and reasoning.
    Approval {
        gate: ApprovalGate,
        by: ApprovedBy,
        session: Option<SessionRef>,
        reasoning: Option<String>,
    },
    /// `--prd-approved|--task-approved --carry-from <stamp-id>`: an earlier
    /// approval carried to the current evidence (`approvals`).
    CarriedApproval { gate: ApprovalGate, from: u64 },
    /// `--intent-checked`.
    IntentChecked,
    /// `--kickback --reason <text>`.
    Kickback { reason: String },
    /// `--architecture-skipped <reason>`.
    ArchitectureSkipped { reason: String },
    /// `--diagnosis-settled`.
    DiagnosisSettled,
    /// `--diagnosis-unresolved --reason <text>`.
    DiagnosisUnresolved { reason: String },
    /// `--drift-reviewed <artifact> cosmetic|substantive <reason>`.
    DriftReviewed {
        artifact: ArtifactName,
        class: DriftClass,
        reason: String,
    },
    /// `--build-started`.
    BuildStarted,
    /// `--build-complete`.
    BuildComplete,
    /// `--deferrals-processed`.
    DeferralsProcessed,
    /// `--finalize-ready`.
    FinalizeReady,
    /// `--task-firing-started [--occurrence <RFC 3339>]`: the occurrence
    /// the run executes; now when not given (deliberate pickup).
    TaskFiringStarted { occurrence: Option<Timestamp> },
    /// `--task-outcome success|error [--reason <text>] [--occurrence <RFC 3339>]`.
    TaskOutcome {
        result: TaskResult,
        reason: Option<String>,
        occurrence: Option<Timestamp>,
    },
}

/// What resolving a form needs besides the record.
pub(crate) struct StampInputs<'a> {
    /// The caller's working directory: the worktree whose head commit-bound
    /// stamps name.
    pub cwd: &'a Path,
    /// The project's default branch, from which a build starts.
    pub base_branch: &'a str,
    /// The acting session, when the caller runs in one.
    pub session: Option<&'a SessionRef>,
    pub now: Timestamp,
}

/// What a stamp form resolved to: the fact, and the effects that come with
/// it on the record.
pub(crate) struct Resolved {
    pub fact: StampFact,
    /// The work branch a build-start stamp records.
    pub branch: Option<String>,
    /// The actor an approval is attributed to, when not the caller.
    pub actor: Option<Actor>,
    /// The fact is already recorded (a start the runner stamped for the
    /// caller's run): nothing new is written.
    pub already_recorded: bool,
    /// The outcome ends the execution that holds the claim: the claim is
    /// released (a claim covers one occurrence's run).
    pub releases_claim: bool,
}

pub(crate) fn resolve(
    request: &StampRequest,
    record: &TodoRecord,
    inputs: &StampInputs<'_>,
) -> Result<Resolved, WorkError> {
    let plain = |fact| Resolved {
        fact,
        branch: None,
        actor: None,
        already_recorded: false,
        releases_claim: false,
    };
    Ok(match request {
        StampRequest::PreparationVerdict { phase, verdict } => {
            let (target, report) = match phase {
                PreparationReview::PrdReview => {
                    (artifact_names::PRD, artifact_names::PRD_REVIEW_FINDINGS)
                }
                PreparationReview::ArchitectureReview => (
                    artifact_names::ARCHITECTURE,
                    artifact_names::ARCHITECTURE_REVIEW_FINDINGS,
                ),
                PreparationReview::PlanReview => (
                    plan_artifact(record.kind),
                    artifact_names::PLAN_REVIEW_FINDINGS,
                ),
            };
            plain(StampFact::PreparationVerdict {
                phase: *phase,
                verdict: *verdict,
                target: current(record, target)?,
                report: current(record, report)?,
            })
        }
        StampRequest::BuildVerdict { verdict } => plain(StampFact::BuildVerdict {
            verdict: *verdict,
            target: head(inputs.cwd)?,
            report: current(record, artifact_names::REVIEW_FINDINGS)?,
        }),
        StampRequest::Approval {
            gate,
            by,
            session,
            reasoning,
        } => {
            let target = approval_target(record, *gate)?;
            let reasoning = reasoning
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_owned);
            let session = session.clone().or_else(|| inputs.session.cloned());
            if *by == ApprovedBy::Peer {
                // A peer approval is the peer session's judgment, with its
                // reasoning, of the evidence it approved.
                if session.is_none() {
                    return Err(WorkError::refused(
                        "a peer approval names the peer session that judged it: run it in that session, or pass `--session <runtime:id>`",
                    ));
                }
                if reasoning.is_none() {
                    return Err(WorkError::refused(
                        "a peer approval records the peer's reasoning with `--reasoning <text>`",
                    ));
                }
            } else if reasoning.is_some() {
                return Err(WorkError::refused(
                    "`--reasoning` records a peer's judgment; a human approval carries none",
                ));
            }
            Resolved {
                fact: StampFact::Approval {
                    gate: *gate,
                    by: *by,
                    target,
                    carried_from: None,
                    reasoning,
                },
                branch: None,
                actor: session.map(Actor::Session),
                already_recorded: false,
                releases_claim: false,
            }
        }
        StampRequest::CarriedApproval { gate, from } => {
            let target = approval_target(record, *gate)?;
            let earlier = record
                .stamps
                .iter()
                .find(|s| s.sequence == *from)
                .ok_or_else(|| {
                    WorkError::refused(format!(
                        "this todo has no stamp #{from}; `instrukt todo show` numbers its stamps"
                    ))
                })?;
            let StampFact::Approval {
                gate: earlier_gate,
                by,
                target: earlier_target,
                reasoning,
                ..
            } = &earlier.fact
            else {
                return Err(WorkError::refused(format!(
                    "stamp #{from} is no approval; a carry carries an earlier approval"
                )));
            };
            if earlier_gate != gate {
                return Err(WorkError::refused(format!(
                    "stamp #{from} approves another gate"
                )));
            }
            // Carry is accepted only under a current independent review's
            // approval of the new evidence.
            let reviewed = record.stamps.iter().rev().find_map(|s| match &s.fact {
                StampFact::PreparationVerdict {
                    phase,
                    verdict,
                    target: reviewed,
                    ..
                } if *phase == review_of(*gate) => Some((*verdict, reviewed.clone())),
                _ => None,
            });
            if reviewed != Some((ReviewVerdict::Approve, target.clone())) {
                return Err(WorkError::refused(format!(
                    "carrying an approval needs a current `{}` verdict `approve` binding the current {} ({target}); none is recorded",
                    match review_of(*gate) {
                        PreparationReview::PrdReview => "prd_review",
                        PreparationReview::ArchitectureReview => "architecture_review",
                        PreparationReview::PlanReview => "plan_review",
                    },
                    match gate {
                        ApprovalGate::Prd => artifact_names::PRD,
                        ApprovalGate::Task => artifact_names::TASK_PLAN,
                    }
                )));
            }
            Resolved {
                fact: StampFact::Approval {
                    gate: *gate,
                    by: *by,
                    target,
                    carried_from: Some(earlier_target.clone()),
                    reasoning: reasoning.clone(),
                },
                branch: None,
                // The original approver stays attributed.
                actor: Some(earlier.actor.clone()),
                already_recorded: false,
                releases_claim: false,
            }
        }
        StampRequest::IntentChecked => {
            let input = record
                .artifact(artifact_names::INPUT)
                .and_then(|a| a.versions.last())
                .ok_or_else(|| missing(artifact_names::INPUT))?;
            let Actor::Session(conversation) = &input.writer else {
                return Err(WorkError::refused(
                    "the current `input` version names no originating conversation to check it against; record a new `input` version from that conversation",
                ));
            };
            let Some(checker) = inputs.session else {
                return Err(WorkError::refused(
                    "an intent check is recorded by the checking agent's session; no agent session is active",
                ));
            };
            if checker == conversation {
                return Err(WorkError::refused(
                    "the author of this `input` version cannot check it; its first other recipient does",
                ));
            }
            plain(StampFact::IntentChecked {
                input: input.digest.clone(),
                conversation: conversation.clone(),
            })
        }
        StampRequest::Kickback { reason } => plain(StampFact::Kickback {
            reason: required(reason, "--reason")?,
            input: current(record, artifact_names::INPUT)?,
            invalidated_plan: record.current_digest(plan_artifact(record.kind)).cloned(),
            invalidated_reviews: crate::evidence::plan_review_only(),
        }),
        StampRequest::ArchitectureSkipped { reason } => plain(StampFact::ArchitectureSkipped {
            reason: required(reason, "the skip reason")?,
        }),
        StampRequest::DiagnosisSettled => plain(StampFact::DiagnosisSettled {
            diagnosis: current(record, artifact_names::DIAGNOSIS)?,
        }),
        StampRequest::DiagnosisUnresolved { reason } => plain(StampFact::DiagnosisUnresolved {
            diagnosis: current(record, artifact_names::DIAGNOSIS)?,
            reason: required(reason, "--reason")?,
        }),
        StampRequest::DriftReviewed {
            artifact,
            class,
            reason,
        } => plain(StampFact::DriftReviewed {
            artifact: artifact.clone(),
            class: *class,
            reason: required(reason, "the drift reason")?,
            examined: current(record, artifact.as_str())?,
        }),
        StampRequest::BuildStarted => {
            let plan = current(record, plan_artifact(record.kind))?;
            // Recovery preserves the original base.
            let original = record.stamps.iter().find_map(|s| match &s.fact {
                StampFact::BuildStarted { base, .. } => Some(base.clone()),
                _ => None,
            });
            let base = match original {
                Some(base) => base,
                None => merge_base(inputs.cwd, inputs.base_branch)?,
            };
            Resolved {
                fact: StampFact::BuildStarted { base, plan },
                branch: Some(branch(inputs.cwd)?),
                already_recorded: false,
                releases_claim: false,
                actor: None,
            }
        }
        StampRequest::BuildComplete => plain(StampFact::BuildComplete {
            head: head(inputs.cwd)?,
        }),
        StampRequest::DeferralsProcessed => plain(StampFact::DeferralsProcessed {
            deferrals: current(record, artifact_names::DEFERRALS)?,
        }),
        StampRequest::FinalizeReady => plain(StampFact::FinalizeReady {
            commit: head(inputs.cwd)?,
        }),
        StampRequest::TaskFiringStarted { occurrence } => {
            task_only(record)?;
            let occurrence = occurrence.unwrap_or(inputs.now);
            // The runner stamped this start for the caller's run already.
            let already_recorded = match (firings(record).last(), inputs.session) {
                (Some(latest), Some(caller)) => {
                    latest.occurrence == occurrence
                        && latest.actor == &Actor::Session(caller.clone())
                        && !has_outcome(record, occurrence)
                }
                _ => false,
            };
            Resolved {
                already_recorded,
                ..plain(StampFact::TaskFiringStarted { occurrence })
            }
        }
        StampRequest::TaskOutcome {
            result,
            reason,
            occurrence,
        } => {
            task_only(record)?;
            let stale = |why: &str| {
                WorkError::refused(format!(
                    "this outcome is stale or not this session's: {why}; an outcome is recorded by the session running the current occurrence"
                ))
            };
            let Some(caller) = inputs.session else {
                return Err(stale("no agent session is active"));
            };
            let all = firings(record);
            let mine = all
                .iter()
                .rev()
                .find(|f| {
                    f.actor == &Actor::Session(caller.clone())
                        && occurrence.is_none_or(|o| o == f.occurrence)
                })
                .ok_or_else(|| stale("this session started no firing of that occurrence"))?;
            if has_outcome(record, mine.occurrence) {
                return Err(stale("that occurrence already has its outcome"));
            }
            let latest = all
                .last()
                .map(|f| f.occurrence)
                .ok_or_else(|| stale("the task has no firing"))?;
            if mine.occurrence != latest {
                return Err(stale("a later occurrence has started since"));
            }
            // The claim is this execution's: the runner's claim with this
            // session bound as the task runner, or this session's own.
            let releases_claim = record
                .claim
                .as_ref()
                .is_some_and(|claim| match &claim.session {
                    Some(holder) => holder == caller,
                    None => record
                        .bindings
                        .iter()
                        .any(|b| &b.session == caller && b.role.as_str() == TASK_RUNNER_ROLE),
                });
            let occurrence = mine.occurrence;
            let reason = reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_owned);
            if *result == TaskResult::Error && reason.is_none() {
                return Err(WorkError::refused(
                    "a failed task outcome records its failure evidence with `--reason <text>`",
                ));
            }
            Resolved {
                releases_claim,
                ..plain(StampFact::TaskOutcome {
                    occurrence,
                    result: *result,
                    reason,
                })
            }
        }
    })
}

/// The evidence gate `gate` approves: the current prd, or a task's plan.
fn approval_target(record: &TodoRecord, gate: ApprovalGate) -> Result<ContentDigest, WorkError> {
    match gate {
        ApprovalGate::Prd => current(record, artifact_names::PRD),
        ApprovalGate::Task => {
            if record.kind != TodoKind::Task {
                return Err(WorkError::refused(
                    "only a task carries a task approval; approve a repository change's prd with `--prd-approved`",
                ));
            }
            current(record, artifact_names::TASK_PLAN)
        }
    }
}

/// The independent review whose verdict binds a gate's evidence.
fn review_of(gate: ApprovalGate) -> PreparationReview {
    match gate {
        ApprovalGate::Prd => PreparationReview::PrdReview,
        ApprovalGate::Task => PreparationReview::PlanReview,
    }
}

/// One recorded task firing: its occurrence and who started it.
struct FiringStart<'r> {
    occurrence: Timestamp,
    actor: &'r Actor,
}

/// The task's firings, oldest first.
fn firings(record: &TodoRecord) -> Vec<FiringStart<'_>> {
    record
        .stamps
        .iter()
        .filter_map(|s| match &s.fact {
            StampFact::TaskFiringStarted { occurrence } => Some(FiringStart {
                occurrence: *occurrence,
                actor: &s.actor,
            }),
            _ => None,
        })
        .collect()
}

/// Whether an outcome is recorded for `occurrence`.
fn has_outcome(record: &TodoRecord, occurrence: Timestamp) -> bool {
    record.stamps.iter().any(
        |s| matches!(&s.fact, StampFact::TaskOutcome { occurrence: o, .. } if *o == occurrence),
    )
}

/// The plan artifact of a kind: a task's `task_plan`, otherwise the
/// `implementation_plan`.
fn plan_artifact(kind: TodoKind) -> &'static str {
    match kind {
        TodoKind::Task => artifact_names::TASK_PLAN,
        _ => artifact_names::IMPLEMENTATION_PLAN,
    }
}

fn current(record: &TodoRecord, name: &str) -> Result<ContentDigest, WorkError> {
    record
        .current_digest(name)
        .cloned()
        .ok_or_else(|| missing(name))
}

fn missing(name: &str) -> WorkError {
    WorkError::refused(format!(
        "the stamp binds the current `{name}` artifact, which this todo does not have; write it with `instrukt todo artifacts write <slug> {name} <content>`"
    ))
}

fn required(text: &str, what: &str) -> Result<String, WorkError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(WorkError::refused(format!(
            "the stamp records a reason; {what} is blank"
        )));
    }
    Ok(trimmed.to_owned())
}

fn task_only(record: &TodoRecord) -> Result<(), WorkError> {
    if record.kind == TodoKind::Task {
        Ok(())
    } else {
        Err(WorkError::refused(format!(
            "task firings and outcomes are recorded on tasks; `{}` is a {}",
            record.slug, record.kind
        )))
    }
}

fn head(cwd: &Path) -> Result<CommitSha, WorkError> {
    let sha = git::run(cwd, ["rev-parse", "--verify", "HEAD^{commit}"])?;
    CommitSha::parse(sha.trim()).map_err(|err| WorkError::refused(err.to_string()))
}

fn branch(cwd: &Path) -> Result<String, WorkError> {
    match git::run(cwd, ["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(name) => Ok(name.trim().to_owned()),
        Err(git::GitError::Failed { .. }) => Err(WorkError::refused(
            "a build starts on a work branch; HEAD is detached",
        )),
        Err(err) => Err(err.into()),
    }
}

fn merge_base(cwd: &Path, base_branch: &str) -> Result<CommitSha, WorkError> {
    let sha = match git::run(cwd, ["merge-base", "HEAD", base_branch]) {
        Ok(sha) => sha,
        Err(git::GitError::Failed { .. }) => {
            return Err(WorkError::refused(format!(
                "cannot find where this work branch left `{base_branch}`; a build starts from the default branch"
            )));
        }
        Err(err) => return Err(err.into()),
    };
    CommitSha::parse(sha.trim()).map_err(|err| WorkError::refused(err.to_string()))
}
