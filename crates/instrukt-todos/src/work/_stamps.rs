//! The admitted stamp forms and the evidence each binds (`stamps`,
//! "Mutation boundary" and "Stamp forms"; `approvals`, "Command
//! boundary"). A form resolves against the current record and the
//! caller's worktree into one [`StampFact`], or is refused naming what is
//! missing.

use std::path::Path;

use instrukt_core::git;
use instrukt_core::ids::{CommitSha, SessionRef};
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
    /// `--prd-approved|--task-approved --by human|peer [--session <runtime:id>]`.
    Approval {
        gate: ApprovalGate,
        by: ApprovedBy,
        session: Option<SessionRef>,
    },
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
    /// `--task-firing-started`: the occurrence due now.
    TaskFiringStarted,
    /// `--task-outcome success|error [--reason <text>]`.
    TaskOutcome {
        result: TaskResult,
        reason: Option<String>,
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
        StampRequest::Approval { gate, by, session } => {
            let target = match gate {
                ApprovalGate::Prd => current(record, artifact_names::PRD)?,
                ApprovalGate::Task => {
                    if record.kind != TodoKind::Task {
                        return Err(WorkError::refused(
                            "only a task carries a task approval; approve a repository change's prd with `--prd-approved`",
                        ));
                    }
                    current(record, artifact_names::TASK_PLAN)?
                }
            };
            Resolved {
                fact: StampFact::Approval {
                    gate: *gate,
                    by: *by,
                    target,
                    carried_from: None,
                },
                branch: None,
                actor: session.clone().map(Actor::Session),
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
        StampRequest::TaskFiringStarted => {
            task_only(record)?;
            plain(StampFact::TaskFiringStarted {
                occurrence: inputs.now,
            })
        }
        StampRequest::TaskOutcome { result, reason } => {
            task_only(record)?;
            let occurrence = record
                .stamps
                .iter()
                .rev()
                .find_map(|s| match &s.fact {
                    StampFact::TaskFiringStarted { occurrence } => Some(*occurrence),
                    _ => None,
                })
                .ok_or_else(|| {
                    WorkError::refused(
                        "no task firing has started; an outcome binds the execution `--task-firing-started` recorded",
                    )
                })?;
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
            plain(StampFact::TaskOutcome {
                occurrence,
                result: *result,
                reason,
            })
        }
    })
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
