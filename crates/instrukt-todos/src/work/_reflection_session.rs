//! The runner's per-tick GitHub reflection pass (`github-record`,
//! `work-dispatch` "Execution contract"): the single path that acquires
//! `state/reflection.lock`, reads inbound human controls (`hold` label
//! actions and `/instrukt` commands) before pickup, and publishes work
//! branches and flushes pending reflection after pickup.
//!
//! One [`ReflectionSession`] spans a single tick: `begin` resets the
//! per-pass GitHub App cache once (`UC-GA1`), and any command-wide failure
//! (`Remote::Unavailable` or `Remote::Client { down: Some(_) }`) recorded
//! during `pull_inbound` stays on the session into `flush_pass` so zero
//! further GitHub requests are sent on that tick (`github-apps`,
//! "Installation credentials").

use instrukt_core::config::BoardRef;
use instrukt_core::git;
use instrukt_core::github;
use instrukt_runner::interfaces::{GitHubReach, ProjectRef, ReflectionReport};
use jiff::Timestamp;

use super::_apps::Clock;
use super::_error::WorkError;
use super::_github::{GitHub, GitHubError};
use super::_reflect;
use super::_session::{PendingReason, ReflectionStatus, Session, describe, status_of};
use super::_store::{self, ProviderState, ReflectionLock};
use super::Work;
use crate::evidence::TodoStatus;

pub(super) enum Remote {
    Client {
        gh: Box<GitHub>,
        /// Set when GitHub stopped answering during this pass.
        down: Option<GitHubError>,
    },
    Unavailable(PendingReason),
}

/// Runner-only GitHub reflection pass for one project on one tick.
pub(super) struct ReflectionSession<'w> {
    pub(super) session: Session<'w>,
    api_url: String,
    repository: String,
    clock: Clock,
    pub(super) remote: Option<Remote>,
    /// Why this pass's flush could not take the reflection lock:
    /// another process reflected for longer than the bounded wait.
    reflection_busy: Option<String>,
    /// A cockpit setup failure retained independently of item reflection.
    board_setup_failure: Option<String>,
}

impl<'w> ReflectionSession<'w> {
    /// Opens a reflection pass for `project`, resetting the per-pass GitHub
    /// App credential cache once and deferring client construction until the
    /// first GitHub need. Takes no lock and performs no GitHub I/O.
    pub(super) fn begin(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
        work.github.begin_pass().map_err(GitHubError::from)?;
        let (session, api_url, repository) = Session::open_parts(work, project, false)?;
        // Anchored here, before any capability lock wait, so a wait for it
        // is part of every GitHub answer's elapsed time.
        let clock = Clock::starting(session.now);
        Ok(Self {
            session,
            api_url,
            repository,
            clock,
            remote: None,
            reflection_busy: None,
            board_setup_failure: None,
        })
    }

