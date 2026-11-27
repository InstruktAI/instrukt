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
mod _rendering;
mod _store;

use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::config::Configuration;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use jiff::Timestamp;
use serde::Serialize;

pub use _admission::{Coordinate, Coordinates, FormRefusal, Kind, RawCoordinates, SourcePointer};

use crate::interfaces::{HeadVerdicts, ProjectRef};
use crate::keyed::{self, KeyGuard, RuntimeInvocation};
use crate::runtime::{Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, Thread};
use crate::sessions::{SessionState, SessionsError};
use crate::state::RunnerState;
use _store::{Ask, Delivery};

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
    Procedure,
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
    /// The same ask was already outstanding: this one started or sent
    /// nothing new.
    pub repeated: bool,
    pub report: PathBuf,
    /// Where the verdict shows once the reviewer records it.
    pub show: String,
}

/// Asks for the independent review of the head `coordinates` name.
pub fn ask(context: &AskContext<'_>, coordinates: Coordinates) -> Result<Reply, AskError> {
    let kind = coordinates.kind()?;
    let slug = &context.project.slug;
    let head = &coordinates.head;
    _admission::procedure(context.home)?;
    let runtime = _admission::runtime(context.config)?;
    let adapter = crate::runtime::adapter(runtime);
    let checkout = registered_checkout(context.home, slug)?;
    _admission::placement(&checkout, &coordinates)?;
    _admission::cleanliness(&coordinates.worktree)?;
    let verdict = context
        .verdicts
        .head_verdict(context.project, head)
        .map_err(AskError::Failed)?;
    _admission::range(&coordinates, verdict.pinned_base.as_ref())?;
    let state = RunnerState::new(context.home);
    let requests = crate::review::requests(&state, slug, head).map_err(failed)?;

    let rendering = _rendering::render(slug, &coordinates);
    let key = keyed::review_key(slug.as_str(), head.as_str());
    let invocation = RuntimeInvocation::acquire(context.home, DISPATCH_WAIT).map_err(failed)?;
    let guard = KeyGuard::claim(
        &invocation,
        adapter.as_ref(),
        &key,
        Some(slug.as_str()),
        context.now,
    )
    .map_err(|error| match error {
        SessionsError::Refused(_) => refused(
            Check::Outstanding,
            format!(
                "another ask of {head} is reaching its requested reviewer right now ({error}); run the same command again once it returns"
            ),
        ),
        error => failed(error),
    })?;
    let delivery = Delivering {
        context,
        adapter: adapter.as_ref(),
        invocation: &invocation,
        guard: &guard,
        coordinates: &coordinates,
        kind,
        key: &key,
        checkout: &checkout,
        rendering: &rendering,
    };
    let reply = delivery.run(&state, &verdict.standing, &requests)?;
    drop(guard);
    retire(context, slug);
    Ok(reply)
}

/// The assignment of the requested reviewer started under `start_key`, from
/// the latest ask recorded under it, accepted or not: role
/// `commit-reviewer`, the project and the registered checkout it was
/// started in (`roles-and-capabilities`, Caller and object resolution).
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

/// The project's registered checkout, where the requested reviewer starts.
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
    Send {
        session: SessionRef,
        generation: u64,
    },
}

/// One admitted ask under its key.
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
}

