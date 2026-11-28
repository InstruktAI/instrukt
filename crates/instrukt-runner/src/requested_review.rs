//! Requested review (`project/instrukt/spec/feature/runner/requested-review`):
//! an authoring session asks for the independent review of an exact
//! committed worktree head by naming its coordinates only, and the runner
//! admits the ask, renders the reviewer's assignment, routes it by kind to
//! start or wake the requested reviewer itself, and returns recorded verdicts
//! to the asking session.
//!
//! One entry, [`ask`], runs the flow in this order: Form, Procedure, Runtime,
//! and (for session callers) Delivery wake-up inspection, then Placement,
//! Cleanliness, Range, and (for re-reviews) project-scoped prior-report trace;
//! the head's start key is claimed; under it the same ask while outstanding
//! returns that ask or replays its delivery (closing dead unaccepted wakes
//! whose target was archived, deleted, or superseded so re-reviews fall back
//! and disputes refuse under Review state); any other ask passes Review state,
//! Request, and Outstanding before routing by kind (starting or succeeding the
//! head's own reviewer, or waking a project-scoped requested reviewer under
//! its target key lock); the ask (with its asking session, trace, and
//! canonical report destination) is recorded; the runtime is reached with no
//! store lock held; acceptance is recorded. A refused ask records nothing and
//! reaches no runtime. When work records a verdict version,
//! [`queue_verdict_notices`] queues durable verdict notices for each asking
//! session on the runner's input route.

mod _admission;
mod _deliver;
mod _notices;
mod _rendering;
mod _route;
mod _store;

use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::config::Configuration;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use jiff::Timestamp;
use serde::Serialize;

pub use _admission::{Coordinate, Coordinates, FormRefusal, Kind, RawCoordinates, SourcePointer};
pub use _notices::{NoticeError, RecordedVersion, queue_verdict_notices};
pub use _route::{ReReviewOutcome, ReuseImpossible};
pub(crate) use _store::store;
pub use _store::{EarlierHead, TracedBy};

use crate::installation::RunnerWakeup;
use crate::interfaces::{HeadVerdicts, ProjectRef};
use crate::keyed::{self, KeyGuard, RuntimeInvocation};
use crate::runtime::RuntimeAdapter;
use crate::sessions::SessionsError;
use crate::state::RunnerState;
use _deliver::Delivering;

/// How long an ask waits for the machine's dispatch gate, which runner
/// wake-ups hold only while admitting their own invocations.
const DISPATCH_WAIT: Duration = Duration::from_secs(120);

/// The remote whose canonical branch decides when a head's asks are
/// history.
const REMOTE: &str = "origin";

/// The admission check that refused an ask
/// (`requested-review`, Admission).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Placement,
    Cleanliness,
    Range,
    ReviewState,
    Request,
    Outstanding,
    Runtime,
    Delivery,
    Procedure,
}

impl Check {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Placement => "placement",
            Self::Cleanliness => "cleanliness",
            Self::Range => "range",
            Self::ReviewState => "review_state",
            Self::Request => "request",
            Self::Outstanding => "outstanding",
            Self::Runtime => "runtime",
            Self::Delivery => "delivery",
            Self::Procedure => "procedure",
        }
    }
}

/// Why an ask was not admitted or not accepted.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// A value does not have its coordinate's form.
    #[error(transparent)]
    Form(#[from] FormRefusal),
    /// An admission check refused it; the message names what was found and
    /// the door.
    #[error("{message}")]
    Refused { check: Check, message: String },
    /// What admission or delivery reads could not be read.
    #[error("{0}")]
    Failed(String),
}

fn refused(check: Check, message: impl Into<String>) -> AskError {
    AskError::Refused {
        check,
        message: message.into(),
    }
}

fn failed(error: impl std::fmt::Display) -> AskError {
    AskError::Failed(error.to_string())
}

/// What an ask is made in: the home, the configuration where the caller
/// stands, the project, and work's answers.
pub struct AskContext<'a> {
    pub home: &'a Home,
    pub config: &'a Configuration,
    /// The project the caller stands in, at the caller's worktree root.
    pub project: &'a ProjectRef,
    pub verdicts: &'a dyn HeadVerdicts,
    /// The session that asked, when one resolved for the caller.
    pub asker: Option<SessionRef>,
    /// Read-only inspection answering whether a runner wake-up serves this
    /// computer; invoked only when `asker` is `Some`.
    pub delivery: &'a dyn Fn() -> Result<RunnerWakeup, String>,
    pub now: Timestamp,
}

