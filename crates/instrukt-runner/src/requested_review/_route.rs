//! Routing for requested reviews (`requested-review`, Reaching the reviewer):
//! project-scoped prior-report trace (digest then canonical path), reviewer of
//! record validation under project-scoped requested-reviewer keys, and
//! pre-record target inspection.

use std::fmt;
use std::path::Path;

use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use serde::Serialize;

use crate::interfaces::VerdictStanding;
use crate::keyed::{self, WakeTarget, WakeTargetLookup};
use crate::runtime::{Presence, RuntimeAdapter};
use crate::sessions::{SessionState, SessionsError};
use crate::state::RunnerState;

use super::_admission;
use super::_store::{self, EarlierHead, TracedBy};
use super::{AskContext, AskError, Check, failed, refused};

/// Why a re-review could not reuse an earlier head's reviewer of record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ReuseImpossible {
    /// Neither the prior report's digest nor its canonical path identified an
    /// earlier head of this project.
    NoEarlierHead,
    /// The earlier head's verdict does not stand on this computer.
    VerdictDoesNotStand { earlier: EarlierHead },
    /// The earlier head's reviewer of record is not a requested reviewer of
    /// this project.
    NotRequestedReviewer { earlier: EarlierHead },
    /// The earlier head's requested reviewer was archived, deleted, or
    /// succeeded.
    ReviewerGone { earlier: EarlierHead },
}

impl ReuseImpossible {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NoEarlierHead => "no_earlier_head",
            Self::VerdictDoesNotStand { .. } => "verdict_does_not_stand",
            Self::NotRequestedReviewer { .. } => "not_requested_reviewer",
            Self::ReviewerGone { .. } => "reviewer_gone",
        }
    }

    pub fn earlier(&self) -> Option<&EarlierHead> {
        match self {
            Self::NoEarlierHead => None,
            Self::VerdictDoesNotStand { earlier }
            | Self::NotRequestedReviewer { earlier }
            | Self::ReviewerGone { earlier } => Some(earlier),
        }
    }
}

impl fmt::Display for ReuseImpossible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEarlierHead => {
                f.write_str("neither the prior report's digest nor its path identified an earlier head of this project")
            }
            Self::VerdictDoesNotStand { .. } => {
                f.write_str("the earlier head's verdict does not stand on this computer")
            }
            Self::NotRequestedReviewer { .. } => {
                f.write_str("the earlier head's reviewer of record is not a requested reviewer of this project")
            }
            Self::ReviewerGone { .. } => {
                f.write_str("the earlier head's requested reviewer was archived, deleted, or succeeded")
            }
        }
    }
}

/// How a re-review resolved its earlier head and reviewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReReviewOutcome {
    /// Reached the earlier head's reviewer of record.
    ReachedReviewer { earlier: EarlierHead },
    /// Reuse was impossible; fell back to the head's own requested reviewer.
    FreshReviewer(ReuseImpossible),
}

impl ReReviewOutcome {
    pub fn earlier(&self) -> Option<&EarlierHead> {
        match self {
            Self::ReachedReviewer { earlier } => Some(earlier),
            Self::FreshReviewer(reason) => reason.earlier(),
        }
    }
}

/// A project-scoped requested reviewer identified by tracing a prior report.
pub(super) struct TracedReviewer {
    pub earlier: EarlierHead,
    pub session: SessionRef,
    pub target: WakeTarget,
}

/// Pre-lock trace result for a re-review ask.
pub(super) enum ReReviewPlan {
    Wake(TracedReviewer),
    Fresh(ReuseImpossible),
}

pub(super) fn log_reuse_impossible(
    project: &ProjectSlug,
    head: &CommitSha,
    reason: &ReuseImpossible,
) {
    tracing::info!(
        event = "requested_review_reuse_impossible",
        project = %project,
        head = %head,
        reason = reason.as_str()
    );
}

