//! Locked delivery of an admitted requested review (`requested-review`,
//! Reaching the reviewer): handles outstanding repeats/replays, dead-wake
//! closure, routing by kind under key locks, recording, runtime start/wake,
//! and acceptance.

use std::path::{Path, PathBuf};

use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};

use crate::keyed::{
    self, KeyGuard, NotWoken, RuntimeInvocation, WakeOutcome, WakeTarget, WakeTargetLookup,
};
use crate::runtime::{Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, Thread};
use crate::sessions::{SessionState, SessionsError};
use crate::state::RunnerState;

use super::_admission::{self, Coordinates, Kind};
use super::_rendering;
use super::_route::{self, ReReviewOutcome, ReReviewPlan, ReuseImpossible};
use super::_store::{self, Ask, Delivery, RecordOutcome};
use super::{
    AskContext, AskError, Check, Reached, Reply, canonicalize_path, claim_key, failed,
    linked_worktree, refused,
};

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
    Done(Box<Reply>),
    ClosedDeadWake(Option<ReuseImpossible>),
}

enum WakeAttempt {
    Accepted(Box<Reply>),
    ClosedDeadWake(Option<ReuseImpossible>),
}

/// One admitted ask under its head key.
pub(super) struct Delivering<'a> {
    pub context: &'a AskContext<'a>,
    pub adapter: &'a dyn RuntimeAdapter,
    pub invocation: &'a RuntimeInvocation,
    pub guard: &'a KeyGuard,
    pub coordinates: &'a Coordinates,
    pub kind: Kind,
    pub key: &'a str,
    pub checkout: &'a Path,
    pub rendering: &'a _rendering::Rendering,
    pub wakeup_stale: bool,
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
    pub(super) fn run(
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
                RepeatOutcome::Done(reply) => return Ok(*reply),
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
                let active_guard = target_guard.as_ref().unwrap_or(self.guard);
                self.check_preflight_for_dispute(active_guard, &target, &session)?;
                (Route::Wake { target, session }, target_guard, None)
            }
            Kind::ReReview => match rereview_plan {
                Some(ReReviewPlan::Wake(reviewer)) => {
                    let target_guard = self.claim_target_if_distinct(&reviewer.target.key)?;
                    let active_guard = target_guard.as_ref().unwrap_or(self.guard);
                    match keyed::check_wake(
                        active_guard,
                        self.adapter,
                        &reviewer.target,
                        &reviewer.session,
                    )
                    .map_err(failed)?
                    {
                        None => (
                            Route::Wake {
                                target: reviewer.target,
                                session: reviewer.session,
                            },
                            target_guard,
                            Some(ReReviewOutcome::ReachedReviewer {
                                earlier: reviewer.earlier,
                            }),
                        ),
                        Some(
                            NotWoken::Gone { .. }
                            | NotWoken::NotBound { .. }
                            | NotWoken::DifferentGeneration { .. },
                        ) => {
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
                        Some(reason) => return Err(self.refuse_not_woken(reason, false)),
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
        active_guard: &KeyGuard,
        target: &WakeTarget,
        session: &SessionRef,
    ) -> Result<(), AskError> {
        match keyed::check_wake(active_guard, self.adapter, target, session).map_err(failed)? {
            None => Ok(()),
            Some(
                NotWoken::Gone { .. }
                | NotWoken::NotBound { .. }
                | NotWoken::DifferentGeneration { .. },
            ) => Err(refused(
                Check::ReviewState,
                format!(
                    "the requested reviewer {session} that recorded {}'s verdict was archived or deleted, and a successor did not record it; a revised head is the way forward",
                    self.head()
                ),
            )),
            Some(reason) => Err(self.refuse_not_woken(reason, true)),
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
                match keyed::check_wake(self.guard, self.adapter, &target, &bound.session)
                    .map_err(failed)?
                {
                    None => Ok(Route::Wake {
                        target,
                        session: bound.session,
                    }),
                    Some(
                        NotWoken::Gone { .. }
                        | NotWoken::NotBound { .. }
                        | NotWoken::DifferentGeneration { .. },
                    ) => Ok(Route::Successor),
                    Some(reason) => Err(self.refuse_not_woken(reason, true)),
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
                    WakeAttempt::Accepted(reply) => Ok(*reply),
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
            return Ok(RepeatOutcome::Done(Box::new(
                self.reply(ask, session, true, re_review),
            )));
        }
        tracing::info!(event = "requested_review_replayed", project = %self.slug(), head = %self.head(), ask = ask.number);
        match ask.delivery {
            Delivery::Start => {
                let successor = self.start_replay_needs_successor(state)?;
                let session = self.start_with_brief(successor, &ask.rendering)?;
                Ok(RepeatOutcome::Done(Box::new(
                    self.accepted(ask, &session, true, re_review)?,
                )))
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
            WakeOutcome::Woken => Ok(WakeAttempt::Accepted(Box::new(
                self.accepted(ask, session, replayed, re_review)?,
            ))),
            WakeOutcome::Refused(
                reason @ (NotWoken::Gone { .. }
                | NotWoken::NotBound { .. }
                | NotWoken::DifferentGeneration { .. }),
            ) => {
                let fallback_reason = self.close_dead_wake(ask, reason.kind())?;
                Ok(WakeAttempt::ClosedDeadWake(fallback_reason))
            }
            WakeOutcome::Refused(reason) => Err(self.refuse_not_woken(reason, false)),
        }
    }

    fn refuse_not_woken(&self, reason: NotWoken, head_scoped: bool) -> AskError {
        match reason {
            NotWoken::NotAccepted { error, .. } => self.not_accepted("wake", &error),
            NotWoken::TurnInProgress { session } if head_scoped => refused(
                Check::Outstanding,
                format!(
                    "the requested reviewer {session} of {} has a turn in progress; ask again once it ends",
                    self.head()
                ),
            ),
            NotWoken::TurnInProgress { session } => refused(
                Check::Outstanding,
                format!(
                    "the requested reviewer {session} has a turn in progress; ask again once it ends"
                ),
            ),
            NotWoken::ReadinessUnknown { session } => refused(
                Check::Runtime,
                format!(
                    "the runtime cannot say whether the requested reviewer {session} has a turn in progress; run the same command again once it answers"
                ),
            ),
            NotWoken::Misplaced { session, cause } => refused(
                Check::Runtime,
                format!(
                    "the requested reviewer {session} is misplaced ({cause}); run the same command again once the placement issue is resolved"
                ),
            ),
            NotWoken::NotBound { .. }
            | NotWoken::DifferentGeneration { .. }
            | NotWoken::Gone { .. } => failed(reason),
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
                Delivery::Send => Reached::Woke,
            },
            repeated,
            asker: ask.asker.clone(),
            re_review,
            wakeup_stale: self.wakeup_stale,
            report: self.coordinates.report.clone(),
            head,
        }
    }
}