/// How this ask reached the requested reviewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reached {
    /// It started the reviewer, with the rendering as its opening input.
    Started,
    /// It woke the reviewer's idle session under a fixed command identity.
    Woke,
}

/// What the requester receives (`requested-review`, What the requester
/// receives).
#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    pub head: CommitSha,
    pub base: CommitSha,
    pub kind: &'static str,
    pub reviewer: SessionRef,
    pub start_key: String,
    pub reached: Reached,
    /// The same ask was already outstanding: this one started or woke
    /// nothing new.
    pub repeated: bool,
    pub asker: Option<SessionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub re_review: Option<ReReviewOutcome>,
    pub wakeup_stale: bool,
    pub report: PathBuf,
    /// Where the verdict shows once the reviewer records it.
    pub show: String,
}

/// Asks for the independent review of the head `coordinates` name.
pub fn ask(context: &AskContext<'_>, coordinates: Coordinates) -> Result<Reply, AskError> {
    let slug = &context.project.slug;
    let head = &coordinates.head;
    let result = ask_inner(context, &coordinates);
    if let Err(AskError::Refused { check, message }) = &result {
        tracing::warn!(
            event = "requested_review_refused",
            project = %slug,
            head = %head,
            check = check.as_str(),
            reason = %message
        );
    }
    result
}

fn ask_inner(context: &AskContext<'_>, coordinates: &Coordinates) -> Result<Reply, AskError> {
    let kind = coordinates.kind()?;
    let slug = &context.project.slug;
    let head = &coordinates.head;
    _admission::procedure(context.home)?;
    let runtime = _admission::runtime(context.config)?;
    let wakeup_stale = if context.asker.is_some() {
        match (context.delivery)() {
            Ok(RunnerWakeup::Served { stale }) => stale,
            Ok(RunnerWakeup::Refused { reason, door }) => {
                return Err(refused(Check::Delivery, format!("{reason}; {door}")));
            }
            Err(error) => {
                return Err(AskError::Failed(format!(
                    "cannot inspect whether a runner wake-up serves this computer: {error}; check `instrukt ticks status`"
                )));
            }
        }
    } else {
        false
    };
    let adapter = crate::runtime::adapter(runtime);
    let checkout = registered_checkout(context.home, slug)?;
    _admission::placement(&checkout, coordinates)?;
    _admission::cleanliness(&coordinates.worktree)?;
    let verdict = context
        .verdicts
        .head_verdict(context.project, head)
        .map_err(AskError::Failed)?;
    _admission::range(coordinates, verdict.pinned_base.as_ref())?;
    let state = RunnerState::new(context.home);
    let requests = crate::review::requests(&state, slug, head).map_err(failed)?;
    let rereview_plan = match (kind, coordinates.prior_report.as_deref()) {
        (Kind::ReReview, Some(prior)) => Some(_route::trace_rereview(
            context,
            &state,
            runtime.name(),
            head,
            prior,
        )?),
        _ => None,
    };

    let rendering = _rendering::render(slug, coordinates);
    let key = keyed::review_key(slug.as_str(), head.as_str());
    let invocation = RuntimeInvocation::acquire(context.home, DISPATCH_WAIT).map_err(failed)?;
    let guard = claim_key(&invocation, adapter.as_ref(), &key, slug, head, context.now)?;
    let delivery = Delivering {
        context,
        adapter: adapter.as_ref(),
        invocation: &invocation,
        guard: &guard,
        coordinates,
        kind,
        key: &key,
        checkout: &checkout,
        rendering: &rendering,
        wakeup_stale,
    };
    let reply = delivery.run(&state, &verdict.standing, &requests, rereview_plan)?;
    drop(guard);
    retire(context, slug);
    Ok(reply)
}

fn claim_key(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    slug: &ProjectSlug,
    head: &CommitSha,
    now: Timestamp,
) -> Result<KeyGuard, AskError> {
    KeyGuard::claim(invocation, adapter, key, Some(slug.as_str()), now).map_err(|error| match error {
        SessionsError::Refused(_) => refused(
            Check::Outstanding,
            format!(
                "another ask of {head} is reaching its requested reviewer right now ({error}); run the same command again once it returns"
            ),
        ),
        error => failed(error),
    })
}

