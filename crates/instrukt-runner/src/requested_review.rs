//! Requested review (`project/instrukt/spec/feature/runner/requested-review`):
//! an authoring session asks for the independent review of an exact
//! committed worktree head by naming its coordinates only, and the runner
//! admits the ask, renders the reviewer's assignment and starts or sends to
//! the head's requested reviewer itself.
//!
//! One entry, [`ask`], runs the whole flow in this order: Form, Procedure
//! and Runtime, then Placement, Cleanliness and Range; the head's start key
//! is claimed; under it the same ask while outstanding returns that ask,
//! and any other ask passes Review state, Request and Outstanding before the
//! route is decided from the store and the runtime; the ask is recorded
//! (with a send's command identity); the runtime is reached with no store
//! lock held; the acceptance is recorded. A refused ask records nothing and
//! reaches no runtime.

mod _admission;
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
pub use _store::{EarlierHead, TracedBy};

use crate::installation::RunnerWakeup;
use crate::interfaces::{HeadVerdicts, ProjectRef};
use crate::keyed::{
    self, KeyGuard, NotWoken, RuntimeInvocation, WakeOutcome, WakeTarget, WakeTargetLookup,
};
use crate::runtime::{Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, Thread};
use crate::sessions::{SessionState, SessionsError};
use crate::state::RunnerState;
use _route::{Preflight, ReReviewPlan};
use _store::{Ask, Delivery, RecordOutcome};

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
    /// It sent the rendering into the reviewer's idle session.
    Sent,
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

/// The reviewer's turn as the runtime reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    InProgress,
    Idle,
}

fn turn(thread: &Thread) -> Result<Turn, AskError> {
    match thread.state {
        SessionState::Running | SessionState::NeedsInput => Ok(Turn::InProgress),
        SessionState::InputReady | SessionState::EndedFailed | SessionState::Gone => Ok(Turn::Idle),
        SessionState::Unknown => Err(refused(
            Check::Runtime,
            format!(
                "the runtime cannot say whether the requested reviewer {} has a turn in progress; run the same command again once it answers",
                thread.session
            ),
        )),
    }
}

/// How an admitted ask reaches the runtime.
enum Route {
    Start,
    Successor,
    Wake {
        target: WakeTarget,
        session: SessionRef,
    },
}

enum RepeatOutcome {
    Done(Reply),
    ClosedDeadWake(Option<ReuseImpossible>),
}

enum WakeAttempt {
    Accepted(Reply),
    ClosedDeadWake(Option<ReuseImpossible>),
}

/// One admitted ask under its head key.
struct Delivering<'a> {
    context: &'a AskContext<'a>,
    adapter: &'a dyn RuntimeAdapter,
    invocation: &'a RuntimeInvocation,
    guard: &'a KeyGuard,
    coordinates: &'a Coordinates,
    kind: Kind,
    key: &'a str,
    checkout: &'a Path,
    rendering: &'a _rendering::Rendering,
    wakeup_stale: bool,
}