    /// Reads people's inbound controls (`hold` label actions, `/instrukt`
    /// commands, and output-label drift) from the issues of live local todos
    /// before `pickup`, under `reflection.lock`, and releases the lock before
    /// returning. Carries any command-wide GitHub failure on `self.remote`
    /// into the subsequent [`Self::flush_pass`].
    pub(super) fn pull_inbound(&mut self) -> Result<(), WorkError> {
        self.session.reload()?;
        let live: Vec<usize> = (0..self.session.items.len())
            .filter(|&index| {
                let item = &self.session.items[index];
                item.reflection.issue.is_some()
                    && !matches!(
                        item.record.status,
                        TodoStatus::Delivered | TodoStatus::Removed
                    )
            })
            .collect();
        if live.is_empty() {
            return Ok(());
        }
        let lock = match ReflectionLock::acquire(self.session.home()) {
            Ok(lock) => lock,
            Err(WorkError::Fs(busy @ instrukt_core::fsx::FsError::LockTimeout { .. })) => {
                tracing::warn!(
                    event = "work_reflection_busy",
                    project = %self.session.project.slug,
                    error = %busy,
                    "another reflection holds the reflection lock; skipping inbound pull for this tick"
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.ensure_remote()?;
        for index in live {
            if self.down_reason().is_some() {
                break;
            }
            self.pull_through(&lock, index)?;
        }
        drop(lock);
        Ok(())
    }

    /// Reloads local items (picking up `pickup`'s writes), publishes work
    /// branches outside `reflection.lock`, then flushes pending reflection
    /// under `reflection.lock`, returning the pass report.
    pub(super) fn finish(mut self) -> Result<ReflectionReport, WorkError> {
        self.flush_pass()?;
        if let Some(cause) = &self.board_setup_failure {
            return Err(WorkError::refused(format!(
                "cockpit setup pending: {cause}"
            )));
        }
        let reach = match self.down_reason() {
            None => GitHubReach::Reachable,
            Some(PendingReason::RateLimited { until }) => GitHubReach::RateLimited { until },
            Some(_) => GitHubReach::Unreachable,
        };
        let pending = self
            .session
            .items
            .iter()
            .filter(|item| matches!(status_of(item), ReflectionStatus::Pending(_)))
            .map(|item| item.record.slug.clone())
            .collect();
        Ok(ReflectionReport { reach, pending })
    }

    /// Initializes `self.remote` on first GitHub need, preserving any client
    /// or command-wide failure already established on this pass.
    fn ensure_remote(&mut self) -> Result<(), WorkError> {
        if self.remote.is_some() {
            return Ok(());
        }
        let provider = self.session.store.provider()?;
        let remote = match unavailable(self.session.work, &provider, self.session.now) {
            Some(reason) => Remote::Unavailable(reason),
            None => match GitHub::for_app(
                self.session.work.github.clone(),
                &self.api_url,
                &self.repository,
                None,
                github::Operation::Record,
                self.clock,
            ) {
                Ok(gh) => Remote::Client {
                    gh: Box::new(gh),
                    down: None,
                },
                Err(err) => Remote::Unavailable(reason_of(&err)),
            },
        };
        self.remote = Some(remote);
        Ok(())
    }

    fn down_reason(&self) -> Option<PendingReason> {
        match self.remote.as_ref() {
            Some(Remote::Unavailable(reason)) => Some(reason.clone()),
            Some(Remote::Client {
                down: Some(err), ..
            }) => Some(reason_of(err)),
            _ => None,
        }
    }

    /// Publishes work branches outside `reflection.lock`, then flushes
    /// pending reflection of every stored item of the project under
    /// `reflection.lock`. Persists command-wide reach/unavailable/rate-limit
    /// failure causes onto every pending/owed item in `reflections.failure`
    /// without issuing further GitHub requests once down.
    fn flush_pass(&mut self) -> Result<(), WorkError> {
        self.session.reload()?;
        // Publication runs outside the reflection lock so a git push never
        // holds it.
        self.publish()?;
        let lock = match ReflectionLock::acquire(self.session.home()) {
            Ok(lock) => lock,
            Err(WorkError::Fs(busy @ instrukt_core::fsx::FsError::LockTimeout { .. })) => {
                tracing::warn!(
                    event = "work_reflection_busy",
                    project = %self.session.project.slug,
                    error = %busy,
                    "another reflection holds the reflection lock; this flush reflects nothing and leaves what is unconfirmed to a later flush"
                );
                self.reflection_busy = Some(busy.to_string());
                for row in self.session.store.items()? {
                    self.session.learned(Some(row));
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.ensure_remote()?;
        let (board, board_error) = if self.down_reason().is_some() {
            (None, None)
        } else {
            let board_result = (|| {
                // A board the developer names in the project layer leads; otherwise
                // the one this computer recorded. Setup reconciles either with
                // GitHub, finding an existing board before creating one.
                let configured = self.session.config.board()?;
                let recorded = self.session.store.recorded_board()?;
                let known = configured
                    .clone()
                    .or_else(|| recorded.as_deref().and_then(BoardRef::parse));
                let Some(Remote::Client { gh, down: None }) = self.remote.as_ref() else {
                    return Ok(None);
                };
                let account = gh.for_role(None, github::Operation::Board);
                let board = super::_board::setup(&account, gh, known.as_ref())?;
                // The reference is the machine's record: reflection never writes
                // the committed project layer.
                if configured.as_ref() != Some(&board)
                    && recorded.as_deref() != Some(&board.to_string())
                {
                    self.session.store.record_board(&board.to_string())?;
                }
                super::_board::configure(&account, &board)?;
                Ok(Some(board))
            })();
            match board_result {
                Ok(board) => (board, None),
                Err(error) => {
                    if let WorkError::GitHub(
                        gh_error @ (GitHubError::RateLimited { .. }
                        | GitHubError::App(github::AppError::RateLimited { .. })),
                    ) = &error
                    {
                        self.note(gh_error.clone());
                    }
                    tracing::warn!(event="work_board_setup_pending", error=%error);
                    (None, Some(error.to_string()))
                }
            }
        };
        self.board_setup_failure = board_error.clone();
        for loaded in self.session.store.items()? {
            let id = loaded.record.id.clone();
            let mut reflected = loaded.clone();
            if !reflected.reflection.pending {
                reflected.reflection.failure = None;
            }
            if let Some(error) = &board_error {
                reflected.reflection.board_failure = Some(error.clone());
            }
            if let Some(issue) = reflected.reflection.issue {
                self.post_replies(issue, id.as_str(), &lock)?;
            }
            let owed = reflected.reflection.pending
                || _reflect::pull_request_owed(&reflected)
                || (board.is_some() && reflected.reflection.issue.is_some());
            if owed {
                if let Some(reason) = self.down_reason() {
                    reflected.reflection.failure = Some(describe(&reason));
                } else {
                    // A todo shown as held already needs you: its record is not
                    // projected and no approver is resolved for it.
                    let needs_you = super::_ports::waits_on_person(
                        super::_envelope::record_status(&reflected.record),
                        || {
                            Ok::<_, instrukt_core::config::ConfigError>(
                                crate::workflow::waiting_for_human_approval(
                                    &super::_ports::to_evidence(
                                        reflected.record.clone(),
                                        Vec::new(),
                                    ),
                                    self.session
                                        .config
                                        .approver(instrukt_core::config::ApprovalGate::Prd)?,
                                    self.session
                                        .config
                                        .approver(instrukt_core::config::ApprovalGate::Task)?,
                                ),
                            )
                        },
                    )?;
                    let result = match self.remote.as_ref() {
                        Some(Remote::Client { gh, down: None }) => Some(_reflect::flush(
                            gh,
                            &mut reflected,
                            &self.session.project.root,
                            &self.session.base,
                            board.as_ref(),
                            needs_you,
                        )),
                        _ => None,
                    };
                    if let Some(Err(err)) = result {
                        if err.is_reach() {
                            let reason = reason_of(&err);
                            self.note(err);
                            reflected.reflection.failure = Some(describe(&reason));
                        } else {
                            tracing::warn!(
                                event = "work_reflection_pending",
                                project = %self.session.project.slug,
                                todo = %reflected.record.slug,
                                error = %err,
                                "reflection failed; the local record keeps the change"
                            );
                            reflected.reflection.failure = Some(err.to_string());
                        }
                    }
                    if let Some(error) = &board_error {
                        reflected.reflection.board_failure = Some(error.clone());
                    }
                }
            }
            // A row this flush left as it was may still be news to the
            // session: another process can have reflected it already.
            let row = if _store::stored(&reflected) != _store::stored(&loaded) {
                self.session.store.commit_reflection(&loaded, &reflected)?
            } else {
                Some(loaded)
            };
            self.session.learned(row);
        }
        drop(lock);
        Ok(())
    }

    /// Publishes the work branch of each repository todo still owed it
    /// (`github-record`, Issues and visible work), outside `reflection.lock`.
    fn publish(&mut self) -> Result<(), WorkError> {
        let remote = github::tracked_remote(&self.session.project.root, &self.session.base)?;
        for index in 0..self.session.items.len() {
            let item = &self.session.items[index];
            let id = item.record.id.clone();
            let owed = super::_envelope::is_repository_work(item.record.kind)
                && item.record.delivery.is_none()
                && item.record.status != TodoStatus::Removed
                && self.session.store.withdrawal_intent(&id)?.is_none();
            let Some(branch) = item.record.branch.clone().filter(|_| owed) else {
                if item.reflection.publication_outstanding.is_some() {
                    self.session
                        .amend(index, |r| r.publication_outstanding = None)?;
                }
                continue;
            };
            let head = match git::run(
                &self.session.project.root,
                [
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}^{{commit}}"),
                ],
            ) {
                Ok(head) => instrukt_core::ids::CommitSha::parse(head.trim())
                    .map_err(|e| WorkError::refused(e.to_string()))?,
                Err(git::GitError::Failed { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            if item.reflection.published_head.as_ref() == Some(&head)
                && item.reflection.publication_outstanding.is_none()
            {
                continue;
            }
            let Some(remote) = remote.as_deref() else {
                let why = format!(
                    "the default branch `{}` tracks no remote, so branch `{branch}` is not published",
                    self.session.base
                );
                if item.reflection.publication_outstanding.as_deref() != Some(why.as_str()) {
                    self.session
                        .amend(index, |r| r.publication_outstanding = Some(why.clone()))?;
                }
                continue;
            };
            match super::_publication::push(&self.session.project.root, remote, &branch) {
                super::_publication::Pushed::Published => {
                    tracing::info!(
                        event = "work_branch_published",
                        project = %self.session.project.slug,
                        todo = %self.session.items[index].record.slug,
                        branch,
                        head = %head,
                        "the work branch is on the remote"
                    );
                    self.session.amend(index, |r| {
                        r.published_head = Some(head.clone());
                        r.publication_outstanding = None;
                    })?;
                }
                super::_publication::Pushed::Diverged => {
                    let why = format!(
                        "branch `{branch}` on `{remote}` moved where this computer's branch cannot follow it, so its commits are not published; the branch on `{remote}` is left as it is"
                    );
                    if self.session.items[index]
                        .reflection
                        .publication_outstanding
                        .as_deref()
                        == Some(why.as_str())
                    {
                        tracing::debug!(event = "work_branch_diverged", project = %self.session.project.slug, todo = %self.session.items[index].record.slug, branch, "the remote work branch is still moved");
                        continue;
                    }
                    tracing::warn!(
                        event = "work_branch_diverged",
                        project = %self.session.project.slug,
                        todo = %self.session.items[index].record.slug,
                        branch,
                        "the remote work branch moved; publication is left to the developer"
                    );
                    self.session
                        .amend(index, |r| r.publication_outstanding = Some(why.clone()))?;
                }
                super::_publication::Pushed::Unreached(reason) => {
                    let why = format!(
                        "branch `{branch}` could not reach `{remote}` ({reason}); the next tick publishes it"
                    );
                    if self.session.items[index]
                        .reflection
                        .publication_outstanding
                        .as_deref()
                        == Some(why.as_str())
                    {
                        tracing::debug!(event = "work_branch_unpublished", project = %self.session.project.slug, todo = %self.session.items[index].record.slug, branch, reason, "the work branch still cannot reach its remote");
                        continue;
                    }
                    tracing::warn!(
                        event = "work_branch_unpublished",
                        project = %self.session.project.slug,
                        todo = %self.session.items[index].record.slug,
                        branch,
                        reason,
                        "the work branch could not reach its remote; the next tick tries again"
                    );
                    self.session
                        .amend(index, |r| r.publication_outstanding = Some(why.clone()))?;
                }
            }
        }
        Ok(())
    }

    /// Records that GitHub stopped answering; a rate limit is remembered
    /// across ticks until the provider permits requests again.
    pub(super) fn note(&mut self, err: GitHubError) {
        if let GitHubError::RateLimited { until: Some(until) }
        | GitHubError::App(github::AppError::RateLimited { until: Some(until) }) = &err
        {
            let saved = self.session.store.provider().and_then(|mut state| {
                state.deferred_until = Some(*until);
                self.session.store.save_provider(&state)
            });
            if let Err(save) = saved {
                tracing::warn!(event = "work_rate_limit_unsaved", error = %save, "cannot remember GitHub's rate limit");
            }
        }
        if err.is_reach() {
            tracing::warn!(
                event = "work_github_unavailable",
                project = %self.session.project.slug,
                error = %err,
                "GitHub is not answering; the local record continues"
            );
            if let Some(Remote::Client { down, .. }) = self.remote.as_mut() {
                *down = Some(err);
            }
        } else {
            tracing::warn!(
                event = "work_github_read_failed",
                project = %self.session.project.slug,
                error = %err,
                "a GitHub read failed"
            );
        }
    }
}

/// Why GitHub cannot be reached for `work` at `now`, read from this
/// computer alone: a rate limit GitHub set, or App credentials that cannot
/// be used. `None` when a client can be built.
fn unavailable(work: &Work, provider: &ProviderState, now: Timestamp) -> Option<PendingReason> {
    match provider.deferred_until {
        Some(until) if until > now => Some(PendingReason::RateLimited { until: Some(until) }),
        _ => work
            .github
            .validate()
            .err()
            .map(|err| reason_of(&GitHubError::from(err))),
    }
}

fn reason_of(err: &GitHubError) -> PendingReason {
    match err {
        GitHubError::Unreachable(_) => PendingReason::Unreachable,
        GitHubError::App(github::AppError::Unreachable) => PendingReason::Unreachable,
        GitHubError::App(github::AppError::RateLimited { until }) => {
            PendingReason::RateLimited { until: *until }
        }
        GitHubError::RateLimited { until } => PendingReason::RateLimited { until: *until },
        GitHubError::Unauthorized { .. } => PendingReason::Unauthorized,
        other => PendingReason::Failed(other.to_string()),
    }
}