/// The assignment of the requested reviewer started under `start_key`, from
/// the latest unclosed ask recorded under it, accepted or not: role
/// `commit-reviewer`, the project and the worktree it was started in
/// (`roles-and-capabilities`, Caller and object resolution).
pub(crate) fn assignment(
    home: &Home,
    start_key: &str,
) -> Result<Option<instrukt_core::capabilities::Assignment>, AskError> {
    let Some(ask) = _store::latest_for_key(home, start_key).map_err(failed)? else {
        return Ok(None);
    };
    Ok(Some(instrukt_core::capabilities::Assignment {
        worker: false,
        role: instrukt_core::ids::RoleName::parse(instrukt_core::config::REVIEW_ROLE)
            .map_err(failed)?,
        project: Some(ProjectSlug::parse(&ask.project).map_err(failed)?),
        procedure: Some(
            instrukt_core::ids::SnippetId::parse(instrukt_core::config::REVIEW_PROCEDURE)
                .map_err(failed)?,
        ),
        todo: None,
        workspace: Some(ask.checkout),
        key: start_key.to_owned(),
        platform: false,
        company: None,
        dispatcher: None,
    }))
}

/// The project's registered checkout, used to validate linked worktrees and
/// resolve the runtime project.
fn registered_checkout(home: &Home, slug: &ProjectSlug) -> Result<PathBuf, AskError> {
    instrukt_core::publication::PublishedRecord::load(home, slug)
        .map_err(failed)?
        .map(|record| record.checkout)
        .ok_or_else(|| {
            refused(
                Check::Placement,
                format!(
                    "project {slug} has no registered checkout on this computer; publish it with `instrukt sync` in its checkout"
                ),
            )
        })
}

/// Retires the project's asks that became history past the retention
/// period. A failure is logged and retried by the next ask; it never
/// refuses or delays an admitted one.
fn retire(context: &AskContext<'_>, slug: &ProjectSlug) {
    let root = &context.project.root;
    let canonical = match canonical_branch(root) {
        Ok(canonical) => canonical,
        Err(error) => {
            tracing::warn!(event = "requested_review_retire_failed", project = %slug, error = %error);
            return;
        }
    };
    let contained = |head: &str| {
        instrukt_core::git::answers(root, ["merge-base", "--is-ancestor", head, &canonical])
            .unwrap_or_else(|error| {
                tracing::warn!(event = "requested_review_retire_failed", project = %slug, head, error = %error);
                false
            })
    };
    let cutoff = context.verdicts.history_cutoff(context.now);
    if let Err(error) = _store::retire(context.home, slug, cutoff, &contained) {
        tracing::warn!(event = "requested_review_retire_failed", project = %slug, error = %error);
    }
}

/// Resolves `path` to its canonical form, naming `label` and `path` on error.
fn canonicalize_path(path: &Path, label: &str) -> Result<PathBuf, AskError> {
    std::fs::canonicalize(path).map_err(|error| {
        failed(format!(
            "cannot resolve {label} path {}: {error}",
            path.display()
        ))
    })
}

/// Returns `Some(worktree)` when the canonical candidate worktree is a linked
/// worktree distinct from the canonical registered `checkout`, or `None` when
/// the candidate lives in the registered checkout itself.
fn linked_worktree(worktree: PathBuf, checkout: &Path) -> Option<PathBuf> {
    (worktree != checkout).then_some(worktree)
}

/// The canonical branch as the checkout knows it: the remote-tracking
/// branch when it exists, else the local one.
fn canonical_branch(root: &Path) -> Result<String, instrukt_core::git::GitError> {
    let branch = instrukt_core::git::default_branch(root, REMOTE)?;
    let remote = format!("refs/remotes/{REMOTE}/{branch}");
    Ok(
        if instrukt_core::git::succeeds(
            root,
            ["rev-parse", "--verify", "--quiet", remote.as_str()],
        )? {
            remote
        } else {
            branch
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_worktree_returns_none_for_registered_checkout() {
        let checkout = PathBuf::from("/tmp/repo");
        assert_eq!(linked_worktree(checkout.clone(), &checkout), None);
    }

    #[test]
    fn linked_worktree_returns_some_for_distinct_worktree() {
        let checkout = PathBuf::from("/tmp/repo");
        let worktree = PathBuf::from("/tmp/repo-wt");
        assert_eq!(linked_worktree(worktree.clone(), &checkout), Some(worktree));
    }
}