impl Delivering<'_> {
    fn slug(&self) -> &ProjectSlug {
        &self.context.project.slug
    }

    fn head(&self) -> &CommitSha {
        &self.coordinates.head
    }

    fn read_present(&self, session: &SessionRef) -> Result<Option<Thread>, AskError> {
        let thread = match self.adapter.read_state(session) {
            Ok(thread) => Some(thread),
            Err(SessionsError::Gone(_)) => None,
            Err(error) => return Err(failed(error)),
        };
        Ok(thread.filter(|t| t.presence == Presence::Present && t.state != SessionState::Gone))
    }

    /// Under the head's key: returns the same ask while it is outstanding
    /// (closing dead unaccepted wakes on replay); admits any other ask under
    /// Review state, Request and Outstanding; routes by kind, records the ask,
    /// reaches the runtime with no store lock held, and records acceptance.
    fn run(
        &self,
        state: &RunnerState,
        standing: &crate::interfaces::VerdictStanding,
        requests: &[crate::review::Request],
        mut rereview_plan: Option<ReReviewPlan>,
    ) -> Result<Reply, AskError> {
        let runtime = self.adapter.runtime().name();
        let asks = _store::asks(self.context.home, self.slug(), self.head()).map_err(failed)?;
        let latest_unclosed = asks.iter().rev().find(|ask| ask.closed_at.is_none());
        let mut expected_last_number = latest_unclosed.map(|ask| ask.number);
        let last_bound = latest_unclosed
            .map(|ask| keyed::bound(state, runtime, &ask.start_key))
            .transpose()
            .map_err(failed)?
            .flatten();
        let last_present = last_bound
            .as_ref()
            .map(|bound| self.read_present(&bound.session))
            .transpose()?
            .flatten();
        let last_turn = last_present.as_ref().map(turn).transpose()?;
        let mut outstanding = latest_unclosed
            .filter(|ask| ask.accepted_at.is_none() || last_turn == Some(Turn::InProgress));

        // The same ask while outstanding returns it, whatever its reviewer
        // recorded meanwhile within the turn it opened (Outstanding).
        if let Some(ask) = outstanding.filter(|ask| ask.digest == self.rendering.digest) {
            match self.repeat(state, ask, last_bound.as_ref(), rereview_plan.as_ref())? {
                RepeatOutcome::Done(reply) => return Ok(reply),
                RepeatOutcome::ClosedDeadWake(fallback_reason) => {
                    outstanding = None;
                    let refreshed = _store::asks(self.context.home, self.slug(), self.head())
                        .map_err(failed)?;
                    expected_last_number = refreshed
                        .iter()
                        .rev()
                        .find(|a| a.closed_at.is_none())
                        .map(|a| a.number);
                    if let Some(reason) = fallback_reason {
                        rereview_plan = Some(ReReviewPlan::Fresh(reason));
                    }
                }
            }
        }

        _admission::review_state(self.kind, self.head(), standing)?;
        _admission::request(self.head(), requests)?;
        if let Some(ask) = outstanding {
            return Err(refused(
                Check::Outstanding,
                format!(
                    "ask {} of {} ({}) is outstanding with its requested reviewer {}; ask again once the turn it opened ends",
                    ask.number,
                    self.head(),
                    ask.kind,
                    last_bound
                        .as_ref()
                        .map_or_else(|| "(not started yet)".to_owned(), |b| b.session.to_string())
                ),
            ));
        }

        let (route, target_guard, re_review) = match self.kind {
            Kind::Review => (self.plan_head_key(state)?, None, None),
            Kind::Dispute => {
                let (session, target) = _route::resolve_dispute_target(
                    state,
                    runtime,
                    self.slug(),
                    self.head(),
                    standing,
                )?;
                let target_guard = self.claim_target_if_distinct(&target.key)?;
                self.check_preflight_for_dispute(state, &target, &session)?;
                (Route::Wake { target, session }, target_guard, None)
            }
            Kind::ReReview => match rereview_plan {
                Some(ReReviewPlan::Wake(reviewer)) => {
                    let target_guard = self.claim_target_if_distinct(&reviewer.target.key)?;
                    match _route::preflight_wake(
                        state,
                        self.adapter,
                        &reviewer.target,
                        &reviewer.session,
                    )? {
                        Preflight::Ready => (
                            Route::Wake {
                                target: reviewer.target,
                                session: reviewer.session,
                            },
                            target_guard,
                            Some(ReReviewOutcome::ReachedReviewer {
                                earlier: reviewer.earlier,
                            }),
                        ),
                        Preflight::Gone => {
                            drop(target_guard);
                            let reason = ReuseImpossible::ReviewerGone {
                                earlier: reviewer.earlier,
                            };
                            _route::log_reuse_impossible(self.slug(), self.head(), &reason);
                            (
                                self.plan_head_key(state)?,
                                None,
                                Some(ReReviewOutcome::FreshReviewer(reason)),
                            )
                        }
                        Preflight::TurnInProgress => {
                            return Err(refused(
                                Check::Outstanding,
                                format!(
                                    "the requested reviewer {} has a turn in progress; ask again once it ends",
                                    reviewer.session
                                ),
                            ));
                        }
                        Preflight::ReadinessUnknown => {
                            return Err(refused(
                                Check::Runtime,
                                format!(
                                    "the runtime cannot say whether the requested reviewer {} has a turn in progress; run the same command again once it answers",
                                    reviewer.session
                                ),
                            ));
                        }
                        Preflight::Misplaced(cause) => {
                            return Err(refused(
                                Check::Runtime,
                                format!(
                                    "the requested reviewer {} is misplaced ({cause}); run the same command again once the placement issue is resolved",
                                    reviewer.session
                                ),
                            ));
                        }
                    }
                }
                Some(ReReviewPlan::Fresh(reason)) => (
                    self.plan_head_key(state)?,
                    None,
                    Some(ReReviewOutcome::FreshReviewer(reason)),
                ),
                None => {
                    let reason = ReuseImpossible::NoEarlierHead;
                    _route::log_reuse_impossible(self.slug(), self.head(), &reason);
                    (
                        self.plan_head_key(state)?,
                        None,
                        Some(ReReviewOutcome::FreshReviewer(reason)),
                    )
                }
            },
        };

        self.record_and_deliver(state, route, target_guard, expected_last_number, re_review)
    }

    fn claim_target_if_distinct(&self, target_key: &str) -> Result<Option<KeyGuard>, AskError> {
        if target_key == self.key {
            return Ok(None);
        }
        Ok(Some(claim_key(
            self.invocation,
            self.adapter,
            target_key,
            self.slug(),
            self.head(),
            self.context.now,
        )?))
    }

    fn check_preflight_for_dispute(
        &self,
        state: &RunnerState,
        target: &WakeTarget,
        session: &SessionRef,
    ) -> Result<(), AskError> {
        match _route::preflight_wake(state, self.adapter, target, session)? {
            Preflight::Ready => Ok(()),
            Preflight::Gone => Err(refused(
                Check::ReviewState,
                format!(
                    "the requested reviewer {session} that recorded {}'s verdict was archived or deleted, and a successor did not record it; a revised head is the way forward",
                    self.head()
                ),
            )),
            Preflight::TurnInProgress => Err(refused(
                Check::Outstanding,
                format!(
                    "the requested reviewer {session} of {} has a turn in progress; ask again once it ends",
                    self.head()
                ),
            )),
            Preflight::ReadinessUnknown => Err(refused(
                Check::Runtime,
                format!(
                    "the runtime cannot say whether the requested reviewer {session} has a turn in progress; run the same command again once it answers"
                ),
            )),
            Preflight::Misplaced(cause) => Err(refused(
                Check::Runtime,
                format!(
                    "the requested reviewer {session} is misplaced ({cause}); run the same command again once the placement issue is resolved"
                ),
            )),
        }
    }

    /// Evaluates the head's own requested reviewer across all four states
    /// (`requested-review`, Reaching the reviewer): none yet -> Start,
    /// idle -> Wake, busy -> Outstanding, archived/deleted -> Successor.
    fn plan_head_key(&self, state: &RunnerState) -> Result<Route, AskError> {
        let runtime = self.adapter.runtime().name();
        let Some(bound) = keyed::bound(state, runtime, self.key).map_err(failed)? else {
            return Ok(Route::Start);
        };
        let Some(thread) = self.read_present(&bound.session)? else {
            return Ok(Route::Successor);
        };
        if turn(&thread)? == Turn::InProgress {
            return Err(refused(
                Check::Outstanding,
                format!(
                    "the requested reviewer {} of {} has a turn in progress; ask again once it ends",
                    thread.session,
                    self.head()
                ),
            ));
        }
        match keyed::wake_target(state, runtime, &bound.session).map_err(failed)? {
            WakeTargetLookup::Found(target) if target.key == self.key => {
                match _route::preflight_wake(state, self.adapter, &target, &bound.session)? {
                    Preflight::Ready => Ok(Route::Wake {
                        target,
                        session: bound.session,
                    }),
                    Preflight::Gone => Ok(Route::Successor),
                    Preflight::TurnInProgress => Err(refused(
                        Check::Outstanding,
                        format!(
                            "the requested reviewer {} of {} has a turn in progress; ask again once it ends",
                            bound.session,
                            self.head()
                        ),
                    )),
                    Preflight::ReadinessUnknown => Err(refused(
                        Check::Runtime,
                        format!(
                            "the runtime cannot say whether the requested reviewer {} has a turn in progress; run the same command again once it answers",
                            bound.session
                        ),
                    )),
                    Preflight::Misplaced(cause) => Err(refused(
                        Check::Runtime,
                        format!(
                            "the requested reviewer {} is misplaced ({cause}); run the same command again once the placement issue is resolved",
                            bound.session
                        ),
                    )),
                }
            }
            _ => Ok(Route::Successor),
        }
    }

    fn record_and_deliver(
        &self,
        state: &RunnerState,
        route: Route,
        target_guard: Option<KeyGuard>,
        expected_last_number: Option<u64>,
        re_review: Option<ReReviewOutcome>,
    ) -> Result<Reply, AskError> {
        let (start_key, delivery, send_generation) = match &route {
            Route::Start | Route::Successor => (self.key.to_owned(), Delivery::Start, None),
            Route::Wake { target, .. } => {
                (target.key.clone(), Delivery::Send, Some(target.generation))
            }
        };
        let worktree_path = self.worktree_path()?;
        let report_path = _store::canonical_report(&self.coordinates.report).map_err(failed)?;
        let named = serde_json::to_string(&self.coordinates.named()).map_err(failed)?;
        let trace = re_review
            .as_ref()
            .and_then(ReReviewOutcome::earlier)
            .cloned();
        let ask = match _store::record(
            self.context.home,
            &_store::NewAsk {
                project: self.slug(),
                head: self.head(),
                kind: self.kind.as_str(),
                coordinates: &named,
                rendering: &self.rendering.text,
                digest: &self.rendering.digest,
                start_key: &start_key,
                checkout: &worktree_path,
                delivery,
                send_generation,
                admitted_at: self.context.now,
                asker: self.context.asker.as_ref(),
                trace: trace.as_ref(),
                report: &report_path,
            },
            expected_last_number,
        )
        .map_err(failed)?
        {
            RecordOutcome::Recorded(ask) => *ask,
            RecordOutcome::Conflict => {
                return Err(refused(
                    Check::Outstanding,
                    format!(
                        "another ask of {} was admitted concurrently; run the same command again",
                        self.head()
                    ),
                ));
            }
        };
        tracing::info!(
            event = "requested_review_admitted",
            project = %self.slug(),
            head = %self.head(),
            ask = ask.number,
            kind = self.kind.as_str(),
            asker = ?ask.asker,
            trace = ?ask.trace,
            report = %ask.report.display()
        );
        match route {
            Route::Start => {
                let session = self.start_with_brief(false, &ask.rendering)?;
                self.accepted(&ask, &session, false, re_review)
            }
            Route::Successor => {
                let session = self.start_with_brief(true, &ask.rendering)?;
                self.accepted(&ask, &session, false, re_review)
            }
            Route::Wake { target, session } => {
                let active_guard = target_guard.as_ref().unwrap_or(self.guard);
                match self.execute_wake(active_guard, &ask, &target, &session, false, re_review)? {
                    WakeAttempt::Accepted(reply) => Ok(reply),
                    WakeAttempt::ClosedDeadWake(fallback_reason) => {
                        drop(target_guard);
                        self.deliver_after_dead_wake(state, fallback_reason)
                    }
                }
            }
        }
    }

    fn deliver_after_dead_wake(
        &self,
        state: &RunnerState,
        fallback_reason: Option<ReuseImpossible>,
    ) -> Result<Reply, AskError> {
        let refreshed =
            _store::asks(self.context.home, self.slug(), self.head()).map_err(failed)?;
        let expected_last_number = refreshed
            .iter()
            .rev()
            .find(|a| a.closed_at.is_none())
            .map(|a| a.number);
        let (route, re_review) = match self.kind {
            Kind::ReReview => {
                let reason = fallback_reason.unwrap_or(ReuseImpossible::NoEarlierHead);
                _route::log_reuse_impossible(self.slug(), self.head(), &reason);
                (
                    self.plan_head_key(state)?,
                    Some(ReReviewOutcome::FreshReviewer(reason)),
                )
            }
            Kind::Review => (Route::Successor, None),
            Kind::Dispute => {
                return Err(refused(
                    Check::ReviewState,
                    format!(
                        "the requested reviewer that recorded {}'s verdict was archived or deleted; a revised head is the way forward",
                        self.head()
                    ),
                ));
            }
        };
        self.record_and_deliver(state, route, None, expected_last_number, re_review)
    }

    /// The same ask while it is outstanding returns it, replaying its
    /// delivery while the runtime's acceptance is not recorded.
    fn repeat(
        &self,
        state: &RunnerState,
        ask: &Ask,
        bound: Option<&keyed::Bound>,
        rereview_plan: Option<&ReReviewPlan>,
    ) -> Result<RepeatOutcome, AskError> {
        let re_review = self.re_review_from_ask(ask, rereview_plan);
        if ask.accepted_at.is_some() {
            let session = bound.map(|b| b.session.clone()).ok_or_else(|| {
                failed(format!(
                    "ask {} of {} was accepted but its start key `{}` binds no session",
                    ask.number,
                    self.head(),
                    ask.start_key
                ))
            })?;
            tracing::info!(event = "requested_review_repeated", project = %self.slug(), head = %self.head(), ask = ask.number);
            return Ok(RepeatOutcome::Done(
                self.reply(ask, session, true, re_review),
            ));
        }
        tracing::info!(event = "requested_review_replayed", project = %self.slug(), head = %self.head(), ask = ask.number);
        match ask.delivery {
            Delivery::Start => {
                let successor = self.start_replay_needs_successor(state)?;
                let session = self.start_with_brief(successor, &ask.rendering)?;
                Ok(RepeatOutcome::Done(
                    self.accepted(ask, &session, true, re_review)?,
                ))
            }
            Delivery::Send => {
                let target_guard = self.claim_target_if_distinct(&ask.start_key)?;
                let runtime = self.adapter.runtime().name();
                let current_bound = keyed::bound(state, runtime, &ask.start_key).map_err(failed)?;
                let target_lookup = match &current_bound {
                    Some(b) if Some(b.generation) == ask.generation => {
                        match keyed::wake_target(state, runtime, &b.session).map_err(failed)? {
                            WakeTargetLookup::Found(target) if target.key == ask.start_key => {
                                Ok((target, b.session.clone()))
                            }
                            WakeTargetLookup::DifferentGeneration { .. } => {
                                Err("different_generation")
                            }
                            WakeTargetLookup::NotBound | WakeTargetLookup::Found(_) => {
                                Err("not_bound")
                            }
                        }
                    }
                    Some(_) => Err("different_generation"),
                    None => Err("not_bound"),
                };
                match target_lookup {
                    Ok((target, session)) => {
                        let active_guard = target_guard.as_ref().unwrap_or(self.guard);
                        match self.execute_wake(
                            active_guard,
                            ask,
                            &target,
                            &session,
                            true,
                            re_review,
                        )? {
                            WakeAttempt::Accepted(reply) => Ok(RepeatOutcome::Done(reply)),
                            WakeAttempt::ClosedDeadWake(fallback_reason) => {
                                drop(target_guard);
                                Ok(RepeatOutcome::ClosedDeadWake(fallback_reason))
                            }
                        }
                    }
                    Err(reason_kind) => {
                        drop(target_guard);
                        let fallback_reason = self.close_dead_wake(ask, reason_kind)?;
                        Ok(RepeatOutcome::ClosedDeadWake(fallback_reason))
                    }
                }
            }
        }
    }

    fn start_replay_needs_successor(&self, state: &RunnerState) -> Result<bool, AskError> {
        let runtime = self.adapter.runtime().name();
        let Some(bound) = keyed::bound(state, runtime, self.key).map_err(failed)? else {
            return Ok(false);
        };
        let Some(thread) = self.read_present(&bound.session)? else {
            return Ok(true);
        };
        let misplaced = self
            .adapter
            .verify_thread_project(self.checkout, &thread, Some(self.key))
            .is_err();
        Ok(misplaced)
    }

    fn execute_wake(
        &self,
        active_guard: &KeyGuard,
        ask: &Ask,
        target: &WakeTarget,
        session: &SessionRef,
        replayed: bool,
        re_review: Option<ReReviewOutcome>,
    ) -> Result<WakeAttempt, AskError> {
        let command = ask.command_id.as_deref().ok_or_else(|| {
            failed(format!(
                "ask {} of {} is a wake without its recorded command identity",
                ask.number,
                self.head()
            ))
        })?;
        match keyed::wake(
            active_guard,
            self.adapter,
            target,
            session,
            command,
            &ask.rendering,
            self.context.now,
        )
        .map_err(failed)?
        {
            WakeOutcome::Woken => Ok(WakeAttempt::Accepted(
                self.accepted(ask, session, replayed, re_review)?,
            )),
            WakeOutcome::Refused(NotWoken::NotAccepted { error, .. }) => {
                Err(self.not_accepted("wake", &error))
            }
            WakeOutcome::Refused(NotWoken::TurnInProgress { session }) => Err(refused(
                Check::Outstanding,
                format!(
                    "the requested reviewer {session} has a turn in progress; ask again once it ends"
                ),
            )),
            WakeOutcome::Refused(NotWoken::ReadinessUnknown { session }) => Err(refused(
                Check::Runtime,
                format!(
                    "the runtime cannot say whether the requested reviewer {session} has a turn in progress; run the same command again once it answers"
                ),
            )),
            WakeOutcome::Refused(NotWoken::Misplaced { session, cause }) => Err(refused(
                Check::Runtime,
                format!(
                    "the requested reviewer {session} is misplaced ({cause}); run the same command again once the placement issue is resolved"
                ),
            )),
            WakeOutcome::Refused(
                reason @ (NotWoken::Gone { .. }
                | NotWoken::NotBound { .. }
                | NotWoken::DifferentGeneration { .. }),
            ) => {
                let fallback_reason = self.close_dead_wake(ask, reason.kind())?;
                Ok(WakeAttempt::ClosedDeadWake(fallback_reason))
            }
        }
    }

    /// Closes an unaccepted wake whose target session is archived, deleted, or
    /// superseded (F2). Refuses disputes under `Check::ReviewState`; returns
    /// the fallback `ReuseImpossible` for re-reviews.
    fn close_dead_wake(
        &self,
        ask: &Ask,
        reason_kind: &'static str,
    ) -> Result<Option<ReuseImpossible>, AskError> {
        _store::close_unaccepted(
            self.context.home,
            self.slug(),
            self.head(),
            ask.number,
            self.context.now,
        )
        .map_err(failed)?;
        tracing::info!(
            event = "requested_review_wake_closed",
            project = %self.slug(),
            head = %self.head(),
            ask = ask.number,
            reason = reason_kind
        );
        match self.kind {
            Kind::Dispute => Err(refused(
                Check::ReviewState,
                format!(
                    "the requested reviewer that recorded {}'s verdict was archived or deleted; a revised head is the way forward",
                    self.head()
                ),
            )),
            Kind::ReReview => Ok(Some(match ask.trace.clone() {
                Some(earlier) => ReuseImpossible::ReviewerGone { earlier },
                None => ReuseImpossible::NoEarlierHead,
            })),
            Kind::Review => Ok(None),
        }
    }

    fn re_review_from_ask(
        &self,
        ask: &Ask,
        rereview_plan: Option<&ReReviewPlan>,
    ) -> Option<ReReviewOutcome> {
        if self.kind != Kind::ReReview {
            return None;
        }
        if ask.start_key != self.key
            && let Some(earlier) = ask.trace.clone()
        {
            return Some(ReReviewOutcome::ReachedReviewer { earlier });
        }
        if let Some(ReReviewPlan::Fresh(reason)) = rereview_plan {
            return Some(ReReviewOutcome::FreshReviewer(reason.clone()));
        }
        Some(ReReviewOutcome::FreshReviewer(match ask.trace.clone() {
            Some(earlier) => ReuseImpossible::ReviewerGone { earlier },
            None => ReuseImpossible::NoEarlierHead,
        }))
    }

    fn start_with_brief(&self, successor: bool, brief: &str) -> Result<SessionRef, AskError> {
        let context = self.context;
        let (request, existing_worktree) = self.request(brief)?;
        let thread = if successor {
            keyed::succeed_owned(
                self.guard,
                self.invocation,
                self.adapter,
                &request,
                existing_worktree.as_deref(),
                context.now,
            )
        } else {
            let assignment = RuntimeAssignment {
                bindings: None,
                invocation: self.invocation,
                project: Some(self.slug().as_str()),
                request: &request,
                existing_worktree: existing_worktree.as_deref(),
                now: context.now,
            };
            let result = keyed::start_owned(self.adapter, &assignment, self.guard);
            keyed::note(
                &RunnerState::new(context.home),
                self.adapter.runtime(),
                self.key,
                context.now,
                result.as_ref().map(|_| ()),
            );
            result
        }
        .map_err(|error| self.not_accepted("start", &error))?;
        Ok(thread.session)
    }

    fn worktree_path(&self) -> Result<PathBuf, AskError> {
        canonicalize_path(&self.coordinates.worktree, "candidate worktree")
    }

    /// The start request and optional linked existing worktree: rooted in the
    /// project's registered checkout so the runtime resolves the project, and
    /// attaching the candidate's linked worktree when it differs from that
    /// checkout, under the review procedure's role.
    fn request(&self, brief: &str) -> Result<(RuntimeStart, Option<PathBuf>), AskError> {
        let context = self.context;
        let preferences = crate::agents::role_order(
            context.config,
            None,
            Some(instrukt_core::config::REVIEW_ROLE),
            None,
        )
        .map_err(failed)?;
        let selection =
            crate::agents::selections(context.config, context.home, &preferences, context.now)
                .map_err(failed)?
                .into_iter()
                .next();
        let head = self.head().as_str();
        let worktree_path = self.worktree_path()?;
        let checkout_path = canonicalize_path(self.checkout, "registered checkout")?;
        let existing_worktree = linked_worktree(worktree_path, &checkout_path);
        Ok((
            RuntimeStart {
                preferences,
                selection,
                selection_evidence: Some(
                    self.adapter
                        .selection_realization(crate::runtime::SelectionOperation::ThreadStart),
                ),
                checkout: self.checkout.to_path_buf(),
                title: format!(
                    "review · {} @ {}",
                    self.coordinates.branch,
                    &head[..head.len().min(12)]
                ),
                key: self.key.to_owned(),
                brief: brief.to_owned(),
                worktree: None,
                launch_modes: None,
                native_launch: None,
                profiles: crate::runtime::Profiles::default(),
            },
            existing_worktree,
        ))
    }

    fn not_accepted(&self, what: &str, error: &SessionsError) -> AskError {
        refused(
            Check::Runtime,
            format!(
                "the {} runtime did not accept the {what} of {}'s requested reviewer: {error}; run the same command again: it replays this ask with the identities it recorded",
                self.adapter.runtime().name(),
                self.head()
            ),
        )
    }

    /// Records the acceptance at the generation `ask.start_key` reached.
    fn accepted(
        &self,
        ask: &Ask,
        session: &SessionRef,
        replayed: bool,
        re_review: Option<ReReviewOutcome>,
    ) -> Result<Reply, AskError> {
        let context = self.context;
        let state = RunnerState::new(context.home);
        let generation = match ask.delivery {
            Delivery::Send => ask.generation,
            Delivery::Start => keyed::bound(&state, self.adapter.runtime().name(), &ask.start_key)
                .map_err(failed)?
                .map(|bound| bound.generation),
        }
        .unwrap_or_default();
        _store::accept(
            context.home,
            self.slug(),
            self.head(),
            ask.number,
            generation,
            context.now,
        )
        .map_err(failed)?;
        let reached = match ask.delivery {
            Delivery::Start => "started",
            Delivery::Send => "woke",
        };
        tracing::info!(
            event = "requested_review_accepted",
            project = %self.slug(),
            head = %self.head(),
            ask = ask.number,
            session = %session,
            start_key = %ask.start_key,
            reached,
            replayed
        );
        Ok(self.reply(ask, session.clone(), replayed, re_review))
    }

    fn reply(
        &self,
        ask: &Ask,
        reviewer: SessionRef,
        repeated: bool,
        re_review: Option<ReReviewOutcome>,
    ) -> Reply {
        let head = self.head().clone();
        Reply {
            show: format!("instrukt review show {head}"),
            base: self.coordinates.base.clone(),
            kind: self.kind.as_str(),
            reviewer,
            start_key: ask.start_key.clone(),
            reached: match ask.delivery {
                Delivery::Start => Reached::Started,
                Delivery::Send => Reached::Sent,
            },
            repeated,
            re_review,
            wakeup_stale: self.wakeup_stale,
            report: self.coordinates.report.clone(),
            head,
        }
    }
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