/// Traces `prior_report` to an earlier head of the asking project and resolves
/// its reviewer of record (if bound under a requested-reviewer key of the same
/// project). Runs before claiming any `KeyGuard`.
pub(super) fn trace_rereview(
    context: &AskContext<'_>,
    state: &RunnerState,
    runtime: &str,
    head: &CommitSha,
    prior_report: &Path,
) -> Result<ReReviewPlan, AskError> {
    let slug = &context.project.slug;
    let bytes = std::fs::read(prior_report).map_err(|error| {
        AskError::Failed(format!(
            "cannot read prior report {}: {error}",
            prior_report.display()
        ))
    })?;
    let canonical_prior = _store::canonical_report(prior_report).map_err(failed)?;

    let mut trace = None;
    if let Ok(text) = std::str::from_utf8(&bytes)
        && let Some(earlier) = context
            .verdicts
            .commit_with_report(context.project, text)
            .map_err(AskError::Failed)?
        && &earlier != head
    {
        trace = Some(EarlierHead {
            head: earlier,
            traced_by: TracedBy::Digest,
        });
    }
    if trace.is_none()
        && let Some(earlier) =
            _store::latest_by_report(context.home, slug, &canonical_prior, head).map_err(failed)?
    {
        trace = Some(EarlierHead {
            head: earlier,
            traced_by: TracedBy::Path,
        });
    }

    let Some(earlier) = trace else {
        let reason = ReuseImpossible::NoEarlierHead;
        log_reuse_impossible(slug, head, &reason);
        return Ok(ReReviewPlan::Fresh(reason));
    };
    tracing::info!(
        event = "requested_review_traced",
        project = %slug,
        head = %head,
        earlier_head = %earlier.head,
        by = earlier.traced_by.as_str()
    );

    let earlier_verdict = context
        .verdicts
        .head_verdict(context.project, &earlier.head)
        .map_err(AskError::Failed)?;
    let VerdictStanding::Stands { recorder, .. } = earlier_verdict.standing else {
        let reason = ReuseImpossible::VerdictDoesNotStand { earlier };
        log_reuse_impossible(slug, head, &reason);
        return Ok(ReReviewPlan::Fresh(reason));
    };

    match keyed::wake_target(state, runtime, &recorder).map_err(failed)? {
        WakeTargetLookup::Found(target) if keyed::is_review_key_for_project(&target.key, slug) => {
            Ok(ReReviewPlan::Wake(TracedReviewer {
                earlier,
                session: recorder,
                target,
            }))
        }
        WakeTargetLookup::Found(_) => {
            let reason = ReuseImpossible::NotRequestedReviewer { earlier };
            log_reuse_impossible(slug, head, &reason);
            Ok(ReReviewPlan::Fresh(reason))
        }
        WakeTargetLookup::NotBound | WakeTargetLookup::DifferentGeneration { .. } => {
            let reason = ReuseImpossible::ReviewerGone { earlier };
            log_reuse_impossible(slug, head, &reason);
            Ok(ReReviewPlan::Fresh(reason))
        }
    }
}

/// Resolves the dispute target from the head's standing verdict, verifying the
/// recorder is bound under a requested-reviewer key of the asking project.
pub(super) fn resolve_dispute_target(
    state: &RunnerState,
    runtime: &str,
    slug: &ProjectSlug,
    head: &CommitSha,
    standing: &VerdictStanding,
) -> Result<(SessionRef, WakeTarget), AskError> {
    let VerdictStanding::Stands { recorder, .. } = standing else {
        _admission::dispute_recorder(head, standing, false)?;
        return Err(failed(format!(
            "dispute admission passed without a standing verdict for {head}"
        )));
    };
    match keyed::wake_target(state, runtime, recorder).map_err(failed)? {
        WakeTargetLookup::Found(target) if keyed::is_review_key_for_project(&target.key, slug) => {
            Ok((recorder.clone(), target))
        }
        WakeTargetLookup::DifferentGeneration { key, .. }
            if keyed::is_review_key_for_project(&key, slug) =>
        {
            Err(refused(
                Check::ReviewState,
                format!(
                    "the requested reviewer {recorder} that recorded {head}'s verdict was archived or deleted, and a successor did not record it; a revised head is the way forward"
                ),
            ))
        }
        _ => {
            _admission::dispute_recorder(head, standing, false)?;
            Err(failed(format!(
                "dispute recorder admission failed to reject non-project reviewer {recorder}"
            )))
        }
    }
}

/// Pre-record inspection of a wake target under its held `KeyGuard`.
pub(super) enum Preflight {
    Ready,
    Gone,
    TurnInProgress,
    ReadinessUnknown,
    Misplaced(String),
}

/// Inspects a wake target's binding, presence, placement, and turn before
/// recording an ask.
pub(super) fn preflight_wake(
    state: &RunnerState,
    adapter: &dyn RuntimeAdapter,
    target: &WakeTarget,
    session: &SessionRef,
) -> Result<Preflight, AskError> {
    let runtime = adapter.runtime().name();
    let Some(current) = keyed::bound(state, runtime, &target.key).map_err(failed)? else {
        return Ok(Preflight::Gone);
    };
    if current.generation != target.generation || &current.session != session {
        return Ok(Preflight::Gone);
    }
    let thread = match adapter.read_state(session) {
        Ok(thread) => thread,
        Err(SessionsError::Gone(_)) => return Ok(Preflight::Gone),
        Err(error) => return Err(failed(error)),
    };
    if thread.presence != Presence::Present || thread.state == SessionState::Gone {
        return Ok(Preflight::Gone);
    }
    let placement = (|| {
        if target.recorded_project {
            keyed::verify_recorded_project(state, &target.key, &thread)?;
        }
        let checkout = target
            .checkout
            .as_deref()
            .ok_or_else(|| keyed::unverified_project(&target.key, session))?;
        adapter.verify_thread_project(checkout, &thread, Some(&target.key))
    })();
    if let Err(error) = placement {
        return Ok(Preflight::Misplaced(error.to_string()));
    }
    match thread.state {
        SessionState::Running | SessionState::NeedsInput => Ok(Preflight::TurnInProgress),
        SessionState::Unknown => Ok(Preflight::ReadinessUnknown),
        SessionState::Gone => Ok(Preflight::Gone),
        SessionState::InputReady | SessionState::EndedFailed => Ok(Preflight::Ready),
    }
}