impl Delivering<'_> {
    fn slug(&self) -> &ProjectSlug {
        &self.context.project.slug
    }

    fn head(&self) -> &CommitSha {
        &self.coordinates.head
    }

    /// Under the key: returns the same ask while it is outstanding; admits
    /// any other ask under Review state, Request and Outstanding; decides
    /// the route, records the ask, reaches the runtime and records the
    /// acceptance.
    fn run(
        &self,
        state: &RunnerState,
        standing: &crate::interfaces::VerdictStanding,
        requests: &[crate::review::Request],
    ) -> Result<Reply, AskError> {
        let runtime = self.adapter.runtime();
        let bound = keyed::bound(state, runtime.name(), self.key).map_err(failed)?;
        let thread = bound
            .as_ref()
            .map(|bound| self.adapter.read_state(&bound.session))
            .transpose()
            .map_err(failed)?;
        let present = thread
            .as_ref()
            .filter(|thread| thread.presence == Presence::Present);
        let reviewer_turn = present.map(turn).transpose()?;
        let asks = _store::asks(self.context.home, self.slug(), self.head()).map_err(failed)?;
        let outstanding = asks
            .last()
            .filter(|ask| ask.accepted_at.is_none() || reviewer_turn == Some(Turn::InProgress));
        // The same ask while outstanding returns it, whatever its reviewer
        // recorded meanwhile within the turn it opened (Outstanding).
        if let Some(ask) = outstanding.filter(|ask| ask.digest == self.rendering.digest) {
            return self.repeat(ask, bound.as_ref().map(|bound| &bound.session));
        }
        _admission::review_state(self.kind, self.head(), standing)?;
        _admission::request(self.head(), requests)?;
        if let Some(ask) = outstanding {
            return self.repeat(ask, bound.as_ref().map(|bound| &bound.session));
        }
        if let (Some(Turn::InProgress), Some(thread)) = (reviewer_turn, present) {
            return Err(refused(
                Check::Outstanding,
                format!(
                    "the requested reviewer {} of {} has a turn in progress; ask again once it ends",
                    thread.session,
                    self.head()
                ),
            ));
        }
        if self.kind == Kind::Dispute {
            let reviewer = bound.as_ref().map(|bound| &bound.session);
            _admission::dispute_recorder(self.head(), standing, reviewer)?;
            if let (Some(reviewer), None) = (reviewer, present) {
                return Err(refused(
                    Check::ReviewState,
                    format!(
                        "the requested reviewer {reviewer} that recorded {}'s verdict was archived or deleted, and a successor did not record it; a revised head is the way forward",
                        self.head()
                    ),
                ));
            }
        }
        let route = match (&bound, present) {
            (None, _) => Route::Start,
            (Some(bound), Some(_)) => Route::Send {
                session: bound.session.clone(),
                generation: bound.generation,
            },
            (Some(_), None) => Route::Successor,
        };
        let (delivery, send_generation) = match &route {
            Route::Start | Route::Successor => (Delivery::Start, None),
            Route::Send { generation, .. } => (Delivery::Send, Some(*generation)),
        };
        let named = serde_json::to_string(&self.coordinates.named()).map_err(failed)?;
        let ask = _store::record(
            self.context.home,
            &_store::NewAsk {
                project: self.slug(),
                head: self.head(),
                kind: self.kind.as_str(),
                coordinates: &named,
                rendering: &self.rendering.text,
                digest: &self.rendering.digest,
                start_key: self.key,
                checkout: self.checkout,
                delivery,
                send_generation,
                admitted_at: self.context.now,
            },
        )
        .map_err(failed)?;
        tracing::info!(event = "requested_review_admitted", project = %self.slug(), head = %self.head(), ask = ask.number, kind = self.kind.as_str());
        let session = match route {
            Route::Start => self.start(false)?,
            Route::Successor => self.start(true)?,
            Route::Send { session, .. } => {
                self.send(&ask, &session)?;
                session
            }
        };
        self.accepted(&ask, &session, false)
    }

    /// The same ask while it is outstanding returns it, replaying its
    /// delivery while the runtime's acceptance is not recorded; a different
    /// ask is refused.
    fn repeat(&self, ask: &Ask, bound: Option<&SessionRef>) -> Result<Reply, AskError> {
        if ask.digest != self.rendering.digest {
            return Err(refused(
                Check::Outstanding,
                format!(
                    "ask {} of {} ({}) is outstanding with its requested reviewer {}; ask again once the turn it opened ends",
                    ask.number,
                    self.head(),
                    ask.kind,
                    bound.map_or_else(|| "(not started yet)".to_owned(), ToString::to_string)
                ),
            ));
        }
        if ask.accepted_at.is_some() {
            let session = bound.cloned().ok_or_else(|| {
                failed(format!(
                    "ask {} of {} was accepted but its start key `{}` binds no session",
                    ask.number,
                    self.head(),
                    self.key
                ))
            })?;
            tracing::info!(event = "requested_review_repeated", project = %self.slug(), head = %self.head(), ask = ask.number);
            return Ok(self.reply(ask, session, true));
        }
        tracing::info!(event = "requested_review_replayed", project = %self.slug(), head = %self.head(), ask = ask.number);
        let session = match (ask.delivery, &ask.command_id, bound) {
            (Delivery::Send, Some(_), Some(session)) => {
                self.send(ask, session)?;
                session.clone()
            }
            (Delivery::Send, _, _) => {
                return Err(failed(format!(
                    "ask {} of {} is a send without its recorded command identity or target session",
                    ask.number,
                    self.head()
                )));
            }
            (Delivery::Start, _, _) => self.start_with_brief(false, &ask.rendering)?,
        };
        self.accepted(ask, &session, true)
    }

    /// The keyed start of the requested reviewer with the rendering as its
    /// opening input; `successor` succeeds an archived or deleted generation.
    fn start(&self, successor: bool) -> Result<SessionRef, AskError> {
        self.start_with_brief(successor, &self.rendering.text)
    }

    fn start_with_brief(&self, successor: bool, brief: &str) -> Result<SessionRef, AskError> {
        let context = self.context;
        let request = self.request(brief)?;
        let thread = if successor {
            keyed::succeed_owned(
                self.guard,
                self.invocation,
                self.adapter,
                &request,
                context.now,
            )
        } else {
            let assignment = RuntimeAssignment {
                bindings: None,
                invocation: self.invocation,
                project: Some(self.slug().as_str()),
                request: &request,
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

    /// The start request: the same construction a standing start uses, in
    /// the registered checkout, under the review procedure's role.
    fn request(&self, brief: &str) -> Result<RuntimeStart, AskError> {
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
        Ok(RuntimeStart {
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
        })
    }

    /// Sends the ask's stored rendering under its recorded command identity;
    /// the runtime accepts one identity once.
    fn send(&self, ask: &Ask, session: &SessionRef) -> Result<(), AskError> {
        let command = ask.command_id.as_deref().ok_or_else(|| {
            failed(format!(
                "ask {} of {} is a send without its recorded command identity",
                ask.number,
                self.head()
            ))
        })?;
        self.adapter
            .dispatch_send(session, command, &ask.rendering, &mut || Ok(()))
            .map_err(|error| self.not_accepted("send", &error.into_inner()))
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

    /// Records the acceptance at the generation the key reached.
    fn accepted(&self, ask: &Ask, session: &SessionRef, replayed: bool) -> Result<Reply, AskError> {
        let context = self.context;
        let state = RunnerState::new(context.home);
        let generation = match ask.delivery {
            Delivery::Send => ask.generation,
            Delivery::Start => keyed::bound(&state, self.adapter.runtime().name(), self.key)
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
        tracing::info!(event = "requested_review_accepted", project = %self.slug(), head = %self.head(), ask = ask.number, session = %session, replayed);
        Ok(self.reply(ask, session.clone(), replayed))
    }

    fn reply(&self, ask: &Ask, reviewer: SessionRef, repeated: bool) -> Reply {
        let head = self.head().clone();
        Reply {
            show: format!("instrukt review show {head}"),
            base: self.coordinates.base.clone(),
            kind: self.kind.as_str(),
            reviewer,
            start_key: self.key.to_owned(),
            reached: match ask.delivery {
                Delivery::Start => Reached::Started,
                Delivery::Send => Reached::Sent,
            },
            repeated,
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
